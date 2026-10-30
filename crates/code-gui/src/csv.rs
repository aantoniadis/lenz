//! CSV owns no second document: cells, projection and edits come from the shared session. This adapter keeps
//! viewport hit rectangles and native input selection only.
use super::*;
use code_core::csv::{Command as C, InputKind};
use std::ops::Range;

#[derive(Default)]
pub(super) struct Ui {
    hits: Vec<Hit>,
    pub(super) selected: Option<(FieldKey, String)>,
    pub(super) marked: Option<Marked>,
    ended: Option<(FieldKey, String)>,
    pub(super) input_bounds: Option<Bounds<Pixels>>,
    wheel: [gpui::Point<f32>; 2],
    wheel_tab: [Option<TabId>; 2],
}
impl Ui {
    pub(super) fn hits_clear(&mut self) { self.hits.clear(); self.input_bounds = None; }
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct FieldKey { tab: TabId, source: code_core::text::SnapshotKey, revision: u64, row: usize, column: usize, kind: InputKind }
pub(super) struct Marked {
    key: FieldKey, pub(super) before: String,
    pub(super) current: String, pub(super) range: Range<usize>, pub(super) selection: Range<usize>,
}
struct Hit {
    bounds: Bounds<Pixels>, tab: TabId, pane: u8, source: code_core::text::SnapshotKey,
    revision: u64, top: usize, left: usize, row: usize, column: usize, header: bool,
}
pub(super) struct Cell { pub(super) bounds: Bounds<Pixels>, pub(super) text: String, selected: bool, header: bool, pub(super) editing: bool, selected_text: bool, marked: bool }
pub(super) struct Frame { bounds: Bounds<Pixels>, pub(super) cells: Vec<Cell>, pub(super) status: String, pub(super) input: Option<String>, selected_input: bool, footer_editing: bool }

fn short(text: &str, maximum: usize, tail: bool) -> String {
    let value: String = if tail { text.chars().rev().take(maximum).collect::<Vec<_>>().into_iter().rev().collect() } else { text.chars().take(maximum).collect() };
    value.chars().map(|c| match c { '\n' | '\r' => '↵', '\t' => ' ', c if c.is_control() => '�', c => c }).collect()
}

impl ReaderView {
    pub(super) fn csv_active(&self) -> bool { self.session.state().active().is_some_and(|t| self.session.state().csv(t.id()).is_some()) }
    pub(super) fn csv_field(&self) -> Option<(TabId, InputKind, &str)> {
        if self.modal.is_some() || self.session.state().terminal().focused() || self.tree_focus || self.results_focus || self.find_focus.is_some() || self.crumb_focus.is_some() { return None; }
        let tab = self.session.state().active()?.id();
        let input = self.session.state().csv(tab)?.input.as_ref()?;
        Some((tab, input.kind, &input.value))
    }
    fn csv_field_key(&self) -> Option<FieldKey> {
        let (tab, kind, _) = self.csv_field()?;
        let view = self.session.state().csv(tab)?;
        Some(FieldKey { tab, source: self.session.state().view(tab)?.snapshot().key(), revision: view.revision(), row: view.row, column: view.column, kind })
    }
    pub(super) fn csv_input_selected(&self) -> bool {
        self.csv_field_key().zip(self.csv_field()).is_some_and(|(key, (_, _, value))| self.csv_ui.selected.as_ref().is_some_and(|s| s.0 == key && s.1 == value))
    }
    pub(super) fn csv_marked(&self) -> Option<&Marked> {
        let key = self.csv_field_key()?;
        let (_, _, value) = self.csv_field()?;
        self.csv_ui.marked.as_ref().filter(|m| m.key == key && m.current == value)
    }
    pub(super) fn csv_finish_mark(&mut self) {
        if let Some(marked) = self.csv_marked() { self.csv_ui.ended = Some((marked.key, marked.current.clone())); }
        self.csv_ui.marked = None;
    }
    pub(super) fn csv_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.csv_active() || self.tree_focus || self.results_focus || self.find_focus.is_some() || self.crumb_focus.is_some() { return false; }
        let k = &ev.keystroke;
        let ctrl = k.modifiers.control || k.modifiers.platform;
        let input = self.csv_field().is_some();
        if input && !ctrl && !k.modifiers.alt && k.key_char.as_deref().is_some_and(|s| !s.chars().any(char::is_control)) { self.csv_ui.ended = None; }
        if ctrl {
            match k.key.as_str() {
                "z" | "y" if !k.modifiers.alt && (k.key == "z" || !k.modifiers.shift) => {
                    let id = if k.key == "y" || k.modifiers.shift { CommandId::Redo } else { CommandId::Undo };
                    match self.session.disabled(id) {
                        Some(reason) => self.dispatch(Command::Notice(reason.into()), cx),
                        None => self.run(id, window, cx),
                    }
                    cx.stop_propagation();
                    return true;
                }
                "a" if input => {
                    self.csv_ui.ended = None;
                    self.csv_ui.selected = self.csv_field_key().zip(self.csv_field()).map(|(key, (_, _, value))| (key, value.to_owned()));
                    cx.notify(); return true;
                }
                "v" if input => {
                    self.csv_ui.ended = None;
                    if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) { self.csv_replace(None, &text, false, None, cx); }
                    return true;
                }
                "c" if input && self.csv_input_selected() => {
                    if let Some((_, _, value)) = self.csv_field() { self.dispatch(Command::CopyText(value.to_owned()), cx); }
                    return true;
                }
                "f" => { self.dispatch(Command::Csv(C::BeginFilter), cx); return true; }
                _ => return input,
            }
        }
        if k.modifiers.alt { return false; }
        if input {
            match k.key.as_str() {
                "escape" if self.csv_marked().is_some() => {
                    let Some(before) = self.csv_marked().map(|m| m.before.clone()) else { return true; }; self.csv_ui.marked = None;
                    self.dispatch(Command::Csv(C::SetInput(before)), cx);
                }
                "escape" => { self.csv_ui.selected = None; self.dispatch(Command::Csv(C::Cancel), cx); }
                "enter" if self.csv_marked().is_some() => { self.csv_finish_mark(); cx.notify(); }
                "enter" => { self.csv_ui.selected = None; self.dispatch(Command::Csv(C::Commit), cx); }
                "backspace" | "delete" if self.csv_input_selected() => { self.csv_ui.selected = None; self.dispatch(Command::Csv(C::SetInput(String::new())), cx); }
                "backspace" => { self.csv_ui.ended = None; self.dispatch(Command::Csv(C::Backspace), cx); }
                "tab" => { self.dispatch(Command::Csv(C::Commit), cx); self.dispatch(Command::Csv(C::Move { rows: 0, columns: if k.modifiers.shift { -1 } else { 1 } }), cx); }
                _ if k.key_char.as_deref().is_some_and(|s| !s.chars().any(char::is_control)) => {
                    // The registered native handler commits the text once, including composed Unicode.
                    if !self.input_state.registered.get() && let Some(text) = &k.key_char { self.csv_replace(None, text, false, None, cx); cx.stop_propagation(); }
                    return true;
                }
                _ => {},
            }
            return true;
        }
        let command = match k.key.as_str() {
            "up" => C::Move { rows: -1, columns: 0 }, "down" => C::Move { rows: 1, columns: 0 },
            "left" => C::Move { rows: 0, columns: -1 }, "right" => C::Move { rows: 0, columns: 1 },
            "tab" => C::Move { rows: 0, columns: if k.modifiers.shift { -1 } else { 1 } },
            "pageup" => C::Move { rows: -(self.rows.max(1) as isize), columns: 0 },
            "pagedown" => C::Move { rows: self.rows.max(1) as isize, columns: 0 },
            "home" => C::Move { rows: 0, columns: isize::MIN }, "end" => C::Move { rows: 0, columns: isize::MAX },
            "enter" | "f2" => C::BeginEdit, "/" => C::BeginFilter, "s" => C::Sort,
            "escape" => C::Cancel,
            "backspace" | "delete" => {
                self.dispatch(Command::Csv(C::BeginEdit), cx);
                self.dispatch(Command::Csv(C::SetInput(String::new())), cx);
                return true;
            }
            _ if k.key_char.as_deref().is_some_and(|s| !s.chars().any(char::is_control)) => {
                self.dispatch(Command::Csv(C::BeginEdit), cx);
                if let Some(text) = &k.key_char { self.dispatch(Command::Csv(C::SetInput(text.clone())), cx); }
                cx.stop_propagation();
                return true;
            }
            _ => return false,
        };
        if command == C::BeginEdit { self.csv_ui.ended = None; }
        self.dispatch(Command::Csv(command), cx);
        let _ = window;
        true
    }
    pub(super) fn csv_replace(&mut self, range: Option<Range<usize>>, text: &str, marking: bool, selected: Option<Range<usize>>, cx: &mut Context<Self>) -> bool {
        if !self.csv_active() { return false; }
        let Some((key, value)) = self.csv_field_key().zip(self.csv_field()).map(|(key, (_, _, value))| (key, value.to_owned())) else { return true; };
        if !marking && self.csv_ui.ended.as_ref().is_some_and(|(ended, current)| *ended == key && *current == value) { return true; }
        let marked = self.csv_marked();
        let finishing = marked.is_some();
        let explicit = range.is_some();
        let bytes = match range {
            Some(range) => match super::input_method::utf16_bytes(&value, range) { Some(range) => range, None => return true },
            None if self.csv_input_selected() => 0..value.len(),
            None => marked.map_or(value.len()..value.len(), |m| m.range.clone()),
        };
        if !marking && text.is_empty() && explicit && let Some(marked) = marked.filter(|m| m.range == bytes) {
            let before = marked.before.clone(); self.csv_ui.marked = None;
            self.csv_ui.ended = Some((key, before.clone()));
            self.dispatch(Command::Csv(C::SetInput(before)), cx); return true;
        }
        if value.len().saturating_sub(bytes.len()).checked_add(text.len()).is_none_or(|len| len > code_core::csv::MAX_INPUT) { return true; }
        let selection = if marking {
            match selected { Some(range) => match super::input_method::utf16_bytes(text, range) { Some(range) => Some(bytes.start + range.start..bytes.start + range.end), None => return true }, None => Some(bytes.start + text.len()..bytes.start + text.len()) }
        } else { None };
        let before = marked.map_or_else(|| value.clone(), |m| m.before.clone());
        let mut next = value;
        next.replace_range(bytes.clone(), text);
        self.csv_ui.selected = None;
        self.csv_ui.ended = (!marking && finishing).then(|| (key, next.clone()));
        self.csv_ui.marked = selection.map(|selection| Marked { key, before, current: next.clone(), range: bytes.start..bytes.start + text.len(), selection });
        self.dispatch(Command::Csv(C::SetInput(next)), cx);
        true
    }
    pub(super) fn csv_frame(&mut self, geom: PaneGeom) -> Option<Frame> {
        let tab = self.session.state().shown(geom.pane)?.id();
        let view = self.session.state().csv(tab)?;
        let bounds = geom.text;
        let width = f32::from(bounds.size.width);
        let gutter = (5.0 * self.char_w).min(width);
        let cell_w = (18.0 * self.char_w).min((width - gutter).max(1.0));
        let columns = ((width - gutter) / cell_w).floor().max(1.0) as usize;
        let shown_columns = ((width - gutter) / cell_w).ceil().max(1.0) as usize;
        let rows = geom.rows.saturating_sub(2).max(1);
        if geom.pane == self.session.state().focused_pane() && (view.rows != rows || view.columns != columns) {
            self.session.dispatch(Command::Csv(C::Size { rows, columns }));
        }
        let view = self.session.state().csv(tab)?;
        let source = self.session.state().view(tab)?.snapshot().key();
        let mut cells = Vec::new();
        let max_chars = ((cell_w - 8.0).max(0.0) / self.char_w).ceil().max(1.0) as usize;
        let status = short(&view.status(), (width / self.char_w).ceil().max(1.0) as usize, false);
        let footer = Bounds { origin: gpui::point(bounds.origin.x, (bounds.bottom() - px(line_h())).max(bounds.top())), size: gpui::size(bounds.size.width, px(line_h().min(f32::from(bounds.size.height)))) };
        let input = view.input.as_ref().filter(|_| view.error.is_none()).map(|input| {
            if input.kind == InputKind::Cell { return "Enter apply · Esc cancel · Ctrl+A select cell".into(); }
            let label = "Filter: ";
            format!("{label}{}", short(&input.value, ((width / self.char_w) as usize).saturating_sub(label.len() + 2), true))
        });
        if geom.pane == self.session.state().focused_pane() { self.csv_ui.input_bounds = view.input.as_ref().filter(|i| view.error.is_none() && i.kind == InputKind::Filter).map(|_| footer); }
        if !view.pending && view.error.is_none() {
            let number_bounds = |row: usize| Bounds { origin: gpui::point(bounds.origin.x, bounds.origin.y + px(row as f32 * line_h())), size: gpui::size(px(gutter), px(line_h())) };
            cells.push(Cell { bounds: number_bounds(0), text: "#".into(), selected: false, header: true, editing: false, selected_text: false, marked: false });
            for (visible, row) in (view.top..view.row_count()).take(rows).enumerate() {
                let b = number_bounds(visible + 1);
                if b.bottom() > footer.top() { break; }
                cells.push(Cell { bounds: b, text: (row + 1).to_string(), selected: row == view.row, header: true, editing: false, selected_text: false, marked: false });
            }
            for (visible, column) in (view.left..view.column_count()).take(shown_columns).enumerate() {
                let x = bounds.origin.x + px(gutter + visible as f32 * cell_w);
                let cell_bounds = |row: usize| Bounds { origin: gpui::point(x, bounds.origin.y + px(row as f32 * line_h())), size: gpui::size(px(cell_w.min(f32::from(bounds.right() - x)).max(0.0)), px(line_h())) };
                let heading = format!("{}{}", view.heading_preview(column, max_chars.saturating_sub(2)), match view.sort { Some((sorted, false)) if sorted == column => " ↑", Some((sorted, true)) if sorted == column => " ↓", _ => "" });
                cells.push(Cell { bounds: cell_bounds(0), text: heading, selected: column == view.column, header: true, editing: false, selected_text: false, marked: false });
                for (visible_row, row) in (view.top..view.row_count()).take(rows).enumerate() {
                    let b = cell_bounds(visible_row + 1);
                    if b.bottom() > footer.top() { break; }
                    let selected = row == view.row && column == view.column;
                    let editing = view.input.as_ref().filter(|i| i.kind == InputKind::Cell && selected);
                    let text = editing.map_or_else(|| view.cell_preview(row, column, max_chars), |i| short(&i.value, max_chars.saturating_sub(1), true));
                    if editing.is_some() && geom.pane == self.session.state().focused_pane() { self.csv_ui.input_bounds = Some(b); }
                    cells.push(Cell { bounds: b, text, selected, header: false, editing: editing.is_some(), selected_text: editing.is_some() && self.csv_input_selected(), marked: editing.is_some() && self.csv_marked().is_some() });
                    self.csv_ui.hits.push(Hit { bounds: b, tab, pane: geom.pane, source, revision: view.revision(), top: view.top, left: view.left, row, column, header: false });
                }
                self.csv_ui.hits.push(Hit { bounds: cell_bounds(0), tab, pane: geom.pane, source, revision: view.revision(), top: view.top, left: view.left, row: view.row, column, header: true });
            }
        }
        Some(Frame { bounds, cells, status, input, selected_input: self.csv_input_selected(), footer_editing: view.error.is_none() && view.input.as_ref().is_some_and(|i| i.kind == InputKind::Filter) })
    }
    pub(super) fn csv_click(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) -> bool {
        let Some(geom) = self.panes.iter().find(|g| g.text.contains(&event.position)).copied() else { return false };
        let Some(tab) = self.session.state().shown(geom.pane).map(|t| t.id()) else { return false };
        if self.session.state().csv(tab).is_none() { return false; }
        self.tree_focus = false; self.results_focus = false; self.find_focus = None; self.crumb_focus = None;
        let hit = self.csv_ui.hits.iter().find(|h| h.bounds.contains(&event.position) && h.tab == tab && self.session.state().view(tab).is_some_and(|v| v.snapshot().key() == h.source)
            && self.session.state().csv(tab).is_some_and(|v| !v.pending && v.revision() == h.revision && v.top == h.top && v.left == h.left))
            .map(|h| (h.pane, h.row, h.column, h.header));
        self.dispatch(Command::FocusPane(geom.pane), cx);
        if let Some((_, row, column, header)) = hit {
            if self.session.state().csv(tab).is_some_and(|v| v.input.is_some()) { return true; }
            self.csv_ui.selected = None; self.csv_ui.marked = None;
            self.dispatch(Command::Csv(C::Select { row, column }), cx);
            if event.click_count >= 2 { self.dispatch(Command::Csv(if header { C::Sort } else { C::BeginEdit }), cx); }
        }
        true
    }
    pub(super) fn csv_wheel(&mut self, event: &ScrollEvent, cx: &mut Context<Self>) -> bool {
        let Some(g) = self.panes.iter().find(|g| g.text.contains(&event.position)).copied() else { return false };
        let Some(tab) = self.session.state().shown(g.pane).map(|t| t.id()).filter(|id| self.session.state().csv(*id).is_some()) else { return false; };
        if self.csv_field().is_some() { return true; }
        let (mut x, mut y) = match event.delta { ScrollDelta::Lines(l) => (-l.x, -l.y * 3.0), ScrollDelta::Pixels(p) => (-f32::from(p.x) / (18.0 * self.char_w), -f32::from(p.y) / line_h()) };
        if event.modifiers.shift { x += y; y = 0.0; }
        let pane = g.pane as usize;
        if self.csv_ui.wheel_tab[pane] != Some(tab) || matches!(event.delta, ScrollDelta::Lines(_)) {
            self.csv_ui.wheel[pane] = Default::default(); self.csv_ui.wheel_tab[pane] = Some(tab);
        }
        let columns = lenz_gpui::scroll::steps(&mut self.csv_ui.wheel[pane].x, x);
        let rows = lenz_gpui::scroll::steps(&mut self.csv_ui.wheel[pane].y, y);
        self.dispatch(Command::FocusPane(g.pane), cx);
        self.dispatch(Command::Csv(C::Scroll { rows, columns }), cx);
        if let Some(view) = self.session.state().csv(tab) {
            lenz_gpui::scroll::clamp(&mut self.csv_ui.wheel[pane].y, view.top == 0, view.top >= view.row_count().saturating_sub(view.rows.max(1)));
            lenz_gpui::scroll::clamp(&mut self.csv_ui.wheel[pane].x, view.left == 0, view.left >= view.column_count().saturating_sub(view.columns.max(1)));
        }
        true
    }

}

impl Frame {
    pub(super) fn paint(&self, window: &mut Window, cx: &mut App, m: Metrics<'_>, palette: &lenz_content::theme::Palette) {
        window.with_content_mask(Some(gpui::ContentMask { bounds: self.bounds }), |window| {
            let x = f32::from(self.bounds.origin.x);
            let y = f32::from(self.bounds.origin.y);
            let width = f32::from(self.bounds.size.width);
            lenz_gpui::fill(window, x, y, width, f32::from(self.bounds.size.height), palette.bg);
            for cell in &self.cells {
                let b = cell.bounds;
                let (x, y, w, h) = (f32::from(b.origin.x), f32::from(b.origin.y), f32::from(b.size.width), f32::from(b.size.height));
                let bg = if cell.selected_text { palette.sel_bg } else if cell.selected { palette.selected_bg } else if cell.header { palette.header_bg } else { palette.bg };
                lenz_gpui::fill(window, x, y, w, h, bg);
                lenz_gpui::fill(window, x + w - 1.0, y, 1.0, h, palette.border);
                lenz_gpui::fill(window, x, y + h - 1.0, w, 1.0, if cell.selected && !cell.header { palette.accent } else { palette.border });
                window.with_content_mask(Some(gpui::ContentMask { bounds: b }), |window| {
                    lenz_gpui::text(window, cx, m, x + 4.0, y, &cell.text, palette.fg);
                    if cell.editing {
                        let shown = display::show_bidi(&display::cap_marks(&cell.text)).into_owned();
                        let run = gpui::TextRun { len: shown.len(), font: m.font.clone(), color: rgb(palette.fg).into(), background_color: None, underline: None, strikethrough: None };
                        let line = window.text_system().shape_line(shown.into(), px(m.font_size), &[run], None);
                        let end = f32::from(line.width).min((w - 8.0).max(0.0));
                        lenz_gpui::fill(window, x + 4.0 + end, y + 2.0, 1.0, (h - 4.0).max(0.0), palette.caret);
                        if cell.marked { lenz_gpui::fill(window, x + 4.0, y + h - 3.0, end, 1.0, palette.accent); }
                    }
                });
            }
            let footer = (f32::from(self.bounds.bottom()) - line_h()).max(y);
            lenz_gpui::fill(window, x, footer, width, line_h(), if self.footer_editing && self.selected_input { palette.sel_bg } else { palette.header_bg });
            lenz_gpui::text(window, cx, m, x + 4.0, footer, self.input.as_ref().unwrap_or(&self.status), palette.fg);
        });
    }
}
