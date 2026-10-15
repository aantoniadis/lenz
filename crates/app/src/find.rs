//! In-document find (gpui-free) over the shared matcher: one Side, a merge pane, or the Change Set.

use std::ops::Range;

pub use lenz_content::find::{FindOption, FindOptions};
use lenz_content::find::{Matcher, Query};
use lenz_engine::{Options, compare_lines, split_lines};

use crate::{changeset::ChangeSet, view::Side};

/// Change Set search stops here and says so.
pub const MAX_HITS: usize = 10_000;
/// Files bigger than this (per Side) are skipped.
const MAX_BYTES: usize = 20 << 20;

/// Matches in order: (line, byte range).
pub type Hits = Vec<(u32, Range<usize>)>;

/// Find state and wrap decisions shared by the two adapters.
pub struct Find {
    pub(crate) query: String,
    pub(crate) side: Side,
    pub(crate) hits: Vec<(u32, Range<usize>)>,
    pub(crate) at: Option<usize>,
    /// Why the query cannot be matched (a bad regular expression); no hits while set.
    pub(crate) error: Option<String>,
}

impl Find {
    pub fn new(query: String, options: FindOptions, side: Side, lines: &[String], from: crate::selection::Pos) -> Self {
        let query = Query { text: query, options };
        let (hits, at, error) = match Self::search(lines, &query, from) { Ok((hits, at)) => (hits, at, None), Err(e) => (Vec::new(), None, Some(e)) };
        Self { query: query.text, side, hits, at, error }
    }
    pub fn search(lines: &[String], query: &Query, from: crate::selection::Pos) -> Result<(Hits, Option<usize>), String> {
        let matcher = Matcher::new(query).map_err(|e| e.to_string())?;
        let hits = in_lines(lines, &matcher);
        let at = hits.iter().position(|(l, r)| crate::selection::Pos::new(*l, r.start) >= from).or((!hits.is_empty()).then_some(0));
        Ok((hits, at))
    }
    pub fn next_index(&self, forward: bool) -> Option<(usize, bool)> {
        let n = self.hits.len();
        if n == 0 { return None; }
        let i = match self.at { Some(i) if forward => (i + 1) % n, Some(i) => (i + n - 1) % n, None => 0 };
        Some((i, self.at.is_some() && (forward && i == 0 || !forward && i == n - 1)))
    }
    pub fn query(&self) -> &str { &self.query }
    pub fn side(&self) -> Side { self.side }
    pub fn hits(&self) -> &[(u32, Range<usize>)] { &self.hits }
    pub fn at(&self) -> Option<usize> { self.at }
    pub fn error(&self) -> Option<&str> { self.error.as_deref() }
}

/// Every match in `lines`, in order: (line, byte range).
pub fn in_lines(lines: &[String], matcher: &Matcher) -> Vec<(u32, Range<usize>)> {
    lines.iter().enumerate().flat_map(|(i, l)| matcher.ranges(l).map(move |r| (i as u32, r))).collect()
}

#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    /// Comparison index.
    pub index: usize,
    pub side: Side,
    pub line: u32,
    pub range: Range<usize>,
    /// The line, for the results panel.
    pub preview: String,
}

/// Why a Change Set search ended.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Ended {
    Done,
    /// Stopped at `MAX_HITS`.
    Capped,
}

/// Search both Sides of every Comparison, reporting each file's hits as soon as it's done
/// (`on_file` returns false to stop early). Binary and oversized files are skipped;
/// `changed_only` keeps hits on lines inside Change Blocks.
pub fn in_change_set(set: &ChangeSet, matcher: &Matcher, changed_only: bool, on_file: impl FnMut(Vec<Hit>) -> bool) -> Ended {
    in_change_set_cancellable(set, matcher, changed_only, || false, on_file)
}

pub fn in_change_set_cancellable(set: &ChangeSet, matcher: &Matcher, changed_only: bool, mut cancelled: impl FnMut() -> bool, mut on_file: impl FnMut(Vec<Hit>) -> bool) -> Ended {
    let mut total = 0;
    for index in 0..set.comparisons.len() {
        if cancelled() { return Ended::Done; }
        if set.sizes(index).is_ok_and(|sizes| sizes.iter().any(|n| *n > MAX_BYTES as u64)) { continue; }
        let Ok((left, right)) = set.texts(index) else { continue };
        if cancelled() { return Ended::Done; }
        let skip = |t: &str| t.len() > MAX_BYTES || t.as_bytes()[..t.len().min(8000)].contains(&0);
        if skip(&left) || skip(&right) {
            continue;
        }
        let (l, r) = (split_lines(&left), split_lines(&right));
        let changed: Option<[Vec<bool>; 2]> = changed_only.then(|| {
            let mut marks = [vec![false; l.len()], vec![false; r.len()]];
            for b in compare_lines(&l, &r, &Options::default()) {
                b.left.for_each(|i| marks[0][i as usize] = true);
                b.right.for_each(|i| marks[1][i as usize] = true);
            }
            marks
        });
        let mut hits = Vec::new();
        for (k, (side, lines)) in [(Side::Left, &l), (Side::Right, &r)].into_iter().enumerate() {
            for (line, text) in lines.iter().enumerate() {
                if cancelled() { return Ended::Done; }
                if changed.as_ref().is_some_and(|m| !m[k][line]) {
                    continue;
                }
                for range in matcher.ranges(text) {
                    if cancelled() { return Ended::Done; }
                    let preview = text.chars().take(300).collect();
                    hits.push(Hit { index, side, line: line as u32, range, preview });
                    if total + hits.len() == MAX_HITS {
                        return if on_file(hits) { Ended::Capped } else { Ended::Done };
                    }
                }
            }
        }
        if hits.is_empty() {
            continue;
        }
        total += hits.len();
        if !on_file(hits) {
            return Ended::Done;
        }
    }
    Ended::Done
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        changeset::{ChangeSet, Spec},
        git::testing::{commit_all, repo, write},
        selection::Pos,
    };
    use lenz_content::find::{FindOptions, Matcher, Query};

    fn m(text: &str, options: FindOptions) -> Matcher {
        Matcher::new(&Query { text: text.into(), options }).unwrap()
    }

    /// Plain find: literal, case ignored.
    fn matches(line: &str, query: &str) -> Vec<Range<usize>> {
        m(query, FindOptions::default()).ranges(line).collect()
    }

    #[test]
    fn find_uses_the_options_and_reports_a_bad_expression() {
        let lines: Vec<String> = ["Foo foo", "foobar foo"].map(String::from).into();
        let f = Find::new("foo".into(), FindOptions { case: true, word: true, regex: false }, Side::Left, &lines, Pos::new(0, 0));
        assert_eq!(f.hits(), [(0, 4..7), (1, 7..10)]);
        let bad = Find::new("(".into(), FindOptions { regex: true, ..Default::default() }, Side::Left, &lines, Pos::new(0, 0));
        assert!(bad.hits().is_empty() && bad.at().is_none());
        assert_eq!(bad.error(), Some("unclosed group"));
        assert_eq!(bad.next_index(true), None, "F3 does nothing while the expression is bad");
    }

    #[test]
    fn change_set_search_takes_the_options() {
        let dir = tempfile::tempdir().unwrap();
        let (left, right) = (dir.path().join("left.txt"), dir.path().join("right.txt"));
        std::fs::write(&left, "Needle needle").unwrap();
        std::fs::write(&right, "needles").unwrap();
        let set = ChangeSet::load(&Spec::Plain { left, right }, dir.path()).unwrap();
        let mut hits = Vec::new();
        in_change_set(&set, &m("needle", FindOptions { case: true, word: true, regex: false }), false, |h| { hits.extend(h); true });
        assert_eq!(hits.iter().map(|h| (h.side, h.range.clone())).collect::<Vec<_>>(), [(Side::Left, 7..13)]);
    }

    #[test]
    fn matching_ignores_case_and_reports_byte_ranges() {
        assert_eq!(matches("Foo foo FOO", "foo"), vec![0..3, 4..7, 8..11]);
        assert_eq!(matches("aaaa", "aa"), vec![0..2, 2..4]);
        assert!(matches("Größe größe", "GRÖSSE").is_empty(), "no case folding beyond lowercase");
        assert_eq!(matches("Ärger ärger", "ÄRGER"), vec![0..6, 7..13]);
        assert!(matches("abc", "").is_empty());
        assert_eq!(matches("a.b", "."), vec![1..2], "no regex");
        assert_eq!(matches("Kk K", "k"), vec![0..3, 3..4, 5..6], "an ASCII query can match a Unicode lowercase equivalent");
        assert_eq!(matches("kK", "K"), vec![0..1, 1..2]);
        assert_eq!(matches("İi", "i"), vec![2..3], "lowercase expansions must retain the existing per-character semantics");
        assert_eq!(matches("éNEEDLEéneedle", "needle"), vec![2..8, 10..16]);
        assert!(matches("short", "a longer query").is_empty());
    }

    #[test]
    fn finds_every_match_by_line() {
        let lines: Vec<String> = ["let x = 1;", "", "x + X"].map(String::from).to_vec();
        assert_eq!(in_lines(&lines, &m("x", FindOptions::default())), vec![(0, 4..5), (2, 0..1), (2, 4..5)]);
    }

    #[test]
    fn capped_search_keeps_the_first_matches_even_with_many_in_one_line() {
        let dir = tempfile::tempdir().unwrap();
        let (left, right) = (dir.path().join("left.txt"), dir.path().join("right.txt"));
        std::fs::write(&left, "needle ".repeat(MAX_HITS + 100)).unwrap();
        std::fs::write(&right, "needle").unwrap();
        let set = ChangeSet::load(&Spec::Plain { left, right }, dir.path()).unwrap();
        let mut hits = Vec::new();
        let ended = in_change_set(&set, &m("needle", FindOptions::default()), false, |h| { hits.extend(h); true });
        assert_eq!(ended, Ended::Capped);
        assert_eq!(hits.len(), MAX_HITS);
        assert!(hits.iter().all(|h| h.side == Side::Left && h.line == 0));
        assert_eq!(hits[0].range, 0..6);
        assert_eq!(hits.last().unwrap().range, 69_993..69_999);
        assert!(hits.iter().all(|h| h.preview.len() == 300));
    }

    /// Interactive find and the capped background search, including a very frequent query.
    #[test]
    #[ignore = "timing: run in release with --ignored --nocapture"]
    fn search_perf() {
        use std::{hint::black_box, time::Instant};
        let lines: Vec<String> = (0..100_000).map(|i| format!("let value_{i} = compute({i}, 42); // needle")).collect();
        for (query, expected) in [("needle", 100_000), ("absent", 0), ("e", 600_000)] {
            let matcher = m(query, FindOptions::default());
            let mut times = Vec::new();
            for _ in 0..7 {
                let started = Instant::now();
                let hits = black_box(in_lines(&lines, &matcher));
                times.push(started.elapsed().as_secs_f64() * 1000.0);
                assert_eq!(hits.len(), expected);
            }
            crate::perf::record(&format!("search/find/{query}"), "ms", &times);
            times.sort_by(f64::total_cmp);
            eprintln!("find 100k lines {query:?}: median {:.3} ms, {} matches", times[3], expected);
            crate::perf::check(times[3] < 25.0, "find query exceeds 25 ms");
        }
        let regex = m(r"value_\d+ = compute", FindOptions { regex: true, ..Default::default() });
        let mut times = Vec::new();
        for _ in 0..7 {
            let started = Instant::now();
            assert_eq!(black_box(in_lines(&lines, &regex)).len(), 100_000);
            times.push(started.elapsed().as_secs_f64() * 1000.0);
        }
        crate::perf::record("search/find-regex", "ms", &times);
        times.sort_by(f64::total_cmp);
        eprintln!("regex find 100k lines: median {:.3} ms", times[3]);
        crate::perf::check(times[3] < 25.0, "regex find exceeds 25 ms");
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("many.txt");
        std::fs::write(&path, "needle needle needle needle needle needle needle needle\n".repeat(100_000)).unwrap();
        let other = dir.path().join("other.txt");
        std::fs::write(&other, "different").unwrap();
        let set = ChangeSet::load(&Spec::Plain { left: path, right: other }, dir.path()).unwrap();
        let started = Instant::now();
        let mut count = 0;
        assert_eq!(in_change_set(&set, &m("needle", FindOptions::default()), false, |hits| { count += hits.len(); true }), Ended::Capped);
        assert_eq!(count, MAX_HITS);
        let ms = started.elapsed().as_secs_f64() * 1000.0;
        crate::perf::record("search/capped", "ms", &[ms]);
        eprintln!("capped search 100k lines: {ms:.3} ms");
        crate::perf::check(ms < 50.0, "capped search exceeds 50 ms");
    }

    fn set_with_two_files() -> (tempfile::TempDir, ChangeSet) {
        let r = repo();
        write(r.path(), "a.txt", "needle one\nsame needle\nend\n");
        write(r.path(), "b.txt", "nothing\n");
        commit_all(r.path(), "first");
        write(r.path(), "a.txt", "needle uno\nsame needle\nend\n");
        write(r.path(), "b.txt", "nothing\nNEEDLE\n");
        let spec = Spec::Git { cached: false, revs: vec![], paths: vec![], untracked: false };
        let set = ChangeSet::load(&spec, r.path()).unwrap();
        (r, set)
    }

    #[test]
    fn change_set_search_covers_both_sides_of_every_file() {
        let (_r, set) = set_with_two_files();
        let mut hits = Vec::new();
        assert_eq!(in_change_set(&set, &m("needle", FindOptions::default()), false, |h| { hits.extend(h); true }), Ended::Done);
        let found: Vec<_> = hits.iter().map(|h| (set.comparisons[h.index].path.as_str(), h.side, h.line)).collect();
        assert_eq!(
            found,
            vec![
                ("a.txt", Side::Left, 0),
                ("a.txt", Side::Left, 1),
                ("a.txt", Side::Right, 0),
                ("a.txt", Side::Right, 1),
                ("b.txt", Side::Right, 1),
            ]
        );
        assert_eq!(hits[4].preview, "NEEDLE");
    }

    #[test]
    fn changed_lines_only_drops_hits_on_unchanged_lines() {
        let (_r, set) = set_with_two_files();
        let mut hits = Vec::new();
        in_change_set(&set, &m("needle", FindOptions::default()), true, |h| {
            hits.extend(h);
            true
        });
        let lines: Vec<_> = hits.iter().map(|h| (h.index, h.side, h.line)).collect();
        assert_eq!(lines, vec![(0, Side::Left, 0), (0, Side::Right, 0), (1, Side::Right, 1)]);
    }

    #[test]
    fn binary_files_are_skipped() {
        let r = repo();
        write(r.path(), "bin.dat", "needle\0\0");
        commit_all(r.path(), "first");
        std::fs::write(r.path().join("bin.dat"), b"needle\0\x01").unwrap();
        let spec = Spec::Git { cached: false, revs: vec![], paths: vec![], untracked: false };
        let set = ChangeSet::load(&spec, r.path()).unwrap();
        let mut hits = 0;
        in_change_set(&set, &m("needle", FindOptions::default()), false, |h| {
            hits += h.len();
            true
        });
        assert_eq!(hits, 0);
    }
}
