//! Incremental list search: candidate navigation is separate from opening a row.
use std::ops::Range;

/// Bound each scan so large, streamed commit lists keep accepting input.
pub const SCAN_ROWS: usize = 4096;

#[derive(Default)]
pub struct PaneSearch {
    active: bool,
    query: String,
    origin: usize,
    hits: Vec<usize>,
    scanned: usize,
    current: Option<usize>,
    moved: bool,
    confirmed: bool,
}
impl PaneSearch {
    pub fn active(&self) -> bool { self.active }
    pub fn query(&self) -> &str { &self.query }
    pub fn count(&self) -> usize { self.hits.len() }
    pub fn current(&self) -> Option<usize> { self.current }
    pub fn confirmed(&self) -> bool { self.confirmed }
    pub fn origin(&self) -> usize { self.origin }
    pub fn start(&mut self, anchor: usize) {
        if !self.active { *self = Self { active: true, origin: anchor, ..Self::default() }; }
    }
    pub fn set_query(&mut self, query: String) {
        if self.query == query { return; }
        self.query = query;
        self.invalidate();
    }
    pub fn append(&mut self, text: &str) {
        let mut query = self.query.clone(); query.push_str(text); self.set_query(query);
    }
    pub fn backspace(&mut self) {
        let mut query = self.query.clone(); query.pop(); self.set_query(query);
    }
    /// Rebuild candidate IDs when rows are replaced or reordered.
    pub fn invalidate(&mut self) {
        self.hits.clear(); self.scanned = 0; self.current = None; self.moved = false;
    }
    pub fn cancel(&mut self) { *self = Self::default(); }
    /// Confirming leaves the query visible; callers alone open the candidate.
    pub fn confirm(&mut self) -> Option<usize> {
        let row = self.current?; self.confirmed = true; self.origin = row; Some(row)
    }
    pub fn searching(&self, total: usize) -> bool {
        self.active && !self.query.is_empty() && self.scanned < total
    }
    /// Scan a bounded batch, incrementally including appended history rows.
    /// The predicate receives the current query without cloning it per row.
    pub fn poll(&mut self, total: usize, mut matches: impl FnMut(usize, &str) -> bool) -> bool {
        if !self.active || self.query.is_empty() { return false; }
        if total < self.scanned { self.invalidate(); }
        let end = total.min(self.scanned.saturating_add(SCAN_ROWS));
        if end == self.scanned { return false; }
        for row in self.scanned..end { if matches(row, &self.query) { self.hits.push(row); } }
        self.scanned = end;
        if !self.moved {
            self.current = self.hits.iter().copied().find(|&row| row >= self.origin)
                .or_else(|| self.hits.first().copied());
        }
        true
    }
    pub fn step(&mut self, forward: bool) -> Option<usize> {
        if self.hits.is_empty() { return None; }
        let at = self.current.and_then(|row| self.hits.binary_search(&row).ok());
        let next = match at {
            None => if forward { 0 } else { self.hits.len() - 1 },
            Some(i) => if forward { (i + 1) % self.hits.len() } else { (i + self.hits.len() - 1) % self.hits.len() },
        };
        self.moved = true; self.current = Some(self.hits[next]); self.current
    }
    pub fn ranges(&self, text: &str) -> Vec<Range<usize>> { ranges(text, &self.query) }
    pub fn status(&self) -> String {
        if self.query.is_empty() { return "Find: ▏ · type to search".into(); }
        let at = self.current.and_then(|row| self.hits.binary_search(&row).ok());
        match at {
            Some(at) => format!("Find: {}▏ · {}/{}", self.query, at + 1, self.count()),
            None => format!("Find: {}▏ · No matches", self.query),
        }
    }
}

pub fn contains(text: &str, query: &str) -> bool { crate::find::match_ranges(text, query).next().is_some() }
pub fn ranges(text: &str, query: &str) -> Vec<Range<usize>> { crate::find::match_ranges(text, query).collect() }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preview_wrap_cancel_and_confirm_are_separate() {
        let mut search = PaneSearch::default(); search.start(2); search.append("fix");
        let rows = ["fix earlier", "other", "Fix current", "fix later"];
        search.poll(rows.len(), |i,q| contains(rows[i],q));
        assert_eq!(search.current(), Some(2)); assert!(!search.confirmed());
        assert_eq!(search.step(true), Some(3)); assert_eq!(search.step(true), Some(0));
        assert_eq!(search.step(false), Some(3)); assert_eq!(search.confirm(), Some(3));
        assert!(search.confirmed()); assert_eq!(search.query(), "fix");
        search.cancel(); assert!(!search.active()); assert_eq!(search.current(), None);
    }
    #[test]
    fn scanning_is_bounded_and_includes_new_streamed_rows() {
        let mut search = PaneSearch::default(); search.start(SCAN_ROWS); search.append("needle");
        let mut calls = 0;
        search.poll(SCAN_ROWS+2, |i,_| { calls += 1; i == 1 || i == SCAN_ROWS+1 });
        assert_eq!(calls, SCAN_ROWS); assert!(search.searching(SCAN_ROWS+2));
        assert_eq!(search.current(),Some(1));
        search.poll(SCAN_ROWS+2, |i,_| i == SCAN_ROWS+1);
        assert_eq!(search.current(),Some(SCAN_ROWS+1)); assert!(!search.searching(SCAN_ROWS+2));
        search.step(true);
        search.poll(SCAN_ROWS+3, |_,_| true);
        assert_eq!(search.count(),3); assert_eq!(search.current(),Some(1));
        search.set_query("new".into()); assert_eq!(search.current(),None); assert_eq!(search.count(),0);
        search.poll(1, |_,_| true); assert_eq!(search.current(),Some(0));
    }
    #[test]
    fn highlight_ranges_are_literal_and_utf8_boundaries() {
        assert_eq!(ranges("café CAFÉ café", "CAFÉ"),vec![0..5,6..11,12..17]);
        assert!(contains("a.b", ".")); assert!(!contains("anything", ""));
        let mut search = PaneSearch::default();search.start(0);search.append("é");
        search.backspace();assert_eq!(search.query(),"");
    }
    #[test]
    fn history_candidates_preserve_committed_range_and_use_display_rows() {
        use crate::{history::Commit, repository_session::Picker};
        let mut picker = Picker::new(std::path::PathBuf::new(), true);
        picker.log.extend(vec![
            Commit { id: "1111111111111111111111111111111111111111".into(), parents: vec![], author: "Élodie".into(), email: "elodie@example.test".into(), time: 0, offset: 0, subject: "Fix navigation".into() },
            Commit { id: "2222222222222222222222222222222222222222".into(), parents: vec![], author: "Sam".into(), email: "sam@example.test".into(), time: 1, offset: 0, subject: "Other commit".into() },
        ]);
        picker.anchor = 2; picker.second = Some(1); picker.range = true;
        picker.pane_search.start(2); picker.pane_search.append("fix");
        picker.poll_pane_search(); assert_eq!(picker.pane_search.current(), Some(1));
        assert_eq!((picker.anchor,picker.second,picker.range),(2,Some(1),true));
        picker.pane_search.set_query("ÉLODIE".into());picker.poll_pane_search();
        assert_eq!(picker.pane_search.current(), Some(1));
        picker.matches = Some(vec![1]); picker.pane_search.invalidate();
        picker.pane_search.set_query("22222".into());picker.poll_pane_search();
        assert_eq!(picker.pane_search.current(), Some(1));
        picker.pane_search.set_query("Fix".into());picker.poll_pane_search();
        assert_eq!(picker.pane_search.current(),None);
        picker.pane_search.cancel();assert_eq!(picker.anchor,2);
    }
}
