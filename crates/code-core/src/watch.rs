//! Filesystem notifications, coalesced: a burst of events (an editor's save, an agent's rewrite, a checkout) becomes
//! one change set after a short quiet period. Renames that the platform reports as one event keep both paths, so a
//! move can be confirmed by file identity; nothing is guessed from names or content.

use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::mpsc,
    time::{Duration, Instant},
};

use notify::{
    EventKind, RecursiveMode, Watcher as _,
    event::{ModifyKind, RenameMode},
};

/// How long the watcher waits for a burst to settle.
pub const SETTLE: Duration = Duration::from_millis(50);
/// A continuous burst still reports at least this often.
pub const BURST_REPORT: Duration = Duration::from_secs(1);
/// Folders watched at most; beyond it the reader relies on rechecks (focus, Reload) and says so.
pub const MAX_WATCHED: usize = 8192;

/// One coalesced burst.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Change {
    pub paths: BTreeSet<PathBuf>,
    /// Newly registered folders: check for missed changes without treating every file as rewritten.
    pub registered: BTreeSet<PathBuf>,
    /// Renames reported with both ends, oldest first.
    pub moves: Vec<(PathBuf, PathBuf)>,
    /// The platform dropped events or failed: everything must be rechecked.
    pub overflow: bool,
    /// Something was created, removed or renamed (not only rewritten): the project inventory may differ.
    pub structure: bool,
}

impl Change {
    /// Whether `path` itself, or a directory above it, changed.
    pub fn touches(&self, path: &Path) -> bool {
        self.overflow || path.ancestors().any(|p| self.paths.contains(p) || self.registered.contains(p))
    }

    fn merge(&mut self, event: notify::Result<notify::Event>) {
        let event = match event {
            Ok(e) => e,
            Err(_) => return self.overflow = true,
        };
        if event.info() == Some("reader-watch-registration") {
            self.registered.extend(event.paths);
            return;
        }
        if event.need_rescan() {
            self.overflow = true;
        }
        if matches!(event.kind, EventKind::Access(_)) {
            return; // reading a file (the reader's own reads included) changes nothing
        }
        if matches!(event.kind, EventKind::Create(_) | EventKind::Remove(_) | EventKind::Modify(ModifyKind::Name(_)) | EventKind::Any | EventKind::Other) {
            self.structure = true;
        }
        if let (EventKind::Modify(ModifyKind::Name(RenameMode::Both)), [from, to]) = (&event.kind, event.paths.as_slice()) {
            self.moves.push((from.clone(), to.clone()));
        }
        self.paths.extend(event.paths);
    }
}

/// Folders to watch, and where to say the registration is done.
type Request = (BTreeSet<PathBuf>, Option<mpsc::Sender<()>>);

/// Watches a set of folders (each non-recursively); changes arrive coalesced through `send`. Registration runs on
/// the watcher's own thread, so a large set never blocks the caller.
pub struct Watcher {
    /// Requests for the registration thread: the folders to watch (latest wins), and where to say it is done.
    requests: Option<mpsc::Sender<Request>>,
    last: BTreeSet<PathBuf>,
}

impl Watcher {
    /// `send` gets each coalesced change and returns `false` once nobody listens. Without a platform watcher, the
    /// reader still rechecks on focus and Reload.
    pub fn new(send: impl Fn(Change) -> bool + Send + 'static) -> Self {
        let (tx, rx) = mpsc::channel();
        let missed = tx.clone();
        let Ok(mut inner) = notify::recommended_watcher(move |e| {
            let _ = tx.send(e);
        }) else {
            return Self { requests: None, last: BTreeSet::new() };
        };
        if std::thread::Builder::new().name("reader-watch".into()).spawn(move || coalesce(&rx, &send)).is_err() {
            return Self { requests: None, last: BTreeSet::new() };
        }
        let (requests, wanted) = mpsc::channel::<Request>();
        let registered = std::thread::Builder::new().name("reader-watch-set".into()).spawn(move || {
            let mut watched = BTreeSet::new();
            while let Ok((mut want, mut done)) = wanted.recv() {
                let mut waiting = Vec::new();
                while let Ok((newer, newer_done)) = wanted.try_recv() {
                    waiting.extend(done.take());
                    (want, done) = (newer, newer_done); // only the latest set matters
                }
                for p in watched.difference(&want) {
                    let _ = inner.unwatch(p);
                }
                let mut now = BTreeSet::new();
                let mut added = Vec::new();
                for p in want {
                    if watched.contains(&p) {
                        now.insert(p);
                    } else if inner.watch(&p, RecursiveMode::NonRecursive).is_ok() {
                        added.push(p.clone());
                        now.insert(p);
                    }
                }
                watched = now;
                // Check for changes between discovery and registration without claiming a directory rewrite.
                if !added.is_empty() {
                    let event = notify::Event::new(EventKind::Modify(ModifyKind::Any)).set_info("reader-watch-registration");
                    let _ = missed.send(Ok(added.into_iter().fold(event, notify::Event::add_path)));
                }
                for d in waiting.into_iter().chain(done) {
                    let _ = d.send(());
                }
            }
        });
        Self { requests: registered.ok().map(|_| requests), last: BTreeSet::new() }
    }

    pub fn available(&self) -> bool {
        self.requests.is_some()
    }

    /// Register a few immediately needed folders without dropping the inventory's existing watches.
    pub(crate) fn include(&mut self, wanted: BTreeSet<PathBuf>) -> bool {
        if wanted.is_subset(&self.last) { return true; }
        self.set(self.last.union(&wanted).cloned().collect())
    }

    /// Watch exactly `wanted` (at most [`MAX_WATCHED`]; returns whether all fit). Folders that cannot be watched now
    /// (missing) are retried with the next different set.
    pub fn set(&mut self, mut wanted: BTreeSet<PathBuf>) -> bool {
        let all = wanted.len() <= MAX_WATCHED;
        while wanted.len() > MAX_WATCHED {
            wanted.pop_last();
        }
        if wanted != self.last && let Some(r) = &self.requests {
            // A few new folders (an opened file's) are registered before returning, so a change right after opening
            // is not missed; a large inventory registers in the background.
            let few = wanted.difference(&self.last).count() <= 64;
            let (done, registered) = mpsc::channel();
            let _ = r.send((wanted.clone(), few.then_some(done)));
            if few {
                let _ = registered.recv_timeout(Duration::from_millis(200));
            }
            self.last = wanted;
        }
        all
    }
}

fn coalesce(rx: &mpsc::Receiver<notify::Result<notify::Event>>, send: &impl Fn(Change) -> bool) {
    while let Ok(first) = rx.recv() {
        let mut change = Change::default();
        change.merge(first);
        let started = Instant::now();
        let mut deadline = started + SETTLE;
        loop {
            match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(e) => {
                    change.merge(e);
                    // Quiet for SETTLE, or a second into a continuous burst, whichever comes first.
                    deadline = (Instant::now() + SETTLE).min(started + BURST_REPORT);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => break,
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
        }
        if (!change.paths.is_empty() || !change.registered.is_empty() || change.overflow) && !send(change) {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_does_not_hide_real_changes_in_the_same_burst() {
        let mut change = Change::default();
        change.merge(Ok(notify::Event::new(EventKind::Modify(ModifyKind::Any)).set_info("reader-watch-registration").add_path("/project/app".into())));
        change.merge(Ok(notify::Event::new(EventKind::Modify(ModifyKind::Data(notify::event::DataChange::Any))).add_path("/project/app/a.rb".into())));
        assert_eq!(change.registered, [PathBuf::from("/project/app")].into());
        assert_eq!(change.paths, [PathBuf::from("/project/app/a.rb")].into());
        assert!(change.touches(Path::new("/project/app/b.rb")));
        assert!(!change.touches(Path::new("/project/apple/b.rb")));
    }

    #[test]
    fn bursts_coalesce_and_renames_keep_both_ends() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().canonicalize().unwrap();
        let (tx, rx) = mpsc::channel();
        let mut w = Watcher::new(move |c| tx.send(c).is_ok());
        assert!(w.available());
        assert!(w.set([dir.clone()].into()));
        assert!(rx.recv_timeout(Duration::from_secs(5)).unwrap().registered.contains(&dir));
        let a = dir.join("a.txt");
        for i in 0..20 {
            std::fs::write(&a, format!("{i}")).unwrap();
        }
        let change = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(change.touches(&a));
        // Twenty writes arrive as one or a few changes, not twenty.
        let mut more = 0;
        while rx.recv_timeout(SETTLE * 4).is_ok() {
            more += 1;
        }
        assert!(more < 5, "{more} extra changes");
        std::fs::rename(&a, dir.join("b.txt")).unwrap();
        let change = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(change.touches(&a) && change.touches(&dir.join("b.txt")));
        if cfg!(target_os = "linux") {
            assert_eq!(change.moves, [(a.clone(), dir.join("b.txt"))]);
            assert!(change.structure);
        }
    }

    #[test]
    fn a_continuous_burst_still_reports_every_second() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().canonicalize().unwrap();
        let (tx, rx) = mpsc::channel();
        let mut w = Watcher::new(move |c| tx.send(c).is_ok());
        w.set([dir.clone()].into());
        assert!(rx.recv_timeout(Duration::from_secs(5)).unwrap().registered.contains(&dir));
        let writer = std::thread::spawn(move || {
            let until = Instant::now() + Duration::from_millis(2500);
            let mut i = 0;
            while Instant::now() < until {
                std::fs::write(dir.join("hot.txt"), format!("{i}")).unwrap();
                i += 1;
                std::thread::sleep(Duration::from_millis(10));
            }
        });
        let first = rx.recv_timeout(Duration::from_millis(1800)).expect("a report within the burst");
        assert!(!first.paths.is_empty());
        assert!(!first.structure || first.paths.len() <= 2, "rewrites are not structural beyond the create");
        writer.join().unwrap();
    }
}
