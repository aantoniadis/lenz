//! The Folder Summary in the diff area: a folder's changed files and their added and removed lines, from the shared
//! `lenz::folders` state. This adapter keeps only its scroll position and the area it drew.
use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind};
use lenz::{changeset::ChangeSet, folders::{Folders, Summary, Tone, stat_parts}, navigator};
use ratatui::{Frame, layout::Rect, style::Modifier};
use crate::chrome::{color, style_with};

/// What a summary event asks the app to do; the shared session does it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Select(isize),
    /// A click on the file at this summary row.
    OpenRow(usize),
    Open, Close, Collapse, Expand,
    /// The pointer over the summary, with nothing to do.
    Taken,
}

#[derive(Default)]
pub struct SummaryPane {
    scroll: usize, area: Rect,
    /// The wheel scrolled away from the keyboard row: leave the view where it is.
    free: bool,
}

/// Rows above the files: the heading and a gap.
const TOP: u16 = 2;

impl SummaryPane {
    fn rows(&self) -> usize { usize::from(self.area.height.saturating_sub(TOP + 1)) }

    /// Keys and the pointer while the summary shows; others pass through (`None`) to the app.
    pub fn handle(&mut self, event: &Event, summary: &Summary) -> Option<Outcome> {
        match event {
            Event::Key(k) if k.kind != KeyEventKind::Release && k.modifiers.difference(KeyModifiers::SHIFT).is_empty() => {
                self.free = false;
                let page = self.rows().max(1) as isize;
                Some(match k.code {
                    KeyCode::Down | KeyCode::Char('j') => Outcome::Select(1),
                    KeyCode::Up | KeyCode::Char('k') => Outcome::Select(-1),
                    KeyCode::PageDown | KeyCode::Char(' ') => Outcome::Select(page),
                    KeyCode::PageUp => Outcome::Select(-page),
                    KeyCode::Home => Outcome::Select(isize::MIN / 2),
                    KeyCode::End => Outcome::Select(isize::MAX / 2),
                    KeyCode::Enter => Outcome::Open,
                    KeyCode::Esc => Outcome::Close,
                    KeyCode::Left => Outcome::Collapse,
                    KeyCode::Right => Outcome::Expand,
                    _ => return None,
                })
            }
            Event::Mouse(m) if self.area.contains((m.column, m.row).into()) => Some(match m.kind {
                MouseEventKind::ScrollDown => { self.free = true; self.scroll = (self.scroll + 3).min(summary.files.len().saturating_sub(self.rows())); Outcome::Taken }
                MouseEventKind::ScrollUp => { self.free = true; self.scroll = self.scroll.saturating_sub(3); Outcome::Taken }
                // a click on a file opens it; elsewhere it only takes the pointer
                MouseEventKind::Down(MouseButton::Left) => m.row.checked_sub(self.area.y + TOP).map(|r| self.scroll + usize::from(r))
                    .filter(|&r| r < summary.files.len() && m.row < self.area.bottom().saturating_sub(1)).map_or(Outcome::Taken, Outcome::OpenRow),
                _ => Outcome::Taken,
            }),
            _ => None,
        }
    }

    pub fn draw(&mut self, frame: &mut Frame, area: Rect, set: &ChangeSet, folders: &Folders, palette: &lenz::theme::Palette) {
        self.area = area;
        let Some(summary) = folders.summary() else { return };
        let base = style_with(palette).bg(color(palette.bg));
        // over the comparison: none of its cells may show through
        frame.render_widget(ratatui::widgets::Clear, area);
        frame.buffer_mut().set_style(area, base);
        if area.height < TOP + 2 || area.width < 8 { return; }
        let tone = |t: Tone| base.fg(color(match t { Tone::Added => palette.status[0], Tone::Removed => palette.status[1], Tone::Dim => palette.dim }));
        let mut x = area.x + 1;
        for (i, (text, t)) in summary.heading(folders).into_iter().enumerate() {
            let style = if i == 0 { base.fg(color(palette.accent)).add_modifier(Modifier::BOLD) } else { tone(t) };
            x = frame.buffer_mut().set_stringn(x, area.y, &text, usize::from(area.right().saturating_sub(x)), style).0;
        }
        let rows = self.rows();
        // the keyboard row stays in view
        if self.free {} else if summary.selected < self.scroll { self.scroll = summary.selected; } else if summary.selected >= self.scroll + rows { self.scroll = summary.selected + 1 - rows; }
        self.scroll = self.scroll.min(summary.files.len().saturating_sub(rows));
        for (row, &index) in summary.files.iter().enumerate().skip(self.scroll).take(rows) {
            let y = area.y + TOP + (row - self.scroll) as u16;
            let c = &set.comparisons[index];
            let selected = row == summary.selected;
            let style = if selected { base.bg(color(palette.selected_bg)) } else { base };
            frame.buffer_mut().set_style(Rect::new(area.x, y, area.width, 1), style);
            let stat = stat_parts(folders.stat(index));
            let stat_w: usize = stat.iter().map(|(t, _)| t.chars().count() + 1).sum();
            let kind = match navigator::badge(&c.status) { "A" => 0, "D" => 1, "M" => 2, "R" => 3, _ => 4 };
            let name = match &c.status {
                lenz::changeset::Status::Renamed { .. } => format!("{} ← {}", summary.relative(&c.path), summary.relative(&c.old_path)),
                _ => summary.relative(&c.path).to_string(),
            };
            let room = usize::from(area.width).saturating_sub(stat_w + 5);
            let line = format!(" {} {}", navigator::badge(&c.status), name);
            frame.buffer_mut().set_stringn(area.x + 1, y, &line, room, style.fg(color(palette.status[kind])));
            let right = area.right().saturating_sub(1);
            let mut x = right.saturating_sub(stat_w as u16).max(area.x + 1);
            for (text, t) in stat {
                x = frame.buffer_mut().set_stringn(x, y, &text, usize::from(right.saturating_sub(x)), tone(t).bg(style.bg.unwrap_or_default())).0.saturating_add(1).min(right);
            }
        }
        let hint_y = area.bottom().saturating_sub(1);
        frame.buffer_mut().set_stringn(area.x + 1, hint_y, Summary::HINT, usize::from(area.width.saturating_sub(2)), base.fg(color(palette.dim)));
    }
}
