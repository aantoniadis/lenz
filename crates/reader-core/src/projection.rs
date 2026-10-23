//! Resumable source display. A cursor never owns current text; callers supply its immutable snapshot.

use std::{ops::Range, sync::Arc};
use unicode_segmentation::UnicodeSegmentation;

use crate::{decor::{HintKind, Hints}, document::SourceMeta, shown::Shown, tabs::TabId, text::{SnapshotKey, TextSnapshot}};

/// Work counts consumed source/hint bytes and emitted display bytes, including suppressed marks.
pub const WINDOW_WORK: usize = 4096;
/// One atomic scalar can expand to ten ASCII bytes (or a tab to thirteen guarded bytes).
pub const MIN_WORK: usize = 16;
/// Ordinary graphemes are unchanged; overflowing scalars are shown as `[U+XXXX]`.
pub const GRAPHEME_BYTES: usize = 256;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Guard {
    tail: String,
}

impl Guard {
    fn push(&mut self, c: char, out: &mut String) {
        // Printable ASCII breaks after printable ASCII; avoid segmenting the common source path.
        if c.is_ascii() && self.tail.as_bytes().last().is_some_and(u8::is_ascii) {
            self.tail.clear();
            self.tail.push(c);
            out.push(c);
            return;
        }
        let before = self.tail.len();
        self.tail.push(c);
        let last = self.tail.grapheme_indices(true).next_back().map_or(0, |(i, _)| i);
        if last == 0 && self.tail.len() > GRAPHEME_BYTES {
            self.tail.truncate(before);
            let escaped = format!("[U+{:04X}]", c as u32);
            // A Prepend may join `[`, but `U` breaks. The maximum projected grapheme is 257 bytes.
            for c in escaped.chars() {
                self.tail.push(c);
                let last = self.tail.grapheme_indices(true).next_back().map_or(0, |(i, _)| i);
                self.tail.drain(..last);
            }
            out.push_str(&escaped);
        } else {
            self.tail.drain(..last);
            out.push(c);
        }
    }
}

/// An exact place in a projected line, including continuation inside a hint and Unicode guard state.
#[derive(Clone, Debug)]
pub struct Cursor {
    key: SnapshotKey,
    metadata: Arc<SourceMeta>,
    line: usize,
    byte: usize,
    display: usize,
    column: usize,
    marks: usize,
    hints: Option<Arc<Hints>>,
    hint: usize,
    hint_byte: usize,
    before: usize,
    type_caret: Option<usize>,
    guard: Guard,
}

impl PartialEq for Cursor {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key && Arc::ptr_eq(&self.metadata, &other.metadata)
            && (self.line, self.byte, self.display, self.column, self.marks, self.hint, self.hint_byte, self.before, self.type_caret)
                == (other.line, other.byte, other.display, other.column, other.marks, other.hint, other.hint_byte, other.before, other.type_caret)
            && self.guard == other.guard && match (&self.hints, &other.hints) {
                (None, None) => true, (Some(a), Some(b)) => Arc::ptr_eq(a, b), _ => false,
            }
    }
}
impl Eq for Cursor {}

/// Adapter metric identity is opaque to the core; measurement itself remains in the adapter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub pane: u8,
    pub tab: TabId,
    pub cursor: Cursor,
    /// Monotonically increasing per pane, including changes of metric context.
    pub request: u64,
    pub context: u64,
}

/// A bounded source-only worker response, consumed with an adapter's own measurement budget.
#[derive(Debug)]
pub struct Batch {
    pub request: Request,
    pub windows: Vec<Window>,
    pub next: Cursor,
    pub complete: bool,
    pub failed: bool,
}

/// Opaque worker completion: only the Session's bounded scheduler constructs one.
pub struct Finished {
    pub(crate) pane: u8,
    pub(crate) ticket: u64,
    pub(crate) batch: Option<Batch>,
}

/// One bounded piece. Source coordinates are absolute; display ranges index the expanded line.
#[derive(Clone, Debug)]
pub struct Window {
    pub key: SnapshotKey,
    pub line: usize,
    pub source: Range<usize>,
    pub display: Range<usize>,
    /// Logical scalar-width measures, not native pixels or terminal grapheme cells.
    pub columns: Range<usize>,
    pub shown: Shown,
    pub work: usize,
    pub complete: bool,
    from: Cursor,
    to: Cursor,
    end_ready: bool,
}

impl Cursor {
    pub fn start(snapshot: &TextSnapshot, line: usize, hints: Option<Arc<Hints>>) -> Option<Self> {
        Some(Self {
            key: snapshot.key(), metadata: snapshot.source_meta().clone(), line, byte: snapshot.line_range(line)?.start,
            display: 0, column: 0, marks: 0, hints, hint: 0, hint_byte: 0,
            before: 0, type_caret: None, guard: Guard::default(),
        })
    }

    pub fn key(&self) -> SnapshotKey { self.key }
    pub fn line(&self) -> usize { self.line }
    pub fn source_byte(&self) -> usize { self.byte }
    pub fn display_byte(&self) -> usize { self.display }
    pub fn logical_column(&self) -> usize { self.column }
    pub fn hints(&self) -> Option<&Arc<Hints>> { self.hints.as_ref() }
    pub(crate) fn context_bytes(&self) -> usize { self.guard.tail.capacity() }

    pub fn matches(&self, snapshot: &TextSnapshot, hints: Option<&Arc<Hints>>) -> bool {
        self.key == snapshot.key() && Arc::ptr_eq(&self.metadata, snapshot.source_meta()) && match (&self.hints, hints) {
            (None, None) => true,
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }

    /// Advance by at most `work_bytes` (16..=4096), pausing even inside a label or hidden mark run.
    pub fn window(&mut self, snapshot: &TextSnapshot, work_bytes: usize) -> Option<Window> {
        if self.key != snapshot.key() || !Arc::ptr_eq(&self.metadata, snapshot.source_meta()) || !(MIN_WORK..=WINDOW_WORK).contains(&work_bytes) { return None; }
        let line = snapshot.line_range(self.line)?;
        let from = self.clone();
        let held = self.hints.clone();
        let hints = held.as_ref().map_or(&[][..], |h| h.line(self.line));
        let mut shown = Shown { text: String::new(), map: vec![0], hints: Vec::new() };
        let (mut work, mut last_hint) = (0, None);
        while work_bytes.saturating_sub(work) >= MIN_WORK {
            let source = snapshot.line_window(self.line, self.byte, work_bytes)?;
            loop {
                if work_bytes.saturating_sub(work) < MIN_WORK { break; }
                let col = self.byte - line.start;
                if let Some(h) = hints.get(self.hint).filter(|h| (h.byte as usize) < col) {
                    let _ = h;
                    self.hint += 1;
                    self.hint_byte = 0;
                    work += 1;
                    continue;
                }
                if let Some(h) = hints.get(self.hint).filter(|h| h.byte as usize == col) {
                    if h.kind == HintKind::Type && self.type_caret.is_none() { self.type_caret = Some(self.display); }
                    let Some(c) = h.label.get(self.hint_byte..).and_then(|s| s.chars().next()) else {
                        self.hint += 1;
                        self.hint_byte = 0;
                        work += 1;
                        continue;
                    };
                    let start = shown.text.len();
                    self.guard.push(c, &mut shown.text);
                    let end = shown.text.len();
                    let local = self.byte - from.byte;
                    shown.map.resize(local + 1, start);
                    if let Some(at) = shown.map.get_mut(local) { *at = end; }
                    if last_hint == Some(self.hint) {
                        if let Some((_, range, _)) = shown.hints.last_mut() { range.end = end; }
                    } else {
                        shown.hints.push((local, start..end, h.kind));
                        last_hint = Some(self.hint);
                    }
                    self.hint_byte += c.len_utf8();
                    self.display += end - start;
                    self.column += shown.text.get(start..end).map_or(0, lenz_content::wrap::width);
                    work += c.len_utf8() + end - start;
                    continue;
                }
                if self.byte == line.end { break; }
                let Some(c) = source.text.get(self.byte - source.bytes.start..).and_then(|s| s.chars().next()) else { break };
                let start = shown.text.len();
                let local = self.byte - from.byte;
                shown.map.resize(local + 1, start);
                if lenz_content::display::shows_mark(c, &mut self.marks) {
                    if c == '\t' {
                        for _ in 0..4 { self.guard.push(' ', &mut shown.text); }
                    } else {
                        self.guard.push(if lenz_content::display::hides_text(c) { '\u{FFFD}' } else { c }, &mut shown.text);
                    }
                }
                self.marks = self.marks.min(lenz_content::display::MAX_MARKS + 1);
                let end = shown.text.len();
                self.byte += c.len_utf8();
                self.display += end - start;
                self.column += shown.text.get(start..end).map_or(0, lenz_content::wrap::width);
                self.before = self.display;
                self.type_caret = None;
                shown.map.resize(self.byte - from.byte + 1, end);
                work += c.len_utf8() + end - start;
            }
            if self.byte == line.end || work_bytes.saturating_sub(work) < MIN_WORK { break; }
        }
        let next_hint = hints.get(self.hint);
        let end_ready = next_hint.is_none_or(|h| h.byte as usize > self.byte - line.start);
        let complete = self.byte == line.end && end_ready;
        Some(Window {
            key: self.key, line: self.line, source: from.byte..self.byte,
            display: from.display..self.display, columns: from.column..self.column,
            shown, work, complete, from, to: self.clone(), end_ready,
        })
    }
}

impl Window {
    pub fn next(&self) -> &Cursor { &self.to }

    fn local(&self, display: usize) -> usize { display.saturating_sub(self.display.start).min(self.shown.text.len()) }

    /// Expanded byte offset after the hints at `source`, clipped to this window.
    pub fn at(&self, source: usize) -> usize {
        if source < self.source.start { 0 }
        else { self.shown.at(source - self.source.start) }
    }

    /// Expanded byte offset before the hints at `source`, clipped to this window.
    pub fn end(&self, source: usize) -> usize {
        if source < self.source.start { 0 }
        else if source > self.source.end { self.shown.text.len() }
        else if source == self.to.byte { self.local(self.to.before) }
        else { self.shown.end(source.saturating_sub(self.source.start)) }
    }

    pub fn range(&self, source: Range<usize>) -> Range<usize> {
        let end = if source.end > self.source.end { self.shown.text.len() } else { self.end(source.end) };
        self.at(source.start).min(end)..end
    }

    /// A caret is omitted when its exact position is outside the window or still inside a pending label set.
    pub fn caret_display(&self, source: usize) -> Option<usize> {
        if !self.source.contains(&source) && source != self.source.end { return None; }
        let inherited = (source == self.from.byte).then_some(self.from.type_caret).flatten()
            .or_else(|| (source == self.to.byte).then_some(self.to.type_caret).flatten());
        if let Some(global) = inherited { return Some(global); }
        if source == self.to.byte && !self.end_ready { return None; }
        Some(self.display.start + self.shown.caret(source - self.source.start))
    }

    pub fn caret(&self, source: usize) -> Option<usize> {
        let global = self.caret_display(source)?;
        (self.display.start..=self.display.end).contains(&global).then(|| self.local(global))
    }

    /// Every visible hint/escape maps to original source boundaries, never editable projection bytes.
    pub fn source_at(&self, display: usize) -> usize {
        if let Some((byte, _, _)) = self.shown.hints.iter().find(|(_, r, _)| r.contains(&display)) {
            return self.source.start + byte;
        }
        self.source.start + self.shown.map.partition_point(|&at| at <= display).saturating_sub(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn guarded(text: &str) -> String {
        let mut guard = Guard::default();
        let mut out = String::new();
        for c in text.chars() { guard.push(c, &mut out); }
        out
    }

    #[test]
    fn ordinary_unicode_is_unchanged_and_a_prepend_overflow_is_bounded() {
        for text in ["👨‍👩‍👧‍👦", "👩🏽‍💻", "🇬🇷", "1\u{FE0F}\u{20E3}", "e\u{301}", "क्षि", "🧑🏼‍🚀"] {
            assert_eq!(guarded(text), text);
        }
        let below = "\u{600}".repeat(128);
        assert_eq!(guarded(&below), below);
        let above = below + "\u{600}";
        let drawn = guarded(&above);
        assert!(drawn.ends_with("[U+0600]"));
        assert_eq!(drawn.graphemes(true).map(str::len).max(), Some(257));
        let chain = "👩\u{200D}".repeat(20_000) + "💻 ordinary";
        let drawn = guarded(&chain);
        assert!(drawn.contains("[U+"));
        assert!(drawn.ends_with("💻 ordinary"));
        assert!(drawn.graphemes(true).all(|s| s.len() <= GRAPHEME_BYTES + 1));
    }
}
