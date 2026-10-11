//! Terminal layout/input only. The shared service validates, selects and saves.
use crate::chrome::{color, style_with};
use crossterm::event::{Event, KeyCode, KeyEventKind, MouseButton, MouseEventKind};
use lenz::theme::{Palette, ThemeEntry, ThemeSelection, ThemeService};
use ratatui::{
    Frame,
    layout::Rect,
    widgets::{Block, Borders, Clear},
};
use unicode_width::UnicodeWidthStr;
pub struct Chooser {
    pub entries: Vec<ThemeEntry>,
    selected: usize,
    scroll: usize,
    hits: Vec<(usize, Rect)>,
    /// The box as last drawn: a click outside closes the chooser.
    rect: Rect,
    source: String,
}
pub enum Action {
    None,
    Close,
    Select(ThemeSelection),
}
impl Chooser {
    pub fn new(service: &ThemeService) -> Self {
        let mut c = Self {
            entries: Vec::new(),
            selected: 0,
            scroll: 0,
            hits: Vec::new(),
            rect: Rect::default(),
            source: String::new(),
        };
        c.refresh(service);
        c.selected = c
            .entries
            .iter()
            .position(|e| e.id == service.snapshot().selection.id())
            .unwrap_or(0);
        c
    }
    pub(crate) fn refresh(&mut self, service: &ThemeService) {
        let id = self.entries.get(self.selected).map(|e| e.id.clone());
        self.entries = vec![ThemeEntry {
            id: "system".into(),
            label: "Follow system".into(),
            palette: std::sync::Arc::new(lenz::theme::DARCULA),
        }];
        self.entries.extend_from_slice(service.entries());
        self.selected = id
            .and_then(|id| self.entries.iter().position(|e| e.id == id))
            .unwrap_or(0);
        self.source = service.snapshot().source;
    }
    pub(crate) fn selected(&self) -> usize { self.selected }
    /// The highlighted theme's id, for the live preview.
    pub(crate) fn highlighted(&self) -> Option<&str> { self.entries.get(self.selected).map(|e| e.id.as_str()) }
    fn select(&self, index: usize) -> Action {
        self.entries
            .get(index)
            .and_then(|e| ThemeSelection::from_id(&e.id))
            .map_or(Action::None, Action::Select)
    }
    pub fn handle(&mut self, event: &Event) -> Action {
        self.selected = self.selected.min(self.entries.len().saturating_sub(1));
        match event {
            Event::Key(k) if k.kind != KeyEventKind::Release => match k.code {
                KeyCode::Esc => return Action::Close,
                KeyCode::Enter => return self.select(self.selected),
                KeyCode::Up => self.selected = self.selected.saturating_sub(1),
                KeyCode::Down => {
                    self.selected = (self.selected + 1).min(self.entries.len().saturating_sub(1))
                }
                KeyCode::Home => self.selected = 0,
                KeyCode::End => self.selected = self.entries.len().saturating_sub(1),
                KeyCode::PageUp => self.selected = self.selected.saturating_sub(8),
                KeyCode::PageDown => {
                    self.selected = (self.selected + 8).min(self.entries.len().saturating_sub(1))
                }
                _ => {}
            },
            Event::Mouse(m) if m.kind == MouseEventKind::Down(MouseButton::Left) => {
                if let Some((i, _)) = self
                    .hits
                    .iter()
                    .find(|(_, r)| r.contains((m.column, m.row).into()))
                {
                    return self.select(*i);
                }
                if !self.rect.contains((m.column, m.row).into()) {
                    return Action::Close;
                }
            }
            _ => {}
        }
        Action::None
    }
    pub fn draw(&mut self, frame: &mut Frame, service: &ThemeService, palette: &Palette) {
        self.refresh(service);
        let area = frame.area();
        let w = area.width.saturating_sub(2).min(70);
        let h = area.height.saturating_sub(2).min(22);
        self.hits.clear();
        if w < 3 || h < 3 {
            return;
        }
        let rect = Rect::new(
            area.x + (area.width - w) / 2,
            area.y + (area.height - h) / 2,
            w,
            h,
        );
        self.rect = rect;
        frame.render_widget(Clear, rect);
        let base = style_with(palette).bg(color(palette.status_bg));
        frame.render_widget(
            Block::default()
                .borders(Borders::ALL)
                .title(" Theme ")
                .style(base),
            rect,
        );
        let cols = usize::from(w - 2);
        let mut rows = Vec::new();
        for (label, index) in
            std::iter::once(("↑/↓ choose · Enter select · Esc close".to_owned(), None))
                .chain(std::iter::once((format!("Current: {}", self.source), None)))
                .chain(
                    self.entries
                        .iter()
                        .enumerate()
                        .map(|(i, e)| (e.label.clone(), Some(i))),
                )
        {
            let mut line = String::new();
            let mut width = 0;
            for c in label.chars() {
                let cw = c.to_string().width().max(1);
                if width + cw > cols && !line.is_empty() {
                    rows.push((std::mem::take(&mut line), index));
                    width = 0;
                }
                line.push(c);
                width += cw;
            }
            if !line.is_empty() {
                rows.push((line, index));
            }
        }
        let visible = usize::from(h - 2);
        self.scroll = self.scroll.min(rows.len().saturating_sub(visible));
        if let Some(at) = rows.iter().position(|(_, i)| *i == Some(self.selected)) {
            if at < self.scroll {
                self.scroll = at;
            } else if at >= self.scroll + visible {
                self.scroll = at + 1 - visible;
            }
        }
        for (row, (label, index)) in rows.iter().skip(self.scroll).take(visible).enumerate() {
            let r = Rect::new(rect.x + 1, rect.y + 1 + row as u16, w - 2, 1);
            let style = if *index == Some(self.selected) {
                base.bg(color(palette.selected_bg))
            } else if index.is_none() {
                base.fg(color(palette.accent))
            } else {
                base
            };
            frame.buffer_mut().set_style(r, style);
            crate::visual::safe_text(frame.buffer_mut(), r, label, style);
            if let Some(i) = index {
                self.hits.push((*i, r));
            }
        }
    }
}
