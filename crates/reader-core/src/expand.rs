//! Extend / Shrink Selection: a chain of ranges, each holding the last, walked forward and back. The chain comes from
//! the syntax tree (`lenz_content::highlight::selection_steps_while`) or, without one, from words, lines and
//! paragraphs.

use std::ops::Range;

use crate::{document::Document, session::Selection};

/// One expansion of a tab's selection: where it started, the ranges it grows through, and how many it has taken.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Expansion {
    /// The document version the steps were made for.
    pub version: u64,
    pub origin: Selection,
    pub steps: Vec<Range<usize>>,
    pub at: usize,
}

impl Expansion {
    /// The selection after the steps taken.
    pub fn current(&self) -> Selection {
        match self.at {
            0 => self.origin,
            n => Selection { anchor: self.steps[n - 1].start, head: self.steps[n - 1].end },
        }
    }

    pub fn extend(&mut self) -> Option<Selection> {
        (self.at < self.steps.len()).then(|| {
            self.at += 1;
            self.current()
        })
    }

    pub fn shrink(&mut self) -> Option<Selection> {
        (self.at > 0).then(|| {
            self.at -= 1;
            self.current()
        })
    }
}

/// Without a syntax tree: the word at (or just before) a caret, the lines (without the last one's ending), the
/// paragraph (up to blank lines), the whole text; each kept only when it strictly holds `range` and the step before.
pub fn fallback_steps(doc: &Document, range: Range<usize>) -> Vec<Range<usize>> {
    let text = doc.text();
    let mut out: Vec<Range<usize>> = Vec::new();
    let mut push = |r: Range<usize>| {
        let last = out.last().cloned().unwrap_or(range.clone());
        if r.start <= last.start && r.end >= last.end && r != last {
            out.push(r);
        }
    };
    if text.is_empty() {
        return Vec::new();
    }
    let (first, last) = (doc.line_of(range.start), doc.line_of(range.end));
    if range.is_empty() {
        let word = |c: char| c.is_alphanumeric() || c == '_';
        let (before, after) = (text.get(..range.start).unwrap_or_default(), text.get(range.start..).unwrap_or_default());
        let start = before.char_indices().rev().take_while(|(_, c)| word(*c)).last().map_or(range.start, |(i, _)| i);
        let end = after.char_indices().take_while(|(_, c)| word(*c)).last().map_or(range.start, |(i, c)| range.start + i + c.len_utf8());
        if start < end {
            push(start..end);
        }
    }
    push(doc.line_range(first).start..doc.line_range(last).end);
    let blank = |l: usize| doc.line(l).trim().is_empty();
    let (mut a, mut b) = (first, last);
    while a > 0 && !blank(a - 1) && !blank(a) {
        a -= 1;
    }
    while b + 1 < doc.line_count() && !blank(b + 1) && !blank(b) {
        b += 1;
    }
    push(doc.line_range(a).start..doc.line_range(b).end);
    push(0..text.len());
    out
}

/// Ordinary plain-text expansion stays immediate; longer scans use admitted background analysis.
/// At most 4 KiB of source is read, plus a UTF-8 boundary adjustment for the word window.
pub(crate) fn snapshot_fallback_steps(doc: &crate::text::TextSnapshot, range: Range<usize>) -> Option<Vec<Range<usize>>> {
    let mut budget = 4096usize;
    let mut out: Vec<Range<usize>> = Vec::new();
    let mut push = |r: Range<usize>| {
        let last = out.last().unwrap_or(&range);
        if r.start <= last.start && r.end >= last.end && &r != last { out.push(r); }
    };
    if doc.len_bytes() == 0 { return Some(out); }
    let (first, last) = (doc.line_of(range.start), doc.line_of(range.end));
    if range.is_empty() {
        let line = doc.reader_line_range(first);
        let from = doc.floor_char_boundary(range.start.saturating_sub(budget / 2).max(line.start));
        let to = doc.floor_char_boundary(range.start.saturating_add(budget / 2).min(line.end));
        let text = doc.range(from..to)?;
        budget = budget.saturating_sub(text.len());
        let at = range.start - from;
        let word = |c: char| c.is_alphanumeric() || c == '_';
        let start = text[..at].char_indices().rev().take_while(|(_, c)| word(*c)).last().map_or(at, |(i, _)| i);
        let end = text[at..].char_indices().take_while(|(_, c)| word(*c)).last().map_or(at, |(i, c)| at + i + c.len_utf8());
        if start == 0 && from > line.start || end == text.len() && to < line.end { return None; }
        if start < end { push(from + start..from + end); }
    }
    push(doc.reader_line_range(first).start..doc.reader_line_range(last).end);
    let mut blank = |line| {
        let range = doc.reader_line_range(line);
        budget = budget.checked_sub(range.len())?;
        Some(doc.range(range)?.trim().is_empty())
    };
    let (mut a, mut b) = (first, last);
    while a > 0 && !blank(a - 1)? && !blank(a)? { a -= 1; }
    while b + 1 < doc.line_count() && !blank(b + 1)? && !blank(b)? { b += 1; }
    push(doc.reader_line_range(a).start..doc.reader_line_range(b).end);
    push(0..doc.len_bytes());
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;

    fn steps(src: &str, at: Range<usize>) -> Vec<&str> {
        let d = Document::decode("a.txt".into(), 1, src.as_bytes());
        fallback_steps(&d, at).into_iter().map(|r| &src[r]).collect()
    }

    #[test]
    fn fallback_grows_word_line_paragraph_file() {
        let src = "alpha beta\ngamma\n\ndelta\n";
        assert_eq!(steps(src, 7..7), vec!["beta", "alpha beta", "alpha beta\ngamma", src]);
        assert_eq!(steps(src, 10..10), vec!["beta", "alpha beta", "alpha beta\ngamma", src], "a caret just after a word takes it");
    }

    #[test]
    fn fallback_skips_steps_that_do_not_grow() {
        assert_eq!(steps("word\n", 1..1), vec!["word", "word\n"]);
        assert_eq!(steps("", 0..0), Vec::<&str>::new());
        assert_eq!(steps("a b\n", 0..4), Vec::<&str>::new());
        assert_eq!(steps("x\n\n  \n", 3..3), vec!["  ", "x\n\n  \n"], "a blank line is its own paragraph");
    }

    #[test]
    fn extend_and_shrink_walk_the_chain_back_to_the_origin() {
        let origin = Selection { anchor: 3, head: 3 };
        let mut e = Expansion { version: 1, origin, steps: vec![2..5, 0..9], at: 0 };
        assert_eq!(e.extend(), Some(Selection { anchor: 2, head: 5 }));
        assert_eq!(e.extend(), Some(Selection { anchor: 0, head: 9 }));
        assert_eq!(e.extend(), None);
        assert_eq!(e.shrink(), Some(Selection { anchor: 2, head: 5 }));
        assert_eq!(e.shrink(), Some(origin));
        assert_eq!(e.shrink(), None);
    }
}
