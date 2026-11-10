//! Literal acceleration prepared once per bounded search, not per line or UI frame.
use super::{Range, whole_word};

pub(super) struct Scanner<'a> {
    needle: &'a [u8],
    case: bool,
    finder: Option<memchr::memmem::Finder<'a>>,
    anchor: usize,
    lower: u8,
    upper: u8,
}
impl<'a> Scanner<'a> {
    pub(super) fn new(needle: &'a [u8], case: bool) -> Self {
        let first = needle.first().copied().unwrap_or_default();
        let anchor = needle.iter().rposition(|b| !b.eq_ignore_ascii_case(&first)).unwrap_or(0);
        let byte = needle.get(anchor).copied().unwrap_or_default();
        Self { needle, case, finder: case.then(|| memchr::memmem::Finder::new(needle)), anchor, lower: byte.to_ascii_lowercase(), upper: byte.to_ascii_uppercase() }
    }
    /// Very frequent hits benefit from the compact original iterator, without SIMD setup per hit.
    pub(super) fn dense_prefix(&self, line: &str) -> bool {
        if line.len() < 256 || !line.is_ascii() { return false; }
        let Some(mut prefix) = line.as_bytes().get(..128) else { return false };
        for _ in 0..4 {
            let Some(at) = prefix.windows(self.needle.len()).position(|part| if self.case { part == self.needle } else { part.eq_ignore_ascii_case(self.needle) }) else { return false };
            prefix = &prefix[at + self.needle.len()..];
        }
        true
    }
    pub(super) fn ranges<'s>(&'s self, line: &'s str, word: bool) -> Ranges<'s, 'a> {
        Ranges { scanner: self, line, word, from: 0 }
    }
    fn find(&self, text: &[u8]) -> Option<Range<usize>> {
        if let Some(finder) = &self.finder { return finder.find(text).map(|at| at..at + self.needle.len()); }
        let mut from = self.anchor;
        while let Some(rest) = text.get(from..) {
            let at = if self.lower == self.upper { memchr::memchr(self.lower, rest) } else { memchr::memchr2(self.lower, self.upper, rest) }?;
            let start = from + at - self.anchor;
            let end = start.checked_add(self.needle.len())?;
            if text.get(start..end)?.eq_ignore_ascii_case(self.needle) { return Some(start..end); }
            from += at + 1;
        }
        None
    }
}

pub(super) struct Ranges<'s, 'a> { scanner: &'s Scanner<'a>, line: &'s str, word: bool, from: usize }
impl Iterator for Ranges<'_, '_> {
    type Item = Range<usize>;
    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let range = self.scanner.find(self.line.as_bytes().get(self.from..)?)?;
            let start = self.from + range.start; self.from += range.end;
            let range = start..self.from;
            if !self.word || whole_word(self.line, &range) { return Some(range); }
        }
    }
}
