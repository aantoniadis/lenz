//! Table commands share draft transactions and the admitted-version worker scheduler.
use super::*;
use crate::csv::{self, Command as C, InputKind};

pub struct Reply { pub tab: TabId, pub request: u64, pub result: Result<Arc<csv::Prepared>, String>, pub edits: Option<Vec<crate::text::TextEdit>>, pub order: Option<Vec<usize>> }
impl State {
    pub fn csv(&self, tab: TabId) -> Option<&csv::View> {
        self.tabs.iter().find(|t| t.id == tab)?.csv.as_ref()
    }
}
pub(super) const PENDING_FIELD: &str = "Finish the CSV field with Enter or Escape first";
impl Session {
    pub(super) fn csv_pending(&self) -> bool { self.state.tabs.iter().any(|tab| tab.csv.as_ref().is_some_and(|view| view.input.is_some())) }
    pub(super) fn csv_pending_blocks(&self, command: &Command) -> bool {
        self.csv_pending() && !matches!(command, Command::Csv(_) | Command::Theme(_) | Command::PreviewTheme(_) | Command::PrepareLayout { .. } | Command::PrepareProjection(_) | Command::CancelProjection { .. } | Command::SetNativeContext { .. } | Command::MeasuredReveal { .. } | Command::ClampColumn { .. } | Command::SetTextSize { .. } | Command::SetLayout(_) | Command::ClearPresentation | Command::CopyText(_) | Command::Copy | Command::SaveWorkspace | Command::Recheck | Command::CloseDocumentation | Command::CloseSignature | Command::CloseCodeActions)
    }
    pub fn csv_control(&self, pane: u8) -> Option<Action> {
        let tab = self.state.shown(pane)?;
        if !self.state.view(tab.id()).is_some_and(|view| is_csv(view.snapshot())) { return None; }
        Some(Action { id: CommandId::CsvTable, label: if tab.csv.is_some() { "Source" } else { "Table" }, disabled: self.csv_disabled(CommandId::CsvTable), on: None, shortcut: None })
    }
    pub(super) fn csv_disabled(&self, id: CommandId) -> Option<&'static str> {
        let Some(tab) = self.state.active() else { return Some("No file is open") };
        let Some(view) = self.state.view(tab.id()) else { return Some("No file is open") };
        if !is_csv(view.snapshot()) { return Some("Open a CSV file first"); }
        if tab.csv.as_ref().is_some_and(|table| table.input.is_some()) { return Some(PENDING_FIELD); }
        if id == CommandId::CsvTable { return None; }
        let Some(table) = tab.csv.as_ref() else { return Some("Switch CSV to Table first") };
        if table.pending { return Some("CSV table is being prepared"); }
        if table.error.is_some() { return Some("CSV cannot be shown as a table; use Source"); }
        if matches!(id, CommandId::CsvEditCell | CommandId::CsvMoveLeft | CommandId::CsvMoveRight | CommandId::CsvDropColumn) {
            if let Some(why) = view.edit_disabled() { return Some(why); }
            if !self.text_input_enabled { return Some("Text editing is gated on this unvalidated platform/backend"); }
        }
        match id {
            CommandId::CsvDropColumn if table.column_count() < 2 => Some("Keep at least one CSV column"),
            CommandId::CsvMoveLeft if table.column == 0 => Some("This is the first column"),
            CommandId::CsvMoveRight if table.column + 1 >= table.column_count() => Some("This is the last column"),
            CommandId::CsvEditCell if table.row_count() == 0 => Some("No CSV data row is selected"),
            CommandId::CsvClearFilter if table.filter.is_empty() => Some("No column filter is active"),
            _ => None,
        }
    }
    pub(super) fn csv_blocks_source(&self, command: &Command) -> bool {
        let tab = match command { Command::Edit { tab, .. } | Command::PresentText { tab, .. } => Some(*tab), _ => self.state.active };
        if !tab.is_some_and(|tab| self.state.csv(tab).is_some()) { return false; }
        matches!(command, Command::Edit { .. } | Command::PresentText { .. } | Command::InsertText { .. } | Command::Delete { .. } | Command::Cut | Command::Paste(_) | Command::Indent { .. } | Command::ToggleComment | Command::MoveLines { .. } | Command::DuplicateLines { .. } | Command::InsertSnippet { .. } | Command::Select(_) | Command::MoveCaret(_) | Command::Extend(_) | Command::SelectAll | Command::AddCursorAbove | Command::AddCursorBelow | Command::SelectNextOccurrence | Command::ExtendSelection | Command::ShrinkSelection | Command::ReplaceFind { .. } | Command::Complete | Command::Format(_) | Command::QuickFix | Command::ApplyCodeAction | Command::AcceptCompletion)
    }
    pub(super) fn sync_csv(&mut self) -> bool {
        let tabs: Vec<_> = self.state.tabs.iter().filter(|tab| tab.csv.as_ref().is_some_and(|csv| csv.input.is_none()) && self.is_shown(tab.id))
            .filter(|tab| tab.csv.as_ref().is_some_and(|csv| self.state.source(tab.id).is_some_and(|snapshot| csv.source.as_ref().is_none_or(|source| !source.matches(snapshot)))))
            .map(|tab| tab.id).collect();
        let changed = !tabs.is_empty();
        for tab in tabs { self.csv_schedule(tab, None); }
        changed
    }
    fn csv_schedule(&mut self, tab: TabId, order: Option<Vec<usize>>) {
        let Some(snapshot) = self.state.source(tab).cloned() else { return };
        if !is_csv(&snapshot) { if let Some(tab) = self.tab_mut(tab) { tab.csv = None; } return; }
        if snapshot.len_bytes() > csv::MAX_BYTES {
            if let Some(tab) = self.tab_mut(tab) { tab.csv = None; }
            self.state.messages.push("CSV table limit: 32 MiB; showing Source".into());
            return;
        }
        let source = SemanticSource::new(&snapshot);
        self.semantic_seq += 1;
        let request = self.semantic_seq;
        let Some(view) = self.tab_mut(tab).and_then(|tab| tab.csv.as_mut()) else { return };
        let same = view.source.as_ref().is_some_and(|source| source.matches(&snapshot));
        let cached = same.then(|| view.data.as_ref().map(|data| data.table.clone())).flatten();
        if !same { view.data = None; view.input = None; }
        view.source = Some(source); view.pending = true; view.error = None;
        view.request = request;
        let (request, header, filter, column, sort) = (view.request, view.header, view.filter.clone(), view.filter_column, view.sort);
        let loaded = self.state.sources.buffers.get(&snapshot.key().buffer).map(|entry| entry.document.clone()).unwrap_or_default();
        let input = crate::analysis::Input::new(snapshot, loaded, self.project_generation.load(Ordering::Acquire), true);
        if !self.analysis.request(input, crate::analysis::Consumer::Csv(tab), request, Priority::Visible, move |doc, lease| {
            let wanted = lease.cancellation();
            let table = match cached.map_or_else(|| csv::parse(doc.text().clone(), &wanted).map(Arc::new), Ok) {
                Ok(table) => table, Err(error) => { lease.send(Completion::CsvPrepared(Reply { tab, request, result: Err(error), edits: None, order: None })); return; }
            };
            let (edits, order) = if let Some(order) = order { match csv::columns(&table, &order, &wanted) {
                Some(edits) => (Some(edits), Some(order)), None => { if wanted() { lease.send(Completion::CsvPrepared(Reply { tab, request, result: Err("CSV column operations need equally wide records; use Source for uneven rows".into()), edits: None, order: None })); } return; }
            } } else { (None, None) };
            if let Some(prepared) = csv::prepare(table, header, &filter, column, sort, &wanted) {
                lease.send(Completion::CsvPrepared(Reply { tab, request, result: Ok(Arc::new(prepared)), edits, order }));
            }
        }) && let Some(view) = self.tab_mut(tab).and_then(|tab| tab.csv.as_mut()) {
            view.pending = false; view.error = Some("CSV worker is busy; switch to Source and retry Table".into());
        }
    }
    pub(super) fn csv_prepared(&mut self, reply: Reply) -> Effect {
        let Some(snapshot) = self.state.source(reply.tab).cloned() else { return Effect::default() };
        let Some(view) = self.tab_mut(reply.tab).and_then(|tab| tab.csv.as_mut())
            .filter(|view| view.request == reply.request && view.source.as_ref().is_some_and(|source| source.matches(&snapshot))) else { return Effect::default() };
        view.pending = false;
        match reply.result {
            Ok(data) => { view.data = Some(data); view.error = None; view.clamp(true); }
            Err(error) => {
                if let Some(tab) = self.tab_mut(reply.tab) { tab.csv = None; }
                self.state.messages.push(error);
                return Effect { repaint: true, ..Effect::default() };
            }
        }
        if let Some(edits) = reply.edits {
            if !self.text_input_enabled || self.state.view(reply.tab).is_none_or(|view| view.edit_disabled().is_some()) {
                self.state.messages.push("CSV columns were not changed: editing permissions changed while preparing them".into());
                return Effect { repaint: true, ..Effect::default() };
            }
            if let Some(order) = reply.order && let Some(view) = self.tab_mut(reply.tab).and_then(|tab| tab.csv.as_mut()) {
                remap(view, &order);
                view.column = order.iter().position(|column| *column == view.column).unwrap_or_else(|| view.column.min(order.len().saturating_sub(1)));
            }
            return self.edit(reply.tab, snapshot.key(), edits, crate::text::EditKind::Replace);
        }
        Effect { repaint: true, ..Effect::default() }
    }
    pub(super) fn csv_command(&mut self, command: C) -> Effect {
        let repaint = Effect { repaint: true, ..Effect::default() };
        let Some(tab) = self.state.active else { return Effect::default() };
        if command == C::Toggle {
            if self.state.csv(tab).is_some_and(|view| view.input.is_none()) {
                if let Some(buffer) = self.state.tabs.iter().find(|t| t.id == tab).and_then(|t| t.buffer) { self.analysis.cancel_consumer(buffer, crate::analysis::Consumer::Csv(tab)); }
                if let Some(tab) = self.tab_mut(tab) { tab.csv = None; }
                return repaint;
            }
            if self.csv_disabled(CommandId::CsvTable).is_some() { return Effect::default(); }
            let Some(buffer) = self.state.source(tab).map(|s| s.key().buffer) else { return Effect::default() };
            self.analysis.cancel_consumer(buffer, crate::analysis::Consumer::Csv(tab));
            let Some(tab) = self.tab_mut(tab) else { return Effect::default() };
            tab.csv = if tab.csv.is_some() { None } else { Some(csv::View::default()) };
            self.close_completion(); self.close_signature(); self.close_code_actions();
            return repaint;
        }
        if command == C::Cancel {
            if let Some(view) = self.tab_mut(tab).and_then(|tab| tab.csv.as_mut()) {
                view.input = None;
                if view.error.is_some() { view.source = None; }
            }
            return repaint;
        }
        let Some(snapshot) = self.state.source(tab).cloned() else { return Effect::default() };
        let editable = self.text_input_enabled && self.state.view(tab).is_some_and(|view| view.edit_disabled().is_none());
        let Some(view) = self.tab_mut(tab).and_then(|tab| tab.csv.as_mut()) else { return Effect::default() };
        if let C::Size { rows, columns } = command {
            let changed = view.rows != rows.max(1) || view.columns != columns.max(1);
            view.rows = rows.max(1); view.columns = columns.max(1);
            view.clamp(changed || view.input.is_some()); return Effect::default();
        }
        if view.input.is_some() && !matches!(command, C::Input(_) | C::SetInput(_) | C::Backspace | C::Commit) { return Effect::default(); }
        if view.pending || view.error.is_some() { return Effect::default(); }
        match command {
            C::Move { rows, columns } => { view.input = None; view.row = view.row.saturating_add_signed(rows); view.column = view.column.saturating_add_signed(columns); view.clamp(true); }
            C::Scroll { rows, columns } => { view.top = view.top.saturating_add_signed(rows); view.left = view.left.saturating_add_signed(columns); view.clamp(false); }
            C::Select { row, column } if (row < view.row_count() || row == 0 && view.row_count() == 0) && column < view.column_count() => { view.row = row; view.column = column; view.input = None; view.clamp(true); }
            C::BeginEdit if editable && view.row_count() > 0 => {
                let raw_len = view.data.as_ref().and_then(|data| data.rows.get(view.row).and_then(|row| data.table.cell_range(*row, view.column))).map_or(0, |range| range.len());
                if raw_len > csv::MAX_INPUT { self.state.messages.push("CSV cell editor limit: 64 KiB; edit this field in Source".into()); }
                else { view.input = Some(csv::Input { kind: InputKind::Cell, value: view.cell(view.row, view.column) }); }
            }
            C::BeginFilter => { view.filter_column = view.column; view.input = Some(csv::Input { kind: InputKind::Filter, value: view.filter.clone() }); }
            C::Input(text) => if let Some(input) = view.input.as_mut().filter(|input| input.value.len().saturating_add(text.len()) <= csv::MAX_INPUT) { input.value.push_str(&text); },
            C::SetInput(text) if text.len() <= csv::MAX_INPUT => if let Some(input) = &mut view.input { input.value = text; },
            C::Backspace => if let Some(input) = &mut view.input { input.value.pop(); },
            C::Commit => {
                if view.input.as_ref().is_some_and(|input| input.kind == InputKind::Cell) && !editable {
                    self.state.messages.push("CSV field is still pending: editing is unavailable; Escape cancels it".into());
                    return repaint;
                }
                let Some(input) = view.input.take() else { return repaint };
                if input.kind == InputKind::Filter { view.filter = input.value; view.row = 0; view.top = 0; self.csv_schedule(tab, None); }
                else if editable {
                    let Some(data) = &view.data else { return repaint };
                    let Some(&row) = data.rows.get(view.row) else { return repaint };
                    let encoded = csv::encoded(&input.value);
                    let (range, text) = if let Some(range) = data.table.cell_range(row, view.column) { (range, encoded) } else {
                        let record = &data.table.records[row];
                        (record.range.end..record.range.end, format!("{}{}", ",".repeat(view.column + 1 - record.cells.len()), encoded))
                    };
                    return self.edit(tab, snapshot.key(), vec![crate::text::TextEdit { range, text: text.into() }], crate::text::EditKind::Replace);
                }
            }
            C::ToggleHeader => { view.header = !view.header; view.row = 0; view.top = 0; view.input = None; self.csv_schedule(tab, None); }
            C::ClearFilter => { view.filter.clear(); view.row = 0; view.top = 0; view.input = None; self.csv_schedule(tab, None); }
            C::Sort => { view.sort = match view.sort { Some((column, false)) if column == view.column => Some((column, true)), Some((column, true)) if column == view.column => None, _ => Some((view.column, false)) }; view.row = 0; view.top = 0; view.input = None; self.csv_schedule(tab, None); }
            C::MoveColumn { right } if editable => {
                let target = if right { view.column.checked_add(1).filter(|column| *column < view.column_count()) } else { view.column.checked_sub(1) };
                let Some(target) = target else { return Effect::default() };
                let mut order: Vec<_> = (0..view.column_count()).collect(); order.swap(view.column, target);
                view.input = None; self.csv_schedule(tab, Some(order));
            }
            C::DropColumn if editable && view.column_count() > 1 => {
                let order: Vec<_> = (0..view.column_count()).filter(|column| *column != view.column).collect();
                view.input = None; self.csv_schedule(tab, Some(order));
            }
            C::Toggle | C::Size { .. } | C::Cancel | C::Select { .. } | C::BeginEdit | C::SetInput(_) | C::MoveColumn { .. } | C::DropColumn => {}
        }
        repaint
    }
}
fn remap(view: &mut csv::View, order: &[usize]) {
    if let Some((column, descending)) = view.sort { view.sort = order.iter().position(|current| *current == column).map(|column| (column, descending)); }
    if let Some(column) = order.iter().position(|current| *current == view.filter_column) { view.filter_column = column; }
    else { view.filter.clear(); view.filter_column = 0; }
}
fn is_csv(snapshot: &crate::text::TextSnapshot) -> bool { snapshot.source().extension().is_some_and(|extension| extension.eq_ignore_ascii_case("csv")) && snapshot.image().is_none() }
