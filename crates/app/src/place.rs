//! Where a repository was left: the log's selected commit (or local changes), the file and the line. Restored for
//! plain `lenz` and `lenz log`.
use std::{io::{Read, Write}, path::{Path, PathBuf}, sync::mpsc, time::Duration};

use serde::{Deserialize, Serialize};

pub const VERSION: u32 = 1;
const MAX_BYTES: u64 = 64 << 10;
const DELAY: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Place { pub version: u32, pub root: PathBuf, pub source: Source, pub file: Option<String>, pub line: Option<u32> }

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Source { Local, Log { commit: String } }

/// FNV-1a 64: stable across processes and versions (std's hasher is not).
fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3))
}

pub fn path_in(state_home: &Path, root: &Path) -> PathBuf {
    state_home.join("lenz/places").join(format!("{:016x}.json", fnv(root.as_os_str().as_encoded_bytes())))
}

/// `$XDG_STATE_HOME`, else `~/.local/state`.
pub(crate) fn state_home() -> Option<PathBuf> {
    std::env::var_os("XDG_STATE_HOME").map(PathBuf::from).filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from).filter(|p| p.is_absolute()).map(|h| h.join(".local/state")))
}

/// The repository root a place file records, whatever its other fields.
pub(crate) fn root_of(file: &Path) -> Option<PathBuf> {
    let mut bytes = Vec::new();
    std::fs::File::open(file).ok()?.take(MAX_BYTES + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > MAX_BYTES { return None; }
    serde_json::from_slice::<Place>(&bytes).ok().filter(|p| p.version == VERSION).map(|p| p.root)
}

pub fn load_from(file: &Path, root: &Path) -> Option<Place> {
    let mut bytes = Vec::new();
    std::fs::File::open(file).ok()?.take(MAX_BYTES + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > MAX_BYTES {
        return None;
    }
    serde_json::from_slice::<Place>(&bytes).ok().filter(|p| p.version == VERSION && p.root == root)
}

pub fn save_to(file: &Path, place: &Place) -> std::io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    let dir = file.parent().ok_or_else(|| std::io::Error::other("place file has no directory"))?;
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    let mut tmp = tempfile::Builder::new().prefix(".place-").tempfile_in(dir)?;
    tmp.as_file().set_permissions(std::fs::Permissions::from_mode(0o600))?;
    tmp.write_all(&serde_json::to_vec(place).map_err(std::io::Error::other)?)?;
    tmp.as_file().sync_all()?;
    tmp.persist(file).map_err(|e| e.error)?;
    Ok(())
}

/// What a restored launch selects once its comparisons are loaded.
#[derive(Debug, Clone, PartialEq)]
pub struct Restore { pub commit: Option<String>, pub file: Option<String>, pub line: Option<u32> }

/// What a plain repository launch restores and where it records: `None` when the launch is not eligible, restoring
/// is switched off, it is not in a repository, or a merge, rebase, cherry-pick, revert or stash pop is in progress
/// (its Conflict List opens, and the place from before it is kept).
#[derive(Debug, Clone, PartialEq)]
pub struct Launch { pub restore: Option<Restore>, pub file: PathBuf, pub root: PathBuf }

/// [`for_launch_in`] with the settings file and the user's state directory.
pub fn for_launch(request: &mut crate::cli::Request, cwd: &Path) -> Option<Launch> {
    let enabled = crate::settings::get().session.restore_place;
    if !enabled || !request.restores_place() {
        return None;
    }
    for_launch_in(request, cwd, enabled, &state_home()?)
}

/// The launch decision with `enabled` and `state_home` given (tests use their own).
pub fn for_launch_in(request: &mut crate::cli::Request, cwd: &Path, enabled: bool, state_home: &Path) -> Option<Launch> {
    if !enabled || !request.restores_place() {
        return None;
    }
    let root = crate::changeset::toplevel(cwd).ok()?;
    // unmerged index entries say an operation is in progress, without the working-tree scan of `git status`
    if crate::git::run(&root, &["ls-files", "-u", "-z"]).map_or(true, |out| !out.is_empty()) {
        return None;
    }
    let file = path_in(state_home, &root);
    let restore = restore_from(request, &root, &file);
    Some(Launch { restore, file, root })
}

/// The saved place in `file` for `request` in `root`, checked against the repository as it is now.
pub fn restore_from(request: &mut crate::cli::Request, root: &Path, file: &Path) -> Option<Restore> {
    if !request.restores_place() {
        return None;
    }
    let place = load_from(file, root)?;
    let commit = match place.source {
        Source::Log { commit } => {
            let id = commit.bytes().all(|b| b.is_ascii_hexdigit()) && matches!(commit.len(), 40 | 64);
            if !id || !crate::git::is_commit(root, &commit) {
                return None;
            }
            if matches!(request.launch, crate::cli::Launch::Diff) {
                let (options, scroll, blame, markdown) = (request.options, request.scroll, request.blame, request.markdown);
                *request = crate::cli::parse_with(&["log".into()], root, &Default::default()).ok()?;
                (request.options, request.scroll, request.blame, request.markdown) = (options, scroll, blame, markdown);
            }
            Some(commit)
        }
        Source::Local if matches!(request.launch, crate::cli::Launch::Diff) => None,
        Source::Local => return None, // `lenz log` restores only a log place
    };
    Some(Restore { commit, file: place.file, line: place.line })
}

/// Writes the latest place `delay` after the last change, off the caller's thread; the pending one on drop.
pub struct Recorder { last: Option<Place>, tx: Option<mpsc::Sender<Place>>, worker: Option<std::thread::JoinHandle<()>> }

impl Recorder {
    pub fn new(file: PathBuf) -> Recorder { Self::with_delay(file, DELAY) }

    pub fn with_delay(file: PathBuf, delay: Duration) -> Recorder {
        let (tx, rx) = mpsc::channel::<Place>();
        let worker = std::thread::Builder::new().name("lenz-place".into()).spawn(move || {
            let mut pending: Option<Place> = None;
            loop {
                let next = if pending.is_some() { rx.recv_timeout(delay) } else { rx.recv().map_err(|_| mpsc::RecvTimeoutError::Disconnected) };
                match next {
                    Ok(place) => pending = Some(place),
                    Err(mpsc::RecvTimeoutError::Timeout) => { if let Some(p) = pending.take() { let _ = save_to(&file, &p); } }
                    Err(mpsc::RecvTimeoutError::Disconnected) => { if let Some(p) = pending.take() { let _ = save_to(&file, &p); } break; }
                }
            }
        }).ok();
        Recorder { last: None, tx: worker.is_some().then_some(tx), worker }
    }

    /// The latest place handed to [`Recorder::record`].
    #[cfg(any(test, feature = "gui-test"))]
    pub fn last(&self) -> Option<&Place> { self.last.as_ref() }

    pub fn record(&mut self, place: Place) {
        if self.last.as_ref() == Some(&place) { return; }
        if let Some(tx) = &self.tx { let _ = tx.send(place.clone()); }
        self.last = Some(place);
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        drop(self.tx.take()); // the worker writes what is pending and stops
        if let Some(worker) = self.worker.take() { let _ = worker.join(); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn place(root: &Path) -> Place {
        Place { version: VERSION, root: root.into(), source: Source::Log { commit: "a".repeat(40) }, file: Some("src/x.rs".into()), line: Some(42) }
    }

    #[test]
    fn restore_follows_the_spec_rules() {
        use crate::git::testing::{commit_all, git, repo, write};
        let r = repo();
        write(r.path(), "a.txt", "1\n"); commit_all(r.path(), "one");
        let first = git(r.path(), &["rev-parse", "HEAD"]).trim().to_string();
        write(r.path(), "a.txt", "2\n"); commit_all(r.path(), "two");
        let state = tempfile::tempdir().unwrap();
        let file = path_in(state.path(), r.path());
        let saved = |source| save_to(&file, &Place { version: VERSION, root: r.path().into(), source, file: Some("a.txt".into()), line: Some(1) }).unwrap();
        let launch = |a: &[&str]| crate::cli::parse_with(&a.iter().map(|s| s.to_string()).collect::<Vec<_>>(), r.path(), &Default::default()).unwrap();

        // eligible launches only
        saved(Source::Log { commit: first.clone() });
        for explicit in [&["HEAD~1"][..], &["--staged"], &["log", "HEAD"], &["--", "a.txt"]] {
            assert!(restore_from(&mut launch(explicit), r.path(), &file).is_none(), "{explicit:?}");
        }
        // plain `lenz` with a log place becomes `lenz log` and selects the commit
        let mut req = launch(&[]);
        let got = restore_from(&mut req, r.path(), &file).unwrap();
        assert_eq!((got.commit.as_deref(), got.file.as_deref(), got.line), (Some(first.as_str()), Some("a.txt"), Some(1)));
        assert!(matches!(req.launch, crate::cli::Launch::Log { follow: None }));
        // a commit that no longer exists: the normal start
        saved(Source::Log { commit: "f".repeat(40) });
        let mut req = launch(&[]);
        assert!(restore_from(&mut req, r.path(), &file).is_none());
        assert!(matches!(req.launch, crate::cli::Launch::Diff));
        // local changes keep the launch and restore file and line
        saved(Source::Local);
        let mut req = launch(&[]);
        let got = restore_from(&mut req, r.path(), &file).unwrap();
        assert_eq!((got.commit, got.file.as_deref()), (None, Some("a.txt")));
    }

    #[test]
    fn launches_record_only_when_enabled_eligible_and_no_operation_is_in_progress() {
        use crate::git::testing::{commit_all, git, repo, write};
        let r = repo();
        write(r.path(), "a.txt", "1\n"); commit_all(r.path(), "one");
        let state = tempfile::tempdir().unwrap();
        let launch = |a: &[&str]| crate::cli::parse_with(&a.iter().map(|s| s.to_string()).collect::<Vec<_>>(), r.path(), &Default::default()).unwrap();
        // switched off: nothing read, nothing to record
        assert!(for_launch_in(&mut launch(&[]), r.path(), false, state.path()).is_none());
        // ineligible: nothing either
        assert!(for_launch_in(&mut launch(&["HEAD"]), r.path(), true, state.path()).is_none());
        // eligible: the repository's place file, nothing to restore yet
        let got = for_launch_in(&mut launch(&[]), r.path(), true, state.path()).unwrap();
        assert_eq!((got.file, got.restore), (path_in(state.path(), &got.root), None));
        // a merge in progress: neither restored nor recorded, so the place from before the merge survives
        git(r.path(), &["checkout", "-qb", "topic"]); write(r.path(), "a.txt", "2\n"); commit_all(r.path(), "two");
        git(r.path(), &["checkout", "-q", "main"]); write(r.path(), "a.txt", "3\n"); commit_all(r.path(), "three");
        assert!(!crate::git::testing::try_git(r.path(), &["merge", "topic"]));
        assert!(for_launch_in(&mut launch(&[]), r.path(), true, state.path()).is_none());
    }

    #[test]
    fn a_place_round_trips_privately() {
        let dir = tempfile::tempdir().unwrap();
        let root = Path::new("/repo");
        let file = path_in(dir.path(), root);
        save_to(&file, &place(root)).unwrap();
        assert_eq!(load_from(&file, root), Some(place(root)));
        assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(std::fs::metadata(file.parent().unwrap()).unwrap().permissions().mode() & 0o777, 0o700);
        assert_eq!(file.file_name().unwrap().len(), 16 + ".json".len());
    }

    #[test]
    fn bad_files_are_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let root = Path::new("/repo");
        let file = path_in(dir.path(), root);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        for bad in ["", "{", "{\"version\":99}", "null", &"x".repeat(70_000)] {
            std::fs::write(&file, bad).unwrap();
            assert_eq!(load_from(&file, root), None, "{bad:.20}");
        }
        save_to(&file, &place(Path::new("/other"))).unwrap();
        assert_eq!(load_from(&file, root), None, "another root's place");
        assert_eq!(load_from(&dir.path().join("missing.json"), root), None);
    }

    #[test]
    fn the_recorder_writes_after_a_pause_and_on_drop_only_when_changed() {
        let dir = tempfile::tempdir().unwrap();
        let root = Path::new("/repo");
        let file = path_in(dir.path(), root);
        let mut r = Recorder::with_delay(file.clone(), std::time::Duration::from_millis(50));
        r.record(place(root));
        assert!(!file.exists(), "not before the pause");
        std::thread::sleep(std::time::Duration::from_millis(400));
        assert_eq!(load_from(&file, root), Some(place(root)));
        let moved = Place { line: Some(7), ..place(root) };
        r.record(moved.clone());
        drop(r);
        assert_eq!(load_from(&file, root), Some(moved), "drop writes the pending place");
    }
}
