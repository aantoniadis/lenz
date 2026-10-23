//! A line as the text panes draw it: Lenz's display line with inlay hint labels between its characters. Hints are
//! display only: the map still takes each drawn source byte to where its character is drawn, so selections, find
//! tints, links and hit tests keep source coordinates and never land inside a hint.

use std::ops::Range;

use crate::decor::{Hint, HintKind};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shown {
    pub text: String,
    /// Each drawn source byte's display offset (after any hint before it), one past the end included.
    pub map: Vec<usize>,
    /// Each hint's source byte, display range and kind, in order.
    pub hints: Vec<(usize, Range<usize>, HintKind)>,
}

impl Shown {
    /// Where source byte `b` is drawn: after the hints before it.
    pub fn at(&self, b: usize) -> usize {
        self.map[b.min(self.map.len() - 1)]
    }
    /// Where a range ending at source byte `b` ends: before the hints at `b`.
    pub fn end(&self, b: usize) -> usize {
        let b = b.min(self.map.len() - 1);
        let i = self.hints.partition_point(|(h, ..)| *h < b);
        self.hints.get(i).filter(|(h, ..)| *h == b).map_or(self.map[b], |(_, r, _)| r.start)
    }
    /// Where the caret at source byte `b` is drawn: before a type hint at `b` (`x▮: i32`), after a parameter hint
    /// there (`a: ▮1`).
    pub fn caret(&self, b: usize) -> usize {
        let b = b.min(self.map.len() - 1);
        let i = self.hints.partition_point(|(h, ..)| *h < b);
        self.hints[i..].iter().take_while(|(h, ..)| *h == b).find(|(.., k)| *k == HintKind::Type).map_or_else(|| self.at(b), |(_, r, _)| r.start)
    }
    /// A source range's display range (empty ranges sit before the hints at their byte).
    pub fn range(&self, r: Range<usize>) -> Range<usize> {
        let end = self.end(r.end);
        self.at(r.start).min(end)..end
    }
}

/// `line` as drawn ([`lenz_content::display::display_line`]) with `hints` (sorted by byte) inserted before their
/// bytes. Hints past the drawn part of a cut line, or inside a character, are left out.
#[cfg(test)]
pub(crate) fn shown_line(line: &str, hints: &[Hint]) -> Shown {
    let (text, mut map) = lenz_content::display::display_line(line);
    let cut = map.len() - 1;
    let hints: Vec<&Hint> = hints.iter().filter(|h| (h.byte as usize) <= cut && line.is_char_boundary(h.byte as usize)).collect();
    if hints.is_empty() {
        return Shown { text, map, hints: Vec::new() };
    }
    let mut out = String::with_capacity(text.len() + hints.iter().map(|h| h.label.len()).sum::<usize>());
    let mut ranges = Vec::with_capacity(hints.len());
    let (mut from, mut shift, mut next) = (0, 0, 0);
    for (b, d) in map.iter_mut().enumerate() {
        while let Some(h) = hints.get(next).filter(|h| h.byte as usize == b) {
            out.push_str(&text[from..*d]);
            from = *d;
            let start = out.len();
            out.push_str(&h.label);
            ranges.push((b, start..out.len(), h.kind));
            shift += h.label.len();
            next += 1;
        }
        *d += shift;
    }
    out.push_str(&text[from..]);
    Shown { text: out, map, hints: ranges }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decor::{Hint, HintKind};

    fn h(byte: u32, label: &str) -> Hint {
        Hint { byte, label: label.into(), kind: HintKind::Type }
    }

    fn p(byte: u32, label: &str) -> Hint {
        Hint { byte, label: label.into(), kind: HintKind::Parameter }
    }

    #[test]
    fn without_hints_it_is_the_display_line() {
        let s = shown_line("\tab", &[]);
        assert_eq!((s.text, s.map), lenz_content::display::display_line("\tab"));
        assert!(s.hints.is_empty());
    }

    #[test]
    fn a_hint_sits_before_its_byte_and_ranges_leave_it_out() {
        let s = shown_line("let x = 1;", &[h(5, ": i32")]);
        assert_eq!(s.text, "let x: i32 = 1;");
        assert_eq!(s.hints, vec![(5, 5..10, HintKind::Type)]);
        assert_eq!((s.at(4), s.at(5), s.end(5)), (4, 10, 5));
        assert_eq!(s.range(4..5), 4..5, "the name alone, not its hint");
        assert_eq!(s.range(0..10), 0..15);
        assert_eq!(s.range(5..5), 5..5);
        assert_eq!(*s.map.last().unwrap(), s.text.len());
    }

    #[test]
    fn the_caret_sits_before_a_type_hint_and_after_a_parameter_hint() {
        let s = shown_line("let x = f(1);", &[h(5, ": i32"), p(10, "a: ")]);
        assert_eq!(s.text, "let x: i32 = f(a: 1);");
        assert_eq!(s.caret(5), 5, "x▮: i32");
        assert_eq!(s.caret(10), "let x: i32 = f(a: ".len(), "a: ▮1");
        assert_eq!(s.caret(4), 4, "no hint: where the byte is drawn");
        // Selection ends stay before every hint at their byte.
        assert_eq!((s.end(5), s.end(10)), (5, "let x: i32 = f(".len()));
        // Both kinds at one byte: after the parameter hint, before the type hint drawn after it.
        let s = shown_line("ab", &[p(1, "P"), h(1, "T")]);
        assert_eq!((s.text.as_str(), s.caret(1)), ("aPTb", 2));
    }

    #[test]
    fn hints_at_the_start_the_end_and_several_at_one_byte() {
        let s = shown_line("f(1)", &[h(0, "<"), h(2, "a: "), h(2, "b: "), h(4, " ;")]);
        assert_eq!(s.text, "<f(a: b: 1) ;");
        assert_eq!((s.at(0), s.end(0)), (1, 0));
        assert_eq!((s.at(2), s.end(2)), (9, 3));
        assert_eq!((s.at(4), s.end(4)), (13, 11));
    }

    #[test]
    fn tabs_and_multibyte_text_keep_source_bytes_and_a_hint_inside_a_character_is_dropped() {
        let s = shown_line("\té=1", &[h(3, ": T")]);
        assert_eq!(s.text, "    é: T=1");
        assert_eq!((s.end(3), s.at(3)), ("    é".len(), "    é: T".len()));
        assert_eq!(shown_line("é", &[h(1, "x")]).text, "é");
    }

    #[test]
    fn a_display_offset_inside_a_hint_maps_to_the_hints_byte() {
        let s = shown_line("ab", &[h(1, "XYZ")]);
        assert_eq!(s.text, "aXYZb");
        for d in 1..=4 {
            assert_eq!(s.map.partition_point(|&x| x < d).min(s.map.len() - 1), 1, "offset {d}");
        }
        assert_eq!(s.map.partition_point(|&x| x < 5).min(s.map.len() - 1), 2);
    }

    #[test]
    fn a_cut_long_line_keeps_only_the_hints_on_its_drawn_part() {
        let line = "x".repeat(lenz_content::display::LONG_LINE + 5);
        let s = shown_line(&line, &[h(3, "A"), h((lenz_content::display::LONG_LINE + 2) as u32, "B")]);
        assert!(s.text.starts_with("xxxAx"));
        assert!(!s.text.contains('B'));
        assert_eq!(s.hints.len(), 1);
    }

    #[test]
    #[ignore = "timing; run in release"]
    fn per_frame_lookup_perf() {
        use crate::decor::{Hints, Token, overlay};
        let lines: Vec<String> = (0..60).map(|i| format!("    let value_{i} = compute(alpha, beta, gamma); // {i}")).collect();
        let hints = Hints::new((0..60u32).flat_map(|l| [(l, h(14, ": i32")), (l, h(30, "a: "))]).collect());
        let tokens: Vec<Token> = (0..60u32).flat_map(|l| (0..6u32).map(move |i| Token { line: l, start: i * 7, end: i * 7 + 4, capture: Some(lenz_content::highlight::Capture::Variable), mods: 0 })).collect();
        let syntax = vec![(4..7, lenz_content::highlight::Capture::Keyword), (40..60, lenz_content::highlight::Capture::Comment)];
        let t0 = std::time::Instant::now();
        for _ in 0..1000 {
            for (l, line) in lines.iter().enumerate() {
                std::hint::black_box(shown_line(line, hints.line(l)));
                std::hint::black_box(overlay(&syntax, &tokens[l * 6..l * 6 + 6]));
            }
        }
        println!("one 60-line frame: {:?}", t0.elapsed() / 1000);
        let t0 = std::time::Instant::now();
        for _ in 0..1000 {
            for line in &lines {
                std::hint::black_box(lenz_content::display::display_line(line));
            }
        }
        println!("baseline display_line frame: {:?}", t0.elapsed() / 1000);
    }
}
