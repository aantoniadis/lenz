//! Focus's commit message panel, between the Files header and its list; the repository session owns what it shows.
use super::*;
use crate::commit_message::{Line, LineKind};
use crate::repository_session::RepositoryCommand as R;

impl Viewer {
    /// The panel's lines at the Files column's width; none outside Focus or without a message.
    pub(super) fn message_lines(&self) -> Vec<Line> {
        if !self.layout.focused(self.repo.as_ref()) || !self.layout.files() { return Vec::new(); }
        let columns = ((self.sidebar_w() - 16.0) / self.char_w.max(1.0)).floor().max(1.0) as usize;
        // ponytail: rewrapped on each call (messages are capped at 16 KiB); cache by (columns, message) if a long range shows up in profiles
        self.repo.as_ref().and_then(|r| r.message_lines(columns)).unwrap_or_default()
    }
    /// Rows the panel shows of `lines`: all of them, up to a third of the column.
    fn message_rows(&self, lines: usize) -> f32 {
        let column = (self.panes.viewport[1] - self.top() - footer_h()).max(0.0);
        (lines as f32).min((column / 3.0 / line_h()).floor().max(1.0))
    }
    /// The panel's height, its border included; 0 while hidden.
    pub(super) fn message_h(&self) -> f32 {
        match self.message_lines().len() { 0 => 0.0, n => self.message_rows(n) * line_h() + 1.0 }
    }
    /// Window y where the Files list's rows start.
    pub(super) fn nav_top(&self) -> f32 { self.top() + self.message_h() }
    /// The scroll within the panel's current lines: a resize or a shorter message can leave it past the end.
    fn message_scroll_in(&self, lines: usize) -> f32 { self.message_scroll.clamp(0.0, (lines as f32 - self.message_rows(lines)).max(0.0)) }
    fn clamp_message_scroll(&mut self, lines: usize) { self.message_scroll = self.message_scroll_in(lines); }
    fn message_line_at(&self, x: f32, y: f32) -> Option<(usize, Line)> {
        if !self.in_nav(x) || !(self.top()..self.nav_top() - 1.0).contains(&y) { return None; }
        let lines = self.message_lines();
        let row = ((y - self.top()) / line_h() + self.message_scroll_in(lines.len())) as usize;
        lines.into_iter().nth(row).map(|l| (row, l))
    }
    fn in_message(&self, x: f32, y: f32) -> bool { self.in_nav(x) && (self.top()..self.nav_top()).contains(&y) }
    /// The panel line and character under a point, the row clamped into the panel for drags past its edges.
    fn message_pos(&self, x: f32, y: f32) -> Option<(usize, usize)> {
        let lines = self.message_lines();
        let rows = self.message_rows(lines.len());
        let row = (((y - self.top()) / line_h()).clamp(0.0, (rows - 1.0).max(0.0)) + self.message_scroll_in(lines.len())) as usize;
        let line = lines.get(row)?;
        let col = ((x - self.nav_x() - 8.0) / self.char_w.max(1.0)).round().max(0.0) as usize;
        Some((row, col.min(line.text.chars().count())))
    }
    /// A press on the panel takes the keyboard and starts a selection; the release decides whether it was a click.
    pub(super) fn message_click(&mut self, x: f32, y: f32, cx: &mut Context<Self>) -> bool {
        if !self.in_message(x, y) { return false; }
        self.message_focused = true;
        self.message_press = self.message_pos(x, y);
        self.message_sel = self.message_press.map(|p| (p, p));
        cx.notify();
        true
    }
    /// Dragging from a press on the panel selects its text.
    pub(super) fn message_drag(&mut self, x: f32, y: f32, cx: &mut Context<Self>) -> bool {
        let Some(anchor) = self.message_press else { return false };
        if let Some(head) = self.message_pos(x, y) { self.message_sel = Some((anchor, head)); cx.notify(); }
        true
    }
    /// The release of a press that did not move is a click: the first line collapses the panel, a range's commit
    /// line opens that commit.
    pub(super) fn message_release(&mut self, cx: &mut Context<Self>) {
        let Some(anchor) = self.message_press.take() else { return };
        if self.message_sel.is_some_and(|(a, b)| a != b) { return; }
        self.message_sel = None;
        let command = match self.message_lines().get(anchor.0).map(|l| l.kind) {
            Some(LineKind::Toggle) => R::ToggleMessage(None),
            Some(LineKind::Commit(i)) => R::ToggleMessage(Some(i)),
            _ => return,
        };
        self.message_run(command, cx);
    }
    fn message_run(&mut self, command: R, cx: &mut Context<Self>) {
        self.message_sel = None;
        self.repository_command(command, cx);
        self.reveal_message_cursor();
        cx.notify();
    }
    /// Keep the keyboard's commit in view.
    fn reveal_message_cursor(&mut self) {
        let lines = self.message_lines();
        let cursor = self.repo.as_ref().map_or(0, |r| r.message_cursor());
        if let Some(row) = lines.iter().position(|l| l.kind == LineKind::Commit(cursor)) {
            let rows = self.message_rows(lines.len());
            let row = row as f32;
            if row < self.message_scroll { self.message_scroll = row; } else if row >= self.message_scroll + rows { self.message_scroll = row + 1.0 - rows; }
        }
        self.clamp_message_scroll(lines.len());
    }
    /// The selected text, lines joined with newlines.
    fn message_selection_text(&self) -> Option<String> {
        let (a, b) = self.message_sel.filter(|(a, b)| a != b)?;
        let ((r0, c0), (r1, c1)) = if a <= b { (a, b) } else { (b, a) };
        let lines = self.message_lines();
        let text = (r0..=r1.min(lines.len().saturating_sub(1))).map(|r| {
            let chars: Vec<char> = lines[r].text.chars().collect();
            let (from, to) = (if r == r0 { c0 } else { 0 }, if r == r1 { c1 } else { chars.len() });
            chars[from.min(chars.len())..to.min(chars.len()).max(from.min(chars.len()))].iter().collect::<String>()
        }).collect::<Vec<_>>().join("\n");
        Some(text)
    }
    /// Ctrl/Cmd+C in the focused panel: the selection, else the message of the commit the keyboard is on.
    pub(super) fn message_copy(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.message_focused || self.nav_focused || self.message_lines().is_empty() { return false; }
        let text = self.message_selection_text().or_else(|| {
            let Some(crate::repository_session::MessagePanel::Ready(m)) = self.repo.as_ref().and_then(|r| r.message_panel()) else { return None };
            m.commits.get(self.repo.as_ref().map_or(0, |r| r.message_cursor())).map(|c| c.message.trim_end().to_string())
        });
        if let Some(text) = text { cx.write_to_clipboard(ClipboardItem::new_string(text)); self.toast("Copied".into(), cx); }
        true
    }
    /// Keys while the panel has the keyboard: ↑/↓ walk a range's commits (or scroll one commit's message), Enter opens
    /// the commit, Esc or ↓ past the end give the keyboard back to Files.
    pub(super) fn message_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        use crate::repository_session::MessageKey as K;
        // anything that gave Files the keyboard (Tab, a click) took it from the panel
        if !self.message_focused || self.nav_focused { self.message_focused = false; return false; }
        let lines = self.message_lines().len();
        if lines == 0 { self.message_focused = false; return false; }
        let key = match ev.keystroke.key.as_str() {
            "up" => K::Up,
            "down" => K::Down,
            "enter" | "space" => K::Enter,
            "escape" => { self.message_focused = false; self.message_sel = None; self.nav_focused = true; cx.notify(); return true; }
            _ => return false,
        };
        match self.repo.as_ref().and_then(|r| r.message_key(key)) {
            Some(command) => self.message_run(command, cx),
            None => {
                let max = (lines as f32 - self.message_rows(lines)).max(0.0);
                match key {
                    K::Up => self.message_scroll = (self.message_scroll - 1.0).max(0.0),
                    K::Down if self.message_scroll < max => self.message_scroll += 1.0,
                    // past the end: back to the list
                    _ => { self.message_focused = false; self.message_sel = None; self.nav_focused = true; }
                }
                cx.notify();
            }
        }
        true
    }
    /// ↑ on Files' first file moves the keyboard into the panel.
    pub(super) fn enter_message_from_files(&mut self, cx: &mut Context<Self>) -> bool {
        if self.message_lines().is_empty() { return false; }
        self.message_focused = true;
        self.nav_focused = false;
        self.reveal_message_cursor();
        cx.notify();
        true
    }
    /// A right click on the panel: copy the message or the hash of the commit under the pointer.
    pub(super) fn message_menu(&mut self, x: f32, y: f32, cx: &mut Context<Self>) -> bool {
        if !self.in_nav(x) || !(self.top()..self.nav_top()).contains(&y) { return false; }
        let Some((row, _)) = self.message_line_at(x, y) else { return true };
        let lines = self.message_lines();
        // the commit a line belongs to: the nearest commit line at or above it, else the only one
        let index = lines[..=row].iter().rev().find_map(|l| match l.kind { LineKind::Commit(i) => Some(i), _ => None }).unwrap_or(0);
        let Some(crate::repository_session::MessagePanel::Ready(m)) = self.repo.as_ref().and_then(|r| r.message_panel()) else { return true };
        let Some(c) = m.commits.get(index) else { return true };
        let items = vec![("Copy commit message".to_string(), MenuAction::Clipboard(c.message.trim_end().to_string())),
                         ("Copy commit hash".to_string(), MenuAction::Clipboard(c.sha.clone()))];
        self.show_menu(Menu { x, y, items, hover: None });
        cx.notify();
        true
    }
    /// The wheel over the panel scrolls the panel.
    pub(super) fn message_wheel(&mut self, x: f32, y: f32, rows: f32, cx: &mut Context<Self>) -> bool {
        if !self.in_nav(x) || !(self.top()..self.nav_top()).contains(&y) { return false; }
        self.message_scroll += rows;
        let n = self.message_lines().len();
        self.clamp_message_scroll(n);
        cx.notify();
        true
    }
    /// What `paint_sidebar` draws for the panel, and its height: each visible line with its color, the selected span
    /// (x offset and width from the text start) and whether the keyboard's commit is on it.
    pub(super) fn message_paint(&self) -> (Vec<MessageRow>, f32) {
        let lines = self.message_lines();
        let scroll = self.message_scroll_in(lines.len()) as usize;
        let h = self.message_h();
        let cursor = self.message_focused.then(|| self.repo.as_ref().map_or(0, |r| r.message_cursor()));
        let sel = self.message_sel.filter(|(a, b)| a != b).map(|(a, b)| if a <= b { (a, b) } else { (b, a) });
        // a single commit (or a collapsed panel) has the keyboard on its first line
        let single = self.repo.as_ref().is_some_and(|r| r.message_collapsed()
            || !matches!(r.message_panel(), Some(crate::repository_session::MessagePanel::Ready(m)) if matches!(m.query, crate::commit_message::Query::Range(_))));
        let visible = lines.into_iter().enumerate().skip(scroll).take(self.message_rows(usize::MAX) as usize).map(|(row, l)| {
            let color = if l.kind == LineKind::Meta { self.palette.dim } else { self.palette.fg };
            let len = l.text.chars().count();
            let span = sel.filter(|((r0, _), (r1, _))| (*r0..=*r1).contains(&row)).map(|((r0, c0), (r1, c1))| {
                let (from, to) = (if row == r0 { c0 } else { 0 }, if row == r1 { c1 } else { len + 1 });
                (from as f32 * self.char_w, to.saturating_sub(from) as f32 * self.char_w)
            });
            let current = cursor.is_some_and(|c| l.kind == if single { LineKind::Toggle } else { LineKind::Commit(c) });
            MessageRow { text: l.text, color, span, current }
        }).collect();
        (visible, h)
    }
}

pub(super) struct MessageRow { pub text: String, pub color: u32, pub span: Option<(f32, f32)>, pub current: bool }

impl Viewer {
}
