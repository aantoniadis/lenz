//! Annotate with Git Blame: per tab, which commit last changed each line of the file on disk. The blame streams
//! from its own worker thread, so a long history never holds the shared jobs that open and colour files.

use std::{
    ops::Range,
    path::PathBuf,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

use lenz_content::blame::{self as git_blame, Commit, CommitDetails};

use crate::{cache::Stamp, session::Completion, tabs::TabId};

/// The annotation column's width in cells: `YYYY-MM-DD` and an author cut to 12.
pub const ANNOTATION_COLS: usize = 23;
const AUTHOR_COLS: usize = ANNOTATION_COLS - 11;

/// A tab's annotations while they are on: what blame said so far about the content of `stamp`.
pub struct Blame {
    pub(crate) stamp: Option<Stamp>,
    pub(crate) request: u64,
    /// The request this tab still wants; a job whose request differs stops and ends its git.
    latest: Arc<AtomicU64>,
    pub(crate) root: Option<PathBuf>,
    pub(crate) lines: Vec<Option<Arc<Commit>>>,
    /// Oldest and newest committed time seen.
    pub(crate) extent: Option<(i64, i64)>,
    pub(crate) done: bool,
}

impl Blame {
    pub(crate) fn new() -> Self {
        Blame { stamp: None, request: 0, latest: Arc::new(AtomicU64::new(0)), root: None, lines: Vec::new(), extent: None, done: false }
    }

    /// Whether this is the blame of `stamp`'s content (complete or still arriving).
    pub(crate) fn of(&self, stamp: Option<Stamp>) -> bool {
        self.request != 0 && self.stamp == stamp
    }

    /// Start over for `file` as of `stamp` with `lines` lines: request `request` streams to `tx` for `tab`.
    pub(crate) fn start(&mut self, tab: TabId, file: PathBuf, stamp: Option<Stamp>, lines: usize, request: u64, tx: async_channel::Sender<Completion>) {
        // Field by field: replacing the whole value would drop it, which stops this very request.
        self.latest.store(request, Ordering::Relaxed);
        (self.stamp, self.request, self.root, self.lines, self.extent, self.done) = (stamp, request, None, vec![None; lines], None, false);
        let latest = self.latest.clone();
        submit(Box::new(move || run(tab, &file, request, &latest, &tx)));
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

/// Batches go out at this many lines or this long after the last, so a big file fills in as git goes without
/// flooding the session's channel.
const BATCH_LINES: usize = 256;
const BATCH_AFTER: Duration = Duration::from_millis(50);

fn run(tab: TabId, file: &std::path::Path, request: u64, latest: &AtomicU64, tx: &async_channel::Sender<Completion>) {
    let current = || latest.load(Ordering::Relaxed) == request;
    if !current() {
        return;
    }
    let send = |root, hunks, done| {
        let _ = tx.send_blocking(Completion::Blamed { tab, request, root, hunks, done });
    };
    let (root, path) = match git_blame::repository(file) {
        Ok(found) => found,
        Err(e) => return send(None, Vec::new(), Some(Err(e))),
    };
    let dir = root.clone();
    // Sent once, with the first batch.
    let mut root = Some(root);
    let (mut batch, mut lines, mut since) = (Vec::new(), 0, Instant::now());
    let result = git_blame::blame_worktree(&dir, &path, &current, |h| {
        lines += h.lines.len();
        batch.push((h.lines, h.commit));
        if lines >= BATCH_LINES || since.elapsed() >= BATCH_AFTER {
            send(root.take(), std::mem::take(&mut batch), None);
            (lines, since) = (0, Instant::now());
        }
    });
    if current() {
        send(root, batch, Some(result));
    }
}

/// The blame worker: one thread, started on first use, running jobs in order.
fn submit(job: Box<dyn FnOnce() + Send>) {
    static WORKER: OnceLock<Option<mpsc::Sender<Box<dyn FnOnce() + Send>>>> = OnceLock::new();
    let worker = WORKER.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Box<dyn FnOnce() + Send>>();
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
            None => return format!("Loading commit {}…", &self.sha[..self.sha.len().min(7)]),
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
