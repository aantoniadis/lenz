//! What a double and a triple click select, the same in Lenz Diff and Lenz Code: the word under the pointer, then
//! the syntax group around it (Extend Selection's next step), or the line without a syntax tree.

use std::{ops::Range, sync::Arc};

use crate::highlight::{Language, selection_steps_while};

/// Larger texts are not parsed for a triple click; the line is selected instead (Lenz Code's large-file size).
pub const GROUP_MAX_BYTES: usize = 5 << 20;

/// Byte range of the word (alphanumerics and `_`), whitespace run, or punctuation run at `col`.
pub fn word_at(line: &str, col: usize) -> Range<usize> {
    let class = |c: char| {
        if c.is_alphanumeric() || c == '_' {
            0
        } else if c.is_whitespace() {
            1
        } else {
            2
        }
    };
    // the character under the caret, or the one before it at the end of the line
    let (before, after) = (line.get(..col).unwrap_or_default(), line.get(col..).unwrap_or_default());
    let Some((at, c)) = after.chars().next().map(|c| (col, c)).or_else(|| before.char_indices().next_back()) else {
        return col..col;
    };
    let k = class(c);
    let start = line.get(..at).unwrap_or_default().char_indices().rev().take_while(|&(_, c)| class(c) == k).last().map_or(at, |(i, _)| i);
    let end = line.get(at..).unwrap_or_default().char_indices().find(|&(_, c)| class(c) != k).map_or(line.len(), |(i, _)| at + i);
    start..end
}

/// The triple-click selection around `word` in `text`: the first Extend Selection step that strictly holds it, else
/// `line` (the word's line without its line break). Parses `text` once; call it off the UI thread.
pub fn group_around(lang: Option<Language>, text: &Arc<str>, word: Range<usize>, line: Range<usize>) -> Range<usize> {
    let Some(lang) = lang.filter(|_| text.len() <= GROUP_MAX_BYTES) else { return line };
    selection_steps_while(lang, text.clone(), word.clone(), || true)
        .and_then(|steps| steps.into_iter().find(|s| s.start <= word.start && s.end >= word.end && *s != word))
        .unwrap_or(line)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn double_click_picks_the_word_or_symbol_run_under_the_pointer() {
        assert_eq!(word_at("let foo_bar = 1;", 6), 4..11);
        assert_eq!(word_at("let foo_bar = 1;", 4), 4..11);
        assert_eq!(word_at("a  ->  b", 3), 3..5);
        assert_eq!(word_at("end", 3), 0..3);
        assert_eq!(word_at("", 0), 0..0);
        assert_eq!(word_at("héllo wörld", 8), 7..13);
    }

    #[test]
    fn group_is_the_next_extend_selection_step() {
        let text: Arc<str> = "fn f() { foo(bar, baz); }\n".into();
        let bar = 13..16;
        let line = 0..text.len() - 1;
        let group = group_around(Some(Language::Rust), &text, bar.clone(), line);
        let steps = selection_steps_while(Language::Rust, text.clone(), bar.clone(), || true).unwrap();
        assert_eq!(group, steps.into_iter().find(|s| *s != bar).unwrap());
        assert_eq!(&text[group], "bar, baz");
    }

    #[test]
    fn group_without_a_language_is_the_line() {
        let text: Arc<str> = "one two\nthree four\n".into();
        assert_eq!(group_around(None, &text, 8..13, 8..18), 8..18);
    }

    #[test]
    fn group_of_a_large_text_is_the_line() {
        let text: Arc<str> = format!("fn f() {{ foo(bar); }}\n{}", "x".repeat(GROUP_MAX_BYTES)).into();
        assert_eq!(group_around(Some(Language::Rust), &text, 13..16, 0..21), 0..21);
    }
}
