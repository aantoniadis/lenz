//! Shared Change Set search state, background scheduling, cancellation and result navigation.
use crate::{
    changeset::ChangeSet,
    find::{self, Ended, Hit},
};
use lenz_content::find::{FindOption, FindOptions, Matcher, Query};
use std::sync::{
    Arc, Mutex, OnceLock,
    atomic::{AtomicBool, Ordering},
    mpsc,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelRow {
    File(usize, usize),
    Hit(usize),
}
#[derive(Debug, Clone, PartialEq)]
pub enum SearchCommand {
    Query(String),
    ToggleChangedOnly,
    Start,
    Edit,
    Select(usize),
    Step(bool),
    Cancel,
}
impl SearchCommand {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Query(_) => "Find in Change Set",
            Self::ToggleChangedOnly => "Changed lines only",
            Self::Start => "Search Change Set",
            Self::Edit => "Edit search query",
            Self::Select(_) => "Open search result",
            Self::Step(true) => "Next search result",
            Self::Step(false) => "Previous search result",
            Self::Cancel => "Close Change Set search",
        }
    }
}
#[derive(Default)]
struct Inbox {
    hits: Vec<Hit>,
    ended: Option<Ended>,
}

pub struct ChangeSetSearch {
    pub(crate) query: String,
    pub(crate) changed_only: bool,
    pub(crate) options: FindOptions,
    /// Why the query cannot be matched (a bad regular expression): Start is refused.
    error: Option<String>,
    pub(crate) editing: bool,
    pub(crate) hits: Vec<Hit>,
    pub(crate) ended: Option<Ended>,
    pub(crate) at: Option<usize>,
    /// Adapter units: cells for TUI, rows for GUI.
    pub(crate) scroll: f32,
    pub(crate) cancel: Arc<AtomicBool>,
    inbox: Option<Arc<Mutex<Inbox>>>,
    rows: Vec<PanelRow>,
    heading: Option<usize>,
    notice: Option<&'static str>,
}
impl ChangeSetSearch {
    pub fn new(query: String, changed_only: bool, options: FindOptions) -> Self {
        let error = query_error(&query, options);
        Self {
            query,
            changed_only,
            options,
            error,
            editing: true,
            hits: Vec::new(),
            ended: None,
            at: None,
            scroll: 0.0,
            cancel: Arc::new(AtomicBool::new(false)),
            inbox: None,
            rows: Vec::new(),
            heading: None,
            notice: None,
        }
    }
    pub fn options(&self) -> FindOptions {
        self.options
    }
    /// Why the query cannot be matched, if it cannot.
    pub fn query_error(&self) -> Option<&str> {
        self.error.as_deref()
    }
    /// Flip one option; results of the old query are dropped and the query is edited again.
    pub fn toggle_option(&mut self, option: FindOption) {
        self.cancel();
        self.options = self.options.toggled(option);
        self.error = query_error(&self.query, self.options);
        self.clear_results();
        self.editing = true;
    }
    pub fn query(&self) -> &str {
        &self.query
    }
    pub fn changed_only(&self) -> bool {
        self.changed_only
    }
    pub fn editing(&self) -> bool {
        self.editing
    }
    pub fn hits(&self) -> &[Hit] {
        &self.hits
    }
    /// Search batches retain `(comparison index, side, line, byte offset)` order.
    /// Borrow only one side's hits; no per-paint filtering or result copies.
    pub fn hits_for_side(&self, index: usize, side: crate::view::Side) -> &[Hit] {
        let start = self
            .hits
            .partition_point(|hit| (hit.index, hit.side) < (index, side));
        let end = start
            + self.hits[start..].partition_point(|hit| (hit.index, hit.side) == (index, side));
        &self.hits[start..end]
    }
    pub fn hits_on_line(&self, index: usize, side: crate::view::Side, line: u32) -> &[Hit] {
        let start = self
            .hits
            .partition_point(|hit| (hit.index, hit.side, hit.line) < (index, side, line));
        let end = start
            + self.hits[start..]
                .partition_point(|hit| (hit.index, hit.side, hit.line) == (index, side, line));
        &self.hits[start..end]
    }
    pub fn ended(&self) -> Option<Ended> {
        self.ended
    }
    pub fn at(&self) -> Option<usize> {
        self.at
    }
    pub fn scroll(&self) -> f32 {
        self.scroll
    }
    pub fn notice(&self) -> Option<&'static str> {
        self.notice
    }
    pub fn rows(&self) -> &[PanelRow] {
        &self.rows
    }
    pub fn busy(&self) -> bool {
        self.inbox.is_some()
    }
    pub fn cancel(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        self.inbox = None;
    }
    pub fn disabled(&self, command: &SearchCommand) -> Option<&'static str> {
        match command {
            SearchCommand::Start if self.query.is_empty() => Some("Enter search text"),
            SearchCommand::Start if self.error.is_some() => Some("Fix the regular expression"),
            SearchCommand::Select(index) if *index >= self.hits.len() => {
                Some("No result at that position")
            }
            SearchCommand::Step(_) if self.hits.is_empty() => Some("No search results"),
            _ => None,
        }
    }
    pub fn execute(
        &mut self,
        command: SearchCommand,
        set: &Arc<ChangeSet>,
        visible: f32,
    ) -> Option<Hit> {
        if let Some(reason) = self.disabled(&command) {
            self.notice = Some(reason);
            return None;
        }
        match command {
            SearchCommand::Query(query) => {
                self.cancel();
                self.query = query;
                self.error = query_error(&self.query, self.options);
                self.clear_results();
                self.editing = true;
                self.notice = None;
            }
            SearchCommand::ToggleChangedOnly => self.changed_only = !self.changed_only,
            SearchCommand::Start => self.start(set.clone()),
            SearchCommand::Edit => {
                self.cancel();
                self.clear_results();
                self.editing = true;
            }
            SearchCommand::Select(index) => return self.select(index, visible),
            SearchCommand::Step(forward) => {
                let (index, boundary) =
                    self.next_index(forward).expect("enabled result navigation");
                let notice = boundary.then_some(Self::boundary_message(forward));
                let hit = self.select(index, visible);
                self.notice = notice;
                return hit;
            }
            SearchCommand::Cancel => self.cancel(),
        }
        None
    }
    pub fn next_index(&self, forward: bool) -> Option<(usize, bool)> {
        if self.hits.is_empty() {
            return None;
        }
        let index = match self.at {
            Some(index) if forward => (index + 1).min(self.hits.len() - 1),
            Some(index) => index.saturating_sub(1),
            None => 0,
        };
        Some((index, self.at == Some(index)))
    }
    pub fn boundary_message(forward: bool) -> &'static str {
        if forward {
            "Last match in the Change Set"
        } else {
            "First match in the Change Set"
        }
    }
    fn clear_results(&mut self) {
        self.hits.clear();
        self.rows.clear();
        self.heading = None;
        self.at = None;
        self.ended = None;
        self.scroll = 0.0;
    }
    pub fn select(&mut self, index: usize, visible: f32) -> Option<Hit> {
        let hit = self.hits.get(index)?.clone();
        self.at = Some(index);
        self.notice = None;
        let row = self
            .rows
            .iter()
            .position(|row| matches!(row, PanelRow::Hit(hit) if *hit == index))
            .unwrap_or(0) as f32;
        let visible = visible.max(1.0);
        if row < self.scroll || row >= self.scroll + visible {
            self.scroll = (row - visible / 2.0).max(0.0);
        }
        Some(hit)
    }
    pub fn scroll_by(&mut self, amount: f32, visible: f32) {
        self.scroll =
            (self.scroll + amount).clamp(0.0, (self.rows.len() as f32 - visible.max(1.0)).max(0.0));
    }
    pub fn start(&mut self, set: Arc<ChangeSet>) {
        self.cancel();
        self.cancel = Arc::new(AtomicBool::new(false));
        self.hits.clear();
        self.rows.clear();
        self.heading = None;
        self.at = None;
        self.scroll = 0.0;
        self.ended = None;
        self.editing = false;
        self.notice = None;
        if self.query.is_empty() {
            self.ended = Some(Ended::Done);
            return;
        }
        // Start is refused while the query is bad; a bad one here searches nothing.
        let Ok(matcher) = Matcher::new(&Query { text: self.query.clone(), options: self.options }) else {
            self.ended = Some(Ended::Done);
            return;
        };
        let inbox: Arc<Mutex<Inbox>> = Default::default();
        self.inbox = Some(inbox.clone());
        let (changed_only, cancel) = (self.changed_only, self.cancel.clone());
        type Job = Box<dyn FnOnce() + Send>;
        static WORKER: OnceLock<mpsc::Sender<Job>> = OnceLock::new();
        let worker = WORKER.get_or_init(|| {
            let (tx, rx) = mpsc::channel::<Job>();
            std::thread::Builder::new()
                .name("lenz-search".into())
                .spawn(move || {
                    for job in rx {
                        job();
                    }
                })
                .expect("start search worker");
            tx
        });
        worker
            .send(Box::new(move || {
                if cancel.load(Ordering::Relaxed) {
                    return;
                }
                let ended = find::in_change_set_cancellable(
                    &set,
                    &matcher,
                    changed_only,
                    || cancel.load(Ordering::Relaxed),
                    |hits| {
                        if cancel.load(Ordering::Relaxed) {
                            return false;
                        }
                        inbox
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .hits
                            .extend(hits);
                        true
                    },
                );
                if !cancel.load(Ordering::Relaxed) {
                    inbox
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .ended = Some(ended);
                }
            }))
            .expect("search worker running");
    }
    pub fn poll(&mut self) -> bool {
        let Some(inbox) = &self.inbox else {
            return false;
        };
        let (hits, ended) = {
            let mut inbox = inbox
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            (std::mem::take(&mut inbox.hits), inbox.ended)
        };
        let changed = !hits.is_empty() || ended.is_some();
        for hit in hits {
            match self.heading.and_then(|heading| self.rows.get_mut(heading)) {
                Some(PanelRow::File(index, count)) if *index == hit.index => *count += 1,
                _ => {
                    self.heading = Some(self.rows.len());
                    self.rows.push(PanelRow::File(hit.index, 1));
                }
            }
            self.rows.push(PanelRow::Hit(self.hits.len()));
            self.hits.push(hit);
        }
        if let Some(ended) = ended {
            self.ended = Some(ended);
            self.inbox = None;
        }
        changed
    }
}
impl Drop for ChangeSetSearch {
    fn drop(&mut self) {
        self.cancel();
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn change_set_search_reports_a_bad_expression_and_does_not_start() {
        use lenz_content::find::{FindOption, FindOptions};
        let mut search = ChangeSetSearch::new("(".into(), false, FindOptions { regex: true, ..Default::default() });
        assert_eq!(search.query_error(), Some("unclosed group"));
        assert_eq!(search.disabled(&SearchCommand::Start), Some("Fix the regular expression"));
        search.toggle_option(FindOption::Regex);
        assert_eq!(search.query_error(), None);
        assert!(!search.options().regex);
    }

    use super::*;
    use crate::{changeset::Spec, view::Side};
    fn pair_set(left: &str, right: &str) -> (tempfile::TempDir, Arc<ChangeSet>) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("left"), left).unwrap();
        std::fs::write(dir.path().join("right"), right).unwrap();
        let set = ChangeSet::load(
            &Spec::Plain {
                left: dir.path().join("left"),
                right: dir.path().join("right"),
            },
            dir.path(),
        )
        .unwrap();
        (dir, Arc::new(set))
    }
    fn ready(search: &mut ChangeSetSearch) {
        let started = std::time::Instant::now();
        while search.busy() {
            search.poll();
            assert!(started.elapsed().as_secs() < 10);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }
    #[test]
    fn streamed_real_file_results_have_both_sides_groups_and_nonwrapping_navigation() {
        let (_dir, set) = pair_set("common\nneedle old\n", "common\nneedle new\n");
        let mut search = ChangeSetSearch::new("needle".into(), false, Default::default());
        search.start(set.clone());
        ready(&mut search);
        assert_eq!(search.hits.len(), 2);
        assert_eq!(search.rows()[0], PanelRow::File(0, 2));
        assert_eq!(
            search
                .execute(SearchCommand::Step(true), &set, 4.0)
                .unwrap()
                .side,
            Side::Left
        );
        assert_eq!(
            search
                .execute(SearchCommand::Step(true), &set, 4.0)
                .unwrap()
                .side,
            Side::Right
        );
        search.execute(SearchCommand::Step(true), &set, 4.0);
        assert_eq!(search.at, Some(1));
        assert_eq!(search.notice(), Some("Last match in the Change Set"));
    }
    #[test]
    fn changed_only_excludes_unchanged_lines_and_cap_is_observable() {
        let (_dir, set) = pair_set("needle common\nold\n", "needle common\nneedle new\n");
        let mut search = ChangeSetSearch::new("needle".into(), true, Default::default());
        search.start(set);
        ready(&mut search);
        assert_eq!(search.hits.len(), 1);
        assert_eq!(search.hits[0].line, 1);
        let (_dir, set) = pair_set("", &"needle ".repeat(find::MAX_HITS + 50));
        search.start(set);
        ready(&mut search);
        assert_eq!(search.ended, Some(Ended::Capped));
        assert_eq!(search.hits.len(), find::MAX_HITS);
    }
    #[test]
    fn borrowed_lookup_preserves_file_side_line_and_utf8_byte_order() {
        let dir = tempfile::tempdir().unwrap();
        for side in ["left", "right"] {
            std::fs::create_dir(dir.path().join(side)).unwrap();
        }
        for (path, text) in [
            ("left/a.txt", "安全 Kk K\nnone\nk\n"),
            ("right/a.txt", "éK ék\nnone\n"),
            ("left/b.txt", "none\nK\n"),
            ("right/b.txt", "Kk\n"),
        ] {
            std::fs::write(dir.path().join(path), text).unwrap();
        }
        let set = Arc::new(
            ChangeSet::load(
                &Spec::Plain {
                    left: dir.path().join("left"),
                    right: dir.path().join("right"),
                },
                dir.path(),
            )
            .unwrap(),
        );
        let mut search = ChangeSetSearch::new("k".into(), false, Default::default());
        search.start(set);
        ready(&mut search);
        let actual: Vec<_> = search
            .hits()
            .iter()
            .map(|hit| (hit.index, hit.side, hit.line, hit.range.clone()))
            .collect();
        assert_eq!(
            actual,
            vec![
                (0, Side::Left, 0, 7..10),
                (0, Side::Left, 0, 10..11),
                (0, Side::Left, 0, 12..13),
                (0, Side::Left, 2, 0..1),
                (0, Side::Right, 0, 2..3),
                (0, Side::Right, 0, 6..7),
                (1, Side::Left, 1, 0..3),
                (1, Side::Right, 0, 0..1),
                (1, Side::Right, 0, 1..2)
            ]
        );
        for index in 0..4 {
            for side in [Side::Left, Side::Right] {
                let expected: Vec<_> = search
                    .hits()
                    .iter()
                    .filter(|hit| hit.index == index && hit.side == side)
                    .collect();
                let borrowed = search.hits_for_side(index, side);
                assert_eq!(borrowed.iter().collect::<Vec<_>>(), expected);
                if let Some(first) = expected.first() {
                    assert!(std::ptr::eq(borrowed.as_ptr(), *first));
                }
                for line in 0..4 {
                    let expected: Vec<_> = expected
                        .iter()
                        .copied()
                        .filter(|hit| hit.line == line)
                        .collect();
                    let borrowed = search.hits_on_line(index, side, line);
                    assert_eq!(borrowed.iter().collect::<Vec<_>>(), expected);
                    for hit in borrowed {
                        assert!(
                            hit.preview.is_char_boundary(hit.range.start)
                                && hit.preview.is_char_boundary(hit.range.end)
                        );
                    }
                }
            }
        }
    }
    #[test]
    fn replacement_and_drop_cancel_obsolete_work_without_publishing_stale_hits() {
        let (_dir, set) = pair_set("", &"old query\n".repeat(50_000));
        let mut search = ChangeSetSearch::new("old".into(), false, Default::default());
        search.start(set.clone());
        let cancelled = search.cancel.clone();
        search.execute(SearchCommand::Query("missing".into()), &set, 5.0);
        search.start(set);
        ready(&mut search);
        assert!(cancelled.load(Ordering::Relaxed));
        assert!(search.hits.is_empty());
        assert_eq!(search.ended, Some(Ended::Done));
        let cancel = search.cancel.clone();
        drop(search);
        assert!(cancel.load(Ordering::Relaxed));
    }
}

#[cfg(test)]
mod scheduler_perf {
    use super::*;
    #[test]
    #[ignore = "release Change Set search scheduling, streaming and cancellation measurements"]
    fn search_session_perf_stream_cancel_and_retained_rows() {
        let dir = tempfile::tempdir().unwrap();
        for side in ["left", "right"] {
            std::fs::create_dir(dir.path().join(side)).unwrap();
        }
        for file in 0..24 {
            let left: String = (0..4000)
                .map(|line| format!("value {file} {line}\n"))
                .collect();
            let right: String = (0..4000)
                .map(|line| {
                    if line % 17 == 0 {
                        format!("needle {file} {line}\n")
                    } else {
                        format!("value {file} {line}\n")
                    }
                })
                .collect();
            std::fs::write(dir.path().join("left").join(format!("{file:02}.txt")), left).unwrap();
            std::fs::write(
                dir.path().join("right").join(format!("{file:02}.txt")),
                right,
            )
            .unwrap();
        }
        let set = Arc::new(
            ChangeSet::load(
                &crate::changeset::Spec::Plain {
                    left: dir.path().join("left"),
                    right: dir.path().join("right"),
                },
                dir.path(),
            )
            .unwrap(),
        );
        let mut search = ChangeSetSearch::new(String::new(), false, Default::default());
        let (mut complete, mut first, mut cancel, mut row_queries) =
            (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        let mut rss = Vec::new();
        for sample in 0..21 {
            let query = if sample % 3 == 0 {
                "absent-query"
            } else if sample % 3 == 1 {
                "needle"
            } else {
                "value"
            };
            search.execute(SearchCommand::Query(query.into()), &set, 10.0);
            let started = std::time::Instant::now();
            search.start(set.clone());
            let mut first_at = None;
            while search.busy() {
                search.poll();
                if first_at.is_none() && !search.hits().is_empty() {
                    first_at = Some(started.elapsed().as_secs_f64() * 1000.0);
                }
                assert!(started.elapsed().as_secs() < 30);
                std::thread::sleep(std::time::Duration::from_micros(100));
            }
            complete.push(started.elapsed().as_secs_f64() * 1000.0);
            if let Some(first_at) = first_at {
                first.push(first_at);
            }
            let expected = if query == "absent-query" {
                0
            } else if query == "needle" {
                24 * 236
            } else {
                find::MAX_HITS
            };
            assert_eq!(search.hits().len(), expected);
            if query == "value" {
                assert_eq!(search.ended(), Some(Ended::Capped));
            }
            let started = std::time::Instant::now();
            for _ in 0..1000 {
                for row in std::hint::black_box(search.rows()).iter().skip(10).take(20) {
                    std::hint::black_box(row);
                }
            }
            row_queries.push(started.elapsed().as_secs_f64() * 1000.0);
            search.execute(SearchCommand::Query("absent-query".into()), &set, 10.0);
            search.start(set.clone());
            let started = std::time::Instant::now();
            search.cancel();
            assert!(!search.busy());
            cancel.push(started.elapsed().as_secs_f64() * 1000.0);
            if sample % 3 == 0 && let Some(value) = crate::perf::rss_mib() {
                rss.push(value);
            }
        }
        // Equivalent source-paint lookup over a real capped search: 50 lines on each side.
        search.execute(SearchCommand::Query("value".into()), &set, 10.0);
        search.start(set.clone());
        while search.busy() {
            search.poll();
            std::thread::sleep(std::time::Duration::from_micros(100));
        }
        assert_eq!(search.ended(), Some(Ended::Capped));
        assert_eq!(search.hits().len(), find::MAX_HITS);
        let paint = |lookup: bool| {
            let mut count = 0usize;
            for side in [crate::view::Side::Left, crate::view::Side::Right] {
                for line in 0..50 {
                    if lookup {
                        for hit in std::hint::black_box(&search).hits_on_line(0, side, line) {
                            std::hint::black_box(&hit.range);
                            count += 1;
                        }
                    } else {
                        for hit in std::hint::black_box(search.hits())
                            .iter()
                            .filter(|hit| hit.index == 0 && hit.side == side && hit.line == line)
                        {
                            std::hint::black_box(&hit.range);
                            count += 1;
                        }
                    }
                }
            }
            count
        };
        let expected = paint(false);
        assert_eq!(paint(true), expected);
        assert!(expected > 0);
        let (mut linear, mut lookup) = (Vec::new(), Vec::new());
        for sample in 0..11 {
            for optimized in if sample % 2 == 0 {
                [false, true]
            } else {
                [true, false]
            } {
                let started = std::time::Instant::now();
                let mut count = 0;
                for _ in 0..100 {
                    count += std::hint::black_box(paint(optimized));
                }
                assert_eq!(count, expected * 100);
                let duration = started.elapsed().as_secs_f64() * 1000.0 / 100.0;
                if optimized {
                    lookup.push(duration);
                } else {
                    linear.push(duration);
                }
            }
        }
        crate::perf::record(
            "search_session.paint_100_visible_lines_linear",
            "ms/paint",
            &linear,
        );
        crate::perf::record(
            "search_session.paint_100_visible_lines_lookup",
            "ms/paint",
            &lookup,
        );
        crate::perf::record("search_session.complete_96k_lines", "ms", &complete);
        crate::perf::record("search_session.first_batch_96k_lines", "ms", &first);
        crate::perf::record("search_session.cancel_to_idle", "ms", &cancel);
        crate::perf::record(
            "search_session.visible_rows_1000_queries",
            "ms",
            &row_queries,
        );
        if !rss.is_empty() {
            crate::perf::record("search_session.sustained_rss", "MiB", &rss);
        }
    }
}

fn query_error(text: &str, options: FindOptions) -> Option<String> {
    Matcher::new(&Query { text: text.into(), options }).err().map(|e| e.to_string())
}
