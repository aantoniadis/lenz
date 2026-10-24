//! Annotate with Git Blame: per tab, which commit last changed each line of the admitted current draft. The blame streams
//! from its own worker thread, so a long history never holds the shared jobs that open and colour files.

use std::{
    ops::Range,
    path::PathBuf,
    sync::{
        Arc, Mutex, OnceLock, PoisonError, Weak,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    time::Duration,
};

use lenz_content::blame::{self as git_blame, Commit, CommitDetails};

use crate::{cache::Stamp, session::Completion, tabs::TabId, text::{SnapshotKey, TextSnapshot}, document::{Document, SourceMeta}, analysis::Lease};

/// The annotation column's width in cells: `YYYY-MM-DD` and an author cut to 12.
pub const ANNOTATION_COLS: usize = 23;
const AUTHOR_COLS: usize = ANNOTATION_COLS - 11;

/// A tab's annotations while they are on: what blame said so far about the content of `stamp`.
pub struct Blame {
    pub(crate) stamp: Option<Stamp>,
    pub(crate) key: Option<SnapshotKey>,
    metadata: Weak<SourceMeta>,
    pub(crate) request: u64,
    pub(crate) project: u64,
    /// The request this tab still wants; a job whose request differs stops and ends its git.
    latest: Arc<AtomicU64>,
    pub(crate) root: Option<PathBuf>,
    pub(crate) lines: Vec<Option<Arc<Commit>>>,
    /// Oldest and newest committed time seen.
    pub(crate) extent: Option<(i64, i64)>,
    pub(crate) done: bool,
    pub(crate) error: Option<String>,
}

impl Blame {
    pub(crate) fn new() -> Self {
        Blame { stamp: None, key: None, metadata: Weak::new(), error: None, request: 0, project: 0, latest: Arc::new(AtomicU64::new(0)), root: None, lines: Vec::new(), extent: None, done: false }
    }

    /// Identity includes draft bytes, path metadata and the saved-file identity used by Git.
    pub(crate) fn of(&self, stamp: Option<Stamp>, snapshot: &TextSnapshot) -> bool {
        self.request != 0 && self.stamp == stamp && self.matches(snapshot)
    }

    pub(crate) fn matches(&self, snapshot: &TextSnapshot) -> bool {
        self.key == Some(snapshot.key()) && self.metadata.ptr_eq(&Arc::downgrade(snapshot.source_meta()))
    }

    /// Clear annotations synchronously; only admitted immutable draft bytes reach the existing worker.
    pub(crate) fn start(&mut self, tab: TabId, file: PathBuf, stamp: Option<Stamp>, snapshot: &TextSnapshot, request: u64)
        -> impl FnOnce(Arc<Document>, Lease) + Send + 'static
    {
        self.latest.store(request, Ordering::Relaxed);
        (self.stamp, self.key, self.metadata, self.request, self.root, self.lines, self.extent, self.done, self.error) =
            (stamp, Some(snapshot.key()), Arc::downgrade(snapshot.source_meta()), request, None, vec![None; snapshot.line_count()], None, false, None);
        let latest = self.latest.clone();
        move |document, lease| submit(Box::new(move || {
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(tab, &file, &document, request, &latest, &lease))).is_err()
                && latest.load(Ordering::Relaxed) == request {
                lease.send(Completion::Blamed { tab, request, root: None, hunks: Vec::new(), done: Some(Err("Blame worker failed; retry annotations".into())) });
            }
        }))
    }

    pub(crate) fn suspend(&mut self) {
        self.latest.store(0, Ordering::Relaxed);
        self.request = 0;
        self.lines.fill(None);
    }

    pub(crate) fn fail(&mut self, error: String) {
        self.lines.fill(None);
        self.root = None;
        self.extent = None;
        self.error = Some(error);
        self.done = true;
    }

    pub(crate) fn status(&self) -> Status<'_> {
        match self.error.as_deref() { Some(error) => Status::Failed(error), None if !self.done => Status::Pending, None => Status::Ready }
    }

    pub(crate) fn apply(&mut self, root: Option<PathBuf>, hunks: Vec<(Range<usize>, Arc<Commit>)>) {
        if root.is_some() {
            self.root = root;
        }
        for (range, commit) in hunks {
            if commit.is_committed() {
                let t = commit.author_time;
                self.extent = Some(self.extent.map_or((t, t), |(old, new)| (old.min(t), new.max(t))));
            }
            let end = range.end.min(self.lines.len());
            for line in range.start.min(end)..end {
                self.lines[line] = Some(commit.clone());
            }
        }
    }

}

impl Drop for Blame {
    /// Turned off or its tab closed: a running blame stops.
    fn drop(&mut self) {
        self.latest.store(0, Ordering::Relaxed);
    }
}

/// Current-draft attribution state; pending and failed attribution cannot supply commit actions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status<'a> { Pending, Ready, Failed(&'a str) }
impl Status<'_> {
    pub fn label(self) -> Option<&'static str> {
        match self { Self::Pending => Some("Blame pending…"), Self::Failed(_) => Some("Blame failed"), Self::Ready => None }
    }
}

/// What the column shows for one line.
pub struct Annotation<'a> {
    pub label: String,
    /// Age between the file's oldest (0) and newest (1) commit; `None` for a line not committed.
    pub shade: Option<f32>,
    pub commit: &'a Arc<Commit>,
}

pub(crate) fn annotation(blame: &Blame, line: usize) -> Option<Annotation<'_>> {
    let commit = blame.lines.get(line)?.as_ref()?;
    if !commit.is_committed() {
        return Some(Annotation { label: "Not committed".into(), shade: None, commit });
    }
    let author = crate::chrome::ellipsize(&commit.author, AUTHOR_COLS);
    let shade = blame.extent.map(|(old, new)| git_blame::age_shade(commit.author_time, new, old));
    Some(Annotation { label: format!("{} {author}", git_blame::date(commit.author_time)), shade, commit })
}

/// Batches go out at this many lines, and this often while git is quiet, so a big file fills in as git goes without
/// flooding the session's channel.
const BATCH_LINES: usize = 256;
const BATCH_AFTER: Duration = Duration::from_millis(50);

fn run(tab: TabId, file: &std::path::Path, document: &Document, request: u64, latest: &AtomicU64, lease: &Lease) {
    let current = || latest.load(Ordering::Relaxed) == request && lease.wanted();
    if !current() {
        return;
    }
    let send = |root, hunks, done| {
        lease.send(Completion::Blamed { tab, request, root, hunks, done });
    };
    let (root, path) = match git_blame::repository(file) {
        Ok(found) => found,
        Err(e) => return send(None, Vec::new(), Some(Err(e))),
    };
    let metadata = document.source_meta();
    if metadata.is_lossy() { return send(None, Vec::new(), Some(Err("The file has undecodable bytes; exact draft attribution is unavailable".into()))); }
    let bytes = if metadata.original_encoding() == "UTF-8" && !metadata.had_bom() {
        std::borrow::Cow::Borrowed(document.text().as_bytes())
    } else {
        match lenz_content::write::encode_exact(document.text(), metadata.original_encoding(), metadata.had_bom()) {
            Ok(bytes) => std::borrow::Cow::Owned(bytes),
            Err(error) => return send(None, Vec::new(), Some(Err(error.to_string()))),
        }
    };
    if !current() { return; }
    send(Some(root.clone()), Vec::new(), None);
    stream(&current, |on| git_blame::blame_contents(&root, &path, &bytes, &current, on), &|hunks, done| send(None, hunks, done));
}

type Hunks = Vec<(Range<usize>, Arc<Commit>)>;

/// Runs `blame`, sending its runs of lines in batches: at [`BATCH_LINES`], and every [`BATCH_AFTER`] while git is
/// quiet, so what it reported shows before its next output. Then the rest with the result (always last).
fn stream(current: &(dyn Fn() -> bool + Sync), blame: impl FnOnce(&mut dyn FnMut(git_blame::Hunk)) -> Result<(), String>, send: &(dyn Fn(Hunks, Option<Result<(), String>>) + Sync)) {
    // Sent while holding the lock, so batches leave in order and the result after every one of them.
    let pending: Mutex<(Hunks, usize)> = Mutex::new((Vec::new(), 0));
    let flush = |p: &mut (Hunks, usize), done: Option<Result<(), String>>| {
        let hunks = std::mem::take(&mut p.0);
        p.1 = 0;
        if current() && (!hunks.is_empty() || done.is_some()) {
            send(hunks, done);
        }
    };
    let finished = AtomicBool::new(false);
    std::thread::scope(|s| {
        s.spawn(|| {
            while !finished.load(Ordering::Relaxed) {
                std::thread::sleep(BATCH_AFTER);
                let mut p = pending.lock().unwrap_or_else(PoisonError::into_inner);
                if !finished.load(Ordering::Relaxed) {
                    flush(&mut p, None);
                }
            }
        });
        let result = blame(&mut |h| {
            let mut p = pending.lock().unwrap_or_else(PoisonError::into_inner);
            p.1 += h.lines.len();
            p.0.push((h.lines, h.commit));
            if p.1 >= BATCH_LINES {
                flush(&mut p, None);
            }
        });
        let mut p = pending.lock().unwrap_or_else(PoisonError::into_inner);
        finished.store(true, Ordering::Relaxed);
        flush(&mut p, Some(result));
    });
}

type Job = Box<dyn FnOnce() + Send>;

/// The blame worker: one thread, started on first use, running jobs in order.
fn submit(job: Job) {
    static WORKER: OnceLock<Option<mpsc::Sender<Job>>> = OnceLock::new();
    let worker = WORKER.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("lenz-reader-blame".into())
            .spawn(move || {
                for job in rx {
                    // One job's panic (in an unwinding build) must not take the worker with it.
                    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job));
                }
            })
            .ok()
            .map(|_| tx)
    });
    match worker {
        Some(tx) => {
            let _ = tx.send(job);
        }
        None => job(), // no thread could start: work inline rather than never
    }
}

/// The commit an annotation opened, and its details once loaded.
pub struct CommitPopup {
    pub tab: TabId,
    pub sha: String,
    pub(crate) root: PathBuf,
    pub details: Option<Result<CommitDetails, String>>,
    pub(crate) request: u64,
}

/// At most this many message lines and changed files are listed.
const MESSAGE_LINES: usize = 8;
const FILES: usize = 10;

impl CommitPopup {
    /// The popup's text, as Lenz Diff's commit popup shows it.
    pub fn text(&self) -> String {
        let d = match &self.details {
            None => return format!("Loading commit {}…", self.sha.get(..7).unwrap_or(&self.sha)),
            Some(Err(e)) => return format!("commit {}\n\n{e}", self.sha),
            Some(Ok(d)) => d,
        };
        let mut rows = vec![format!("commit {}", d.sha), format!("Author: {} <{}>", d.author, d.email), format!("Date:   {}", git_blame::date(d.author_time)), String::new()];
        let message: Vec<&str> = d.message.lines().collect();
        rows.extend(message.iter().take(MESSAGE_LINES).map(|l| format!("    {l}").trim_end().to_string()));
        if message.len() > MESSAGE_LINES {
            rows.push("    …".into());
        }
        rows.push(String::new());
        rows.extend(d.files.iter().take(FILES).map(|(status, path)| format!("{status} {path}")));
        if d.files.len() > FILES {
            rows.push(format!("… {} more files", d.files.len() - FILES));
        }
        rows.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn lines_already_reported_go_out_while_git_is_quiet() {
        let (tx, rx) = mpsc::channel();
        let started = Instant::now();
        let send = |hunks: Vec<(Range<usize>, Arc<Commit>)>, done: Option<Result<(), String>>| {
            let _ = tx.send((started.elapsed(), hunks.len(), done.is_some()));
        };
        stream(&|| true, |on: &mut dyn FnMut(git_blame::Hunk)| {
            on(git_blame::not_committed(1, "a"));
            std::thread::sleep(Duration::from_millis(400));
            on(git_blame::not_committed(2, "a"));
            Ok(())
        }, &send);
        let got: Vec<_> = rx.try_iter().collect();
        assert!(got[0].0 < Duration::from_millis(300) && got[0].1 == 1 && !got[0].2, "the first run before git's next output: {got:?}");
        assert_eq!(got.iter().map(|g| g.1).sum::<usize>(), 2, "{got:?}");
        assert!(got.last().unwrap().2, "done last: {got:?}");
    }
}
