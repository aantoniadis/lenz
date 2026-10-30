//! In-document find: a tab's query, its matches in one document version, and the current match.

use std::{ops::Range, sync::Arc};

use lenz_content::find::{Matcher, Query};

/// A search's matches by (line, byte range in the line), and whether more were left out; or why it cannot run.
pub type Found = Result<(Vec<(usize, Range<usize>)>, bool), String>;

/// More matches are not listed; the counter says "10,000+".
pub const MATCH_LIMIT: usize = lenz_content::find::MATCH_LIMIT;

#[derive(Clone, Debug)]
pub struct FindState {
    pub(crate) query: String,
    /// (line, byte range in the line), in document order.
    pub(crate) matches: Arc<[(usize, Range<usize>)]>,
    pub(crate) current: Option<usize>,
    pub(crate) capped: bool,
    pub(crate) error: Option<String>,
    /// The document version and request the matches are for; a result for another is dropped.
    pub(crate) version: u64,
    pub(crate) request: u64,
    /// A search is running for `query`.
    pub(crate) pending: bool,
    /// Its result goes to the match at or after the caret; else (a rerun for a change on disk) the view stays.
    pub(crate) reveal: bool,
}

impl FindState {
    pub(crate) fn new(query: String, version: u64, request: u64) -> Self {
        Self { query, matches: Vec::new().into(), current: None, capped: false, error: None, version, request, pending: true, reveal: true }
    }
    pub fn query(&self) -> &str {
        &self.query
    }
    pub fn matches(&self) -> &[(usize, Range<usize>)] {
        &self.matches
    }
    pub fn current(&self) -> Option<usize> {
        self.current
    }
    pub fn capped(&self) -> bool {
        self.capped
    }
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
    pub fn pending(&self) -> bool {
        self.pending
    }
    /// The matches on `line`.
    pub fn in_line(&self, line: usize) -> &[(usize, Range<usize>)] {
        let from = self.matches.partition_point(|(l, _)| *l < line);
        let to = self.matches.partition_point(|(l, _)| *l <= line);
        self.matches.get(from..to).unwrap_or_default()
    }
    /// "3/41", "3/10,000+", "No matches", or why the query cannot be matched.
    pub fn counter(&self) -> String {
        lenz_content::find::counter(&self.query, self.matches.len(), self.current, self.capped, self.pending, self.error())
    }
    /// The first match at or after (line, byte), wrapping to the first.
    pub(crate) fn at_or_after(&self, line: usize, byte: usize) -> Option<usize> {
        lenz_content::find::at_or_after(&self.matches, line, byte)
    }
}

/// Every match of `query` in `doc`, at most [`MATCH_LIMIT`] (`true`: more were left out). `None` once `current()`
/// turns false; `Some(Err)` when the query cannot be matched.
pub fn find_all(doc: &crate::document::Document, query: &Query, current: impl Fn() -> bool) -> Option<Found> {
    let m = match Matcher::new(query) {
        Ok(m) => m,
        Err(e) => return Some(Err(e.to_string())),
    };
    lenz_content::find::collect_matches(&m, (0..doc.line_count()).map(|line| (line, doc.line(line))), current).map(Ok)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lenz_content::find::{FindOptions, Query};

    fn doc(text: &str) -> crate::document::Document {
        crate::document::Document::decode("/t.txt".into(), 1, text.as_bytes())
    }

    #[test]
    fn find_all_lists_matches_by_line_and_caps_them() {
        let d = doc("a foo\nfoo foo\nbar\n");
        let q = Query { text: "foo".into(), options: FindOptions::default() };
        let (m, capped) = find_all(&d, &q, || true).unwrap().unwrap();
        assert_eq!(m, vec![(0, 2..5), (1, 0..3), (1, 4..7)]);
        assert!(!capped);
        let many = doc(&"x ".repeat(MATCH_LIMIT + 5));
        let (m, capped) = find_all(&many, &Query { text: "x".into(), ..Default::default() }, || true).unwrap().unwrap();
        assert_eq!((m.len(), capped), (MATCH_LIMIT, true));
    }

    #[test]
    fn find_of_empty_regex_has_no_matches() {
        let d = doc("abc\n");
        let q = Query { text: "x*".into(), options: FindOptions { regex: true, ..Default::default() } };
        assert_eq!(find_all(&d, &q, || true).unwrap().unwrap().0, vec![]);
    }

    #[test]
    fn a_bad_expression_is_an_error_and_an_obsolete_search_stops() {
        let d = doc("abc\n");
        let q = Query { text: "(".into(), options: FindOptions { regex: true, ..Default::default() } };
        assert_eq!(find_all(&d, &q, || true).unwrap(), Err("unclosed group".into()));
        assert_eq!(find_all(&d, &Query { text: "a".into(), ..Default::default() }, || false), None);
    }

    #[test]
    fn the_counter_reads_like_the_find_box() {
        let mut s = FindState::new("foo".into(), 1, 1);
        assert_eq!(s.counter(), "…", "still searching");
        s.pending = false;
        s.matches = vec![(0, 0..3), (2, 1..4)].into();
        s.current = Some(1);
        assert_eq!(s.counter(), "2/2");
        s.capped = true;
        assert_eq!(s.counter(), "2/10,000+");
        s.matches = Vec::new().into();
        s.current = None;
        s.capped = false;
        assert_eq!(s.counter(), "No matches");
        s.error = Some("unclosed group".into());
        assert_eq!(s.counter(), "unclosed group");
        assert_eq!(s.in_line(2), &[]);
    }
}
