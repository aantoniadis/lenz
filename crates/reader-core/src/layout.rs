//! Byte-budgeted logical wrapping. Native font/cell measurement stays in the adapters.

use std::{collections::{HashMap, VecDeque}, ops::Range, sync::Arc};
use unicode_segmentation::UnicodeSegmentation;
use crate::{decor::Hints, projection::{Cursor, Window, WINDOW_WORK}, text::TextSnapshot};

const CHECKPOINTS: usize = 128;
const KEPT_ROWS: usize = 4096;
const KEPT_POINTS: usize = 128;
const CACHE_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub line: usize,
    pub index: usize,
    pub display: Range<usize>,
    pub source: Range<usize>,
    pub indent: usize,
    pub complete: bool,
    pub last: bool,
    pub(crate) column: usize,
    pub(crate) width: usize,
    pub(crate) end_caret: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Position {
    pub line: usize,
    pub row: usize,
    pub display: usize,
    pub column: usize,
}

#[derive(Clone, Debug)]
pub enum Query<T> { Ready(T), Pending(Work) }

#[derive(Debug, Default)]
pub struct Rows { pub rows: Vec<Row>, pub pending: Option<Work> }

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Request { pub pane: u8, pub tab: crate::tabs::TabId, pub work: Work }

/// Opaque result from the shared layout lane; only Session may publish it.
#[derive(Debug)]
pub struct Finished {
    pub(crate) pane: u8,
    pub(crate) ticket: u64,
    pub(crate) request: Request,
    pub(crate) delta: Option<Delta>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Key { seed: Cursor, cols: Option<usize> }

#[derive(Clone, Debug, PartialEq, Eq)]
struct Break {
    display: usize,
    source: usize,
    column: usize,
    last_source: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Scan {
    cursor: Cursor,
    pending: String,
    pending_source: usize,
    processed: usize,
    processed_source: usize,
    column: usize,
    row: usize,
    row_display: usize,
    row_source: usize,
    row_column: usize,
    used: usize,
    indent: usize,
    leading: bool,
    last_break: Option<Break>,
    after_break: usize,
    last_source: usize,
    work: usize,
    complete: bool,
}

impl Scan {
    fn new(cursor: Cursor) -> Self {
        let source = cursor.source_byte();
        Self { cursor, pending: String::new(), pending_source: source, processed: 0, processed_source: source,
            column: 0, row: 0, row_display: 0, row_source: source, row_column: 0, used: 0, indent: 0,
            leading: true, last_break: None, after_break: 0, last_source: source, work: 0, complete: false }
    }

    fn row(&self, line: usize, end: usize, source: usize, complete: bool, last: bool) -> Row {
        Row { line, index: self.row, display: self.row_display..end, source: self.row_source..source,
            indent: if self.row == 0 { 0 } else { self.indent }, complete, last,
            column: self.row_column, width: self.column - self.row_column, end_caret: self.last_source }
    }

    fn cluster(&mut self, line: usize, text: &str, display: Range<usize>, source: Range<usize>, cols: usize) -> Option<Row> {
        let width = lenz_content::wrap::width(text);
        let hanging = text.chars().next().is_some_and(char::is_whitespace);
        let mut can_break = false;
        for c in text.chars() {
            if self.leading && matches!(c, ' ' | '\t') {
                self.indent = (self.indent + lenz_content::wrap::width(c.encode_utf8(&mut [0; 4]))).min(cols / 2);
            } else {
                self.leading = false;
                can_break |= c.is_whitespace() || matches!(c, ',' | ';' | '.' | ')' | ']' | '}' | '>' | '-' | '/' | '|' | '&');
            }
        }
        let room = if self.row == 0 { cols } else { cols.saturating_sub(self.indent).max(1) };
        let mut emitted = None;
        if self.used.saturating_add(width) > room && display.start > self.row_display && !hanging {
            let at = self.last_break.clone().filter(|b| b.display > self.row_display && b.display <= display.start)
                .unwrap_or(Break { display: display.start, source: source.start, column: self.column, last_source: self.last_source });
            let mut row = self.row(line, at.display, at.source, true, false);
            row.width = at.column - self.row_column;
            row.end_caret = at.last_source;
            emitted = Some(row);
            self.row += 1;
            self.row_display = at.display;
            self.row_source = at.source;
            self.row_column = at.column;
            self.used = if self.last_break.is_some() { self.after_break } else { 0 };
            self.last_break = None;
            self.after_break = 0;
        }
        self.used += width;
        self.column += width;
        self.processed = display.end;
        self.processed_source = source.end;
        self.last_source = source.start;
        if can_break {
            self.last_break = Some(Break { display: display.end, source: source.end, column: self.column, last_source: source.start });
            self.after_break = 0;
        } else if self.last_break.is_some() { self.after_break += width; }
        emitted
    }

    fn consume(&mut self, window: &Window, cols: usize) -> Vec<Row> {
        let base = self.processed;
        let mut text = std::mem::take(&mut self.pending);
        text.push_str(&window.shown.text);
        let mut starts: Vec<usize> = text.grapheme_indices(true).map(|(i, _)| i).collect();
        starts.push(text.len());
        let stable = if window.complete { starts.len().saturating_sub(1) } else { starts.len().saturating_sub(2) };
        let mut rows = Vec::new();
        for pair in starts.windows(2).take(stable) {
            let range = base + pair[0]..base + pair[1];
            let start = if range.start < window.display.start { self.pending_source }
                else { window.source_at(range.start - window.display.start) };
            let end = if range.end < window.display.start { self.pending_source }
                else { window.source_at(range.end - window.display.start) };
            if let Some(row) = self.cluster(window.line, text.get(pair[0]..pair[1]).unwrap_or_default(), range, start..end, cols) { rows.push(row); }
        }
        let held = starts.get(stable).copied().unwrap_or(text.len());
        self.pending = text.get(held..).unwrap_or_default().to_owned();
        self.pending_source = if base + held < window.display.start { self.pending_source }
            else { window.source_at(base + held - window.display.start) };
        self.cursor = window.next().clone();
        self.work += window.work;
        self.complete = window.complete;
        if window.complete {
            rows.push(self.row(window.line, self.processed, window.source.end, true, true));
        }
        rows
    }

    fn bytes(&self) -> usize { self.pending.capacity() + self.cursor.context_bytes() }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Target {
    Rows { first: usize, count: usize }, End, Byte(usize), Display(usize), Cursor(usize), Column(usize),
}

/// Immutable resumable work. It owns guard/checkpoint state, not authoritative text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Work { inner: Arc<WorkState> }

#[derive(Clone, Debug, PartialEq, Eq)]
struct WorkState { key: Key, line: usize, scan: Scan, target: Target }

#[derive(Debug)]
pub struct Delta {
    key: Key,
    line: usize,
    scan: Scan,
    checkpoints: Vec<Scan>,
    rows: Vec<Row>,
    byte: Option<(usize, usize)>,
    display: Option<(usize, Cursor, usize)>,
    column: Option<(usize, usize)>,
    target: Target,
    work: usize,
}

impl Delta { pub(crate) fn work(&self) -> usize { self.work } }

impl Work {
    pub fn columns(&self) -> Option<usize> { self.inner.key.cols }
    pub fn matches(&self, snapshot: &TextSnapshot, cols: Option<usize>, hints: Option<&Arc<Hints>>) -> bool {
        self.inner.key.cols == cols && self.inner.key.seed.matches(snapshot, hints)
    }

    /// Bounded worker work; cancellation is checked between independently bounded projection windows.
    pub fn run(self, snapshot: &TextSnapshot, budget: usize, current: impl Fn() -> bool) -> Option<Delta> {
        (*self.inner).clone().run(snapshot, budget, current)
    }
}

impl WorkState {
    fn run(mut self, snapshot: &TextSnapshot, budget: usize, current: impl Fn() -> bool) -> Option<Delta> {
        let budget = budget.min(16 * WINDOW_WORK);
        if budget < crate::projection::MIN_WORK || !self.key.seed.matches(snapshot, self.key.seed.hints()) { return None; }
        let mut delta = Delta { key: self.key.clone(), line: self.line, scan: self.scan.clone(), checkpoints: Vec::new(), rows: Vec::new(), byte: None, display: None, column: None, target: self.target.clone(), work: 0 };
        let mut spent = 0;
        let mut first = true;
        while spent < budget && (!self.scan.complete || first) && current() {
            first = false;
            let before = self.scan.cursor.clone();
            let available = budget.saturating_sub(spent).min(WINDOW_WORK);
            if available < crate::projection::MIN_WORK { break; }
            let window = self.scan.cursor.window(snapshot, available)?;
            if window.work == 0 && !window.complete { return None; }
            match self.target {
                Target::Byte(byte) if window.source.start <= byte && byte <= window.source.end => {
                    if let Some(display) = window.caret_display(byte) { delta.byte = Some((byte, display)); }
                }
                Target::Display(display) | Target::Cursor(display) if window.display.start <= display && display <= window.display.end => {
                    let at = display.saturating_sub(window.display.start).min(window.shown.text.len());
                    let column = window.columns.start + window.shown.text.get(..at).map_or(0, lenz_content::wrap::width);
                    delta.display = Some((display, before.clone(), column));
                }
                Target::Column(target) if window.columns.start <= target && target <= window.columns.end => {
                    let mut column = window.columns.start;
                    let mut at = window.shown.text.len();
                    for (i, c) in window.shown.text.char_indices() {
                        let width = lenz_content::wrap::width(c.encode_utf8(&mut [0; 4]));
                        if column >= target || column + width > target { at = i; break; }
                        column += width;
                    }
                    delta.column = Some((target, window.source_at(at)));
                }
                _ => {}
            }
            let rows = self.scan.consume(&window, self.key.cols.unwrap_or(usize::MAX).max(1));
            for row in rows {
                let wanted = match self.target {
                    Target::Rows { first, count } => row.index >= first.saturating_sub((KEPT_ROWS - count).min(KEPT_ROWS / 2)),
                    Target::Display(display) => contains_display(&row, display),
                    Target::End => row.last,
                    Target::Byte(_) | Target::Cursor(_) | Target::Column(_) => false,
                };
                if wanted && delta.rows.len() < KEPT_ROWS { delta.rows.push(row); }
            }
            spent += window.work.max(1);
            delta.checkpoints.push(self.scan.clone());
            let done = match self.target {
                Target::Rows { first, count } => self.scan.row >= first.saturating_add(count) || self.scan.complete,
                Target::End => self.scan.complete,
                Target::Byte(_) => delta.byte.is_some(),
                Target::Cursor(_) => delta.display.is_some(),
                Target::Display(_) => !delta.rows.is_empty() || self.key.cols.is_none() && delta.display.is_some(),
                Target::Column(_) => delta.column.is_some(),
            };
            if done { break; }
        }
        if !current() { return None; }
        delta.scan = self.scan;
        delta.work = spent;
        Some(delta)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{decor::{Hint, HintKind}, document::Document, text::BufferId};

    fn snapshot(text: &str) -> TextSnapshot {
        TextSnapshot::from_document(BufferId(1), &Document::decode("bounded.txt".into(), 1, text.as_bytes()))
    }

    #[test]
    fn cold_work_yields_inside_whitespace_hidden_marks_and_hint_labels() {
        let mut first_prefix = None;
        for len in [100_000, 1_000_000] {
            let source = snapshot(&" ".repeat(len));
            let mut cache = Cache::default(); cache.configure(&source, Some(10), None);
            let Query::Pending(work) = cache.row(&source, 0, 0, 10) else { panic!("cold row") };
            let delta = work.run(&source, usize::MAX, || true).unwrap();
            assert!(delta.work <= 16 * WINDOW_WORK && !delta.scan.complete && delta.scan.cursor.source_byte() < len);
            assert!(delta.scan.pending.len() <= 257);
            cache.apply(delta);
            let prefix = cache.prefix(&source, 0, 0).unwrap();
            assert!(!prefix.complete && !prefix.last);
            if let Some(previous) = &first_prefix { assert_eq!(&prefix, previous); } else { first_prefix = Some(prefix); }
        }

        for hints in [false, true] {
            let source = snapshot(if hints { "ab" } else { "a" });
            let hints = hints.then(|| Arc::new(Hints::new(vec![(0, Hint { byte: 0, label: "hint ".repeat(200_000).into(), kind: HintKind::Parameter })])));
            let marks = hints.is_none().then(|| snapshot(&("a".to_string() + &"\u{301}".repeat(500_000))));
            let source = marks.as_ref().unwrap_or(&source);
            let mut cache = Cache::default(); cache.configure(source, Some(10), hints.as_ref());
            let Query::Pending(work) = cache.last(source, 0) else { panic!("cold end") };
            let delta = work.clone().run(source, 256, || true).unwrap();
            assert!(delta.work <= 256 && !delta.scan.complete);
            assert!(work.run(source, 256, || false).is_none());
            assert!(cache.apply(delta));
        }
    }

    #[test]
    fn row_metadata_and_query_continuations_stay_bounded() {
        let source = snapshot(&"x".repeat(200_000));
        let mut cache = Cache::default(); cache.configure(&source, Some(1), None);
        for _ in 0..64 {
            match cache.last(&source, 0) {
                Query::Ready(last) => { assert_eq!(last.unwrap().index, 199_999); break; }
                Query::Pending(work) => { cache.apply(work.run(&source, usize::MAX, || true).unwrap()); }
            }
        }
        assert_eq!(cache.lines[&0].last.as_ref().unwrap().index, 199_999);
        assert_eq!(cache.lines[&0].rows.len(), 1, "End must not retain every preceding row");
        for index in 0..6000 {
            loop {
                match cache.row(&source, 0, index, 32) {
                    Query::Ready(Some(row)) => { assert_eq!(row.index, index); break; }
                    Query::Ready(None) => panic!("existing row"),
                    Query::Pending(work) => { cache.apply(work.run(&source, WINDOW_WORK, || true).unwrap()); }
                }
            }
        }
        let held = &cache.lines[&0];
        assert!(held.rows.len() <= KEPT_ROWS && held.checkpoints.len() <= CHECKPOINTS && held.queries.len() <= 16);
        assert!(held.bytes() < CACHE_BYTES);
        let mut work_after_first_seek = 0;
        for (round, index) in (195_000..195_500).chain((195_000..195_500).rev()).enumerate() {
            loop {
                match cache.row(&source, 0, index, 32) {
                    Query::Ready(Some(row)) => { assert_eq!(row.index, index); break; }
                    Query::Ready(None) => panic!("existing row"),
                    Query::Pending(work) => {
                        let delta = work.run(&source, WINDOW_WORK, || true).unwrap();
                        if round > 0 { work_after_first_seek += delta.work; }
                        cache.apply(delta);
                    }
                }
            }
        }
        assert!(work_after_first_seek <= 2 * WINDOW_WORK, "bidirectional scrolling must reuse the bounded row neighborhood: {work_after_first_seek}");
    }

    #[test]
    fn stale_layout_work_cannot_publish_after_width_or_metadata_changes() {
        let source = snapshot(&"word ".repeat(10_000));
        let mut cache = Cache::default(); cache.configure(&source, Some(10), None);
        let Query::Pending(work) = cache.last(&source, 0) else { panic!("cold end") };
        let delta = work.clone().run(&source, WINDOW_WORK, || true).unwrap();
        cache.configure(&source, Some(20), None);
        assert!(!cache.apply(delta));
        assert!(!work.matches(&source, Some(20), None));
        let renamed = TextSnapshot::from_document(BufferId(1), &Document::decode("renamed.txt".into(), 1, b"word"));
        assert_eq!(renamed.key(), source.key());
        assert!(!work.matches(&renamed, Some(10), None));
        assert!(work.run(&renamed, WINDOW_WORK, || true).is_none());
    }
}

fn contains_display(row: &Row, display: usize) -> bool {
    row.display.contains(&display) || row.last && display == row.display.end
}

struct Line {
    checkpoints: Vec<Scan>,
    progress: Scan,
    queries: VecDeque<(Target, Scan)>,
    stride: usize,
    rows: VecDeque<Row>,
    points: VecDeque<(usize, usize)>,
    displays: VecDeque<(usize, Cursor, usize)>,
    columns: VecDeque<(usize, usize)>,
    last: Option<Row>,
    touched: u64,
}

impl Line {
    fn new(cursor: Cursor, touched: u64) -> Self {
        let scan = Scan::new(cursor);
        Self { checkpoints: vec![scan.clone()], progress: scan, queries: VecDeque::new(), stride: WINDOW_WORK, rows: VecDeque::new(), points: VecDeque::new(), displays: VecDeque::new(), columns: VecDeque::new(), last: None, touched }
    }
    fn bytes(&self) -> usize {
        self.checkpoints.capacity() * std::mem::size_of::<Scan>() + self.checkpoints.iter().map(Scan::bytes).sum::<usize>()
            + self.rows.capacity() * std::mem::size_of::<Row>()
            + self.displays.capacity() * std::mem::size_of::<(usize, Cursor, usize)>()
            + self.displays.iter().map(|(_, c, _)| c.context_bytes()).sum::<usize>()
            + (self.points.capacity() + self.columns.capacity()) * 16 + std::mem::size_of::<Self>()
            + self.queries.capacity() * std::mem::size_of::<(Target, Scan)>() + self.queries.iter().map(|(_, s)| s.bytes()).sum::<usize>() + self.progress.bytes()
    }
    fn checkpoint(&self, target: &Target) -> Scan {
        if let Some((_, scan)) = self.queries.iter().rev().find(|(t, s)| t == target && match target {
            Target::Rows { first, .. } => s.row <= *first && !s.complete,
            Target::End => true,
            Target::Byte(byte) => s.cursor.source_byte() <= *byte,
            Target::Display(display) => s.row_display <= *display && (!s.complete || s.cursor.display_byte() == *display),
            Target::Cursor(display) => s.cursor.display_byte() <= *display,
            Target::Column(column) => s.cursor.logical_column() <= *column,
        }) { return scan.clone(); }
        self.checkpoints.iter().chain(std::iter::once(&self.progress)).chain(self.queries.iter().map(|(_, scan)| scan)).filter(|s| match target {
            Target::Rows { first, .. } => s.row <= *first,
            Target::End => true,
            Target::Byte(byte) => s.cursor.source_byte() <= *byte,
            Target::Display(display) | Target::Cursor(display) => s.cursor.display_byte() <= *display,
            Target::Column(column) => s.cursor.logical_column() <= *column,
        }).max_by_key(|s| s.work).unwrap_or(&self.checkpoints[0]).clone()
    }
}

#[derive(Default)]
pub struct Cache {
    key: Option<Key>,
    lines: HashMap<usize, Line>,
    clock: u64,
    steps: VecDeque<Step>,
}

struct Step { visible: Arc<crate::folds::Visible>, from: (usize, usize), delta: isize, at: (usize, usize), left: usize }

impl Cache {
    pub fn configure(&mut self, snapshot: &TextSnapshot, cols: Option<usize>, hints: Option<&Arc<Hints>>) {
        if self.key.as_ref().is_some_and(|k| k.cols == cols && k.seed.matches(snapshot, hints)) { return; }
        *self = Self { key: Cursor::start(snapshot, 0, hints.cloned()).map(|seed| Key { seed, cols }), ..Self::default() };
    }

    pub fn clear(&mut self) { *self = Self::default(); }

    /// Exact multi-line stepping with bounded metadata work as well as bounded source work.
    pub fn step(&mut self, snapshot: &TextSnapshot, visible: &Arc<crate::folds::Visible>, from: (usize, usize), delta: isize) -> Query<(usize, usize)> {
        let old = self.steps.iter().position(|step| step.from == from && step.delta == delta && Arc::ptr_eq(&step.visible, visible)).and_then(|at| self.steps.remove(at));
        let mut step = old.unwrap_or_else(|| Step { visible: visible.clone(), from, delta, at: from, left: delta.unsigned_abs() });
        for _ in 0..1024 {
            if step.left == 0 { return Query::Ready(step.at); }
            let (line, row) = step.at;
            let target = if delta > 0 {
                match self.row(snapshot, line, row.saturating_add(1), 32) {
                    Query::Ready(Some(_)) => Query::Ready((line, row + 1)),
                    Query::Ready(None) => { let next = visible.step(line, 1); Query::Ready(if next == line { step.at } else { (next, 0) }) }
                    Query::Pending(work) => Query::Pending(work),
                }
            } else if row > 0 { Query::Ready((line, row - 1)) }
            else {
                let previous = visible.step(line, -1);
                if previous == line { Query::Ready(step.at) }
                else { match self.last(snapshot, previous) {
                    Query::Ready(last) => Query::Ready((previous, last.map_or(0, |row| row.index))),
                    Query::Pending(work) => Query::Pending(work),
                } }
            };
            match target {
                Query::Ready(at) if at == step.at => return Query::Ready(at),
                Query::Ready(at) => { step.at = at; step.left -= 1; }
                Query::Pending(work) => { self.keep_step(step); return Query::Pending(work); }
            }
        }
        if step.left == 0 { return Query::Ready(step.at); }
        let work = self.work(snapshot, step.at.0, Target::Rows { first: step.at.1, count: 1 });
        let at = step.at;
        self.keep_step(step);
        match work { Some(work) => Query::Pending(work), None => Query::Ready(at) }
    }

    fn keep_step(&mut self, step: Step) {
        self.steps.push_back(step);
        if self.steps.len() > 16 { self.steps.pop_front(); }
    }

    fn line(&mut self, snapshot: &TextSnapshot, line: usize) -> Option<&mut Line> {
        let key = self.key.as_ref()?;
        self.clock = self.clock.wrapping_add(1);
        if !self.lines.contains_key(&line) {
            // Even abandoned cold queries must not retain one scanner per source line.
            if self.lines.len() >= 128 && let Some(oldest) = self.lines.iter().min_by_key(|(_, held)| held.touched).map(|(line, _)| *line) { self.lines.remove(&oldest); }
            self.lines.insert(line, Line::new(Cursor::start(snapshot, line, key.seed.hints().cloned())?, self.clock));
        }
        let held = self.lines.get_mut(&line)?;
        held.touched = self.clock;
        Some(held)
    }

    fn work(&mut self, snapshot: &TextSnapshot, line: usize, target: Target) -> Option<Work> {
        let key = self.key.clone()?;
        let scan = self.line(snapshot, line)?.checkpoint(&target);
        Some(Work { inner: Arc::new(WorkState { key, line, scan, target }) })
    }

    pub fn apply(&mut self, delta: Delta) -> bool {
        if self.key.as_ref() != Some(&delta.key) { return false; }
        let Some(line) = self.lines.get_mut(&delta.line) else { return false };
        for scan in delta.checkpoints {
            if scan.work > line.progress.work { line.progress = scan.clone(); }
            if line.checkpoints.last().is_none_or(|s| scan.work >= s.work.saturating_add(line.stride)) { line.checkpoints.push(scan); }
            if line.checkpoints.len() > CHECKPOINTS {
                let last = line.checkpoints.len() - 1;
                line.checkpoints = line.checkpoints.drain(..).enumerate().filter_map(|(i, s)| (i % 2 == 0 || i == last).then_some(s)).collect();
                line.stride = line.stride.saturating_mul(2);
            }
        }
        if let Some(at) = line.queries.iter().position(|(target, _)| *target == delta.target) { line.queries.remove(at); }
        line.queries.push_back((delta.target, delta.scan.clone()));
        if line.queries.len() > 16 { line.queries.pop_front(); }
        for row in delta.rows {
            if row.last { line.last = Some(row.clone()); }
            if let Some(at) = line.rows.iter().position(|r| r.index == row.index) { line.rows.remove(at); }
            line.rows.push_back(row);
            if line.rows.len() > KEPT_ROWS { line.rows.pop_front(); }
        }
        if let Some(point) = delta.byte { line.points.push_back(point); if line.points.len() > KEPT_POINTS { line.points.pop_front(); } }
        if let Some(point) = delta.display { line.displays.push_back(point); if line.displays.len() > KEPT_POINTS { line.displays.pop_front(); } }
        if let Some(point) = delta.column { line.columns.push_back(point); if line.columns.len() > KEPT_POINTS { line.columns.pop_front(); } }
        if delta.scan.complete && line.last.is_none() {
            line.last = Some(delta.scan.row(delta.line, delta.scan.processed, delta.scan.cursor.source_byte(), true, true));
        }
        while self.lines.values().map(Line::bytes).sum::<usize>() > CACHE_BYTES && self.lines.len() > 1 {
            let oldest = self.lines.iter().filter(|(key, _)| **key != delta.line).min_by_key(|(_, l)| l.touched).map(|(key, _)| *key);
            let Some(oldest) = oldest else { break };
            self.lines.remove(&oldest);
        }
        true
    }

    pub fn row(&mut self, snapshot: &TextSnapshot, line: usize, index: usize, count: usize) -> Query<Option<Row>> {
        if self.key.as_ref().is_some_and(|key| key.cols.is_none()) {
            if index != 0 { return Query::Ready(None); }
            let Some(source) = snapshot.line_range(line) else { return Query::Ready(None) };
            // Unwrapped native projection owns horizontal extent; logical layout never counts its tail.
            return Query::Ready(Some(Row { line, index: 0, display: 0..0, source: source.clone(), indent: 0,
                complete: source.is_empty(), last: true, column: 0, width: 0, end_caret: source.end }));
        }
        let Some(held) = self.line(snapshot, line) else { return Query::Ready(None) };
        if let Some(row) = held.rows.iter().find(|r| r.index == index) { return Query::Ready(Some(row.clone())); }
        if let Some(row) = held.last.as_ref().filter(|r| r.index == index) { return Query::Ready(Some(row.clone())); }
        if held.last.as_ref().is_some_and(|r| index > r.index) { return Query::Ready(None); }
        match self.work(snapshot, line, Target::Rows { first: index, count: count.clamp(1, KEPT_ROWS) }) {
            Some(work) => Query::Pending(work), None => Query::Ready(None),
        }
    }

    /// A stable bounded prefix of the unfinished row. Its end is never a whitespace hit target.
    pub fn prefix(&mut self, snapshot: &TextSnapshot, line: usize, index: usize) -> Option<Row> {
        let held = self.line(snapshot, line)?;
        let scan = &held.progress;
        if scan.complete || scan.row != index { return None; }
        let (display, source) = scan.last_break.as_ref().map_or((scan.processed, scan.processed_source), |at| (at.display, at.source));
        (display > scan.row_display).then(|| scan.row(line, display, source, false, false))
    }

    pub fn last(&mut self, snapshot: &TextSnapshot, line: usize) -> Query<Option<Row>> {
        if self.key.as_ref().is_some_and(|key| key.cols.is_none()) { return self.row(snapshot, line, 0, 1); }
        let Some(held) = self.line(snapshot, line) else { return Query::Ready(None) };
        if let Some(last) = &held.last { return Query::Ready(Some(last.clone())); }
        match self.work(snapshot, line, Target::End) { Some(work) => Query::Pending(work), None => Query::Ready(None) }
    }

    pub fn cursor(&mut self, snapshot: &TextSnapshot, line: usize, display: usize) -> Query<Option<Cursor>> {
        let Some(held) = self.line(snapshot, line) else { return Query::Ready(None) };
        if let Some((_, cursor, _)) = held.displays.iter().rev().find(|(d, ..)| *d == display) { return Query::Ready(Some(cursor.clone())); }
        if held.last.as_ref().is_some_and(|r| display > r.display.end) { return Query::Ready(None); }
        match self.work(snapshot, line, Target::Cursor(display)) { Some(work) => Query::Pending(work), None => Query::Ready(None) }
    }

    pub fn position(&mut self, snapshot: &TextSnapshot, line: usize, byte: usize) -> Query<Option<Position>> {
        let Some(range) = snapshot.line_range(line).filter(|r| r.start <= byte && byte <= r.end && snapshot.floor_char_boundary(byte) == byte) else { return Query::Ready(None) };
        let _ = range;
        let Some(held) = self.line(snapshot, line) else { return Query::Ready(None) };
        let Some(display) = held.points.iter().rev().find(|(b, _)| *b == byte).map(|(_, d)| *d) else {
            return match self.work(snapshot, line, Target::Byte(byte)) { Some(work) => Query::Pending(work), None => Query::Ready(None) };
        };
        let row = held.rows.iter().find(|r| contains_display(r, display)).cloned().or_else(|| held.last.as_ref().filter(|r| contains_display(r, display)).cloned());
        let column = held.displays.iter().rev().find(|(d, ..)| *d == display).map(|(.., column)| *column);
        if self.key.as_ref().is_some_and(|k| k.cols.is_none()) && let Some(column) = column {
            return Query::Ready(Some(Position { line, row: 0, display, column }));
        }
        if let (Some(row), Some(column)) = (row, column) {
            return Query::Ready(Some(Position { line, row: row.index, display, column: row.indent + column.saturating_sub(row.column) }));
        }
        match self.work(snapshot, line, Target::Display(display)) { Some(work) => Query::Pending(work), None => Query::Ready(None) }
    }

    pub fn byte_at(&mut self, snapshot: &TextSnapshot, row: &Row, column: usize) -> Query<Option<usize>> {
        let wanted = column.saturating_sub(row.indent);
        if wanted >= row.width { return Query::Ready(Some(if row.last { row.source.end } else { row.end_caret })); }
        let target = row.column.saturating_add(wanted);
        let Some(held) = self.line(snapshot, row.line) else { return Query::Ready(None) };
        if let Some((_, byte)) = held.columns.iter().rev().find(|(c, _)| *c == target) { return Query::Ready(Some(*byte)); }
        match self.work(snapshot, row.line, Target::Column(target)) { Some(work) => Query::Pending(work), None => Query::Ready(None) }
    }
}
