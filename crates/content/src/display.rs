//! How source text is drawn without changing it: control and bidi characters shown as U+FFFD, runs of
//! zero-width marks capped, tabs expanded and very long lines cut short, with a map from every source byte to
//! its display byte so styles, selections and hit tests keep source coordinates. GPUI-free; both Lenz
//! frontends and the code reader use it.

/// Lines longer than this are cut short on screen.
pub const LONG_LINE: usize = 10_000;

/// A character that would hide or reorder what a reviewer sees: a C0/C1 control, or one of the Unicode
/// bidirectional embedding, override and isolate controls of "Trojan Source" (CVE-2021-42574), which make
/// a terminal or text shaper display source in an order the compiler never sees. Both frontends show U+FFFD
/// instead. Joiners, variation selectors and marks are not included: they are part of emoji and scripts.
pub fn hides_text(c: char) -> bool {
    c.is_control() || is_bidi_control(c)
}

fn is_bidi_control(c: char) -> bool {
    matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// Zero-width characters (combining marks, joiners) drawn on one character: Unicode calls a run longer than 30
/// abnormal (UAX #15, stream-safe text). Thousands stacked on one letter cost the GUI's shaper ~7 ms a line,
/// 381 ms a screenful, and draw over neighbouring rows; past this many, the rest are not drawn. Copy is unchanged.
pub const MAX_MARKS: usize = 30;

/// Whether `c` is drawn, given `run`: the zero-width characters drawn since the last visible one (updated).
pub fn shows_mark(c: char, run: &mut usize) -> bool {
    if unicode_width::UnicodeWidthChar::width(c) != Some(0) {
        *run = 0;
        return true;
    }
    *run += 1;
    *run <= MAX_MARKS
}

/// `text` without the zero-width characters past [`MAX_MARKS`] on one character.
pub fn cap_marks(text: &str) -> std::borrow::Cow<'_, str> {
    let mut run = 0;
    if text.chars().all(|c| shows_mark(c, &mut run)) {
        return text.into();
    }
    let mut run = 0;
    text.chars().filter(|&c| shows_mark(c, &mut run)).collect::<String>().into()
}

/// `text` with its bidi controls shown as U+FFFD. Both are three UTF-8 bytes, so every byte offset into `text`
/// (styles, selections, glyph maps) stays valid: for text shaped as it is, where the offsets are already fixed.
pub fn show_bidi(text: &str) -> std::borrow::Cow<'_, str> {
    if !text.contains(is_bidi_control) {
        return text.into();
    }
    text.chars().map(|c| if is_bidi_control(c) { '\u{FFFD}' } else { c }).collect::<String>().into()
}

/// Expand tabs to 4 spaces and show hidden characters as U+FFFD, mapping each source byte offset (and the end)
/// to its display offset.
pub fn expand_tabs(line: &str) -> (String, Vec<usize>) {
    let mut out = String::with_capacity(line.len());
    let mut map = Vec::with_capacity(line.len() + 1);
    let mut marks = 0;
    for (i, ch) in line.char_indices() {
        map.resize(i + 1, out.len());
        if !shows_mark(ch, &mut marks) {
            continue; // past the cap: maps to where the next drawn character goes
        }
        if ch == '\t' {
            out.push_str("    ");
        } else if hides_text(ch) {
            out.push('\u{FFFD}');
        } else {
            out.push(ch);
        }
    }
    map.resize(line.len() + 1, out.len());
    (out, map)
}

/// The source bytes of `line` that are drawn: all of it, or its first [`LONG_LINE`] characters.
pub fn displayed_bytes(line: &str) -> usize {
    line.char_indices().nth(LONG_LINE).map_or(line.len(), |(b, _)| b)
}

/// A source line as drawn: its displayed prefix with tabs expanded, plus "  … N more characters" when cut, and
/// the display offset of each source byte of the prefix ([`expand_tabs`]): its length is the prefix's byte length
/// plus one.
pub fn display_line(line: &str) -> (String, Vec<usize>) {
    let cut = displayed_bytes(line);
    let (mut text, map) = expand_tabs(&line[..cut]);
    if cut < line.len() {
        text += &format!("  … {} more characters", line[cut..].chars().count());
    }
    (text, map)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tabs_hidden_characters_and_marks_keep_a_source_map() {
        let (shown, map) = expand_tabs("\tlet x = \"a\tb\";");
        assert_eq!(shown, "    let x = \"a    b\";");
        assert_eq!(&shown[map[1]..map[4]], "let");
        assert_eq!(&shown[map[9]..map[14]], "\"a    b\"");
        assert_eq!(map[15], shown.len());
        let (shown, map) = expand_tabs("a\u{202E}b\x1bc");
        assert_eq!(shown, "a\u{FFFD}b\u{FFFD}c");
        assert_eq!(&shown[map[1]..map[4]], "\u{FFFD}");
        assert_eq!(map[7], shown.len());
        let line = format!("a{}b", "\u{300}".repeat(40));
        let (shown, map) = expand_tabs(&line);
        assert_eq!(shown, format!("a{}b", "\u{300}".repeat(MAX_MARKS)));
        assert_eq!(map[line.len() - 1], shown.len() - 1);
        assert_eq!(map[line.len()], shown.len());
    }

    #[test]
    fn long_lines_are_cut_with_a_count_and_short_ones_are_whole() {
        assert_eq!(display_line("a\tb"), ("a    b".to_string(), vec![0, 1, 5, 6]));
        let long = "é".repeat(LONG_LINE + 3);
        let (text, map) = display_line(&long);
        assert!(text.ends_with("  … 3 more characters"), "{}", &text[text.len() - 30..]);
        assert_eq!(map.len(), LONG_LINE * 2 + 1);
        assert_eq!(displayed_bytes(&long), LONG_LINE * 2);
        assert_eq!(display_line(""), (String::new(), vec![0]));
    }

    #[test]
    fn display_transforms_keep_byte_offsets_or_say_so() {
        assert_eq!(show_bidi("a\u{202E}b"), "a\u{FFFD}b");
        assert_eq!(show_bidi("a\u{202E}b").len(), "a\u{202E}b".len());
        assert_eq!(cap_marks(&format!("a{}", "\u{301}".repeat(31))).chars().count(), 1 + MAX_MARKS);
        assert!(hides_text('\u{7}') && hides_text('\u{2066}') && !hides_text('\u{200D}'));
    }
}
