//! Terminal layout/input for choosing a repository when lenz-tui starts outside one. The shared core lists recent
//! repositories, completes paths and validates the choice.

use crate::chrome::{color, style_with};
use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind};
use lenz::line_edit::Edit;
use lenz::repo_picker::{Choice, Mode, PickerCommand, RepoPicker};
use lenz::theme::Palette;
use ratatui::{Frame, layout::Rect};

pub enum Outcome {
    None,
    Quit,
    Chosen(Choice),
}

pub struct View {
    pub core: RepoPicker,
    scroll: usize,
    /// Recent rows as last drawn: (index into matches, row).
    hits: Vec<(usize, Rect)>,
    modes: Vec<(Mode, Rect)>,
    fields: Vec<Rect>,
    submit: Rect,
}

impl View {
    pub fn new(core: RepoPicker) -> Self {
        Self {
            core,
            scroll: 0,
            hits: Vec::new(),
            modes: Vec::new(),
            fields: Vec::new(),
            submit: Rect::default(),
        }
    }

    fn run(&mut self, command: PickerCommand) -> Outcome {
        self.core
            .execute(command)
            .map_or(Outcome::None, Outcome::Chosen)
    }

    pub fn handle(&mut self, event: &Event) -> Outcome {
        match event {
            Event::Key(k) if k.kind != KeyEventKind::Release => {
                let control = k.modifiers.contains(KeyModifiers::CONTROL);
                let word = control || k.modifiers.contains(KeyModifiers::ALT);
                if control && k.code==KeyCode::Char('q') { return Outcome::Quit; }
                if let Some(command)=lenz_terminal_ui::command(*k) {
                    if let Some(text)=self.core.edit_field(command).copy {let _=crate::clipboard::copy(&text);}
                    return Outcome::None;
                }
                if control && matches!(k.code, KeyCode::Char('q')) {
                    return Outcome::Quit;
                }
                if control && k.code == KeyCode::Char('a') {
                    return self.run(PickerCommand::SelectAll);
                }
                if control && k.code == KeyCode::Char(' ') {
                    return self.run(PickerCommand::Complete);
                }
                if self.core.comparisons
                    && let KeyCode::F(n @ 1..=3) = k.code
                {
                    return self.run(PickerCommand::Mode(Mode::ALL[n as usize - 1]));
                }
                let edit = match k.code {
                    KeyCode::Left => Some(if word { Edit::WordLeft } else { Edit::Left }),
                    KeyCode::Right => Some(if word { Edit::WordRight } else { Edit::Right }),
                    KeyCode::Home => Some(Edit::Start),
                    KeyCode::End => Some(Edit::End),
                    KeyCode::Backspace => Some(if word {
                        Edit::BackspaceWord
                    } else {
                        Edit::Backspace
                    }),
                    KeyCode::Delete => Some(if word { Edit::DeleteWord } else { Edit::Delete }),
                    _ => None,
                };
                if let Some(edit) = edit {
                    return self.run(PickerCommand::Edit(edit));
                }
                match k.code {
                    KeyCode::Esc => Outcome::Quit,
                    KeyCode::Enter => self.run(PickerCommand::Submit),
                    KeyCode::Tab => self.run(if self.core.mode == Mode::Repository {
                        PickerCommand::Complete
                    } else {
                        PickerCommand::NextField
                    }),
                    KeyCode::BackTab => self.run(PickerCommand::NextField),
                    KeyCode::Up => self.run(PickerCommand::Up),
                    KeyCode::Down => self.run(PickerCommand::Down),
                    KeyCode::Char(c) if !control => self.run(PickerCommand::Insert(c)),
                    _ => Outcome::None,
                }
            }
            Event::Paste(text) => {
                self.core.execute(PickerCommand::InsertText(text.clone()));
                Outcome::None
            }
            Event::Mouse(m) if m.kind == MouseEventKind::Down(MouseButton::Left) => {
                let at = (m.column, m.row);
                if self.submit.contains(at.into()) {
                    return self.run(PickerCommand::Submit);
                }
                if let Some((mode, _)) = self.modes.iter().find(|(_, r)| r.contains(at.into())) {
                    return self.run(PickerCommand::Mode(*mode));
                }
                if let Some(field) = self.fields.iter().position(|r| r.contains(at.into())) {
                    return self.run(PickerCommand::Field(field));
                }
                let hit = self
                    .hits
                    .iter()
                    .find(|(_, r)| r.contains((m.column, m.row).into()))
                    .map(|(i, _)| *i);
                match hit.and_then(|i| self.core.matches().get(i).map(|p| p.to_path_buf())) {
                    Some(path) => self.run(PickerCommand::Choose(path)),
                    None => Outcome::None,
                }
            }
            _ => Outcome::None,
        }
    }

    pub fn draw(&mut self, frame: &mut Frame, palette: &Palette) {
        let area = frame.area();
        let base = style_with(palette);
        frame.buffer_mut().set_style(area, base);
        self.hits.clear();
        self.fields.clear();
        self.modes.clear();
        let width = area.width.saturating_sub(2);
        if width == 0 || area.height == 0 {
            return;
        }
        let line = |frame: &mut Frame, y: u16, text: &str, style| {
            if y < area.y + area.height {
                let r = Rect::new(area.x + 1, y, width, 1);
                frame.buffer_mut().set_style(r, style);
                crate::visual::safe_text(frame.buffer_mut(), r, text, style);
            }
        };
        let accent = base.fg(color(palette.accent));
        let mut y = area.y;
        line(
            frame,
            y,
            if self.core.comparisons {
                "New comparison"
            } else {
                "Open a repository"
            },
            accent,
        );
        y += 1;
        line(
            frame,
            y,
            &format!("Paths relative to {}", self.core.cwd().display()),
            base,
        );
        y += 1;
        if self.core.comparisons {
            for (i, mode) in Mode::ALL.into_iter().enumerate() {
                let selected = mode == self.core.mode;
                line(
                    frame,
                    y,
                    &format!(
                        "{} {} (F{})",
                        if selected { "●" } else { "○" },
                        mode.label(),
                        i + 1
                    ),
                    if selected {
                        accent.bg(color(palette.selected_bg))
                    } else {
                        accent
                    },
                );
                if y < area.bottom() {
                    self.modes.push((mode, Rect::new(area.x + 1, y, width, 1)));
                }
                y += 1;
            }
        }
        y += 1;
        for i in 0..self.core.fields() {
            let input = if i == 0 {
                &self.core.input
            } else {
                &self.core.right
            };
            let label = self.core.mode.field(i);
            line(frame,y,&format!("{label}: "),base);
            if y<area.bottom(){
                self.fields.push(Rect::new(area.x+1,y,width,1));
                let offset=(label.len() as u16+2).min(width);
                let field=Rect::new(area.x+1+offset,y,width.saturating_sub(offset),1);
                lenz_terminal_ui::paint(frame,lenz_terminal_ui::Field{text:input.text(),cursor:input.cursor(),area:field,active:self.core.field==i},base.bg(color(palette.status_bg)),base.bg(color(palette.selected_bg)));
            }
            y += 1;
        }
        // an error wraps (up to three rows), so a long path isn't cut off
        let error = self.core.error.as_deref().unwrap_or("");
        let starts = lenz::wrap::wrap_starts(error, usize::from(width));
        let ends = starts.iter().copied().chain(std::iter::once(error.len()));
        for (start, end) in std::iter::once(0)
            .chain(starts.iter().copied())
            .zip(ends)
            .take(3)
        {
            line(
                frame,
                y,
                error.get(start..end).unwrap_or_default().trim_end(),
                base.fg(color(palette.error)),
            );
            y += 1;
        }
        y += 1;
        let matches: Vec<String> = self
            .core
            .matches()
            .iter()
            .map(|p| p.display().to_string())
            .collect();
        line(
            frame,
            y,
            if self.core.mode != Mode::Repository {
                ""
            } else if matches.is_empty() {
                "No recent repositories"
            } else {
                "Recent repositories"
            },
            accent,
        );
        y += 1;
        // the last row keeps the key hints
        let visible = usize::from((area.y + area.height).saturating_sub(y + 1));
        let selected = self.core.selected.min(matches.len().saturating_sub(1));
        if selected < self.scroll {
            self.scroll = selected;
        }
        if visible > 0 && selected >= self.scroll + visible {
            self.scroll = selected + 1 - visible;
        }
        self.scroll = self
            .scroll
            .min(matches.len().saturating_sub(visible.max(1)));
        for (i, path) in matches.iter().enumerate().skip(self.scroll).take(visible) {
            let style = if i == selected {
                base.bg(color(palette.selected_bg))
            } else {
                base
            };
            line(frame, y, path, style);
            self.hits.push((i, Rect::new(area.x + 1, y, width, 1)));
            y += 1;
        }
        let label = format!(
            "[{}: Enter]  Tab {} · Ctrl+A select · Esc cancel",
            self.core.mode.submit(),
            if self.core.mode == Mode::Repository {
                "complete"
            } else {
                "next path"
            }
        );
        line(frame, area.bottom() - 1, &label, accent);
        self.submit = Rect::new(
            area.x + 1,
            area.bottom() - 1,
            (self.core.mode.submit().len() as u16 + 9).min(width),
            1,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEvent, KeyEventState};

    fn key(code: KeyCode) -> Event {
        Event::Key(KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        })
    }
    fn git(dir: &std::path::Path) {
        assert!(
            std::process::Command::new("git")
                .args(["init", "-q"])
                .current_dir(dir)
                .status()
                .unwrap()
                .success()
        );
    }
    fn screen(view: &mut View, w: u16, h: u16) -> String {
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(w, h)).unwrap();
        terminal
            .draw(|f| view.draw(f, &lenz::theme::DARCULA))
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<Vec<_>>()
            .chunks(usize::from(w))
            .map(|r| r.concat())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn typing_a_path_opens_the_repository_and_errors_keep_the_picker() {
        let base = tempfile::tempdir().unwrap();
        std::fs::create_dir(base.path().join("project")).unwrap();
        git(&base.path().join("project"));
        std::fs::create_dir(base.path().join("plain")).unwrap();
        let mut view = View::new(RepoPicker::with_recent(base.path().into(), Vec::new()));
        for c in "pl".chars() {
            view.handle(&key(KeyCode::Char(c)));
        }
        view.handle(&key(KeyCode::Tab));
        assert!(matches!(view.handle(&key(KeyCode::Enter)), Outcome::None));
        assert!(screen(&mut view, 80, 12).contains("Not a Git repository"));
        // a narrow terminal wraps the error instead of cutting it off
        let narrow = screen(&mut view, 24, 12);
        let rows = narrow
            .lines()
            .skip_while(|l| !l.contains("Not a Git"))
            .take_while(|l| !l.trim().is_empty())
            .count();
        assert!(rows > 1, "{narrow}");
        for _ in 0.."plain/".len() {
            view.handle(&key(KeyCode::Backspace));
        }
        view.handle(&Event::Paste("project".into()));
        match view.handle(&key(KeyCode::Enter)) {
            Outcome::Chosen(Choice::Repository(root)) => assert_eq!(
                root.canonicalize().unwrap(),
                base.path().join("project").canonicalize().unwrap()
            ),
            _ => panic!("repository not chosen"),
        }
        assert!(matches!(view.handle(&key(KeyCode::Esc)), Outcome::Quit));
    }

    #[test]
    fn recent_rows_render_scroll_in_tiny_terminals_and_open_on_click() {
        let dirs: Vec<_> = (0..6)
            .map(|_| {
                let d = tempfile::tempdir().unwrap();
                git(d.path());
                d
            })
            .collect();
        let mut view = View::new(RepoPicker::with_recent(
            "/".into(),
            dirs.iter().map(|d| d.path().into()).collect(),
        ));
        for _ in 0..10 {
            view.handle(&key(KeyCode::Down));
        }
        // wide enough for a whole temp path (macOS's run ~60 columns); the height is what scrolls
        let text = screen(&mut view, 120, 10);
        assert!(
            text.contains(&dirs[5].path().display().to_string()),
            "selected last row stays visible:\n{text}"
        );
        for (w, h) in [(1, 1), (10, 3), (40, 6)] {
            screen(&mut view, w, h);
        }
        screen(&mut view, 60, 10);
        let row = view.hits.iter().find(|(i, _)| *i == 5).unwrap().1;
        let click = Event::Mouse(crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: row.x,
            row: row.y,
            modifiers: KeyModifiers::NONE,
        });
        assert!(matches!(view.handle(&click), Outcome::Chosen(_)));
    }
}
