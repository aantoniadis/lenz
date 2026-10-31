//! Terminal table cells and input geometry; all mutations live in the shared session.
use super::*;
use std::sync::Arc;
use code_core::csv::{Command as C, InputKind};
use ratatui::{layout::{Rect, Position}, style::Style};
const WIDTH: u16 = 18;

pub(crate) struct Geometry {
    tab: TabId, key: SnapshotKey, metadata: Arc<code_core::document::SourceMeta>, revision: u64,
    area: Rect, top: usize, left: usize, rows: usize, columns: usize, cells: Vec<(usize, usize, Rect)>, headings: Vec<(usize, Rect)>,
}
pub(crate) fn draw(frame: &mut ratatui::Frame, app: &App, pane: u8, area: Rect, palette: &lenz_content::theme::Palette) -> Option<Geometry> {
    let tab = app.session.state().shown(pane)?.id();
    let table = app.session.state().csv(tab)?;
    let snapshot = app.session.state().view(tab)?.snapshot();
    let mut geometry = Geometry { tab, key: snapshot.key(), metadata: snapshot.source_meta().clone(), revision: table.revision(), area, top: table.top, left: table.left, rows: table.rows, columns: table.columns, cells: Vec::new(), headings: Vec::new() };
    if area.is_empty() { return Some(geometry); }
    let paint = |frame: &mut ratatui::Frame, rect: Rect, text: &str, selected: bool, heading: bool| {
        let background = if selected { palette.selected_bg } else if heading { palette.header_bg } else { palette.bg };
        let style = Style::default().fg(crate::render::color(if selected { palette.accent } else { palette.fg })).bg(crate::render::color(background));
        frame.buffer_mut().set_style(rect, style);
        frame.buffer_mut().set_stringn(rect.x, rect.y, text, rect.width as usize, style);
    };
    let width = WIDTH.min(area.width).max(1);
    let columns = usize::from(area.width.div_ceil(width));
    let rows = usize::from(area.height.saturating_sub(2));
    let full_columns = usize::from(area.width / width).max(1);
    let left = table.left.min(table.column_count().saturating_sub(full_columns));
    let top = table.top.min(table.row_count().saturating_sub(rows.max(1)));
    if !table.pending {
        for (shown, column) in (left..table.column_count()).take(columns).enumerate() {
            let x = area.x.saturating_add(shown as u16 * width);
            let rect = Rect::new(x, area.y, width.min(area.right().saturating_sub(x)), 1);
            let marker = table.sort.filter(|(sorted, _)| *sorted == column).map_or("", |(_, desc)| if desc { " ↓" } else { " ↑" });
            paint(frame, rect, &format!("{}{}", table.heading_preview(column, width.saturating_sub(3) as usize), marker), table.column == column, true);
            geometry.headings.push((column, rect));
            for (shown_row, row) in (top..table.row_count()).take(rows).enumerate() {
                let cell = Rect::new(x, area.y.saturating_add(1 + shown_row as u16), rect.width, 1);
                let selected = row == table.row && column == table.column;
                let text = if selected && let Some(input) = table.input.as_ref().filter(|input| input.kind == InputKind::Cell) {
                    let mut shown = input.value.chars().rev().take(width.saturating_sub(2) as usize).map(|c| match c { '\n' | '\r' => '↵', '\t' => ' ', c if c.is_control() => '�', c => c }).collect::<Vec<_>>().into_iter().rev().collect::<String>();
                    shown.push('▏'); shown
                } else { table.cell_preview(row, column, width.saturating_sub(1) as usize) };
                paint(frame, cell, &text, selected, false);
                geometry.cells.push((row, column, cell));
            }
        }
    }
    let footer = if let Some(error) = &table.error { error.clone() } else if let Some(input) = &table.input {
        let kind = if input.kind == InputKind::Cell { "Cell" } else { "Filter" };
        format!("{kind}: {} · Enter commit · Esc cancel · Ctrl+A clear", input.value.chars().rev().take(area.width as usize).collect::<Vec<_>>().into_iter().rev().collect::<String>().replace(['\n', '\r', '\t'], " "))
    } else { table.status() };
    paint(frame, Rect::new(area.x, area.bottom() - 1, area.width, 1), &footer, false, true);
    Some(geometry)
}
impl App {
    pub(super) fn csv_key(&mut self, key: KeyEvent) -> bool {
        if self.focus != Focus::Text { return false; }
        let Some(table) = self.session.state().active().and_then(|tab| self.session.state().csv(tab.id())) else { return false };
        let input = table.input.is_some();
        let (ctrl, alt, shift) = (key.modifiers.contains(KeyModifiers::CONTROL), key.modifiers.contains(KeyModifiers::ALT), key.modifiers.contains(KeyModifiers::SHIFT));
        if key.code == KeyCode::F(10) && shift {
            self.prompt(Prompt::Context { x: 0, y: 1, entries: self.session.context_menu() });
            return true;
        }
        let command = if input {
            match (key.code, ctrl, alt) {
                (KeyCode::Enter, false, false) => Some(C::Commit),
                (KeyCode::Esc, _, _) => Some(C::Cancel),
                (KeyCode::Backspace, false, false) => Some(C::Backspace),
                (KeyCode::Char('a'|'A'), true, false) => Some(C::SetInput(String::new())),
                (KeyCode::Char(c), false, false) => Some(C::Input(c.to_string())),
                _ => None,
            }
        } else {
            match (key.code, ctrl, alt, shift) {
                (KeyCode::Up, false, false, _) => Some(C::Move { rows: -1, columns: 0 }),
                (KeyCode::Down, false, false, _) => Some(C::Move { rows: 1, columns: 0 }),
                (KeyCode::Left, false, false, _) => Some(C::Move { rows: 0, columns: -1 }),
                (KeyCode::Right, false, false, _) => Some(C::Move { rows: 0, columns: 1 }),
                (KeyCode::PageUp, false, false, _) => Some(C::Move { rows: -(table.rows as isize), columns: 0 }),
                (KeyCode::PageDown, false, false, _) => Some(C::Move { rows: table.rows as isize, columns: 0 }),
                (KeyCode::Home, _, false, _) => Some(C::Move { rows: if ctrl { isize::MIN } else { 0 }, columns: isize::MIN }),
                (KeyCode::End, _, false, _) => Some(C::Move { rows: if ctrl { isize::MAX } else { 0 }, columns: isize::MAX }),
                (KeyCode::Left, true, true, _) => Some(C::MoveColumn { right: false }),
                (KeyCode::Right, true, true, _) => Some(C::MoveColumn { right: true }),
                (KeyCode::Enter, false, false, _) | (KeyCode::Char('e'), false, false, _) => Some(C::BeginEdit),
                (KeyCode::Char('/'), false, false, _) => Some(C::BeginFilter),
                (KeyCode::Char('s'), false, false, _) => Some(C::Sort),
                (KeyCode::Char('t'), false, false, _) => Some(C::Toggle),
                _ => None,
            }
        };
        if let Some(command) = command { self.dispatch(Command::Csv(command)); return true; }
        // Global and command-menu bindings remain available; plain source typing never does.
        input && matches!(key.code, KeyCode::Left | KeyCode::Right | KeyCode::Up | KeyCode::Down | KeyCode::Delete | KeyCode::Tab)
            || !ctrl && !alt && matches!(key.code, KeyCode::Char(c) if c != ':' && c != '?')
    }
    pub(super) fn csv_mouse(&mut self, event: MouseEvent) -> bool {
        let point = Position::new(event.column, event.row);
        // The wheel over the table's scrollbar scrolls it as over its cells.
        let wheel = matches!(event.kind, MouseEventKind::ScrollDown | MouseEventKind::ScrollUp | MouseEventKind::ScrollLeft | MouseEventKind::ScrollRight);
        let Some((pane, geometry)) = self.layout.panes.iter().find_map(|pane| pane.csv.as_ref().filter(|csv| csv.area.contains(point) || (wheel && pane.strip.contains(point))).map(|csv| (pane.pane, csv))) else { return false };
        let current = self.session.state().shown(pane).is_some_and(|tab| tab.id() == geometry.tab)
            && self.session.state().view(geometry.tab).is_some_and(|view| view.snapshot().key() == geometry.key && Arc::ptr_eq(view.snapshot().source_meta(), &geometry.metadata))
            && self.session.state().csv(geometry.tab).is_some_and(|table| table.revision() == geometry.revision && !table.pending && table.top == geometry.top && table.left == geometry.left && table.rows == geometry.rows && table.columns == geometry.columns);
        if !current { return true; }
        let cell = geometry.cells.iter().find(|(_, _, rect)| rect.contains(point)).map(|(row, column, _)| (*row, *column));
        let heading = geometry.headings.iter().find(|(_, rect)| rect.contains(point)).map(|(column, _)| *column);
        let tab = geometry.tab;
        self.dispatch(Command::FocusPane(pane)); self.focus = Focus::Text;
        match event.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some((row, column)) = cell { self.dispatch(Command::Csv(C::Select { row, column })); }
                else if let Some(column) = heading {
                    let row = self.session.state().csv(tab).map_or(0, |table| table.row);
                    self.dispatch(Command::Csv(C::Select { row, column })); self.dispatch(Command::Csv(C::Sort));
                }
            }
            MouseEventKind::Down(MouseButton::Right) => {
                if let Some((row, column)) = cell { self.dispatch(Command::Csv(C::Select { row, column })); }
                self.prompt(Prompt::Context { x: event.column, y: event.row, entries: self.session.context_menu() });
            }
            MouseEventKind::ScrollDown => self.dispatch(Command::Csv(C::Scroll { rows: 3, columns: 0 })),
            MouseEventKind::ScrollUp => self.dispatch(Command::Csv(C::Scroll { rows: -3, columns: 0 })),
            MouseEventKind::ScrollLeft => self.dispatch(Command::Csv(C::Scroll { rows: 0, columns: -1 })),
            MouseEventKind::ScrollRight => self.dispatch(Command::Csv(C::Scroll { rows: 0, columns: 1 })),
            _ => {}
        }
        true
    }
}
