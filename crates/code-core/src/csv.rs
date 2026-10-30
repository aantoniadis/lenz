//! CSV table projection. Raw fields remain slices of one immutable source; only visible cells are decoded.
use std::{borrow::Cow, ops::Range, sync::Arc};

pub const MAX_BYTES: usize = 32 * 1024 * 1024;
const MAX_CELLS: usize = 2_000_000;
const MAX_ROWS: usize = 200_000;
pub const MAX_INPUT: usize = 64 * 1024;

#[derive(Clone, Debug)]
pub struct Cell { pub range: Range<usize>, quoted: bool }
#[derive(Clone, Debug)]
pub struct Record { pub range: Range<usize>, pub cells: Range<usize> }
#[derive(Debug)]
pub struct Table { pub source: Arc<str>, pub records: Vec<Record>, cells: Vec<Cell>, pub columns: usize }
impl Table {
    pub fn bytes(&self) -> usize { self.source.len() + self.records.capacity() * std::mem::size_of::<Record>() + self.cells.capacity() * std::mem::size_of::<Cell>() }
    pub fn value(&self, row: usize, column: usize) -> Cow<'_, str> {
        let Some(cell) = self.records.get(row).and_then(|row| self.cells.get(row.cells.clone()).and_then(|cells| cells.get(column))) else { return Cow::Borrowed("") };
        let Some(raw) = self.source.get(cell.range.clone()) else { return Cow::Borrowed("") };
        if !cell.quoted { return Cow::Borrowed(raw); }
        let value = raw.get(1..raw.len().saturating_sub(1)).unwrap_or_default();
        if value.contains("\"\"") { Cow::Owned(value.replace("\"\"", "\"")) } else { Cow::Borrowed(value) }
    }
    pub fn cell_range(&self, row: usize, column: usize) -> Option<Range<usize>> {
        let record = self.records.get(row)?;
        self.cells.get(record.cells.clone())?.get(column).map(|cell| cell.range.clone())
    }
    fn preview(&self, row: usize, column: usize, maximum: usize) -> String {
        let raw = self.raw(row, column);
        let quoted = raw.starts_with('"');
        let mut chars = if quoted { raw.get(1..raw.len().saturating_sub(1)).unwrap_or_default().chars() } else { raw.chars() }.peekable();
        let mut shown = String::new();
        for _ in 0..maximum {
            let Some(c) = chars.next() else { return shown; };
            if quoted && c == '"' && chars.peek() == Some(&'"') { chars.next(); }
            shown.push(match c { '\n' | '\r' => '↵', '\t' => ' ', c if c.is_control() => '�', c => c });
        }
        if chars.peek().is_some() { shown.push('…'); }
        shown
    }
    fn raw(&self, row: usize, column: usize) -> &str { self.cell_range(row, column).and_then(|range| self.source.get(range)).unwrap_or_default() }
}

pub fn parse(source: Arc<str>, wanted: impl Fn() -> bool) -> Result<Table, String> {
    if source.len() > MAX_BYTES { return Err("CSV table limit: 32 MiB; use Source".into()); }
    let bytes = source.as_bytes();
    let (mut records, mut cells) = (Vec::new(), Vec::new());
    let (mut at, mut columns) = (usize::from(source.starts_with('\u{feff}')) * 3, 0);
    let mut next_check = 4096;
    while at < bytes.len() {
        if !wanted() { return Err("CSV table cancelled".into()); }
        let row_start = at;
        let first = cells.len();
        loop {
            if cells.len() >= MAX_CELLS || records.len() >= MAX_ROWS { return Err("CSV table limit: 200,000 rows or 2,000,000 cells; use Source".into()); }
            let start = at;
            let quoted = bytes.get(at) == Some(&b'"');
            if quoted {
                at += 1;
                loop {
                    match bytes.get(at) {
                        None => return Err(format!("CSV: unclosed quote in record {}; use Source", records.len() + 1)),
                        Some(b'"') if bytes.get(at + 1) == Some(&b'"') => at += 2,
                        Some(b'"') => { at += 1; break; }
                        Some(_) => at += 1,
                    }
                    if at >= next_check { if !wanted() { return Err("CSV table cancelled".into()); } next_check = at.saturating_add(4096); }
                }
                if bytes.get(at).is_some_and(|byte| !matches!(byte, b',' | b'\r' | b'\n')) { return Err(format!("CSV: text after closing quote in record {}; use Source", records.len() + 1)); }
            } else {
                while bytes.get(at).is_some_and(|byte| !matches!(byte, b',' | b'\r' | b'\n')) {
                    if bytes[at] == b'"' { return Err(format!("CSV: quote inside an unquoted field in record {}; use Source", records.len() + 1)); }
                    at += 1;
                    if at >= next_check { if !wanted() { return Err("CSV table cancelled".into()); } next_check = at.saturating_add(4096); }
                }
            }
            cells.push(Cell { range: start..at, quoted });
            if bytes.get(at) == Some(&b',') { at += 1; continue; }
            records.push(Record { range: row_start..at, cells: first..cells.len() });
            columns = columns.max(cells.len() - first);
            if bytes.get(at) == Some(&b'\r') { at += 1; if bytes.get(at) == Some(&b'\n') { at += 1; } }
            else if bytes.get(at) == Some(&b'\n') { at += 1; }
            break;
        }
    }
    Ok(Table { source, records, cells, columns })
}

#[derive(Debug)]
pub struct Prepared { pub table: Arc<Table>, pub rows: Vec<usize> }
pub fn prepare(table: Arc<Table>, header: bool, filter: &str, filter_column: usize, sort: Option<(usize, bool)>, wanted: impl Fn() -> bool) -> Option<Prepared> {
    let mut rows = Vec::with_capacity(table.records.len());
    for row in usize::from(header)..table.records.len() {
        if row % 256 == 0 && !wanted() { return None; }
        if filter.is_empty() || table.value(row, filter_column).contains(filter) { rows.push(row); }
    }
    if let Some((column, descending)) = sort {
        let mut keyed = Vec::with_capacity(rows.len());
        for (index, row) in rows.iter().copied().enumerate() {
            if index % 256 == 0 && !wanted() { return None; }
            keyed.push((row, table.value(row, column)));
        }
        let (mut comparisons, mut obsolete) = (0usize, false);
        // Stable sorting preserves source order on ties; keys are decoded once, away from input/rendering.
        keyed.sort_by(|a, b| {
            comparisons += 1;
            if comparisons % 1024 == 0 && !wanted() { obsolete = true; }
            let order = a.1.cmp(&b.1);
            if descending { order.reverse() } else { order }
        });
        if obsolete || !wanted() { return None; }
        // Reuse compact indices; in-place collect would retain the larger sort-key allocation.
        rows.clear();
        rows.extend(keyed.into_iter().map(|(row, _)| row));
    }
    wanted().then_some(Prepared { table, rows })
}

pub fn encoded(value: &str) -> String {
    if value.contains([',', '"', '\r', '\n']) { format!("\"{}\"", value.replace('"', "\"\"")) } else { value.into() }
}
/// Reorder/drop columns without rewriting raw fields or line endings, as one undoable transaction.
pub fn columns(table: &Table, order: &[usize], wanted: impl Fn() -> bool) -> Option<Vec<crate::text::TextEdit>> {
    if table.records.iter().any(|record| record.cells.len() != table.columns) || order.iter().any(|column| *column >= table.columns) { return None; }
    let mut text = String::with_capacity(table.source.len());
    let mut through = 0;
    for (row, record) in table.records.iter().enumerate() {
        if row % 256 == 0 && !wanted() { return None; }
        text.push_str(table.source.get(through..record.range.start)?);
        for (index, column) in order.iter().enumerate() {
            if index != 0 { text.push(','); }
            text.push_str(table.raw(row, *column));
        }
        through = record.range.end;
    }
    text.push_str(table.source.get(through..)?);
    if text == *table.source { return Some(Vec::new()); }
    Some(vec![crate::text::TextEdit { range: 0..table.source.len(), text: text.into() }])
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Toggle, ToggleHeader, Select { row: usize, column: usize }, Move { rows: isize, columns: isize },
    Scroll { rows: isize, columns: isize }, Size { rows: usize, columns: usize },
    BeginEdit, BeginFilter, ClearFilter, Input(String), SetInput(String), Backspace, Commit, Cancel, Sort,
    MoveColumn { right: bool }, DropColumn,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputKind { Cell, Filter }
#[derive(Clone, Debug)]
pub struct Input { pub kind: InputKind, pub value: String }
#[derive(Clone)]
pub struct View {
    pub pending: bool, pub error: Option<String>, pub row: usize, pub column: usize,
    pub top: usize, pub left: usize, pub rows: usize, pub columns: usize,
    pub header: bool, pub filter: String, pub sort: Option<(usize, bool)>, pub input: Option<Input>,
    pub(crate) filter_column: usize,
    pub(crate) data: Option<Arc<Prepared>>, pub(crate) source: Option<crate::session::SemanticSource>,
    pub(crate) request: u64,
}
impl Default for View {
    fn default() -> Self { Self { pending: true, error: None, row: 0, column: 0, top: 0, left: 0, rows: 20, columns: 5, header: true, filter: String::new(), sort: None, input: None, filter_column: 0, data: None, source: None, request: 0 } }
}
impl View {
    pub fn revision(&self) -> u64 { self.request }
    pub fn row_count(&self) -> usize { self.data.as_ref().map_or(0, |data| data.rows.len()) }
    pub fn column_count(&self) -> usize { self.data.as_ref().map_or(0, |data| data.table.columns) }
    pub fn heading(&self, column: usize) -> String {
        if self.header { self.data.as_ref().map_or_else(String::new, |data| data.table.value(0, column).into_owned()) }
        else { format!("Column {}", column + 1) }
    }
    pub fn heading_preview(&self, column: usize, maximum: usize) -> String {
        if self.header { self.data.as_ref().map_or_else(String::new, |data| data.table.preview(0, column, maximum)) }
        else { format!("Column {}", column + 1) }
    }
    pub fn cell_preview(&self, row: usize, column: usize, maximum: usize) -> String {
        self.data.as_ref().and_then(|data| data.rows.get(row).map(|row| data.table.preview(*row, column, maximum))).unwrap_or_default()
    }
    pub fn cell(&self, row: usize, column: usize) -> String { self.data.as_ref().and_then(|data| data.rows.get(row).map(|row| data.table.value(*row, column).into_owned())).unwrap_or_default() }
    pub fn status(&self) -> String {
        if let Some(error) = &self.error { return error.clone(); }
        if self.pending { return "Preparing CSV table… Source remains available".into(); }
        if self.row_count() == 0 { return format!("No data rows · {} columns · Header / Clear Filter / Source in commands", self.column_count()); }
        format!("{} rows × {} columns · row {}, column {} · Enter edit · / filter · s sort · Source in commands", self.row_count(), self.column_count(), self.row + 1, self.column + 1)
    }
    pub(crate) fn clamp(&mut self, reveal: bool) {
        self.row = self.row.min(self.row_count().saturating_sub(1)); self.column = self.column.min(self.column_count().saturating_sub(1));
        self.top = self.top.min(self.row_count().saturating_sub(self.rows.max(1))); self.left = self.left.min(self.column_count().saturating_sub(self.columns.max(1)));
        if reveal {
            if self.row < self.top { self.top = self.row; } else if self.row >= self.top + self.rows.max(1) { self.top = self.row + 1 - self.rows.max(1); }
            if self.column < self.left { self.left = self.column; } else if self.column >= self.left + self.columns.max(1) { self.left = self.column + 1 - self.columns.max(1); }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quoted_multiline_unicode_and_line_endings_survive_cell_and_column_edits() {
        let source = "name,note,empty\r\nλ,\"two\nlines with \"\"quotes\"\"\",\r\nx,last,";
        let table = Arc::new(parse(source.into(), || true).unwrap());
        assert_eq!(table.value(1, 1), "two\nlines with \"quotes\"");
        assert_eq!(table.value(1, 2), "");
        let projected = prepare(table.clone(), true, "lines", 1, None, || true).unwrap();
        assert_eq!(projected.rows, [1]);
        let edits = columns(&table, &[1, 0], || true).unwrap();
        let doc = crate::document::Document::decode("test.csv".into(), 1, source.as_bytes());
        let snapshot = crate::text::TextSnapshot::from_document(crate::text::BufferId(1), &doc).apply_edits(2, &edits).unwrap();
        assert_eq!(snapshot.copy_range(0..snapshot.len_bytes()).unwrap(), "note,name\r\n\"two\nlines with \"\"quotes\"\"\",λ\r\nlast,x");
        assert_eq!(encoded("a,\"b\n"), "\"a,\"\"b\n\"");
    }
    #[test]
    fn escaped_quote_scans_cancel_and_previews_never_decode_entire_large_cells() {
        let source = format!("h\n\"{}\"\n", "\"\"".repeat(100_000));
        let calls = std::cell::Cell::new(0);
        assert!(parse(source.clone().into(), || { calls.set(calls.get() + 1); calls.get() < 3 }).unwrap_err().contains("cancelled"));
        assert_eq!(calls.get(), 3, "escaped quotes cannot jump over every cancellation boundary");
        let table = Arc::new(parse(source.into(), || true).unwrap());
        let ready = prepare(table.clone(), true, "", 0, None, || true).unwrap();
        let view = View { data: Some(Arc::new(ready)), pending: false, ..View::default() };
        assert_eq!(view.cell_preview(0, 0, 4), "\"\"\"\"…");
        let ragged = parse("a,b\nx\n".into(), || true).unwrap();
        assert!(columns(&ragged, &[1, 0], || true).is_none(), "column operations cannot pad unrelated uneven rows");
    }
    #[test]
    fn malformed_source_is_refused_and_sort_is_stable_and_view_only() {
        for source in ["a,\"broken", "a,un\"quoted", "\"a\"oops,b"] { assert!(parse(source.into(), || true).is_err()); }
        let table = Arc::new(parse("h,id\na,first\nb,second\na,third\n".into(), || true).unwrap());
        assert_eq!(prepare(table.clone(), true, "", 0, Some((0, false)), || true).unwrap().rows, [1,3,2]);
        assert_eq!(prepare(table.clone(), true, "", 0, Some((0, true)), || true).unwrap().rows, [2,1,3]);
        assert_eq!(&*table.source, "h,id\na,first\nb,second\na,third\n");
        assert!(parse("a,b".into(), || false).is_err());
    }
}
