//! Immutable indexed UTF-8 drafts. Reads copy only requested bytes; contiguous analysis is explicit.
use std::{borrow::Cow, ops::Range, path::Path, sync::Arc};
use lenz_content::highlight::Language;
use crate::document::{Document, SourceMeta};

const CHUNK_BYTES: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BufferId(pub u64);
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SnapshotKey { pub buffer: BufferId, pub version: u64 }
#[derive(Debug, Clone)]
pub struct TextEdit { pub range: Range<usize>, pub text: Arc<str> }
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditKind { Typing, Delete, Paste, Replace }
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditError { StaleVersion, InvalidRange, OverlappingRanges }

#[derive(Debug)]
pub struct TextWindow<'a> {
    pub bytes: Range<usize>,
    pub utf16: Range<usize>,
    pub text: Cow<'a, str>,
}

#[derive(Debug)]
struct Chunk { text: Arc<str>, range: Range<usize>, shared_source: bool, newlines: Arc<[usize]>, newline_range: Range<usize>, newline_base: usize, utf16: usize }
impl Chunk {
    fn text(&self) -> &str { self.text.get(self.range.clone()).unwrap_or_default() }
    fn newline(&self, index: usize) -> Option<usize> {
        self.newlines.get(self.newline_range.clone())?.get(index)?.checked_sub(self.newline_base)
    }
    fn indexed(text: Arc<str>, range: Range<usize>, shared_source: bool) -> Arc<Self> {
        let part = text.get(range.clone()).unwrap_or_default();
        let newlines: Arc<[usize]> = part.match_indices('\n').map(|(i, _)| i).collect();
        let utf16 = if part.is_ascii() { part.len() } else { part.chars().map(char::len_utf16).sum() };
        let newline_range = 0..newlines.len();
        Arc::new(Self { text, range, shared_source, newlines, newline_range, newline_base: 0, utf16 })
    }
    fn shared(text: Arc<str>, lines: Arc<[usize]>) -> Vec<Arc<Self>> {
        let mut chunks = Vec::with_capacity(text.len().div_ceil(CHUNK_BYTES));
        let (mut from, mut first_line) = (0, 1);
        while from < text.len() {
            let end = text.floor_char_boundary(from.saturating_add(CHUNK_BYTES).min(text.len()));
            let after = lines.partition_point(|&offset| offset <= end);
            let part = text.get(from..end).unwrap_or_default();
            let utf16 = if part.is_ascii() { part.len() } else { part.chars().map(char::len_utf16).sum() };
            chunks.push(Arc::new(Self { text: text.clone(), range: from..end, shared_source: true,
                newlines: lines.clone(), newline_range: first_line..after, newline_base: from + 1, utf16 }));
            from = end;
            first_line = after;
        }
        chunks
    }
    fn split(mut text: &str) -> Vec<Arc<Self>> {
        let mut chunks = Vec::new();
        while !text.is_empty() {
            let end = text.floor_char_boundary(CHUNK_BYTES.min(text.len()));
            let (part, rest) = text.split_at(end);
            chunks.push(Self::indexed(part.into(), 0..part.len(), false));
            text = rest;
        }
        chunks
    }
}

/// Prefix measures let coordinate lookup skip untouched chunks.
#[derive(Debug, Clone)]
struct Entry { chunk: Arc<Chunk>, byte: usize, newline: usize, utf16: usize }

#[derive(Debug, Clone)]
pub struct TextSnapshot {
    key: SnapshotKey,
    source: Arc<SourceMeta>,
    chunks: Arc<[Entry]>,
    bytes: usize,
    newlines: usize,
    utf16: usize,
    owned_bytes: usize,
}

impl TextSnapshot {
    pub fn from_document(buffer: BufferId, document: &Document) -> Self {
        Self::indexed(SnapshotKey { buffer, version: document.version() }, document.source_meta().clone(), Chunk::shared(document.text().clone(), document.line_starts().clone()))
    }

    fn indexed(key: SnapshotKey, source: Arc<SourceMeta>, chunks: Vec<Arc<Chunk>>) -> Self {
        let (mut bytes, mut newlines, mut utf16) = (0, 0, 0);
        let mut owned_bytes = size_of::<Self>();
        let chunks = chunks.into_iter().map(|chunk| {
            let entry = Entry { chunk, byte: bytes, newline: newlines, utf16 };
            bytes += entry.chunk.text().len();
            newlines += entry.chunk.newline_range.len();
            utf16 += entry.chunk.utf16;
            owned_bytes += size_of::<Entry>() + size_of::<Chunk>()
                + if entry.chunk.shared_source { 0 } else { entry.chunk.text().len() + entry.chunk.newline_range.len() * size_of::<usize>() };
            entry
        }).collect();
        let chunks: Arc<[Entry]> = chunks;
        Self { key, source, chunks, bytes, newlines, utf16, owned_bytes }
    }

    pub(crate) fn with_source_meta(&self, source: Arc<SourceMeta>) -> Self { Self { source, ..self.clone() } }
    pub fn key(&self) -> SnapshotKey { self.key }
    pub fn source(&self) -> &Path { self.source.source() }
    pub fn source_meta(&self) -> &Arc<SourceMeta> { &self.source }
    pub fn language(&self) -> Option<Language> { self.source.language() }
    pub fn notices(&self) -> Vec<String> { self.source.notices(self.is_large()) }
    pub fn image(&self) -> Option<&Arc<crate::document::Image>> { self.source.image() }
    pub fn encoding(&self) -> &str { self.source.encoding() }
    pub fn len_bytes(&self) -> usize { self.bytes }
    pub fn len_utf16(&self) -> usize { self.utf16 }
    pub fn line_count(&self) -> usize { self.newlines + 1 }

    /// Additional indexed storage beside the compatibility Document; shared text, line offsets and metadata excluded.
    pub fn storage_bytes(&self) -> usize { self.owned_bytes }

    pub(crate) fn reader_line_range(&self, line: usize) -> Range<usize> {
        self.line_range(line.min(self.line_count() - 1)).unwrap_or(0..0)
    }
    fn char_at(&self, byte: usize) -> Option<char> {
        let entry = self.entry(byte)?;
        entry.chunk.text().get(byte - entry.byte..)?.chars().next()
    }
    /// Word, whitespace or punctuation run at a source caret, without assembling its line.
    pub(crate) fn word_at(&self, byte: usize) -> Range<usize> {
        let range = self.reader_line_range(self.line_of(byte));
        let mut at = self.floor_char_boundary(byte).clamp(range.start, range.end);
        if at == range.end && at > range.start { at = self.previous_char_boundary(at); }
        let class = |c: char| if c.is_alphanumeric() || c == '_' { 0 } else if c.is_whitespace() { 1 } else { 2 };
        let Some(kind) = self.char_at(at).filter(|_| at < range.end).map(class) else { return at..at };
        let index = self.chunks.partition_point(|entry| entry.byte <= at).saturating_sub(1);
        let mut start = at;
        'before: for entry in self.chunks[..=index].iter().rev() {
            if entry.byte + entry.chunk.text().len() <= range.start { break; }
            let from = range.start.saturating_sub(entry.byte);
            let to = (at - entry.byte).min(entry.chunk.text().len());
            if from >= to { continue; }
            for (offset, c) in entry.chunk.text().get(from..to).unwrap_or_default().char_indices().rev() {
                if class(c) != kind { start = entry.byte + from + offset + c.len_utf8(); break 'before; }
            }
            start = entry.byte + from;
            if start == range.start { break; }
        }
        let mut end = at;
        'after: for entry in &self.chunks[index..] {
            if entry.byte >= range.end { break; }
            let from = at.saturating_sub(entry.byte);
            let to = (range.end - entry.byte).min(entry.chunk.text().len());
            for (offset, c) in entry.chunk.text().get(from..to).unwrap_or_default().char_indices() {
                if class(c) != kind { end = entry.byte + from + offset; break 'after; }
            }
            end = entry.byte + to;
        }
        start..end
    }
    pub(crate) fn byte_at_column(&self, line: usize, mut column: usize) -> usize {
        let range = self.reader_line_range(line);
        let first = self.chunks.partition_point(|entry| entry.byte <= range.start).saturating_sub(1);
        for entry in &self.chunks[first..] {
            if entry.byte >= range.end { break; }
            let from = range.start.saturating_sub(entry.byte);
            let to = (range.end - entry.byte).min(entry.chunk.text().len());
            for (offset, _) in entry.chunk.text().get(from..to).unwrap_or_default().char_indices() {
                if column == 0 { return entry.byte + from + offset; }
                column -= 1;
            }
        }
        range.end
    }
    pub fn is_large(&self) -> bool {
        let nonempty_rows = self.line_count() - usize::from(self.bytes > 0 && self.byte_at(self.bytes - 1) == Some(b'\n'));
        self.bytes > crate::document::LARGE_BYTES || nonempty_rows > crate::document::LARGE_LINES
    }

    fn entry(&self, byte: usize) -> Option<&Entry> {
        self.chunks.get(self.chunks.partition_point(|c| c.byte <= byte).checked_sub(1)?)
    }
    fn boundary(&self, byte: usize) -> bool {
        byte == self.bytes || byte < self.bytes && self.entry(byte).is_some_and(|e| e.chunk.text().is_char_boundary(byte - e.byte))
    }
    fn byte_at(&self, byte: usize) -> Option<u8> {
        let e = self.entry(byte)?;
        e.chunk.text().as_bytes().get(byte - e.byte).copied()
    }
    fn line_start(&self, line: usize) -> Option<usize> {
        if line == 0 { return Some(0); }
        if line > self.newlines { return None; }
        let i = self.chunks.partition_point(|e| e.newline < line) - 1;
        let e = &self.chunks[i];
        Some(e.byte + e.chunk.newline(line - e.newline - 1)? + 1)
    }
    pub fn line_with_ending(&self, line: usize) -> Option<Range<usize>> {
        Some(self.line_start(line)?..self.line_start(line + 1).unwrap_or(self.bytes))
    }
    pub fn line_range(&self, line: usize) -> Option<Range<usize>> {
        let mut range = self.line_with_ending(line)?;
        if range.end > range.start && self.byte_at(range.end - 1) == Some(b'\n') {
            range.end -= 1;
            if range.end > range.start && self.byte_at(range.end - 1) == Some(b'\r') { range.end -= 1; }
        }
        Some(range)
    }
    pub fn line_of(&self, byte: usize) -> usize {
        let byte = byte.min(self.bytes);
        self.entry(byte).map_or(0, |e| e.newline + e.chunk.newlines.get(e.chunk.newline_range.clone())
            .map_or(0, |lines| lines.partition_point(|&n| n < byte - e.byte + e.chunk.newline_base)))
    }
    pub fn floor_char_boundary(&self, byte: usize) -> usize {
        let mut byte = byte.min(self.bytes);
        while !self.boundary(byte) { byte -= 1; }
        byte
    }
    pub fn previous_char_boundary(&self, byte: usize) -> usize {
        self.floor_char_boundary(byte.min(self.bytes).saturating_sub(1))
    }
    pub fn next_char_boundary(&self, byte: usize) -> usize {
        let mut byte = byte.saturating_add(1).min(self.bytes);
        while !self.boundary(byte) { byte += 1; }
        byte
    }
    pub fn byte_to_utf16(&self, byte: usize) -> Option<usize> {
        if !self.boundary(byte) { return None; }
        let Some(e) = self.entry(byte) else { return Some(0); };
        Some(e.utf16 + e.chunk.text().get(..byte - e.byte)?.chars().map(char::len_utf16).sum::<usize>())
    }
    pub fn utf16_to_byte(&self, unit: usize) -> Option<usize> {
        if unit > self.utf16 { return None; }
        if unit == self.utf16 { return Some(self.bytes); }
        let e = &self.chunks[self.chunks.partition_point(|e| e.utf16 <= unit) - 1];
        let mut at = e.utf16;
        for (byte, c) in e.chunk.text().char_indices() {
            if at == unit { return Some(e.byte + byte); }
            at += c.len_utf16();
            if at > unit { return None; }
        }
        None
    }

    pub(crate) fn range(&self, bytes: Range<usize>) -> Option<Cow<'_, str>> {
        if bytes.start > bytes.end || !self.boundary(bytes.start) || !self.boundary(bytes.end) { return None; }
        if bytes.is_empty() { return Some(Cow::Borrowed("")); }
        let e = self.entry(bytes.start)?;
        if bytes.end <= e.byte + e.chunk.text().len() {
            return Some(Cow::Borrowed(e.chunk.text().get(bytes.start - e.byte..bytes.end - e.byte)?));
        }
        let mut text = String::with_capacity(bytes.len());
        let start = self.chunks.partition_point(|e| e.byte <= bytes.start) - 1;
        for e in &self.chunks[start..] {
            if e.byte >= bytes.end { break; }
            let from = bytes.start.saturating_sub(e.byte);
            let to = (bytes.end - e.byte).min(e.chunk.text().len());
            text.push_str(e.chunk.text().get(from..to)?);
        }
        Some(Cow::Owned(text))
    }
    pub fn copy_range(&self, bytes: Range<usize>) -> Option<String> { self.range(bytes).map(Cow::into_owned) }

    /// `from` is an absolute byte offset, clamped to this line and rounded down to a character boundary.
    /// A budget smaller than the next UTF-8 character returns an empty window.
    pub fn line_window(&self, line: usize, from: usize, max_bytes: usize) -> Option<TextWindow<'_>> {
        let line = self.line_range(line)?;
        let start = self.floor_char_boundary(from.clamp(line.start, line.end));
        let end = self.floor_char_boundary(start.saturating_add(max_bytes).min(line.end));
        let bytes = start..end;
        Some(TextWindow { utf16: self.byte_to_utf16(start)?..self.byte_to_utf16(end)?, text: self.range(bytes.clone())?, bytes })
    }

    /// Explicit full conversion for admitted background analysis; never call from input/render paths.
    pub fn materialize(&self) -> Document {
        let mut text = String::with_capacity(self.bytes);
        for e in self.chunks.iter() { text.push_str(e.chunk.text()); }
        Document::from_snapshot_parts(self.source.clone(), self.key.version, text.into())
    }

    /// Contiguous worker conversion which can be abandoned when its source/context is superseded.
    pub fn materialize_while(&self, current: impl Fn() -> bool) -> Option<Document> {
        if !current() { return None; }
        let mut text = String::with_capacity(self.bytes);
        let mut lines = Vec::with_capacity(self.newlines + 1);
        lines.push(0);
        for entry in self.chunks.iter() {
            if !current() { return None; }
            text.push_str(entry.chunk.text());
            for index in 0..entry.chunk.newline_range.len() {
                if let Some(local) = entry.chunk.newline(index) { lines.push(entry.byte + local + 1); }
            }
        }
        if !current() { return None; }
        let text: Arc<str> = text.into();
        let lines: Arc<[usize]> = lines.into();
        if !current() { return None; }
        Some(Document::from_indexed_snapshot_parts(self.source.clone(), self.key.version, text, lines))
    }

    /// Apply one exact transaction. Ranges refer to this snapshot, sorted in ascending order.
    /// Validation completes before any copying; held snapshots retain their original chunks.
    pub fn apply_edits(&self, version: u64, edits: &[TextEdit]) -> Result<Self, EditError> {
        if version <= self.key.version { return Err(EditError::StaleVersion); }
        let mut end = 0;
        let mut previous_start = None;
        for edit in edits {
            if edit.range.start > edit.range.end || !self.boundary(edit.range.start) || !self.boundary(edit.range.end) { return Err(EditError::InvalidRange); }
            if edit.range.start < end || previous_start == Some(edit.range.start) { return Err(EditError::OverlappingRanges); }
            end = edit.range.end;
            previous_start = Some(edit.range.start);
        }
        let mut result = self.clone();
        // Reverse order keeps all source ranges valid without retaining intermediate snapshots.
        for edit in edits.iter().rev() { result.replace(edit)?; }
        result.key.version = version;
        Ok(result)
    }

    fn replace(&mut self, edit: &TextEdit) -> Result<(), EditError> {
        if self.chunks.is_empty() {
            *self = Self::indexed(self.key, self.source.clone(), Chunk::split(&edit.text));
            return Ok(());
        }
        let first = self.chunks.partition_point(|e| e.byte <= edit.range.start) - 1;
        let last = self.chunks.partition_point(|e| e.byte <= edit.range.end) - 1;
        let a = &self.chunks[first];
        let b = &self.chunks[last];
        let mut rebuilt = String::new();
        rebuilt.push_str(a.chunk.text().get(..edit.range.start - a.byte).ok_or(EditError::InvalidRange)?);
        rebuilt.push_str(&edit.text);
        rebuilt.push_str(b.chunk.text().get(edit.range.end - b.byte..).ok_or(EditError::InvalidRange)?);
        // Rechunk one neighbour so repeated typing does not accumulate tiny chunks.
        let after = if let Some(next) = self.chunks.get(last + 1) {
            rebuilt.push_str(next.chunk.text());
            last + 2
        } else { last + 1 };
        let mut chunks = Vec::with_capacity(self.chunks.len() + rebuilt.len() / CHUNK_BYTES + 2);
        chunks.extend(self.chunks[..first].iter().map(|e| e.chunk.clone()));
        chunks.extend(Chunk::split(&rebuilt));
        chunks.extend(self.chunks[after..].iter().map(|e| e.chunk.clone()));
        *self = Self::indexed(self.key, self.source.clone(), chunks);
        Ok(())
    }
}
