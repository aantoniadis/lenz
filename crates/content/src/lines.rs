//! Line offsets over immutable text. Lines end at `\n`, and a `\r` right before it belongs to the line
//! ending, exactly as `lenz_engine::split_lines` splits them. Coordinates are byte offsets into the
//! original text, so its actual line endings survive: line `l`'s ending is the bytes between
//! [`line_range`]`(l).end` and the next line's start.

use std::ops::Range;

/// Byte offset of each line's start. Empty text is one empty line; a trailing `\n` starts an empty last line.
pub fn line_starts(text: &str) -> Vec<usize> {
    std::iter::once(0).chain(text.match_indices('\n').map(|(i, _)| i + 1)).collect()
}

/// Line `l`'s byte range in `text`, without its `\n` or `\r\n`. `starts` is [`line_starts`]`(text)`;
/// `l` must be below `starts.len()`.
pub fn line_range(text: &str, starts: &[usize], l: usize) -> Range<usize> {
    let start = starts[l];
    let Some(&next) = starts.get(l + 1) else { return start..text.len() }; // a lone `\r` at the end is content
    let end = next - 1;
    start..if text.as_bytes()[start..end].ends_with(b"\r") { end - 1 } else { end }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> Vec<&str> {
        let starts = line_starts(text);
        (0..starts.len()).map(|l| &text[line_range(text, &starts, l)]).collect()
    }

    /// Each line's ending, recovered from the coordinates.
    fn endings(text: &str) -> Vec<&str> {
        let starts = line_starts(text);
        (0..starts.len()).map(|l| &text[line_range(text, &starts, l).end..starts.get(l + 1).copied().unwrap_or(text.len())]).collect()
    }

    #[test]
    fn empty_text_is_one_empty_line() {
        assert_eq!(line_starts(""), [0]);
        assert_eq!(lines(""), [""]);
    }

    #[test]
    fn lf_crlf_and_mixed_endings_survive() {
        let text = "a\r\nb\nc\r\n";
        assert_eq!(line_starts(text), [0, 3, 5, 8]);
        assert_eq!(lines(text), ["a", "b", "c", ""]);
        assert_eq!(endings(text), ["\r\n", "\n", "\r\n", ""]);
        assert_eq!(endings(text).concat().len() + lines(text).concat().len(), text.len());
    }

    #[test]
    fn lone_carriage_returns_are_content() {
        assert_eq!(lines("a\rb\nc\r"), ["a\rb", "c\r"]);
    }

    #[test]
    fn unicode_offsets_are_byte_offsets_on_char_boundaries() {
        let text = "café\n日本\r\n🦀";
        let starts = line_starts(text);
        assert_eq!(starts, [0, 6, 14]);
        assert_eq!(lines(text), ["café", "日本", "🦀"]);
        assert!((0..starts.len()).all(|l| { let r = line_range(text, &starts, l); text.is_char_boundary(r.start) && text.is_char_boundary(r.end) }));
    }

    #[test]
    fn matches_the_comparison_engines_split() {
        for text in ["", "\n", "\r\n", "a", "a\r", "a\r\r\n", "\u{FFFD}\n\r", "x\ny\r\nz\r"] {
            assert_eq!(lines(text), lenz_engine::split_lines(text), "{text:?}");
        }
    }
}
