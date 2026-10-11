//! Streaming blame jobs and commit inspection without frontend resources.
use crate::{
    blame::{self, BlameLine, CommitDetails, Hunk, Version},
    changeset::ChangeSet,
    view::Side,
};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock, Weak, mpsc},
};

enum Update {
    Details(Result<CommitDetails, String>),
}
enum WorkerKind {
    Annotation(Side),
    Details,
}
fn submit(kind: WorkerKind, job: impl FnOnce() + Send + 'static) {
    type Job = Box<dyn FnOnce() + Send>;
    static LEFT: OnceLock<mpsc::Sender<Job>> = OnceLock::new();
    static RIGHT: OnceLock<mpsc::Sender<Job>> = OnceLock::new();
    static DETAILS: OnceLock<mpsc::Sender<Job>> = OnceLock::new();
    let (worker, name) = match kind {
        WorkerKind::Annotation(Side::Left) => (&LEFT, "lenz-blame-left"),
        WorkerKind::Annotation(Side::Right) => (&RIGHT, "lenz-blame-right"),
        WorkerKind::Details => (&DETAILS, "lenz-commit-details"),
    };
    let tx = worker.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                for job in rx {
                    // One job's panic (in an unwinding build) must not take the worker with it.
                    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job));
                }
            })
            .expect("start annotation worker");
        tx
    });
    // A dead worker drops the job, whose own channel then reports the interruption.
    let _ = tx.send(Box::new(job));
}

const CHUNK_LINES: usize = 256;
#[derive(Clone)]
struct Chunk {
    lines: Box<[Option<BlameLine>]>,
}
struct Pending {
    chunks: Vec<Option<Arc<Chunk>>>,
    dirty: Vec<usize>,
    dirty_bits: Vec<bool>,
    extent: Option<(i64, i64)>,
    ended: Option<Result<(), String>>,
    count_limit: Option<usize>,
    #[cfg(test)]
    finished_at: Option<std::time::Instant>,
}
impl Pending {
    fn new(count: Option<usize>) -> Self {
        let n = count.unwrap_or(0).div_ceil(CHUNK_LINES);
        Self {
            chunks: vec![None; n],
            dirty: Vec::new(),
            dirty_bits: vec![false; n],
            extent: None,
            ended: None,
            count_limit: count,
            #[cfg(test)]
            finished_at: None,
        }
    }
}
#[derive(Default)]
pub struct Annotations {
    token: Option<Arc<()>>,
    pending: Option<Arc<Mutex<Pending>>>,
    chunks: Vec<Option<Arc<Chunk>>>,
    enabled: bool,
    extent: Option<(i64, i64)>,
    error: Option<String>,
}
impl Annotations {
    /// A GUI poller must stop when its request is dropped, even if the same path is reopened.
    #[cfg(feature = "gui")]
    pub(crate) fn request_id(&self) -> Weak<()> {
        self.token.as_ref().map_or_else(Weak::new, Arc::downgrade)
    }
    pub fn enabled(&self) -> bool {
        self.enabled
    }
    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
    pub fn line(&self, line: usize) -> Option<&BlameLine> {
        if !self.enabled {
            return None;
        }
        self.chunks
            .get(line / CHUNK_LINES)?
            .as_ref()?
            .lines
            .get(line % CHUNK_LINES)?
            .as_ref()
    }
    pub fn clear(&mut self) {
        *self = Self::default();
    }
    pub fn toggle(
        &mut self,
        set: Arc<ChangeSet>,
        index: usize,
        side: Side,
        version: Option<Version>,
        count: usize,
    ) -> bool {
        self.enabled = !self.enabled;
        if !self.enabled || self.pending.is_some() || !self.chunks.is_empty() {
            return true;
        }
        self.begin(set, index, side, version, Some(count))
    }
    /// Begin with a known line count, or stream its size when the document is not loaded yet.
    pub fn begin(
        &mut self,
        set: Arc<ChangeSet>,
        index: usize,
        side: Side,
        version: Option<Version>,
        count: Option<usize>,
    ) -> bool {
        self.clear();
        self.enabled = true;
        let Some(version) = version else {
            self.enabled = false;
            self.error = Some("This side has no Git blame version".into());
            return false;
        };
        let token = Arc::new(());
        let weak = Arc::downgrade(&token);
        let inbox = Arc::new(Mutex::new(Pending::new(count)));
        let pending = Arc::downgrade(&inbox);
        self.chunks
            .resize_with(count.unwrap_or(0).div_ceil(CHUNK_LINES), || None);
        self.token = Some(token);
        self.pending = Some(inbox);
        submit(WorkerKind::Annotation(side), move || {
            if weak.strong_count() == 0 {
                return;
            }
            let result =
                blame::blame_incremental(&set, &set.comparisons[index], side, &version, |h| {
                    stage_hunk(&pending, &weak, h)
                });
            if let Some(inbox) = pending.upgrade() {
                let mut p = inbox.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                #[cfg(test)]
                {
                    p.finished_at = Some(std::time::Instant::now());
                }
                p.ended = Some(result);
            }
        });
        true
    }
    pub fn poll(&mut self) -> bool {
        let Some(inbox) = self.pending.as_ref().cloned() else {
            return false;
        };
        let (changed, ended) = {
            let mut p = inbox.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            self.chunks.resize_with(p.chunks.len(), || None);
            let changed = !p.dirty.is_empty() || self.extent != p.extent || p.ended.is_some();
            let mut dirty = std::mem::take(&mut p.dirty);
            for &i in &dirty {
                self.chunks[i] = p.chunks[i].clone();
                p.dirty_bits[i] = false;
            }
            dirty.clear();
            p.dirty = dirty;
            self.extent = p.extent;
            (changed, p.ended.take())
        };
        if let Some(result) = ended {
            self.error = result.err();
            self.pending = None;
        }
        changed
    }
    pub fn age_extent(&self) -> Option<(i64, i64)> {
        self.extent
    }
}
/// Coalesce immutable chunk snapshots instead of queuing individual hunks. The producer
/// never waits for the GUI's 100ms poll; pending storage is bounded by the document's rows.
/// Locks cover at most 256 rows so publication and cancellation remain responsive.
fn stage_hunk(pending: &Weak<Mutex<Pending>>, current: &Weak<()>, h: Hunk) {
    let mut start = h.lines.start;
    let mut first = true;
    while start < h.lines.end {
        if current.strong_count() == 0 {
            return;
        }
        let Some(inbox) = pending.upgrade() else {
            return;
        };
        let mut p = inbox.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if first && h.commit.is_committed() {
            let time = h.commit.author_time;
            p.extent = Some(
                p.extent
                    .map_or((time, time), |(old, new)| (old.min(time), new.max(time))),
            );
        }
        first = false;
        let end = h.lines.end.min(p.count_limit.unwrap_or(usize::MAX));
        if start >= end {
            return;
        }
        let index = start / CHUNK_LINES;
        if index >= p.chunks.len() {
            p.chunks.resize_with(index + 1, || None);
            p.dirty_bits.resize(index + 1, false);
        }
        if !p.dirty_bits[index] {
            p.dirty_bits[index] = true;
            p.dirty.push(index);
        }
        let end = end.min((index + 1) * CHUNK_LINES);
        let chunk = p.chunks[index].get_or_insert_with(|| {
            Arc::new(Chunk {
                lines: vec![None; CHUNK_LINES].into_boxed_slice(),
            })
        });
        let chunk = Arc::make_mut(chunk);
        for line in start..end {
            chunk.lines[line % CHUNK_LINES] = Some(BlameLine {
                commit: h.commit.clone(),
                original_line: h.original_line + line - h.lines.start,
                original_path: h.original_path.clone(),
            });
        }
        start = end;
    }
}

#[cfg(test)]
fn apply(lines: &mut [Option<BlameLine>], h: Hunk) {
    let range = h.lines.start.min(lines.len())..h.lines.end.min(lines.len());
    for i in range {
        lines[i] = Some(BlameLine {
            commit: h.commit.clone(),
            original_line: h.original_line + i - h.lines.start,
            original_path: h.original_path.clone(),
        });
    }
}

#[derive(Default)]
pub struct CommitInspection {
    token: Option<Arc<()>>,
    pending: Option<mpsc::Receiver<Update>>,
    details: Option<CommitDetails>,
    error: Option<String>,
    sha: Option<String>,
}
impl CommitInspection {
    pub fn open(&mut self, root: PathBuf, sha: String) {
        self.close();
        self.sha = Some(sha.clone());
        let token = Arc::new(());
        let weak: Weak<()> = Arc::downgrade(&token);
        let (tx, rx) = mpsc::sync_channel(1);
        self.token = Some(token);
        self.pending = Some(rx);
        submit(WorkerKind::Details, move || {
            if weak.strong_count() > 0 {
                let _ = tx.send(Update::Details(blame::commit_details(&root, &sha)));
            }
        });
    }
    pub fn close(&mut self) {
        *self = Self::default();
    }
    pub fn visible(&self) -> bool {
        self.sha.is_some()
    }
    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }
    pub fn details(&self) -> Option<&CommitDetails> {
        self.details.as_ref()
    }
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
    pub fn sha(&self) -> Option<&str> {
        self.sha.as_deref()
    }
    pub fn poll(&mut self) -> bool {
        let Some(rx) = &self.pending else {
            return false;
        };
        match rx.try_recv() {
            Ok(Update::Details(result)) => {
                match result {
                    Ok(d) => self.details = Some(d),
                    Err(e) => self.error = Some(e),
                };
                self.pending = None;
                true
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.pending = None;
                true
            }
            Err(mpsc::TryRecvError::Empty) => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn streaming_annotations_and_commit_details_follow_the_requested_revision() {
        use crate::{
            changeset::Spec,
            git::testing::{commit_all, git, repo, write},
        };
        let root = repo();
        write(root.path(), "a.rs", "one\ntwo\n");
        commit_all(root.path(), "first");
        write(root.path(), "a.rs", "one\nTWO\nthree\n");
        commit_all(root.path(), "second\n\nFull body");
        let spec = Spec::Git {
            cached: false,
            revs: vec!["HEAD~".into(), "HEAD".into()],
            paths: vec![],
            untracked: true,
        };
        let set = Arc::new(ChangeSet::load(&spec, root.path()).unwrap());
        let index = set
            .comparisons
            .iter()
            .position(|c| c.path == "a.rs")
            .unwrap();
        let mut annotations = Annotations::default();
        let version = blame::version(&spec, &set.comparisons[index], Side::Right);
        annotations.toggle(set.clone(), index, Side::Right, version, 3);
        let mut left = Annotations::default();
        left.begin(
            set.clone(),
            index,
            Side::Left,
            blame::version(&spec, &set.comparisons[index], Side::Left),
            None,
        );
        let started = std::time::Instant::now();
        while annotations.busy() || left.busy() {
            annotations.poll();
            left.poll();
            assert!(started.elapsed().as_secs() < 10);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert_eq!(left.line(1).unwrap().commit.summary, "first");
        assert!(left.line(2).is_none());
        assert_eq!(annotations.line(0).unwrap().commit.summary, "first");
        assert_eq!(annotations.line(1).unwrap().commit.summary, "second");
        assert_eq!(annotations.line(2).unwrap().original_line, 3);
        assert!(annotations.age_extent().is_some());
        let sha = git(root.path(), &["rev-parse", "HEAD"]).trim().to_string();
        let mut inspection = CommitInspection::default();
        inspection.open(root.path().into(), sha);
        while inspection.busy() {
            inspection.poll();
            assert!(started.elapsed().as_secs() < 10);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert_eq!(inspection.details().unwrap().message, "second\n\nFull body");
        inspection.close();
        annotations.clear();
        assert!(!inspection.visible());
        assert!(annotations.line(0).is_none());
    }

    #[test]
    fn handoff_coalesces_chunks_preserves_snapshots_and_drops_on_cancel() {
        let token = Arc::new(());
        let weak = Arc::downgrade(&token);
        let inbox = Arc::new(Mutex::new(Pending::new(Some(1024))));
        let pending = Arc::downgrade(&inbox);
        let mut annotations = Annotations {
            token: Some(token),
            pending: Some(inbox),
            enabled: true,
            ..Default::default()
        };
        let first = Arc::new(blame::Commit {
            sha: "a".repeat(40),
            summary: "first".into(),
            ..Default::default()
        });
        stage_hunk(
            &pending,
            &weak,
            Hunk {
                lines: 0..3,
                commit: first,
                original_line: 1,
                original_path: "a".into(),
            },
        );
        assert!(annotations.poll());
        let snapshot = annotations.chunks[0].as_ref().unwrap().clone();
        let second = Arc::new(blame::Commit {
            sha: "b".repeat(40),
            summary: "second".into(),
            ..Default::default()
        });
        for _ in 0..1000 {
            stage_hunk(
                &pending,
                &weak,
                Hunk {
                    lines: 1..3,
                    commit: second.clone(),
                    original_line: 7,
                    original_path: "a".into(),
                },
            );
        }
        assert_eq!(pending.upgrade().unwrap().lock().unwrap().dirty.len(), 1);
        assert_eq!(annotations.line(1).unwrap().commit.summary, "first");
        assert_eq!(snapshot.lines[1].as_ref().unwrap().commit.summary, "first");
        annotations.poll();
        assert_eq!(annotations.line(1).unwrap().commit.summary, "second");
        stage_hunk(
            &pending,
            &weak,
            Hunk {
                lines: 900..2000,
                commit: second,
                original_line: 1,
                original_path: "a".into(),
            },
        );
        assert_eq!(pending.upgrade().unwrap().lock().unwrap().chunks.len(), 4);
        annotations.poll();
        assert!(annotations.line(1023).is_some());
        assert!(annotations.line(1024).is_none());
        annotations.clear();
        assert!(pending.upgrade().is_none());
        assert_eq!(weak.strong_count(), 0);
    }

    #[test]
    #[ignore = "release fragmented blame throughput and GUI 100ms polling regression"]
    fn annotation_perf_fragmented_polling() {
        use crate::{
            changeset::Spec,
            git::testing::{commit_all, repo, write},
            perf,
        };
        let root = repo();
        let mut lines = (0..1280)
            .map(|i| format!("line {i} old"))
            .collect::<Vec<_>>();
        write(root.path(), "a.txt", lines.join("\n") + "\n");
        commit_all(root.path(), "base");
        for i in (0..lines.len()).step_by(2) {
            lines[i] = format!("line {i} changed");
        }
        write(root.path(), "a.txt", lines.join("\n") + "\n");
        commit_all(root.path(), "alternating");
        let spec = Spec::Git {
            cached: false,
            revs: vec!["HEAD~".into(), "HEAD".into()],
            paths: vec![],
            untracked: true,
        };
        let set = Arc::new(ChangeSet::load(&spec, root.path()).unwrap());
        let index = set
            .comparisons
            .iter()
            .position(|c| c.path == "a.txt")
            .unwrap();
        let version = Version::Rev("HEAD".into());
        let mut hunks = Vec::new();
        blame::blame_incremental(&set, &set.comparisons[index], Side::Right, &version, |h| {
            hunks.push(h)
        })
        .unwrap();
        let count = hunks.len();
        assert_eq!(count, 1280);
        let (tx, rx) = mpsc::sync_channel(32);
        let source = set.clone();
        let v = version.clone();
        let started = std::time::Instant::now();
        let worker = std::thread::spawn(move || {
            blame::blame_incremental(&source, &source.comparisons[index], Side::Right, &v, |h| {
                tx.send(h).unwrap()
            })
            .unwrap();
            std::time::Instant::now()
        });
        let mut received = 0;
        while received < count {
            std::thread::sleep(std::time::Duration::from_millis(100));
            while rx.try_recv().is_ok() {
                received += 1;
            }
        }
        let completed = worker.join().unwrap();
        let throttled = completed.duration_since(started).as_secs_f64() * 1000.0;
        let mut direct = Vec::new();
        let mut coalesced = Vec::new();
        let mut original_poll = Vec::new();
        let mut coalesced_poll = Vec::new();
        let mut original_ready = Vec::new();
        let mut coalesced_ready = Vec::new();
        let mut original_storage = Vec::new();
        let mut coalesced_storage = Vec::new();
        let mut dirty_max = 0;
        let wait_for_poll = |started: std::time::Instant| {
            let remaining = std::time::Duration::from_millis(100).saturating_sub(started.elapsed());
            std::thread::sleep(remaining);
        };
        for _ in 0..11 {
            // Reproduce the original GUI's unbounded Mutex inbox. Pair each direct
            // sample with the new handoff to keep subprocess/cache conditions close.
            let original_inbox = Arc::new(Mutex::new(Vec::new()));
            let source = set.clone();
            let inbox = original_inbox.clone();
            let v = version.clone();
            let (completed_tx, completed_rx) = mpsc::channel();
            let started = std::time::Instant::now();
            submit(WorkerKind::Annotation(Side::Right), move || {
                blame::blame_incremental(
                    &source,
                    &source.comparisons[index],
                    Side::Right,
                    &v,
                    |h| inbox.lock().unwrap().push(h),
                )
                .unwrap();
                completed_tx.send(std::time::Instant::now()).unwrap();
            });
            direct.push(
                completed_rx
                    .recv()
                    .unwrap()
                    .duration_since(started)
                    .as_secs_f64()
                    * 1000.0,
            );
            assert_eq!(original_inbox.lock().unwrap().len(), count);
            wait_for_poll(started);
            let applying = std::time::Instant::now();
            let mut old_lines = Vec::new();
            let mut old_extent: Option<(i64, i64)> = None;
            for h in original_inbox.lock().unwrap().drain(..) {
                old_lines.resize_with(old_lines.len().max(h.lines.end), || None);
                if h.commit.is_committed() {
                    let time = h.commit.author_time;
                    old_extent =
                        Some(old_extent.map_or((time, time), |(a, b)| (a.min(time), b.max(time))));
                }
                apply(&mut old_lines, h);
            }
            original_poll.push(applying.elapsed().as_secs_f64() * 1000.0);
            original_ready.push(started.elapsed().as_secs_f64() * 1000.0);
            original_storage
                .push((old_lines.capacity() * std::mem::size_of::<Option<BlameLine>>()) as f64);
            let mut a = Annotations::default();
            let started = std::time::Instant::now();
            a.begin(
                set.clone(),
                index,
                Side::Right,
                Some(version.clone()),
                Some(1280),
            );
            let finished_at = loop {
                let inbox = a.pending.as_ref().unwrap().lock().unwrap();
                dirty_max = dirty_max.max(inbox.dirty.len());
                let completed = inbox.finished_at;
                drop(inbox);
                if let Some(completed) = completed {
                    break completed;
                }
                assert!(started.elapsed().as_secs() < 10);
                std::thread::sleep(std::time::Duration::from_millis(1));
            };
            coalesced.push(finished_at.duration_since(started).as_secs_f64() * 1000.0);
            wait_for_poll(started);
            let publishing = std::time::Instant::now();
            assert!(a.poll());
            coalesced_poll.push(publishing.elapsed().as_secs_f64() * 1000.0);
            coalesced_ready.push(started.elapsed().as_secs_f64() * 1000.0);
            coalesced_storage.push(
                (a.chunks.capacity() * std::mem::size_of::<Option<Arc<Chunk>>>()
                    + a.chunks
                        .iter()
                        .flatten()
                        .map(|c| {
                            std::mem::size_of::<Chunk>()
                                + c.lines.len() * std::mem::size_of::<Option<BlameLine>>()
                        })
                        .sum::<usize>()) as f64,
            );
            assert!(!a.busy());
            assert_eq!(a.age_extent(), old_extent);
            for (line, old) in old_lines.iter().enumerate() {
                let old = old.as_ref().unwrap();
                assert_eq!(old.commit.sha, a.line(line).unwrap().commit.sha);
                assert_eq!(old.original_line, a.line(line).unwrap().original_line);
                assert_eq!(old.original_path, a.line(line).unwrap().original_path);
                assert_eq!(
                    a.line(line).unwrap().commit.summary,
                    if line % 2 == 0 { "alternating" } else { "base" }
                );
            }
        }
        assert!(dirty_max <= 1280usize.div_ceil(CHUNK_LINES));
        perf::record("blame/fragmented/hunks", "count", &[count as f64]);
        perf::record("blame/fragmented/direct-producer", "ms", &direct);
        perf::record(
            "blame/fragmented/rejected-channel32-producer",
            "ms",
            &[throttled],
        );
        perf::record("blame/fragmented/coalesced-producer", "ms", &coalesced);
        perf::record("blame/fragmented/original-ui-apply", "ms", &original_poll);
        perf::record(
            "blame/fragmented/coalesced-ui-publish",
            "ms",
            &coalesced_poll,
        );
        perf::record(
            "blame/fragmented/original-100ms-ready",
            "ms",
            &original_ready,
        );
        perf::record(
            "blame/fragmented/coalesced-100ms-ready",
            "ms",
            &coalesced_ready,
        );
        perf::record(
            "blame/fragmented/original-line-storage",
            "bytes",
            &original_storage,
        );
        perf::record(
            "blame/fragmented/coalesced-line-storage",
            "bytes",
            &coalesced_storage,
        );
        perf::record(
            "blame/fragmented/coalesced-dirty-chunks",
            "count",
            &[dirty_max as f64],
        );
    }

    #[test]
    fn incremental_hunks_preserve_original_line_and_share_commit() {
        let mut lines = vec![None; 4];
        let commit = Arc::new(blame::Commit {
            sha: "a".repeat(40),
            author: "Ada".into(),
            ..Default::default()
        });
        apply(
            &mut lines,
            Hunk {
                lines: 1..5,
                commit: commit.clone(),
                original_line: 9,
                original_path: "old.rs".into(),
            },
        );
        assert!(lines[0].is_none());
        assert_eq!(lines[3].as_ref().unwrap().original_line, 11);
        assert!(Arc::ptr_eq(
            &lines[1].as_ref().unwrap().commit,
            &lines[3].as_ref().unwrap().commit
        ));
    }
}
