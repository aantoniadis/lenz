//! Caret editing in a one-line text box (Search Everywhere's query, Search in Project's fields): adapters keep the
//! text and a byte caret and map their keys to [`Edit`]s, so the box moves and deletes the same way in the window and
//! the terminal.
use unicode_segmentation::UnicodeSegmentation;
#[cfg(test)]
use unicode_width::UnicodeWidthStr;

/// A bounded view of one input field. `caret` is a display column within `text`, including space for the cursor.
pub struct Visible<'a> {
    pub text: &'a str,
    pub prefix: &'a str,
    pub caret: usize,
}

/// Keep the insertion point visible, including after Home, word movement and resizing. Borrow only the visible
/// graphemes; neither a long input nor a wide Unicode character is copied or split to draw the field.
pub fn visible(text: &str, caret: usize, columns: usize) -> Visible<'_> {
    if columns == 0 {
        return Visible {
            text: "",
            prefix: "",
            caret: 0,
        };
    }
    let caret = clamp(text, caret);
    let before = text.get(..caret).unwrap_or_default();
    let mut width = 0;
    let mut start = caret;
    // Walk back only through the visible prefix. A long pasted query must not be rescanned from its beginning
    // on every paint or caret step.
    for (at, grapheme) in before.grapheme_indices(true).rev() {
        let next = width + crate::display::field_width(grapheme);
        if next >= columns {
            break;
        }
        width = next;
        start = at;
    }
    let mut end = caret;
    let mut used = width;
    for grapheme in text.get(caret..).unwrap_or_default().graphemes(true) {
        let next = used + crate::display::field_width(grapheme);
        if next > columns {
            break;
        }
        end += grapheme.len();
        used = next;
    }
    Visible {
        text: text.get(start..end).unwrap_or_default(),
        prefix: text.get(start..caret).unwrap_or_default(),
        caret: width,
    }
}

/// Owned one-line input. Cursor and selection transitions are shared with the reader's other popup fields.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Input {
    text: String,
    cursor: crate::field_edit::Cursor,
}
impl Input {
    pub fn new(text: String) -> Self {
        Self {
            text,
            cursor: Default::default(),
        }
    }
    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn caret(&self) -> usize {
        self.cursor.caret(&self.text)
    }
    pub fn selected(&self) -> bool {
        self.cursor.all_selected(&self.text)
    }
    pub fn select_all(&mut self) {
        self.command(crate::field_edit::Command::SelectAll);
    }
    pub fn selection(&self) -> Option<&str> {
        self.cursor.selected(&self.text)
    }
    pub fn selection_range(&self) -> Option<std::ops::Range<usize>> {
        self.cursor.selection(&self.text)
    }
    pub fn cursor(&self) -> crate::field_edit::Cursor {
        self.cursor
    }
    pub fn command(&mut self, command: crate::field_edit::Command) -> crate::field_edit::Outcome {
        self.cursor.execute(&mut self.text, command)
    }
    pub fn edit(&mut self, edit: Edit) {
        self.command(crate::field_edit::Command::Edit {
            edit,
            extend: false,
        });
    }
    pub fn insert(&mut self, text: &str) {
        self.command(crate::field_edit::Command::Insert(text.into()));
    }
    pub fn visible(&self, columns: usize) -> Visible<'_> {
        self.cursor.visible(&self.text, columns)
    }
}

/// One caret step or deletion. Word steps stop as the editor's word motions do: spaces go with the word beside them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edit {
    Left,
    Right,
    WordLeft,
    WordRight,
    Start,
    End,
    Backspace,
    Delete,
    BackspaceWord,
    DeleteWord,
}

/// `caret` kept within `text` on a character boundary.
pub fn clamp(text: &str, caret: usize) -> usize {
    text.floor_char_boundary(caret.min(text.len()))
}

/// Grapheme boundaries for bounded native field hit-testing, including the final insertion point.
pub fn boundaries(text: &str) -> impl Iterator<Item = usize> + '_ {
    text.grapheme_indices(true)
        .map(|(at, _)| at)
        .chain(std::iter::once(text.len()))
}

/// Apply `edit` at `caret`; whether `text` changed.
pub fn apply(text: &mut String, caret: &mut usize, edit: Edit) -> bool {
    let at = clamp(text, *caret);
    let (moved, removed) = match edit {
        Edit::Left => (previous(text, at), None),
        Edit::Right => (next(text, at), None),
        Edit::WordLeft => (word_stop(text, at, true), None),
        Edit::WordRight => (word_stop(text, at, false), None),
        Edit::Start => (0, None),
        Edit::End => (text.len(), None),
        Edit::Backspace => (previous(text, at), Some(previous(text, at)..at)),
        Edit::Delete => (at, Some(at..next(text, at))),
        Edit::BackspaceWord => (
            word_stop(text, at, true),
            Some(word_stop(text, at, true)..at),
        ),
        Edit::DeleteWord => (at, Some(at..word_stop(text, at, false))),
    };
    *caret = moved;
    match removed.filter(|range| !range.is_empty()) {
        Some(range) => {
            text.replace_range(range, "");
            true
        }
        None => false,
    }
}

/// [`apply`] to a field whose whole text may be selected (Ctrl/Cmd+A): then a step leaves the selection at its start
/// (left, a word left, the start) or its end, and a deletion clears the field. Whether `text` changed.
pub fn apply_selected(
    text: &mut String,
    caret: &mut usize,
    selected: &mut bool,
    edit: Edit,
) -> bool {
    if !std::mem::take(selected) {
        return apply(text, caret, edit);
    }
    match edit {
        Edit::Left | Edit::WordLeft | Edit::Start => {
            *caret = 0;
            false
        }
        Edit::Right | Edit::WordRight | Edit::End => {
            *caret = text.len();
            false
        }
        Edit::Backspace | Edit::Delete | Edit::BackspaceWord | Edit::DeleteWord => {
            let changed = !text.is_empty();
            text.clear();
            *caret = 0;
            changed
        }
    }
}

/// Type `typed` at `caret` (control characters dropped), leaving the caret after it.
pub fn insert(text: &mut String, caret: &mut usize, typed: &str) {
    let at = clamp(text, *caret);
    let typed: String = typed.chars().filter(|c| !c.is_control()).collect();
    text.insert_str(at, &typed);
    *caret = at + typed.len();
}

fn previous(text: &str, at: usize) -> usize {
    text.get(..at)
        .unwrap_or_default()
        .grapheme_indices(true)
        .next_back()
        .map_or(0, |(i, _)| i)
}

fn next(text: &str, at: usize) -> usize {
    text.get(at..)
        .unwrap_or_default()
        .graphemes(true)
        .next()
        .map_or(at, |g| at + g.len())
}

/// Spaces, then a run of word characters (letters, digits, `_`) or of other symbols.
fn word_stop(text: &str, at: usize, backward: bool) -> usize {
    word_stop_chunks(text.len(), 0..text.len(), at, backward, true, |_| Some((0, text)))
}

/// A word stop over borrowed UTF-8 chunks. Graphemes, including ones spanning chunk boundaries, are
/// classified by their first scalar; combining marks never become separate word/deletion stops.
/// `chunk` returns the complete source chunk containing a byte (which need not itself be a character boundary).
/// No source text is assembled, and only the word/space runs beside the caret and required grapheme context
/// are visited. `spaces` retains the editor's option to cross an adjacent whitespace run as part of the step.
pub fn word_stop_chunks<'a>(
    length: usize,
    bounds: std::ops::Range<usize>,
    at: usize,
    backward: bool,
    spaces: bool,
    chunk: impl FnMut(usize) -> Option<(usize, &'a str)>,
) -> usize {
    if bounds.start > bounds.end || bounds.end > length { return at.min(length); }
    let at = at.clamp(bounds.start, bounds.end);
    let edge = if backward { bounds.start } else { bounds.end };
    if at == edge { return at; }
    let Some(mut graphemes) = ChunkGraphemes::new(length, at, backward, chunk) else { return at };
    let mut position = at;
    let mut kind = None;
    loop {
        let (next, current) = if let Some(run) = graphemes.ascii_run(position, backward, &bounds, kind) {
            run
        } else {
            let Some(next) = graphemes.step(backward) else { return position };
            let next = next.clamp(bounds.start, bounds.end);
            let start = if backward { next } else { position };
            let Some(current) = graphemes.class(start) else { return position };
            (next, current)
        };
        if next == position { return position; }
        if kind.is_none() && spaces && current == 0 {
            position = next;
        } else {
            if kind.is_some_and(|kind| kind != current) { return position; }
            kind = Some(current);
            position = next;
        }
        if position == edge { return position; }
    }
}

struct ChunkGraphemes<'a, F> {
    source: F,
    chunk: (usize, &'a str),
    cursor: unicode_segmentation::GraphemeCursor,
    length: usize,
}
impl<'a, F: FnMut(usize) -> Option<(usize, &'a str)>> ChunkGraphemes<'a, F> {
    fn new(length: usize, at: usize, backward: bool, mut source: F) -> Option<Self> {
        let chunk = source(if backward { at.checked_sub(1)? } else { at })?;
        Some(Self { source, chunk, cursor: unicode_segmentation::GraphemeCursor::new(at, length, true), length })
    }
    /// Consecutive ASCII bytes have grapheme boundaries except CRLF. Leave the byte adjoining an
    /// unknown/non-ASCII neighbour to GraphemeCursor: it may share a cluster with a Prepend or mark.
    fn ascii_run(&mut self, at: usize, backward: bool, bounds: &std::ops::Range<usize>, kind: Option<u8>) -> Option<(usize, u8)> {
        let byte = if backward { at.checked_sub(1)? } else { at };
        if byte < self.chunk.0 || byte >= self.chunk.0 + self.chunk.1.len() {
            self.chunk = (self.source)(byte)?;
        }
        let (start, text) = self.chunk;
        let bytes = text.as_bytes();
        let mut index = at.checked_sub(start)?;
        let first = *bytes.get(if backward { index.checked_sub(1)? } else { index })?;
        let class = ascii_word_class(first)?;
        let low = bounds.start.saturating_sub(start);
        let high = bounds.end.saturating_sub(start).min(bytes.len());
        loop {
            let offset = if backward {
                let Some(offset) = index.checked_sub(1) else { break };
                offset
            } else { index };
            if offset < low || offset >= high { break; }
            if ascii_word_class(bytes[offset]) != Some(class) { break; }
            let safe = if backward {
                if offset == 0 { start == 0 } else { bytes[offset - 1].is_ascii() }
            } else {
                if offset + 1 == bytes.len() { start + bytes.len() == self.length }
                else { bytes[offset + 1].is_ascii() }
            };
            if !safe { break; }
            if kind.is_some_and(|kind| kind != class) { return Some((at, class)); }
            if backward { index -= 1; } else { index += 1; }
        }
        let next = start + index;
        if next == at { return None; }
        self.cursor.set_cursor(next);
        Some((next, class))
    }
    fn step(&mut self, backward: bool) -> Option<usize> {
        use unicode_segmentation::GraphemeIncomplete;
        loop {
            let boundary = if backward {
                self.cursor.prev_boundary(self.chunk.1, self.chunk.0)
            } else {
                self.cursor.next_boundary(self.chunk.1, self.chunk.0)
            };
            match boundary {
                Ok(boundary) => return boundary,
                Err(GraphemeIncomplete::PrevChunk) => {
                    self.chunk = (self.source)(self.cursor.cur_cursor().checked_sub(1)?)?;
                }
                Err(GraphemeIncomplete::NextChunk) => {
                    self.chunk = (self.source)(self.cursor.cur_cursor())?;
                }
                Err(GraphemeIncomplete::PreContext(end)) => {
                    let (start, text) = (self.source)(end.checked_sub(1)?)?;
                    let context = text.get(..end.checked_sub(start)?)?;
                    if context.is_empty() { return None; }
                    self.cursor.provide_context(context, start);
                }
                Err(GraphemeIncomplete::InvalidOffset) => return None,
            }
        }
    }
    fn class(&mut self, at: usize) -> Option<u8> {
        let (start, text) = if self.chunk.0 <= at && at < self.chunk.0 + self.chunk.1.len() {
            self.chunk
        } else { (self.source)(at)? };
        let first = text.get(at.checked_sub(start)?..)?.chars().next()?;
        Some(if first.is_whitespace() { 0 } else if first.is_alphanumeric() || first == '_' { 1 } else { 2 })
    }
}

fn ascii_word_class(byte: u8) -> Option<u8> {
    if !byte.is_ascii() || matches!(byte, b'\r' | b'\n') { return None; }
    Some(if matches!(byte, b' ' | b'\t'..=b'\r') { 0 }
         else if byte.is_ascii_alphanumeric() || byte == b'_' { 1 } else { 2 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unicode_word_stops_and_deletions_keep_whole_graphemes() {
        for token in ["cafe\u{301}", "नमस्ते", "👩🏽‍💻", "🇬🇷🇯🇵", "1\u{fe0f}\u{20e3}"] {
            assert_eq!(run(token, token.len(), &[Edit::WordLeft]).1, 0, "{token}");
            assert_eq!(run(token, 0, &[Edit::WordRight]).1, token.len(), "{token}");
            assert_eq!(run(token, token.len(), &[Edit::BackspaceWord]), (String::new(), 0), "{token}");
            assert_eq!(run(token, 0, &[Edit::DeleteWord]), (String::new(), 0), "{token}");
        }
    }

    #[test]
    fn borrowed_word_chunks_preserve_graphemes_and_whitespace_policy() {
        for text in ["foo cafe\u{301}  👩🏽‍💻🇬🇷1\u{fe0f}\u{20e3} end",
            "\u{600}abc \u{600}!def", "a\r\nb\t\u{b}c", "ab\u{301}cd \u{600}abc",
            "\u{600}a\u{301}z", "x\u{200d}abcdef", "12\u{fe0f}\u{20e3}345"] {
        for size in 1..=16 {
            let mut chunks = Vec::new();
            let mut start = 0;
            while start < text.len() {
                let end = text.floor_char_boundary((start + size).min(text.len()));
                let end = if end == start { start + text.get(start..).unwrap().chars().next().unwrap().len_utf8() } else { end };
                chunks.push((start, &text[start..end]));
                start = end;
            }
            for at in boundaries(text) { for backward in [false, true] { for spaces in [false, true] {
                let expected = word_stop_chunks(text.len(), 0..text.len(), at, backward, spaces, |_| Some((0, text)));
                let actual = word_stop_chunks(text.len(), 0..text.len(), at, backward, spaces, |byte| {
                    chunks.get(chunks.partition_point(|(start, _)| *start <= byte).checked_sub(1)?).copied()
                });
                assert_eq!(actual, expected, "chunk size {size}, at {at}, backward {backward}, spaces {spaces}");
                assert!(boundaries(text).any(|boundary| boundary == actual));
            }}}
        }
        }
        assert_eq!(word_stop_chunks(7, 0..7, 4, true, false, |_| Some((0, "one two"))), 3);
        assert_eq!(word_stop_chunks(7, 0..7, 4, true, true, |_| Some((0, "one two"))), 0);
    }

    #[test]
    fn long_fields_reveal_home_middle_and_end_without_splitting_unicode() {
        let text = "START_abcdefghijklmnopqrstuvwxyz_é界👩🏽‍💻_TAIL";
        for columns in [0, 1, 2, 5, 20, 100] {
            for at in
                std::iter::once(0).chain(text.grapheme_indices(true).map(|(i, g)| i + g.len()))
            {
                let shown = visible(text, at, columns);
                assert!(shown.text.width() <= columns);
                assert!(shown.caret < columns.max(1));
                assert!(text.contains(shown.text));
            }
        }
        assert!(visible(text, 0, 10).text.starts_with("START_"));
        assert!(visible(text, text.len(), 10).text.ends_with("_TAIL"));
        assert_eq!(visible("é界x", "é界".len(), 3).text, "界x");
        assert_eq!(visible("é界x", "é界".len(), 3).caret, 2);
        assert_eq!(visible("ab\u{301}c", 0, 2).text, "ab\u{301}");
        let large = format!("START_{}TAIL", "long-path/".repeat(100_000));
        assert_eq!(visible(&large, 0, 6).text, "START_");
        assert!(visible(&large, large.len(), 70).text.ends_with("TAIL"));
        let mut field = Input::new(text.into());
        field.edit(Edit::Start);
        field.insert("INSERTED_");
        assert!(field.visible(20).text.starts_with("INSERTED_START_"));
        field.select_all();
        field.insert("/valid/path");
        assert_eq!(field.text(), "/valid/path");
        field.edit(Edit::WordLeft);
        field.edit(Edit::DeleteWord);
        assert_eq!(field.text(), "/valid/");
    }

    fn run(text: &str, caret: usize, edits: &[Edit]) -> (String, usize) {
        let (mut text, mut caret) = (text.to_owned(), caret);
        for edit in edits {
            apply(&mut text, &mut caret, *edit);
        }
        (text, caret)
    }

    #[test]
    fn word_steps_stop_at_the_value_the_colon_and_the_key() {
        let text = "column: timestamp";
        let stops: Vec<_> = (1..=4)
            .map(|n| run(text, text.len(), &vec![Edit::WordLeft; n]).1)
            .collect();
        assert_eq!(stops, [8, 6, 0, 0]);
        let stops: Vec<_> = (1..=4)
            .map(|n| run(text, 0, &vec![Edit::WordRight; n]).1)
            .collect();
        assert_eq!(stops, [6, 7, 17, 17]);
    }

    #[test]
    fn typing_and_deleting_happen_at_the_caret() {
        let (mut text, mut caret) = ("sketch.yaml".to_owned(), 6);
        insert(&mut text, &mut caret, " pad\n");
        assert_eq!((text.as_str(), caret), ("sketch pad.yaml", 10));
        assert_eq!(
            run(&text, caret, &[Edit::BackspaceWord]),
            ("sketch .yaml".into(), 7)
        );
        assert_eq!(
            run(&text, caret, &[Edit::BackspaceWord, Edit::BackspaceWord]),
            (".yaml".into(), 0),
            "the space goes with the word"
        );
        assert_eq!(
            run("a_b c", 3, &[Edit::BackspaceWord]),
            (" c".into(), 0),
            "`_` is a word character"
        );
        assert_eq!(
            run(&text, caret, &[Edit::DeleteWord]),
            ("sketch padyaml".into(), 10)
        );
        assert_eq!(
            run(
                &text,
                caret,
                &[Edit::Start, Edit::Delete, Edit::End, Edit::Backspace]
            ),
            ("ketch pad.yam".into(), 13)
        );
        assert_eq!(
            run(&text, caret, &[Edit::Left, Edit::Left, Edit::Right]),
            (text.clone(), 9)
        );
        assert_eq!(
            run(
                "",
                5,
                &[
                    Edit::Backspace,
                    Edit::Delete,
                    Edit::BackspaceWord,
                    Edit::Left
                ]
            ),
            (String::new(), 0)
        );
    }

    #[test]
    fn a_selected_field_collapses_on_a_step_and_clears_on_a_deletion() {
        let step = |edit| {
            let (mut text, mut caret, mut selected) = ("draft notes".to_owned(), 3, true);
            let changed = apply_selected(&mut text, &mut caret, &mut selected, edit);
            (text, caret, selected, changed)
        };
        assert_eq!(step(Edit::Left), ("draft notes".into(), 0, false, false));
        assert_eq!(
            step(Edit::WordRight),
            ("draft notes".into(), 11, false, false)
        );
        assert_eq!(step(Edit::BackspaceWord), (String::new(), 0, false, true));
        let (mut text, mut caret, mut selected) = ("draft notes".to_owned(), 5, false);
        assert!(apply_selected(
            &mut text,
            &mut caret,
            &mut selected,
            Edit::BackspaceWord
        ));
        assert_eq!(
            (text.as_str(), caret),
            (" notes", 0),
            "nothing selected: as apply"
        );
    }

    #[test]
    fn steps_cross_whole_characters() {
        assert_eq!(run("aé👩🏽‍💻b", 1, &[Edit::Right]), ("aé👩🏽‍💻b".into(), 3));
        assert_eq!(
            run("aé👩🏽‍💻b", 18, &[Edit::Left]),
            ("aé👩🏽‍💻b".into(), 3),
            "an emoji sequence is one step"
        );
        assert_eq!(run("aé👩🏽‍💻b", 18, &[Edit::Backspace]), ("aéb".into(), 3));
        assert_eq!(clamp("é", 1), 0);
    }
}
