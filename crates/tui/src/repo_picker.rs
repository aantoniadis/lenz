//! Terminal layout/input for choosing a repository when lenz-tui starts outside one. The shared core lists recent
//! repositories, completes paths and validates the choice.
use std::path::PathBuf;

use crate::chrome::{color, style_with};
use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind};
use lenz::repo_picker::{PickerCommand, RepoPicker};
use lenz::theme::Palette;
use ratatui::{Frame, layout::Rect};

pub enum Outcome { None, Quit, Chosen(PathBuf) }

pub struct View {
    pub core: RepoPicker,
    scroll: usize,
    /// Recent rows as last drawn: (index into matches, row).
    hits: Vec<(usize, Rect)>,
}

impl View {
    pub fn new(core: RepoPicker) -> Self { Self { core, scroll: 0, hits: Vec::new() } }

    fn run(&mut self, command: PickerCommand) -> Outcome {
        self.core.execute(command).map_or(Outcome::None, Outcome::Chosen)
    }

    pub fn handle(&mut self, event: &Event) -> Outcome {
        match event {
            Event::Key(k) if k.kind != KeyEventKind::Release => {
                if k.modifiers.contains(KeyModifiers::CONTROL) {
                    return if matches!(k.code, KeyCode::Char('c' | 'q')) { Outcome::Quit } else { Outcome::None };
                }
                match k.code {
                    KeyCode::Esc => Outcome::Quit,
                    KeyCode::Enter => self.run(PickerCommand::Submit),
                    KeyCode::Tab => self.run(PickerCommand::Complete),
                    KeyCode::Backspace => self.run(PickerCommand::Backspace),
                    KeyCode::Up => self.run(PickerCommand::Up),
                    KeyCode::Down => self.run(PickerCommand::Down),
                    KeyCode::Char(c) => self.run(PickerCommand::Insert(c)),
                    _ => Outcome::None,
                }
            }
            Event::Paste(text) => {
                for c in lenz::document::one_line(text).chars() { self.core.execute(PickerCommand::Insert(c)); }
                Outcome::None
            }
            Event::Mouse(m) if m.kind == MouseEventKind::Down(MouseButton::Left) => {
                let hit = self.hits.iter().find(|(_, r)| r.contains((m.column, m.row).into())).map(|(i, _)| *i);
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
        let width = area.width.saturating_sub(2);
        if width == 0 || area.height == 0 { return; }
        let line = |frame: &mut Frame, y: u16, text: &str, style| {
            if y < area.y + area.height {
                let r = Rect::new(area.x + 1, y, width, 1);
                frame.buffer_mut().set_style(r, style);
                crate::visual::safe_text(frame.buffer_mut(), r, text, style);
            }
        };
        let accent = base.fg(color(palette.accent));
        let mut y = area.y;
        line(frame, y, "Open a repository", accent);
        y += 1;
        line(frame, y, &format!("Not inside a Git repository: {}", self.core.cwd().display()), base);
        y += 2;
        line(frame, y, &crate::visual::input_line("Path: ", &self.core.input, usize::from(width)), base.bg(color(palette.status_bg)));
        y += 1;
        // an error wraps (up to three rows), so a long path isn't cut off
        let error = self.core.error.as_deref().unwrap_or("");
        let starts = lenz::wrap::wrap_starts(error, usize::from(width));
        let ends = starts.iter().copied().chain(std::iter::once(error.len()));
        for (start, end) in std::iter::once(0).chain(starts.iter().copied()).zip(ends).take(3) {
            line(frame, y, error.get(start..end).unwrap_or_default().trim_end(), base.fg(color(palette.error)));
            y += 1;
        }
        y += 1;
        let matches: Vec<String> = self.core.matches().iter().map(|p| p.display().to_string()).collect();
        line(frame, y, if matches.is_empty() { "No recent repositories" } else { "Recent repositories" }, accent);
        y += 1;
        // the last row keeps the key hints
        let visible = usize::from((area.y + area.height).saturating_sub(y + 1));
        let selected = self.core.selected.min(matches.len().saturating_sub(1));
        if selected < self.scroll { self.scroll = selected; }
        if visible > 0 && selected >= self.scroll + visible { self.scroll = selected + 1 - visible; }
        self.scroll = self.scroll.min(matches.len().saturating_sub(visible.max(1)));
        for (i, path) in matches.iter().enumerate().skip(self.scroll).take(visible) {
            let style = if i == selected { base.bg(color(palette.selected_bg)) } else { base };
            line(frame, y, path, style);
            self.hits.push((i, Rect::new(area.x + 1, y, width, 1)));
            y += 1;
        }
        line(frame, area.y + area.height - 1, "Type a path · Tab complete · ↑/↓ recent · Enter open · Esc quit", base.fg(color(palette.dim)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEvent, KeyEventState};

    fn key(code: KeyCode) -> Event { Event::Key(KeyEvent { code, modifiers: KeyModifiers::NONE, kind: KeyEventKind::Press, state: KeyEventState::NONE }) }
    fn git(dir: &std::path::Path) {
        assert!(std::process::Command::new("git").args(["init", "-q"]).current_dir(dir).status().unwrap().success());
    }
    fn screen(view: &mut View, w: u16, h: u16) -> String {
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(w, h)).unwrap();
        terminal.draw(|f| view.draw(f, &lenz::theme::DARCULA)).unwrap();
        terminal.backend().buffer().content.iter().map(|c| c.symbol()).collect::<Vec<_>>().chunks(usize::from(w)).map(|r| r.concat()).collect::<Vec<_>>().join("\n")
    }

    #[test]
    fn typing_a_path_opens_the_repository_and_errors_keep_the_picker() {
        let base = tempfile::tempdir().unwrap();
        std::fs::create_dir(base.path().join("project")).unwrap();
        git(&base.path().join("project"));
        std::fs::create_dir(base.path().join("plain")).unwrap();
        let mut view = View::new(RepoPicker::with_recent(base.path().into(), Vec::new()));
        for c in "pl".chars() { view.handle(&key(KeyCode::Char(c))); }
        view.handle(&key(KeyCode::Tab));
        assert!(matches!(view.handle(&key(KeyCode::Enter)), Outcome::None));
        assert!(screen(&mut view, 80, 12).contains("Not a Git repository"));
        // a narrow terminal wraps the error instead of cutting it off
        let narrow = screen(&mut view, 24, 12);
        let rows = narrow.lines().skip_while(|l| !l.contains("Not a Git")).take_while(|l| !l.trim().is_empty()).count();
        assert!(rows > 1, "{narrow}");
        for _ in 0.."plain/".len() { view.handle(&key(KeyCode::Backspace)); }
        view.handle(&Event::Paste("project".into()));
        match view.handle(&key(KeyCode::Enter)) {
            Outcome::Chosen(root) => assert_eq!(root.canonicalize().unwrap(), base.path().join("project").canonicalize().unwrap()),
            _ => panic!("repository not chosen"),
        }
        assert!(matches!(view.handle(&key(KeyCode::Esc)), Outcome::Quit));
    }

    #[test]
    fn recent_rows_render_scroll_in_tiny_terminals_and_open_on_click() {
        let dirs: Vec<_> = (0..6).map(|_| { let d = tempfile::tempdir().unwrap(); git(d.path()); d }).collect();
        let mut view = View::new(RepoPicker::with_recent("/".into(), dirs.iter().map(|d| d.path().into()).collect()));
        for _ in 0..10 { view.handle(&key(KeyCode::Down)); }
        // wide enough for a whole temp path (macOS's run ~60 columns); the height is what scrolls
        let text = screen(&mut view, 120, 10);
        assert!(text.contains(&dirs[5].path().display().to_string()), "selected last row stays visible:\n{text}");
        for (w, h) in [(1, 1), (10, 3), (40, 6)] { screen(&mut view, w, h); }
        screen(&mut view, 60, 10);
        let row = view.hits.iter().find(|(i, _)| *i == 5).unwrap().1;
        let click = Event::Mouse(crossterm::event::MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: row.x, row: row.y, modifiers: KeyModifiers::NONE });
        assert!(matches!(view.handle(&click), Outcome::Chosen(_)));
    }
}
