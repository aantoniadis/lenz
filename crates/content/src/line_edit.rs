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
    let class = |c: char| {
        if c.is_alphanumeric() || c == '_' {
            1
        } else {
            2
        }
    };
    if backward {
        let mut chars = text
            .get(..at)
            .unwrap_or_default()
            .char_indices()
            .rev()
            .skip_while(|(_, c)| c.is_whitespace())
            .peekable();
        let Some(&(_, first)) = chars.peek() else {
            return 0;
        };
        chars
            .take_while(|(_, c)| !c.is_whitespace() && class(*c) == class(first))
            .last()
            .map_or(at, |(i, _)| i)
    } else {
        let mut chars = text
            .get(at..)
            .unwrap_or_default()
            .char_indices()
            .skip_while(|(_, c)| c.is_whitespace())
            .peekable();
        let Some(&(_, first)) = chars.peek() else {
            return text.len();
        };
        chars
            .find(|(_, c)| c.is_whitespace() || class(*c) != class(first))
            .map_or(text.len(), |(i, _)| at + i)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
