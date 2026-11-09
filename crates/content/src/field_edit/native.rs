//! Native one-line composition. The preedit is a projection; application text changes only on commit.
use super::{Command, Cursor, Snapshot};
use std::ops::Range;

const MAX_NATIVE_BYTES: usize = 4096;

#[derive(Default)]
pub struct Native {
    marked: Option<Marked>,
}

struct Marked {
    source_len: usize,
    source_hash: u64,
    cursor: Cursor,
    range: Range<usize>,
    text: String,
    selected: Range<usize>,
}

impl Native {
    pub fn is_marked(&self) -> bool {
        self.marked.is_some()
    }
    pub fn len_utf16(&self, text: &str, cursor: Cursor) -> usize {
        self.parts(text, cursor)
            .iter()
            .map(|s| s.encode_utf16().count())
            .sum()
    }
    pub fn set_selection(
        &mut self,
        text: &str,
        cursor: Cursor,
        range: Range<usize>,
    ) -> Option<Command> {
        if self.marked.is_some() && self.current(text, cursor).is_none() {
            return None;
        }
        let bytes = utf16_range(&self.parts(text, cursor), range)?;
        if let Some(m) = self.current(text, cursor) {
            let start = bytes.start.checked_sub(m.range.start)?;
            let end = bytes.end.checked_sub(m.range.start)?;
            if end > m.text.len() {
                return None;
            }
            self.marked.as_mut()?.selected = start..end;
            None
        } else {
            Some(Command::Select {
                anchor: bytes.start,
                caret: bytes.end,
            })
        }
    }
    /// Owners cancel on every field switch and application-driven edit/selection change.
    pub fn cancel(&mut self) {
        self.marked = None;
    }
    fn current(&self, text: &str, cursor: Cursor) -> Option<&Marked> {
        self.marked.as_ref().filter(|m| {
            m.source_len == text.len()
                && m.cursor == cursor
                && m.source_hash == fingerprint(text)
                && text.get(m.range.clone()).is_some()
        })
    }
    fn parts<'a>(&'a self, text: &'a str, cursor: Cursor) -> [&'a str; 3] {
        match self.current(text, cursor) {
            Some(m) => [
                text.get(..m.range.start).unwrap_or_default(),
                &m.text,
                text.get(m.range.end..).unwrap_or_default(),
            ],
            None => [text, "", ""],
        }
    }
    pub fn selected(&self, text: &str, cursor: Cursor) -> Option<(Range<usize>, bool)> {
        let (range, reversed) = match self.current(text, cursor) {
            Some(m) => (
                m.range.start + m.selected.start..m.range.start + m.selected.end,
                false,
            ),
            None => (
                cursor.selection(text).unwrap_or_else(|| {
                    let at = cursor.caret(text);
                    at..at
                }),
                cursor.anchor.is_some_and(|a| a > cursor.caret(text)),
            ),
        };
        let parts = self.parts(text, cursor);
        Some((
            byte_to_utf16(&parts, range.start)?..byte_to_utf16(&parts, range.end)?,
            reversed,
        ))
    }
    pub fn marked_range(&self, text: &str, cursor: Cursor) -> Option<Range<usize>> {
        let m = self.current(text, cursor)?;
        let start = text.get(..m.range.start)?.encode_utf16().count();
        Some(start..start + m.text.encode_utf16().count())
    }
    pub fn text_for_range(
        &self,
        text: &str,
        cursor: Cursor,
        range: Range<usize>,
        adjusted: &mut Option<Range<usize>>,
    ) -> Option<String> {
        let parts = self.parts(text, cursor);
        let bytes = utf16_range(&parts, range)?;
        let caret = self
            .current(text, cursor)
            .map_or_else(|| cursor.caret(text), |m| m.range.start + m.selected.end);
        let start = floor_boundary(
            &parts,
            caret
                .saturating_sub(MAX_NATIVE_BYTES / 2)
                .clamp(bytes.start, bytes.end),
        );
        let end = floor_boundary(
            &parts,
            bytes.end.min(start.saturating_add(MAX_NATIVE_BYTES)),
        );
        let mut result = String::new();
        let mut offset = 0;
        for part in parts {
            let start = start.saturating_sub(offset).min(part.len());
            let end = end.saturating_sub(offset).min(part.len());
            if start < end {
                result.push_str(part.get(start..part.floor_char_boundary(end))?);
            }
            offset += part.len();
        }
        *adjusted =
            Some(byte_to_utf16(&parts, start)?..byte_to_utf16(&parts, start + result.len())?);
        Some(result)
    }
    pub fn mark(
        &mut self,
        text: &str,
        cursor: Cursor,
        range: Option<Range<usize>>,
        value: &str,
        selected: Option<Range<usize>>,
        local: bool,
    ) -> bool {
        if self.marked.is_some() && self.current(text, cursor).is_none() {
            return false;
        }
        if value.len() > MAX_NATIVE_BYTES || value.chars().any(|c| matches!(c, '\r' | '\n')) {
            return false;
        }
        if let Some(m) = self.current(text, cursor) {
            let old = m.text.clone();
            let Some(bytes) = self.local_range(text, cursor, range, local) else {
                return false;
            };
            let selected = match selected {
                Some(r) => {
                    let Some(r) = utf16_range(&[value, "", ""], r) else {
                        return false;
                    };
                    r
                }
                None => value.len()..value.len(),
            };
            let selected = bytes.start + selected.start..bytes.start + selected.end;
            let mut value_text = old;
            value_text.replace_range(bytes, value);
            if value_text.len() > MAX_NATIVE_BYTES {
                return false;
            }
            let Some(m) = &mut self.marked else {
                return false;
            };
            m.text = value_text;
            m.selected = selected;
        } else {
            let range = match range {
                Some(r) => {
                    let Some(r) = utf16_range(&[text, "", ""], r) else {
                        return false;
                    };
                    r
                }
                None => cursor.selection(text).unwrap_or_else(|| {
                    let at = cursor.caret(text);
                    at..at
                }),
            };
            let selected = match selected {
                Some(r) => {
                    let Some(r) = utf16_range(&[value, "", ""], r) else {
                        return false;
                    };
                    r
                }
                None => value.len()..value.len(),
            };
            self.marked = Some(Marked {
                source_len: text.len(),
                source_hash: fingerprint(text),
                cursor,
                range,
                text: value.into(),
                selected,
            });
        }
        true
    }
    fn local_range(
        &self,
        text: &str,
        cursor: Cursor,
        range: Option<Range<usize>>,
        local: bool,
    ) -> Option<Range<usize>> {
        let m = self.current(text, cursor)?;
        match range {
            None => Some(0..m.text.len()),
            Some(r) if local => utf16_range(&[&m.text, "", ""], r),
            Some(r) => {
                let r = utf16_range(&self.parts(text, cursor), r)?;
                let start = r.start.checked_sub(m.range.start)?;
                let end = r.end.checked_sub(m.range.start)?;
                (end <= m.text.len()).then_some(start..end)
            }
        }
    }
    /// Pinned Linux XIM/Wayland input uses an empty replacement of the current marked range to
    /// cancel preedit. It must discard the projection rather than delete the original selection.
    pub fn cancels_marked_text(
        &self,
        text: &str,
        cursor: Cursor,
        range: Option<&Range<usize>>,
        value: &str,
        local: bool,
    ) -> bool {
        !local
            && value.is_empty()
            && range.is_some_and(|range| self.marked_range(text, cursor).as_ref() == Some(range))
    }
    pub fn commit(
        &mut self,
        text: &str,
        cursor: Cursor,
        range: Option<Range<usize>>,
        value: &str,
        local: bool,
    ) -> Option<Command> {
        if self.cancels_marked_text(text, cursor, range.as_ref(), value, local) {
            self.cancel();
            return None;
        }
        if self.marked.is_some() && self.current(text, cursor).is_none() {
            return None;
        }
        if let Some(m) = self.current(text, cursor) {
            let range_local = self.local_range(text, cursor, range, local)?;
            let value = crate::display::one_line(value);
            let mut value_text = m.text.clone();
            let caret = if local {
                m.range.start + value.len()
            } else {
                m.range.start + range_local.start + value.len()
            };
            if local {
                value_text = value;
            } else {
                value_text.replace_range(range_local, &value);
            }
            let range = m.range.clone();
            self.cancel();
            Some(Command::Replace {
                range,
                text: value_text,
                selection: Some(caret..caret),
            })
        } else {
            self.cancel();
            let range = match range {
                Some(r) => utf16_range(&[text, "", ""], r)?,
                None => cursor.selection(text).unwrap_or_else(|| {
                    let at = cursor.caret(text);
                    at..at
                }),
            };
            Some(Command::Replace {
                range,
                text: value.into(),
                selection: None,
            })
        }
    }
    pub fn unmark(&mut self, text: &str, cursor: Cursor) -> Option<Command> {
        let m = self.current(text, cursor)?;
        let result = Command::Replace {
            range: m.range.clone(),
            text: m.text.clone(),
            selection: Some(m.range.start + m.selected.start..m.range.start + m.selected.end),
        };
        self.cancel();
        Some(result)
    }
    /// Visible composition only. At most native surrounding bytes are copied, regardless of query length.
    pub fn snapshot(&self, text: &str, cursor: Cursor, columns: usize) -> Snapshot {
        let Some(m) = self.current(text, cursor) else {
            return cursor.snapshot(text, columns);
        };
        let before = text.get(..m.range.start).unwrap_or_default();
        let after = text.get(m.range.end..).unwrap_or_default();
        let start = before.ceil_char_boundary(before.len().saturating_sub(MAX_NATIVE_BYTES));
        let end = after.floor_char_boundary(after.len().min(MAX_NATIVE_BYTES));
        let mut shown = String::with_capacity(before.len() - start + m.text.len() + end);
        shown.push_str(before.get(start..).unwrap_or_default());
        shown.push_str(&m.text);
        shown.push_str(after.get(..end).unwrap_or_default());
        let selected =
            before.len() - start + m.selected.start..before.len() - start + m.selected.end;
        Cursor {
            caret: Some(selected.end),
            anchor: Some(selected.start),
        }
        .snapshot(&shown, columns)
    }
    pub fn snapshot_range(
        &self,
        text: &str,
        cursor: Cursor,
        range: Range<usize>,
        columns: usize,
    ) -> Option<(Snapshot, Range<usize>)> {
        let range = utf16_range(&self.parts(text, cursor), range)?;
        let caret = self
            .current(text, cursor)
            .map_or_else(|| cursor.caret(text), |m| m.range.start + m.selected.end);
        let shown = self.snapshot(text, cursor, columns);
        let start = caret.checked_sub(shown.cursor.caret(&shown.text))?;
        let end = start + shown.text.len();
        if range.end < start || range.start > end {
            return None;
        }
        let range = range.start.max(start) - start..range.end.min(end) - start;
        Some((shown, range))
    }
}

fn utf16_range(parts: &[&str; 3], range: Range<usize>) -> Option<Range<usize>> {
    (range.start <= range.end).then_some(())?;
    Some(utf16_to_byte(parts, range.start)?..utf16_to_byte(parts, range.end)?)
}
fn utf16_to_byte(parts: &[&str; 3], wanted: usize) -> Option<usize> {
    let (mut units, mut bytes) = (0, 0);
    for c in parts.iter().flat_map(|s| s.chars()) {
        if units == wanted {
            return Some(bytes);
        }
        units += c.len_utf16();
        bytes += c.len_utf8();
        if units > wanted {
            return None;
        }
    }
    (units == wanted).then_some(bytes)
}
fn byte_to_utf16(parts: &[&str; 3], wanted: usize) -> Option<usize> {
    let (mut units, mut bytes) = (0, 0);
    for c in parts.iter().flat_map(|s| s.chars()) {
        if bytes == wanted {
            return Some(units);
        }
        units += c.len_utf16();
        bytes += c.len_utf8();
        if bytes > wanted {
            return None;
        }
    }
    (bytes == wanted).then_some(units)
}

fn fingerprint(text: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hash);
    hash.finish()
}

fn floor_boundary(parts: &[&str; 3], wanted: usize) -> usize {
    let mut offset = 0;
    for part in parts {
        if wanted <= offset + part.len() {
            return offset + part.floor_char_boundary(wanted.saturating_sub(offset));
        }
        offset += part.len();
    }
    offset
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn composition_is_isolated_utf16_accurate_and_commits_once() {
        let mut text = "a😀z".to_string();
        let mut cursor = Cursor {
            caret: Some(5),
            anchor: Some(1),
        };
        let mut native = Native::default();
        assert!(native.mark(&text, cursor, None, "仮", None, false));
        assert_eq!(text, "a😀z");
        assert_eq!(native.marked_range(&text, cursor), Some(1..2));
        let mut adjusted = None;
        assert_eq!(
            native
                .text_for_range(&text, cursor, 0..3, &mut adjusted)
                .as_deref(),
            Some("a仮z")
        );
        assert_eq!(native.selected(&text, cursor), Some((2..2, false)));
        let command = native.commit(&text, cursor, None, "確定", false).unwrap();
        cursor.execute(&mut text, command);
        assert_eq!(text, "a確定z");
        assert_eq!(cursor.caret(&text), 7);
        assert!(native.marked_range(&text, cursor).is_none());
        assert!(!native.mark(
            &text,
            cursor,
            Some(Range { start: 2, end: 1 }),
            "x",
            None,
            false
        ));
    }
    #[test]
    fn surrogate_interiors_and_large_preedit_are_rejected_cancel_keeps_original() {
        let text = "a😀z";
        let cursor = Cursor::default();
        let mut native = Native::default();
        assert!(!native.mark(text, cursor, Some(2..3), "x", None, false));
        assert!(!native.mark(
            text,
            cursor,
            None,
            &"x".repeat(MAX_NATIVE_BYTES + 1),
            None,
            false
        ));
        assert!(native.mark(text, cursor, None, "x", None, false));
        native.cancel();
        assert_eq!(native.snapshot(text, cursor, 20).text, text);
    }
    #[test]
    fn linux_empty_marked_replacement_cancels_without_deleting_the_original_selection() {
        let text = "keep";
        let cursor = Cursor {
            caret: Some(text.len()),
            anchor: Some(0),
        };
        let mut native = Native::default();
        assert!(native.mark(text, cursor, None, "候補", None, false));
        let range = native.marked_range(text, cursor).unwrap();
        assert!(native.cancels_marked_text(text, cursor, Some(&range), "", false));
        assert!(!native.cancels_marked_text(text, cursor, Some(&range), "", true));
        assert!(
            native
                .commit(text, cursor, Some(range), "", false)
                .is_none()
        );
        assert!(!native.is_marked());
        assert_eq!(native.snapshot(text, cursor, 20).text, "keep");
        assert!(
            matches!(
                native.commit(text, cursor, Some(0..4), "", false),
                Some(Command::Replace { .. })
            ),
            "ordinary deletion without marked text remains a replacement"
        );
    }
}
