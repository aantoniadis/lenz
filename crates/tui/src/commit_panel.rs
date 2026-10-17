//! Focus's commit message panel below Files and the diff; the repository session owns what it shows.
use crate::chrome::{color, style_with};
use crossterm::event::{Event, MouseButton, MouseEventKind};
use lenz::commit_message::{Line, LineKind};
use lenz::repository_session::{MessagePanel, RepositoryCommand, RepositorySession};
use ratatui::{Frame, layout::Rect, widgets::{Block, Borders}};

#[derive(Default)]
pub(crate) struct CommitPanel {
    pub(crate) area: Rect, lines: Vec<Line>, scroll: usize, title: String,
    /// The panel has the keyboard; `current` is the line kind it highlights, `cursor` the commit last revealed.
    pub(crate) focused: bool, current: Option<LineKind>, cursor: usize,
}

impl CommitPanel {
    /// Take the panel's rows off the bottom of the combined Files and diff area; `lines` is `None` outside Focus or without a message.
    /// `current` is the keyboard's line (a range's commit, or the first line of one commit) and `cursor` its commit.
    pub(crate) fn place(&mut self, files: &mut Rect, lines: Option<Vec<Line>>, title: String, current: LineKind, cursor: usize) {
        self.lines = lines.unwrap_or_default();
        self.title = title;
        self.current = Some(current);
        if self.lines.is_empty() || files.width < 3 || files.height < 8 { self.area = Rect::default(); self.focused = false; return; }
        let h = u16::try_from(self.lines.len()).unwrap_or(u16::MAX).saturating_add(2).min(files.height / 3).max(3);
        self.area = Rect::new(files.x, files.bottom() - h, files.width, h);
        files.height -= h;
        // the keyboard's commit moved: bring it into view
        if std::mem::replace(&mut self.cursor, cursor) != cursor && let Some(row) = self.lines.iter().position(|l| l.kind == current) {
            if row < self.scroll { self.scroll = row; } else if row >= self.scroll + self.rows() { self.scroll = row + 1 - self.rows(); }
        }
        self.scroll = self.scroll.min(self.lines.len().saturating_sub(self.rows()));
    }
    /// Scroll a single commit's message by a row; `false` when ↓ is already at the end (the keyboard goes to Files).
    pub(crate) fn scroll_by(&mut self, down: bool) -> bool {
        let max = self.lines.len().saturating_sub(self.rows());
        if down && self.scroll >= max || !down && self.scroll == 0 { return false; }
        self.scroll = if down { self.scroll + 1 } else { self.scroll.saturating_sub(1) };
        true
    }
    /// The width the panel's lines wrap to.
    pub(crate) fn columns(files: Rect) -> usize { usize::from(files.width.saturating_sub(2)).max(1) }
    fn rows(&self) -> usize { usize::from(self.area.height.saturating_sub(2)) }
    fn line_at(&self, column: u16, row: u16) -> Option<(usize, &Line)> {
        let inner = Rect::new(self.area.x + 1, self.area.y + 1, self.area.width.saturating_sub(2), self.area.height.saturating_sub(2));
        if !inner.contains((column, row).into()) { return None; }
        let at = self.scroll + usize::from(row - inner.y);
        self.lines.get(at).map(|l| (at, l))
    }
    /// A click or the wheel on the panel: the command a click runs, and whether the panel took the event.
    pub(crate) fn handle(&mut self, event: &Event) -> (bool, Option<RepositoryCommand>) {
        let Event::Mouse(m) = event else { return (false, None) };
        if !self.area.contains((m.column, m.row).into()) {
            if m.kind == MouseEventKind::Down(MouseButton::Left) { self.focused = false; }
            return (false, None);
        }
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) => (true, match self.line_at(m.column, m.row).map(|(_, l)| l.kind) {
                Some(LineKind::Toggle) => Some(RepositoryCommand::ToggleMessage(None)),
                Some(LineKind::Commit(i)) => Some(RepositoryCommand::ToggleMessage(Some(i))),
                _ => None,
            }),
            MouseEventKind::ScrollDown => { self.scroll = (self.scroll + 3).min(self.lines.len().saturating_sub(self.rows())); (true, None) }
            MouseEventKind::ScrollUp => { self.scroll = self.scroll.saturating_sub(3); (true, None) }
            // a right click opens the copy menu
            _ => (false, None),
        }
    }
    /// The commit under the pointer, for the right-click menu: its message and hash.
    pub(crate) fn commit_at(&self, column: u16, row: u16, core: &RepositorySession) -> Option<(String, String)> {
        let (at, _) = self.line_at(column, row)?;
        let index = self.lines[..=at].iter().rev().find_map(|l| match l.kind { LineKind::Commit(i) => Some(i), _ => None }).unwrap_or(0);
        let Some(MessagePanel::Ready(m)) = core.message_panel() else { return None };
        m.commits.get(index).map(|c| (c.message.trim_end().to_string(), c.sha.clone()))
    }
    pub(crate) fn draw(&self, frame: &mut Frame, palette: &lenz::theme::Palette) {
        if self.area.height == 0 { return; }
        let base = style_with(palette).bg(color(palette.sidebar_bg));
        frame.render_widget(Block::default().borders(Borders::ALL).title(crate::repository::safe_label(&self.title)).style(base), self.area);
        for (row, line) in self.lines.iter().skip(self.scroll).take(self.rows()).enumerate() {
            let r = Rect::new(self.area.x + 1, self.area.y + 1 + row as u16, self.area.width.saturating_sub(2), 1);
            let style = if line.kind == LineKind::Meta { base.fg(color(palette.dim)) } else { base };
            let style = if self.focused && Some(line.kind) == self.current { style.bg(color(palette.selected_bg)).add_modifier(ratatui::style::Modifier::BOLD) } else { style };
            frame.buffer_mut().set_style(r, style);
            crate::visual::safe_text(frame.buffer_mut(), r, &line.text, style);
        }
    }
}

impl crate::app::App {
    /// Keys for the commit message panel: ↓ on Files' last file enters it; there ↑/↓ walk a range's commits (or scroll
    /// one commit's message), Enter opens the commit, Esc or ↓ past the end give the keyboard back to Files.
    pub(crate) fn commit_panel_key(&mut self, event: &Event) -> bool {
        use crossterm::event::{KeyCode, KeyEventKind, KeyModifiers};
        use lenz::repository_session::MessageKey as K;
        let Event::Key(k) = event else { return false };
        if k.kind == KeyEventKind::Release || !k.modifiers.is_empty() && k.modifiers != KeyModifiers::SHIFT || self.commit_panel.area.height == 0 { return false; }
        if self.navigator.focused {
            // Files' last file
            if k.code == KeyCode::Down && !self.navigator.search.active() && self.navigator.on_last_file() {
                (self.navigator.focused, self.commit_panel.focused) = (false, true);
                return true;
            }
            self.commit_panel.focused = false;
            return false;
        }
        if !self.commit_panel.focused { return false; }
        let key = match k.code {
            KeyCode::Up => K::Up,
            KeyCode::Down => K::Down,
            KeyCode::Enter | KeyCode::Char(' ') => K::Enter,
            KeyCode::Esc => { (self.navigator.focused, self.commit_panel.focused) = (self.navigator.shown(), false); return true; }
            _ => return false,
        };
        match self.repository.as_ref().and_then(|r| r.core.message_key(key)) {
            Some(command) => self.repository_command(command),
            None if self.commit_panel.scroll_by(key == K::Down) => {}
            // ↓ past the end: back to Files
            None => (self.navigator.focused, self.commit_panel.focused) = (self.navigator.shown(), false),
        }
        true
    }
    /// Ctrl+C in the panel: the message of the commit the keyboard is on.
    pub(crate) fn copy_message(&mut self) {
        let Some(core) = self.repository.as_ref().map(|r| &r.core) else { return };
        let Some(MessagePanel::Ready(m)) = core.message_panel() else { return };
        if let Some(text) = m.commits.get(core.message_cursor()).map(|c| c.message.trim_end().to_string()) { self.copy(&text); }
    }
}
