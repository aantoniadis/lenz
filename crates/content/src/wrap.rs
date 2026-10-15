//! Soft-wrap layout of one displayed line (tabs expanded, long lines cut: [`crate::display::display_line`]) at a
//! width in columns. Rows after the first are indented like the line, by at most half the width.

use unicode_width::UnicodeWidthChar;

/// The columns `text` takes.
pub fn width(text: &str) -> usize {
    text.chars().map(|c| c.width().unwrap_or(0)).sum()
}

/// The line's leading whitespace width, at most half of `cols`: where its continuation rows start.
pub fn continuation_indent(text: &str, cols: usize) -> usize {
    let lead = text.chars().take_while(|c| *c == ' ' || *c == '\t').map(|c| c.width().unwrap_or(0)).sum::<usize>();
    lead.min(cols / 2)
}

fn breaks_after(c: char) -> bool {
    c.is_whitespace() || matches!(c, ',' | ';' | '.' | ')' | ']' | '}' | '>' | '-' | '/' | '|' | '&')
}

/// Byte offsets where `text`'s continuation rows start (empty when it fits `cols`). A row breaks after the last
/// whitespace or punctuation that fits (spaces at the edge hang past it; indentation is never a break); a run with
/// none is cut at the edge, never inside a character.
pub fn wrap_starts(text: &str, cols: usize) -> Vec<usize> {
    let cols = cols.max(1);
    let indent = continuation_indent(text, cols);
    let mut starts = Vec::new();
    // The indentation is never a break point: a row of only indentation is no use.
    let lead = text.len() - text.trim_start_matches([' ', '\t']).len();
    let (mut row, mut used, mut room, mut last_break) = (0, 0, cols, None::<usize>);
    for (i, c) in text.char_indices() {
        let w = c.width().unwrap_or(0);
        // Whitespace hangs past the edge rather than starting a row of its own.
        if used + w > room && i > row && !c.is_whitespace() {
            let at = last_break.filter(|b| *b > row && *b <= i).unwrap_or(i);
            starts.push(at);
            row = at;
            room = (cols - indent).max(1);
            used = text.get(at..i).map_or(0, width);
            last_break = None;
        }
        used += w;
        if breaks_after(c) && i >= lead {
            last_break = Some(i + c.len_utf8());
        }
    }
    starts
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(text: &str, cols: usize) -> Vec<&str> {
        let starts = wrap_starts(text, cols);
        let mut out = Vec::new();
        let mut from = 0;
        for &s in starts.iter().chain(std::iter::once(&text.len())) {
            out.push(text.get(from..s).unwrap());
            from = s;
        }
        out
    }

    #[test]
    fn lines_that_fit_have_no_starts() {
        assert!(wrap_starts("short", 10).is_empty());
        assert!(wrap_starts("exactly10!", 10).is_empty());
        assert!(wrap_starts("", 1).is_empty());
    }

    #[test]
    fn rows_break_after_spaces_and_punctuation() {
        assert_eq!(rows("alpha beta gamma", 11), ["alpha beta ", "gamma"]);
        assert_eq!(rows("call(a,b,c)", 7), ["call(a,", "b,c)"]);
    }

    #[test]
    fn a_run_without_a_break_is_cut_at_the_edge() {
        assert_eq!(rows("abcdefghij", 4), ["abcd", "efgh", "ij"]);
        assert_eq!(rows("ab", 1), ["a", "b"]);
    }

    #[test]
    fn wide_and_zero_width_characters_count_their_columns() {
        assert_eq!(rows("日本語です", 4), ["日本", "語で", "す"]);
        assert_eq!(rows("e\u{301}e\u{301}e\u{301}", 2), ["e\u{301}e\u{301}", "e\u{301}"]);
        assert_eq!(rows("日", 1), ["日"], "a character wider than the row still gets one");
    }

    #[test]
    fn indentation_is_never_a_row_and_spaces_hang_past_the_edge() {
        assert_eq!(rows("    ab cd ef", 6), ["    ab ", "cd ", "ef"]);
        // Indentation wider than the row: it hangs on the first row, the word goes on the next rows.
        assert_eq!(rows("        longword", 6), ["        ", "lon", "gwo", "rd"]);
    }

    #[test]
    fn continuation_rows_keep_the_indent_within_half_the_width() {
        assert_eq!(continuation_indent("    let x = 1;", 40), 4);
        assert_eq!(continuation_indent("                    deep", 10), 5);
        assert_eq!(continuation_indent("flat", 40), 0);
        // Rows after the first have cols - indent columns.
        assert_eq!(rows("    aaaa bbbb cccc", 10), ["    aaaa ", "bbbb ", "cccc"]);
    }
}
