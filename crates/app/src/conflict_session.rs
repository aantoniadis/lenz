//! Shared Conflict List inventory, grouping, navigation and staging safety.
use crate::{
    changeset::{ChangeSet, Comparison, Content as Source, Status},
    conflicts::{self, ConflictedFile, InProgress, Operation},
};
use std::{
    collections::HashMap,
    path::{Component, Path},
};
/// Where a Conflicted File stands; the list groups them in this order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Conflicts left in a text file (`None` for a file resolved whole: binary, deleted, symlink, submodule).
    Unresolved(Option<usize>),
    /// Saved without conflict markers, ready to mark.
    Clean,
    /// Marked resolved: staged.
    Marked,
}

impl State {
    pub fn group(self) -> usize {
        match self {
            State::Unresolved(_) => 0,
            State::Clean => 1,
            State::Marked => 2,
        }
    }
}

/// Live Merge View choices override the last disk inventory for a text-file row.
/// A resolved but unsaved Result stays in Unresolved; saving makes it Clean.
pub fn displayed_state(
    inventory: State,
    opened: Option<&crate::merge_session::MergeSession>,
) -> State {
    match (inventory, opened) {
        (State::Unresolved(Some(_)) | State::Clean, Some(m)) if m.dirty || m.saved => {
            match m.m.unresolved() {
                0 if !m.dirty => State::Clean,
                n => State::Unresolved(Some(n)),
            }
        }
        (state, _) => state,
    }
}

const GROUPS: [&str; 3] = ["Unresolved", "Saved clean", "Marked resolved"];

/// What git says about the operation in progress, read off the UI thread.
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub op: InProgress,
    /// Still unmerged.
    pub files: Vec<ConflictedFile>,
    /// Marked resolved during the operation.
    pub marked: Vec<String>,
    /// Conflicts in each text file on disk (its `<<<<<<<` markers).
    pub on_disk: HashMap<String, usize>,
}

impl Snapshot {
    pub fn is_empty(&self) -> bool {
        self.files.is_empty() && self.marked.is_empty()
    }

    pub fn file(&self, path: &str) -> Option<&ConflictedFile> {
        self.files.iter().find(|f| f.path == path)
    }
}

/// The operation in progress in the repository at `root` and its Conflicted Files under `filter` (top-level
/// paths or directories; all when empty); `None` when nothing is in progress.
pub fn read(root: &Path, filter: &[String]) -> Result<Option<Snapshot>, String> {
    let Some(op) = conflicts::in_progress(root)? else {
        return Ok(None);
    };
    let keep = |p: &str| {
        filter.is_empty()
            || filter
                .iter()
                .any(|f| f.is_empty() || p == f || p.starts_with(&format!("{f}/")))
    };
    let mut files = conflicts::list(root)?;
    files.retain(|f| keep(&f.path));
    let mut marked = conflicts::marked(root)?;
    marked.retain(|p| keep(p));
    let on_disk = files
        .iter()
        .filter(|f| merges_as_text(f))
        .map(|f| {
            let size = conflicts::marker_size(root, &f.path);
            (
                f.path.clone(),
                std::fs::read(root.join(&f.path)).map_or(0, |b| conflict_count(&b, size)),
            )
        })
        .collect();
    Ok(Some(Snapshot {
        op,
        files,
        marked,
        on_disk,
    }))
}

/// `lenz merge <path>…` as typed in `cwd`, from the top level of the repository at `root`.
pub fn from_top(root: &Path, cwd: &Path, paths: &[String]) -> Vec<String> {
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let (root, cwd) = (canon(root), canon(cwd));
    paths
        .iter()
        .map(|p| {
            let full = cwd.join(p);
            let mut parts: Vec<String> = Vec::new();
            for c in full
                .strip_prefix(&root)
                .unwrap_or(Path::new(p))
                .components()
            {
                match c {
                    Component::Normal(s) => parts.push(s.to_string_lossy().into_owned()),
                    Component::ParentDir => _ = parts.pop(),
                    _ => {}
                }
            }
            parts.join("/")
        })
        .collect()
}

/// Opens in the Merge View (the rest in the Version Picker).
pub fn merges_as_text(f: &ConflictedFile) -> bool {
    matches!(f.kind, conflicts::Kind::Text | conflicts::Kind::AddedByBoth)
}

/// How many Conflicts git's markers open in `bytes`: lines of exactly `size` `<` and then whitespace.
pub fn conflict_count(bytes: &[u8], size: usize) -> usize {
    bytes
        .split_inclusive(|&b| b == b'\n')
        .filter(|l| l[0] == b'<' && conflicts::marker_lines(l, size) == 1)
        .count()
}

/// Each file's State, grouped (unresolved, saved clean, marked) and by path within a group. `unsaved`: the
/// Conflicts left in files open in a Merge View with unsaved changes, which count over the file on disk.
pub fn states(s: &Snapshot, unsaved: &HashMap<String, usize>) -> Vec<(String, State)> {
    let mut out: Vec<(String, State)> = s
        .files
        .iter()
        .map(|f| {
            let state = match (unsaved.get(&f.path), s.on_disk.get(&f.path)) {
                _ if !merges_as_text(f) => State::Unresolved(None),
                (Some(&n), _) => State::Unresolved(Some(n)),
                (None, Some(&n)) if n > 0 => State::Unresolved(Some(n)),
                _ => State::Clean,
            };
            (f.path.clone(), state)
        })
        .chain(s.marked.iter().map(|p| (p.clone(), State::Marked)))
        .collect();
    out.sort_by(|a, b| a.1.group().cmp(&b.1.group()).then_with(|| a.0.cmp(&b.0)));
    out
}

/// The Conflict List as a Change Set, in groups: a text file against HEAD, as Local changes would show it; a file
/// resolved whole as the versions its Version Picker chooses between (Ours ⟷ Theirs, or the Base ⟷ the version that
/// survived a deletion).
pub fn change_set(
    root: &Path,
    states: &[(String, State)],
    snap: &Snapshot,
) -> Result<ChangeSet, String> {
    let comparison = |path: &str| {
        let file = root.join(path);
        let stage = |n: usize| match snap.file(path).and_then(|f| f.stages[n - 1].as_ref()) {
            Some(s) if s.mode != 0o160000 => Source::Blob(format!(":{n}:{path}")),
            _ => Source::Empty,
        };
        let (left, right) = match snap.file(path).map(|f| f.kind) {
            Some(conflicts::Kind::Binary | conflicts::Kind::Symlink) => (stage(2), stage(3)),
            Some(conflicts::Kind::DeletedBy(conflicts::Side::Us)) => (stage(1), stage(3)),
            Some(conflicts::Kind::DeletedBy(_)) => (stage(1), stage(2)),
            Some(conflicts::Kind::Submodule) => (Source::Empty, Source::Empty),
            _ => (
                Source::Blob(format!("HEAD:{path}")),
                if file.symlink_metadata().is_ok() {
                    Source::File(file)
                } else {
                    Source::Empty
                },
            ),
        };
        Comparison {
            old_path: path.into(),
            path: path.into(),
            status: Status::Conflicted,
            left,
            right,
        }
    };
    let lists = GROUPS
        .iter()
        .enumerate()
        .map(|(g, name)| {
            (
                name.to_string(),
                states
                    .iter()
                    .filter(|s| s.1.group() == g)
                    .map(|s| comparison(&s.0))
                    .collect(),
            )
        })
        .collect();
    ChangeSet::grouped(root.to_path_buf(), lists)
}

/// The sidebar entry's lines, short for its width: what is in progress; what's left or, once every file is
/// marked, the command that finishes it; and which side is ours or theirs where the first line doesn't say.
pub fn entry_text(op: &InProgress, left: usize, total: usize) -> Vec<String> {
    let step = op
        .step
        .map_or(String::new(), |(k, n)| format!(" ({k}/{n})"));
    let unnamed = |line: String| (line, None);
    let (what, sides) = match op.operation {
        Operation::Merge => unnamed(format!("Merging {} → {}", op.theirs, op.ours)),
        Operation::Rebase => (
            format!("Rebasing onto {}{step}", op.ours),
            Some(format!("theirs = {}", op.theirs)),
        ),
        Operation::Am => (
            format!("Applying patches{step}"),
            Some(format!("theirs = {}", op.theirs)),
        ),
        Operation::CherryPick => (
            format!("Cherry-picking {}", op.theirs),
            Some(format!("ours = {}", op.ours)),
        ),
        Operation::Revert => (
            format!(
                "Reverting {}",
                op.theirs.strip_prefix("parent of ").unwrap_or(&op.theirs)
            ),
            Some(format!("ours = {}", op.ours)),
        ),
        Operation::StashPop => unnamed("Applying a stash".to_string()),
    };
    let rest = if left == 0 {
        op.operation.continue_command().map_or_else(
            || "All resolved".to_string(),
            |c| format!("All resolved — run {c}"),
        )
    } else {
        format!("{left} of {total} left")
    };
    [what, rest].into_iter().chain(sides).collect()
}

/// The next (or previous) unresolved file after `from` in list order, wrapping around; never `from` itself.
pub fn next_unresolved(
    order: &[(&str, State)],
    from: Option<&str>,
    forward: bool,
) -> Option<usize> {
    let n = order.len();
    let at = from.and_then(|p| order.iter().position(|o| o.0 == p));
    (1..=n)
        .map(|k| match (at, forward) {
            (Some(i), true) => (i + k) % n,
            (Some(i), false) => (i + n - k % n) % n,
            (None, true) => k - 1,
            (None, false) => n - k,
        })
        .find(|&i| Some(order[i].0) != from && matches!(order[i].1, State::Unresolved(_)))
}

/// The kind tag of a Navigator row.
pub fn tag(f: &ConflictedFile) -> String {
    use conflicts::{Kind, Side};
    let kind = match f.kind {
        Kind::Text => "text",
        Kind::Binary => "binary",
        Kind::AddedByBoth => "added by both",
        Kind::DeletedBy(Side::Us) => "deleted by us",
        Kind::DeletedBy(Side::Them) => "deleted by them",
        Kind::DeletedBy(Side::Both) => "deleted by both",
        Kind::Symlink => "symlink",
        Kind::Submodule => "submodule",
    };
    if f.rerere {
        format!("{kind} · rerere")
    } else {
        kind.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conflicts::{Kind as FileKind, Side as Lost, Stage};

    fn file(path: &str, kind: FileKind) -> ConflictedFile {
        let stage = Some(Stage {
            mode: 0o100644,
            sha: "0".repeat(40),
            size: Some(1),
        });
        ConflictedFile {
            path: path.into(),
            xy: "UU".into(),
            stages: [stage.clone(), stage.clone(), stage],
            kind,
            rerere: false,
        }
    }

    fn op(operation: Operation, ours: &str, theirs: &str, step: Option<(u32, u32)>) -> InProgress {
        InProgress {
            operation,
            ours: ours.into(),
            theirs: theirs.into(),
            step,
        }
    }

    #[test]
    fn files_group_unresolved_then_saved_clean_then_marked() {
        let snap = Snapshot {
            op: op(Operation::Merge, "main", "feature", None),
            files: vec![
                file("b.txt", FileKind::Text),
                file("a.txt", FileKind::Text),
                file("img.png", FileKind::Binary),
                file("gone.txt", FileKind::DeletedBy(Lost::Them)),
                file("edited.txt", FileKind::Text),
            ],
            marked: vec!["done.txt".into()],
            on_disk: [("a.txt", 0), ("b.txt", 2), ("edited.txt", 0)]
                .into_iter()
                .map(|(p, n)| (p.to_string(), n))
                .collect(),
        };
        // edited.txt is clean on disk but open with unsaved changes and one Conflict left
        let unsaved = [("edited.txt".to_string(), 1)].into_iter().collect();
        let got = states(&snap, &unsaved);
        let want = [
            ("b.txt", State::Unresolved(Some(2))),
            ("edited.txt", State::Unresolved(Some(1))),
            ("gone.txt", State::Unresolved(None)),
            ("img.png", State::Unresolved(None)),
            ("a.txt", State::Clean),
            ("done.txt", State::Marked),
        ];
        assert_eq!(got, want.map(|(p, s)| (p.to_string(), s)));
    }

    #[test]
    fn the_entry_names_the_operation_and_what_is_left() {
        let merge = op(Operation::Merge, "main", "feature", None);
        assert_eq!(
            entry_text(&merge, 3, 5),
            ["Merging feature → main", "3 of 5 left"]
        );
        assert_eq!(
            entry_text(&merge, 0, 5)[1],
            "All resolved — run git merge --continue"
        );
        let rebase = op(
            Operation::Rebase,
            "main",
            "abc1234 (Fix parser)",
            Some((2, 7)),
        );
        assert_eq!(
            entry_text(&rebase, 1, 2),
            [
                "Rebasing onto main (2/7)",
                "1 of 2 left",
                "theirs = abc1234 (Fix parser)"
            ]
        );
        let revert = op(Operation::Revert, "main", "parent of abc1234 (Oops)", None);
        assert_eq!(
            entry_text(&revert, 0, 1),
            [
                "Reverting abc1234 (Oops)",
                "All resolved — run git revert --continue",
                "ours = main"
            ]
        );
        let stash = op(
            Operation::StashPop,
            "Updated upstream",
            "Stashed changes",
            None,
        );
        assert_eq!(
            entry_text(&stash, 0, 1),
            ["Applying a stash", "All resolved"]
        );
        assert_eq!(
            entry_text(
                &op(Operation::CherryPick, "main", "abc1234 (x)", None),
                1,
                1
            ),
            ["Cherry-picking abc1234 (x)", "1 of 1 left", "ours = main"]
        );
    }

    #[test]
    fn next_unresolved_skips_resolved_files_and_wraps() {
        use State::*;
        let order = [
            ("a", Unresolved(Some(1))),
            ("b", Clean),
            ("c", Unresolved(None)),
            ("d", Marked),
        ];
        assert_eq!(next_unresolved(&order, Some("a"), true), Some(2));
        assert_eq!(next_unresolved(&order, Some("c"), true), Some(0));
        assert_eq!(next_unresolved(&order, Some("a"), false), Some(2));
        assert_eq!(next_unresolved(&order, Some("d"), false), Some(2));
        assert_eq!(next_unresolved(&order, None, true), Some(0));
        // never the file itself
        assert_eq!(
            next_unresolved(
                &[("a", Unresolved(Some(1))), ("b", Marked)],
                Some("a"),
                true
            ),
            None
        );
        assert_eq!(next_unresolved(&[], None, true), None);
    }

    #[test]
    fn conflicts_are_counted_by_their_opening_markers() {
        let text = b"a\n<<<<<<< ours\nx\n=======\ny\n>>>>>>> theirs\n<<<<<<<< not\n<<<<<<< b\n|||||||\n=======\n>>>>>>>\n";
        assert_eq!(conflict_count(text, 7), 2);
        assert_eq!(conflict_count(b"", 7), 0);
    }

    #[test]
    fn paths_are_taken_from_the_top_level() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("src/ui")).unwrap();
        let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            from_top(
                root,
                &root.join("src"),
                &args(&["ui", "./main.rs", "../README.md", "."])
            ),
            args(&["src/ui", "src/main.rs", "README.md", "src"])
        );
        assert_eq!(from_top(root, root, &args(&["."])), args(&[""]));
    }
}

pub enum MarkOutcome {
    Marked,
    Confirm(String),
    Notice(String),
}
/// Stage the saved disk entry; unsaved choices and surviving markers share the GUI's checks.
pub fn mark(
    root: &Path,
    snap: &Snapshot,
    path: &str,
    dirty: bool,
    confirmed: bool,
) -> Result<MarkOutcome, String> {
    if dirty {
        return Ok(MarkOutcome::Notice(format!(
            "Unsaved changes in {path} — save them first (ctrl-s): Mark resolved stages the file on disk"
        )));
    }
    let Some(file) = snap.file(path) else {
        return Ok(MarkOutcome::Notice(format!(
            "{path} is already marked resolved"
        )));
    };
    let disk = root.join(path);
    let exists = disk.symlink_metadata().is_ok();
    if merges_as_text(file) && exists && !confirmed {
        let size = conflicts::marker_size(root, path);
        let n = std::fs::read(&disk).map_or(0, |b| conflicts::marker_lines(&b, size));
        if n > 0 {
            return Ok(MarkOutcome::Confirm(format!(
                "{n} conflict marker{} remain in {path} — mark resolved anyway? Mark resolved again: yes · Esc: no",
                if n == 1 { "" } else { "s" }
            )));
        }
    }
    conflicts::mark_resolved(root, path, !exists)?;
    Ok(MarkOutcome::Marked)
}
pub fn reopen(root: &Path, path: &str) -> Result<(), String> {
    conflicts::reopen(root, path, conflicts::conflict_style(root))
}

use crate::{
    merge_session::{self, Act, MergeSession},
    version_session::{self, PickAct, PickerState},
};
use std::{
    path::PathBuf,
    sync::{Arc, mpsc},
};

pub enum FileView {
    Merge(Box<MergeSession>),
    Picker(Box<PickerState>),
    Marked { path: String, text: String },
}
impl FileView {
    pub fn path(&self) -> &str {
        match self {
            Self::Merge(m) => &m.path,
            Self::Picker(p) => &p.o.path,
            Self::Marked { path, .. } => path,
        }
    }
    pub fn dirty(&self) -> bool {
        match self {
            Self::Merge(m) => m.dirty,
            Self::Picker(p) => p.dirty,
            Self::Marked { .. } => false,
        }
    }
}
enum Loaded {
    File(FileView),
    List {
        root: PathBuf,
        filter: Vec<String>,
        snap: Option<Snapshot>,
    },
    Refresh(Option<Snapshot>),
}
/// Owns merge file work and retained choices independently of either frontend.
pub struct MergeWorkspace {
    pub file: Option<FileView>,
    pub root: Option<PathBuf>,
    pub snapshot: Option<Snapshot>,
    pub rows: Vec<(String, State)>,
    pub current: usize,
    pub notice: Option<String>,
    pub quit: bool,
    pub exit_code: u8,
    filter: Vec<String>,
    retained: HashMap<String, Box<MergeSession>>,
    confirm: Option<String>,
    close_asked: bool,
    token: Arc<()>,
    pending: Option<mpsc::Receiver<Option<Result<Loaded, String>>>>,
}
impl MergeWorkspace {
    fn empty() -> Self {
        Self {
            file: None,
            root: None,
            snapshot: None,
            rows: Vec::new(),
            current: 0,
            notice: None,
            quit: false,
            exit_code: 0,
            filter: Vec::new(),
            retained: HashMap::new(),
            confirm: None,
            close_asked: false,
            token: Arc::new(()),
            pending: None,
        }
    }
    pub fn open(request: crate::cli::Request, cwd: PathBuf) -> Self {
        let mut s = Self::empty();
        match request.launch {
            crate::cli::Launch::Merge(crate::cli::Merge::Files {
                base,
                ours,
                theirs,
                merged,
            }) => {
                s.exit_code = 1;
                s.submit(move || {
                    if let Some(o) =
                        version_session::open_files(&base, &ours, &theirs, &merged, &cwd)?
                    {
                        Ok(Loaded::File(FileView::Picker(Box::new(PickerState::new(
                            o, true,
                        )))))
                    } else {
                        let mut m = MergeSession::new(
                            merge_session::open_files(&base, &ours, &theirs, &merged, &cwd)?,
                            30.0,
                        );
                        m.mergetool = true;
                        Ok(Loaded::File(FileView::Merge(Box::new(m))))
                    }
                });
            }
            crate::cli::Launch::Merge(crate::cli::Merge::List(paths)) => s.submit(move || {
                let root = crate::changeset::toplevel(&cwd)?;
                let filter = from_top(&root, &cwd, &paths);
                let snap = read(&root, &filter)?;
                Ok(Loaded::List { root, filter, snap })
            }),
            _ => {
                s.notice = Some("Expected a merge request".into());
                s.exit_code = 2;
            }
        }
        s
    }
    pub fn from_repository(root: PathBuf, paths: Vec<String>) -> Self {
        let mut s = Self::empty();
        s.submit(move || {
            let snap = read(&root, &paths)?;
            Ok(Loaded::List {
                root,
                filter: paths,
                snap,
            })
        });
        s
    }
    fn submit(&mut self, job: impl FnOnce() -> Result<Loaded, String> + Send + 'static) {
        self.token = Arc::new(());
        self.pending = Some(crate::work::submit(Arc::downgrade(&self.token), job));
    }
    pub fn busy(&self) -> bool {
        self.pending.is_some()
            || self
                .file
                .as_ref()
                .is_some_and(|f| matches!(f,FileView::Merge(m) if m.syntax_busy()))
    }
    pub fn poll(&mut self) -> bool {
        let syntax_changed = if let Some(FileView::Merge(m)) = &mut self.file {
            m.poll_syntax()
        } else {
            false
        };
        let Some(rx) = &self.pending else {
            return syntax_changed;
        };
        let loaded = match rx.try_recv() {
            Ok(r) => r,
            Err(mpsc::TryRecvError::Empty) => return syntax_changed,
            Err(_) => Some(Err("Merge worker stopped".into())),
        };
        self.pending = None;
        match loaded {
            Some(Ok(Loaded::File(f))) => self.file = Some(f),
            Some(Ok(Loaded::List { root, filter, snap })) => {
                self.root = Some(root);
                self.filter = filter;
                self.apply_snapshot(snap);
                self.open_index(self.current);
            }
            Some(Ok(Loaded::Refresh(snap))) => {
                let path = self.file.as_ref().map(|f| f.path().to_owned());
                let stale = path.as_deref().is_some_and(|p| {
                    self.snapshot.as_ref().and_then(|s| s.file(p))
                        != snap.as_ref().and_then(|s| s.file(p))
                });
                if stale {
                    self.file = None;
                }
                self.apply_snapshot(snap);
                if let Some(path) = path {
                    self.current = self
                        .rows
                        .iter()
                        .position(|r| r.0 == path)
                        .unwrap_or(self.current.min(self.rows.len().saturating_sub(1)));
                }
                if self.file.is_none() {
                    self.open_index(self.current);
                }
            }
            Some(Err(e)) => {
                self.notice = Some(e);
                if self.root.is_none() {
                    self.exit_code = 2;
                }
            }
            None => {}
        }
        true
    }
    fn apply_snapshot(&mut self, snap: Option<Snapshot>) {
        let unsaved = self
            .retained
            .values()
            .filter(|m| m.dirty)
            .map(|m| (m.path.clone(), m.m.unresolved()))
            .chain(self.file.iter().filter_map(|f| {
                if let FileView::Merge(m) = f {
                    m.dirty.then(|| (m.path.clone(), m.m.unresolved()))
                } else {
                    None
                }
            }))
            .collect();
        if let Some(next) = &snap {
            self.retained.retain(|p, _| {
                next.file(p).is_some()
                    && self.snapshot.as_ref().and_then(|s| s.file(p)) == next.file(p)
            });
            self.rows = states(next, &unsaved);
        } else {
            self.rows.clear();
            self.file = None;
            self.notice = Some("No operation in progress — merge finished".into());
        }
        self.snapshot = snap;
    }
    pub fn row_state(&self, path: &str, inventory: State) -> State {
        let active = match &self.file {
            Some(FileView::Merge(m)) if m.path == path => Some(m.as_ref()),
            _ => None,
        };
        displayed_state(
            inventory,
            active.or_else(|| self.retained.get(path).map(Box::as_ref)),
        )
    }
    pub fn refresh(&mut self) {
        let Some(root) = self.root.clone() else {
            return;
        };
        let filter = self.filter.clone();
        self.submit(move || read(&root, &filter).map(Loaded::Refresh));
    }
    pub fn confirming(&self)->bool{self.confirm.is_some()}
    pub fn open_index(&mut self, index: usize) {
        let Some((path, _)) = self.rows.get(index).cloned() else {
            return;
        };
        if self.file.as_ref().is_some_and(|f| f.path() == path) {
            return;
        }
        self.current = index;
        self.confirm = None;
        if let Some(FileView::Merge(mut m)) = self.file.take() {
            m.release_syntax();
            self.retained.insert(m.path.clone(), m);
        }
        if let Some(m) = self.retained.remove(&path) {
            self.file = Some(FileView::Merge(m));
            return;
        }
        let Some(root) = self.root.clone() else {
            return;
        };
        let file = self.snapshot.as_ref().and_then(|s| s.file(&path)).cloned();
        self.submit(move || {
            let opened = match file {
                Some(f) => match open_file(&root, &f)? {
                    Opening::Merge(o) => FileView::Merge(Box::new(MergeSession::new(*o, 30.0))),
                    Opening::Picker(o) => FileView::Picker(Box::new(PickerState::new(*o, false))),
                },
                None => FileView::Marked {
                    text: String::new(),
                    path,
                },
            };
            Ok(Loaded::File(opened))
        });
    }
    pub fn switch_file(&mut self, forward: bool, unresolved: bool) {
        if self.rows.is_empty() {
            return;
        }
        let index = if unresolved {
            let order: Vec<_> = self.rows.iter().map(|(p, s)| (p.as_str(), *s)).collect();
            next_unresolved(&order, self.file.as_ref().map(|f| f.path()), forward)
        } else {
            Some(if forward {
                (self.current + 1) % self.rows.len()
            } else {
                (self.current + self.rows.len() - 1) % self.rows.len()
            })
        };
        if let Some(i) = index {
            self.open_index(i);
        }
    }
    pub fn execute(&mut self, action: Act) {
        let Some(FileView::Merge(m)) = &mut self.file else {
            return;
        };
        let out = m.execute(action);
        if let Some(t) = out.toast {
            self.notice = Some(t);
        }
        if let Some(code) = out.quit {
            self.exit_code = code;
            self.quit = true;
        }
        if out.mark {
            self.mark_current();
        }
        if let Some(f) = out.cross {
            self.switch_file(f, true);
        }
        if out.done {
            self.switch_file(true, true);
            self.refresh();
        }
        if out.back {
            self.cancel_file();
        }
    }
    pub fn pick(&mut self, action: PickAct) {
        let Some(FileView::Picker(p)) = &mut self.file else {
            return;
        };
        let out = p.execute(action);
        if let Some(t) = out.toast.or(out.error) {
            self.notice = Some(t);
        }
        if let Some(code) = out.quit {
            self.exit_code = code;
            self.quit = true;
        }
        if out.mark.is_some() {
            self.mark_current();
        }
        if out.done {
            self.switch_file(true, true);
            self.refresh();
        }
        if out.back {
            self.cancel_file();
        }
    }
    fn cancel_file(&mut self) {
        if let Some(file) = self.file.take() {
            let path = file.path().to_owned();
            self.retained.remove(&path);
            self.file = Some(FileView::Marked {
                path,
                text: String::new(),
            });
            self.notice = Some("Merge closed — o opens it again".into());
            self.refresh();
        }
    }
    pub fn open_current(&mut self) {
        if matches!(self.file, Some(FileView::Marked { .. })) {
            self.file = None;
        }
        self.open_index(self.current);
    }
    pub fn mark_current(&mut self) {
        let Some(file) = &self.file else { return };
        let path = file.path().to_owned();
        let dirty = file.dirty();
        let (Some(root), Some(snap)) = (&self.root, &self.snapshot) else {
            return;
        };
        match mark(
            root,
            snap,
            &path,
            dirty,
            self.confirm.as_deref() == Some(&path),
        ) {
            Ok(MarkOutcome::Confirm(message)) => {
                self.confirm = Some(path);
                self.notice = Some(message);
            }
            Ok(MarkOutcome::Notice(message)) => self.notice = Some(message),
            Ok(MarkOutcome::Marked) => {
                self.file = None;
                self.retained.remove(&path);
                self.confirm = None;
                self.notice = Some(format!("Marked {path} resolved"));
                self.refresh();
            }
            Err(e) => self.notice = Some(format!("Not marked resolved: {e}")),
        }
    }
    pub fn reopen_current(&mut self) {
        let Some((path, State::Marked)) = self.rows.get(self.current) else {
            return;
        };
        let Some(root) = &self.root else { return };
        match reopen(root, path) {
            Ok(()) => {
                // as the GUI's toast; the last Mark resolved notice is out of date
                self.notice = Some(format!("Re-opened {path}"));
                self.file = None;
                self.refresh();
            }
            Err(e) => self.notice = Some(format!("Not re-opened: {e}")),
        }
    }
    pub fn dismiss(&mut self) -> bool {
        if self.confirm.take().is_some() {
            self.notice = None;
            return true;
        }
        match &mut self.file {
            Some(FileView::Merge(m)) => m.dismiss(),
            Some(FileView::Picker(p)) => p.dismiss(),
            _ => false,
        }
    }
    pub fn close(&mut self) {
        let ready = match &mut self.file {
            Some(FileView::Merge(m)) => m.may_close(),
            Some(FileView::Picker(p)) => p.may_close(),
            _ => true,
        };
        if !ready {
            return;
        }
        if !self.close_asked && self.retained.values().any(|m| m.dirty) {
            self.close_asked = true;
            self.notice = Some("Unsaved Merge Views — quit again discards them".into());
            return;
        }
        self.token = Arc::new(());
        self.pending = None;
        self.quit = true;
    }
    pub fn merge_anyway(&mut self) {
        let Some(FileView::Picker(p)) = &self.file else {
            return;
        };
        let Some([base, ours, theirs, merged]) = p.o.files.clone().filter(|_| p.o.large_text)
        else {
            return;
        };
        let mergetool = p.mergetool;
        self.submit(move || {
            let cwd = std::env::current_dir().unwrap_or_default();
            let mut m = MergeSession::new(
                merge_session::open_files(&base, &ours, &theirs, &merged, &cwd)?,
                30.0,
            );
            m.mergetool = mergetool;
            Ok(Loaded::File(FileView::Merge(Box::new(m))))
        });
    }
}
impl MergeWorkspace {
    pub fn copyable_commands(&self) -> Vec<String> {
        let mut commands = Vec::new();
        if let Some(s) = &self.snapshot
            && s.files.is_empty()
            && let Some(c) = s.op.operation.continue_command()
        {
            commands.push(c.into());
        }
        if let Some(FileView::Picker(p)) = &self.file {
            for i in 0..p.o.info.len() {
                if let Some(c) = p.command(i) {
                    commands.push(c);
                }
            }
        }
        commands
    }
}

pub enum Opening {
    Merge(Box<merge_session::Opened>),
    Picker(Box<version_session::Opened>),
}
/// The same type decision and stage loading for either frontend's Conflict List.
pub fn open_file(root: &Path, file: &ConflictedFile) -> Result<Opening, String> {
    if merges_as_text(file) {
        merge_session::open_conflict(root, file).map(|o| Opening::Merge(Box::new(o)))
    } else {
        Ok(Opening::Picker(Box::new(version_session::open_conflict(
            root, file,
        ))))
    }
}
impl MergeWorkspace {
    /// The same comparison displayed underneath the GUI's Version Picker or a closed merge.
    pub fn comparison(
        &self,
        base: bool,
    ) -> Result<Option<(ChangeSet, crate::changeset::Spec)>, String> {
        let Some(file) = &self.file else {
            return Ok(None);
        };
        if let FileView::Picker(p) = file
            && let Some([b, o, t, _]) = &p.o.files
        {
            let spec = crate::changeset::Spec::Plain {
                left: if base { b.clone() } else { o.clone() },
                right: t.clone(),
            };
            let cwd = p.o.target.parent().unwrap_or(Path::new("."));
            return ChangeSet::load(&spec, cwd).map(|set| Some((set, spec)));
        }
        if !matches!(file, FileView::Picker(_) | FileView::Marked { .. }) {
            return Ok(None);
        }
        let (Some(root), Some(snap)) = (&self.root, &self.snapshot) else {
            return Ok(None);
        };
        let path = file.path().to_owned();
        let state = self
            .rows
            .iter()
            .find(|r| r.0 == path)
            .map_or(State::Marked, |r| r.1);
        let set = change_set(root, &[(path.clone(), state)], snap)?;
        let spec = crate::changeset::Spec::Git {
            cached: false,
            revs: vec!["HEAD".into()],
            paths: vec![path],
            untracked: false,
        };
        Ok(Some((set, spec)))
    }
}
/// Commands that operate on the Conflict List around a file view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListAct {
    MarkResolved,
    Reopen,
    Refresh,
    MergeAnyway,
    Close,
}
impl ListAct {
    pub const ALL: [Self; 5] = [
        Self::MarkResolved,
        Self::Reopen,
        Self::Refresh,
        Self::MergeAnyway,
        Self::Close,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::MarkResolved => "Mark resolved",
            Self::Reopen => "Reopen marked file",
            Self::Refresh => "Refresh Conflict List",
            Self::MergeAnyway => "Merge large text anyway",
            Self::Close => "Quit",
        }
    }
}
impl MergeWorkspace {
    pub fn list_disabled(&self, command: ListAct) -> Option<&'static str> {
        match command {
            ListAct::MarkResolved | ListAct::Reopen | ListAct::Refresh if self.root.is_none() => {
                Some("Only available in the Conflict List")
            }
            ListAct::MarkResolved | ListAct::Reopen if self.file.is_none() => {
                Some("Open a file first")
            }
            ListAct::MergeAnyway if !matches!(&self.file,Some(FileView::Picker(p)) if p.o.large_text && p.o.files.is_some()) => {
                Some("Only available for large text versions")
            }
            _ => None,
        }
    }
    pub fn list_command(&mut self, command: ListAct) {
        if let Some(reason) = self.list_disabled(command) {
            self.notice = Some(reason.into());
            return;
        }
        match command {
            ListAct::MarkResolved => self.mark_current(),
            ListAct::Reopen => self.reopen_current(),
            ListAct::Refresh => self.refresh(),
            ListAct::MergeAnyway => self.merge_anyway(),
            ListAct::Close => self.close(),
        }
    }
}
