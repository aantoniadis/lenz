//! Contextual working-tree commands, with confirmation and guarded, reversible writes.
use crate::{
    changeset::{ChangeSet, Comparison, Content, Spec, Status},
    version_session::TargetSnapshot,
    write::{self, Entry},
};
use std::{
    collections::HashMap,
    os::unix::{ffi::OsStringExt, fs::MetadataExt},
    path::{Component, Path, PathBuf},
    sync::{
        Arc,
        mpsc::{Receiver, TryRecvError},
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileAction {
    CopyPath,
    Editor,
    History,
    UndoLenz,
    Discard,
    UndoDiscard,
}
impl FileAction {
    pub const ALL: &'static [Self] = &[
        Self::CopyPath,
        Self::Editor,
        Self::History,
        Self::UndoLenz,
        Self::Discard,
        Self::UndoDiscard,
    ];
    pub const COMMANDS: &'static [Self] = &[
        Self::History,
        Self::UndoLenz,
        Self::Discard,
        Self::UndoDiscard,
    ];
    pub const fn label(self) -> &'static str {
        match self {
            Self::CopyPath => "Copy file path",
            Self::Editor => "Open in editor",
            Self::History => "Show file history",
            Self::UndoLenz => "Undo last Lenz edit",
            Self::Discard => "Discard working-tree changes…",
            Self::UndoDiscard => "Undo Git discard",
        }
    }
}
struct Pending {
    root: PathBuf,
    relative: String,
    path: PathBuf,
    indexed: Vec<u8>,
    after: Entry,
    before: Option<Entry>,
    snapshot: TargetSnapshot,
}
struct Undo {
    root: PathBuf,
    relative: String,
    path: PathBuf,
    before: Option<Entry>,
    snapshot: TargetSnapshot,
}
enum ResultItem {
    Prepared(Pending, Comparison),
    Discarded(Undo, Comparison),
    Undone(PathBuf),
}
struct Record {
    undo: Arc<Undo>,
    comparison: Comparison,
}
/// One reversible discard per file. Captures only on explicit requests, never while rendering.
#[derive(Default)]
pub struct FileActions {
    token: Arc<()>,
    job: Option<Receiver<Option<Result<ResultItem, String>>>>,
    pending: Option<(Pending, Comparison)>,
    undo: HashMap<PathBuf, Record>,
}
impl FileActions {
    pub fn has_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn busy(&self) -> bool {
        self.job.is_some()
    }
    pub fn confirmation(&self) -> Option<String> {
        self.pending.as_ref().map(|(p,_)|format!("Discard working-tree changes in {}? Restore the indexed version, preserve staged changes. Undo Git discard will restore the current file.",p.relative))
    }
    pub fn pending_path(&self)->Option<&Path>{self.pending.as_ref().map(|(p,_)|p.path.as_path())}
    pub fn cancel(&mut self) {
        self.pending = None;
    }
    pub fn disabled(
        &self,
        action: FileAction,
        set: &ChangeSet,
        index: usize,
        spec: &Spec,
        can_undo_lenz: bool,
    ) -> Option<&'static str> {
        let Some(c) = set.comparisons.get(index) else {
            return Some("No file selected");
        };
        if self.busy() {
            return Some("File action in progress");
        }
        match action {
            FileAction::CopyPath | FileAction::Editor => None,
            FileAction::History if set.git_root().is_none() => Some("Not a repository comparison"),
            FileAction::History => None,
            FileAction::UndoLenz if !can_undo_lenz => Some("No Lenz edit to undo"),
            FileAction::UndoLenz => None,
            FileAction::Discard => discard_disabled(set, index, spec),
            FileAction::UndoDiscard
                if set
                    .git_root()
                    .is_some_and(|root| self.undo.contains_key(&root.join(&c.path))) =>
            {
                None
            }
            FileAction::UndoDiscard => Some("No Git discard to undo"),
        }
    }
    pub fn request(&mut self, set: &ChangeSet, index: usize, spec: &Spec) -> Result<(), String> {
        if let Some(why) = self.disabled(FileAction::Discard, set, index, spec, false) {
            return Err(why.into());
        }
        let root = set
            .git_root()
            .ok_or("Not a repository comparison")?
            .to_path_buf();
        let comparison = set
            .comparisons
            .get(index)
            .ok_or("No file selected")?
            .clone();
        self.pending = None;
        self.job = Some(crate::work::submit(
            Arc::downgrade(&self.token),
            move || prepare(&root, &comparison.path).map(|p| ResultItem::Prepared(p, comparison)),
        ));
        Ok(())
    }
    pub fn confirm(&mut self) -> Result<(), String> {
        if self.busy() {
            return Err("File action in progress".into());
        }
        let (pending, comparison) = self
            .pending
            .take()
            .ok_or("No discard awaiting confirmation")?;
        if self.undo.len() >= 64 && !self.undo.contains_key(&pending.path) {
            return Err("Discard undo storage is full; undo earlier discards first".into());
        }
        let used = self
            .undo
            .iter()
            .filter(|(path, _)| **path != pending.path)
            .map(|(_, r)| entry_size(r.undo.before.as_ref()))
            .sum::<usize>();
        if used + entry_size(pending.before.as_ref()) > 64 << 20 {
            return Err("Discard undo storage is full; undo earlier discards first".into());
        }
        self.job = Some(crate::work::submit(
            Arc::downgrade(&self.token),
            move || discard(pending).map(|u| ResultItem::Discarded(u, comparison)),
        ));
        Ok(())
    }
    pub fn undo(&mut self, set: &ChangeSet, index: usize) -> Result<(), String> {
        if self.busy() {
            return Err("File action in progress".into());
        }
        let c = set.comparisons.get(index).ok_or("No file selected")?;
        let path = set
            .git_root()
            .ok_or("Not a repository comparison")?
            .join(&c.path);
        let record = self.undo.remove(&path).ok_or("No Git discard to undo")?;
        // Keep the record even if a guarded undo fails.
        let undo = record.undo.clone();
        self.undo.insert(path.clone(), record);
        self.job = Some(crate::work::submit(
            Arc::downgrade(&self.token),
            move || undo_discard(&undo).map(|()| ResultItem::Undone(path)),
        ));
        Ok(())
    }
    /// Returns a changed path for adapters to invalidate and refresh; preparation only opens confirmation.
    pub fn poll(&mut self) -> Option<Result<Option<PathBuf>, String>> {
        let rx = self.job.as_ref()?;
        let result = match rx.try_recv() {
            Err(TryRecvError::Empty) => return None,
            Ok(Some(result)) => result,
            Ok(None) | Err(TryRecvError::Disconnected) => Err("File action interrupted".into()),
        };
        self.job = None;
        Some(result.map(|item| match item {
            ResultItem::Prepared(p, c) => {
                self.pending = Some((p, c));
                None
            }
            ResultItem::Discarded(undo, comparison) => {
                let path = undo.path.clone();
                self.undo.insert(
                    path.clone(),
                    Record {
                        undo: Arc::new(undo),
                        comparison,
                    },
                );
                Some(path)
            }
            ResultItem::Undone(path) => {
                self.undo.remove(&path);
                Some(path)
            }
        }))
    }
    pub fn retain_files(&self, set: &mut ChangeSet) {
        let Some(root) = set.git_root().map(Path::to_path_buf) else {
            return;
        };
        for record in self.undo.values() {
            if record.undo.path == root.join(&record.comparison.path)
                && !set
                    .comparisons
                    .iter()
                    .any(|c| c.path == record.comparison.path)
            {
                set.comparisons.push(record.comparison.clone());
            }
        }
    }
}
fn entry_size(entry: Option<&Entry>) -> usize {
    match entry {
        Some(Entry::File { bytes, .. }) => bytes.len(),
        Some(Entry::Link(path)) => path.as_os_str().len(),
        None => 0,
    }
}
pub fn discard_disabled(set: &ChangeSet, index: usize, spec: &Spec) -> Option<&'static str> {
    let c = set.comparisons.get(index)?;
    if set.git_root().is_none() {
        return Some("Not a repository comparison");
    }
    if !matches!(spec,Spec::Git{cached,revs,..}if crate::changeset::right_is_worktree(*cached,revs))
    {
        return Some("This comparison is not the working tree");
    }
    if c.status == Status::Untracked {
        return Some("Untracked file has no indexed version");
    }
    if c.status == Status::Conflicted {
        return Some("Resolve the conflict with the merge workflow");
    }
    if matches!(c.right, Content::Submodule(_)) || matches!(c.left, Content::Submodule(_)) {
        return Some("Submodule discard is unsupported");
    }
    None
}
fn safe_path(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let relative = Path::new(relative);
    if relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err("Invalid repository file path".into());
    }
    let path = root.join(relative);
    let mut parent = path.parent();
    while let Some(dir) = parent.filter(|d| *d != root) {
        if std::fs::symlink_metadata(dir).is_ok_and(|m| m.is_symlink()) {
            return Err("Cannot discard through a symlinked directory".into());
        }
        parent = dir.parent();
    }
    Ok(path)
}
fn indexed(root: &Path, relative: &str) -> Result<(Vec<u8>, Entry), String> {
    let output = crate::git::run(
        root,
        &[
            "--literal-pathspecs",
            "ls-files",
            "--stage",
            "-z",
            "--",
            relative,
        ],
    )?;
    let mut rows = output.split(|b| *b == 0).filter(|r| !r.is_empty());
    let row = rows.next().ok_or("File has no indexed version")?;
    if rows.next().is_some() {
        return Err("Unmerged file: use the merge workflow".into());
    }
    let header = std::str::from_utf8(
        row.split(|b| *b == b'\t')
            .next()
            .ok_or("Invalid index entry")?,
    )
    .map_err(|_| "Invalid index entry")?;
    let mut parts = header.split_whitespace();
    let mode = parts.next().ok_or("Invalid index mode")?;
    let oid = parts.next().ok_or("Invalid index object")?;
    if parts.next() != Some("0") {
        return Err("Unmerged file: use the merge workflow".into());
    }
    let size = crate::git::run(root, &["cat-file", "-s", oid])?;
    if String::from_utf8_lossy(&size)
        .trim()
        .parse::<usize>()
        .map_err(|_| "Invalid blob size")?
        > 64 << 20
    {
        return Err("Discard is limited to 64 MiB files to bound undo memory".into());
    }
    let bytes = crate::git::run(root, &["cat-file", "blob", oid])?;
    let entry = match mode {
        "100644" => Entry::File {
            bytes,
            mode: Some(0o644),
        },
        "100755" => Entry::File {
            bytes,
            mode: Some(0o755),
        },
        "120000" => Entry::Link(std::ffi::OsString::from_vec(bytes).into()),
        _ => return Err("Unsupported indexed file type".into()),
    };
    Ok((output, entry))
}
fn read_entry(path: &Path) -> Result<Option<Entry>, String> {
    let meta = match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        result => result.map_err(|e| e.to_string())?,
    };
    if meta.is_symlink() {
        return std::fs::read_link(path)
            .map(|p| Some(Entry::Link(p)))
            .map_err(|e| e.to_string());
    }
    if !meta.is_file() {
        return Err("Not a regular file or symlink".into());
    }
    if meta.len() > 64 << 20 {
        return Err("Discard is limited to 64 MiB files to bound undo memory".into());
    }
    std::fs::read(path)
        .map(|bytes| {
            Some(Entry::File {
                bytes,
                mode: Some(meta.mode() & 0o7777),
            })
        })
        .map_err(|e| e.to_string())
}
fn prepare(root: &Path, relative: &str) -> Result<Pending, String> {
    let root = std::fs::canonicalize(root).map_err(|e| e.to_string())?;
    let path = safe_path(&root, relative)?;
    let snapshot = TargetSnapshot::capture(&path);
    let before = read_entry(&path)?;
    snapshot.verify(&path).map_err(|e| e.to_string())?;
    let (indexed, after) = indexed(&root, relative)?;
    if before.as_ref() == Some(&after) {
        return Err("No working-tree changes to discard".into());
    }
    Ok(Pending {
        root,
        relative: relative.into(),
        path,
        indexed,
        after,
        before,
        snapshot,
    })
}
fn discard(pending: Pending) -> Result<Undo, String> {
    let Pending {
        root,
        relative,
        path,
        indexed: expected,
        after,
        before,
        snapshot,
    } = pending;
    safe_path(&root, &relative)?;
    if indexed(&root, &relative)?.0 != expected {
        return Err("The index changed — nothing written; reopen Discard".into());
    }
    snapshot.verify(&path).map_err(|e| e.to_string())?;
    write::put(&path, &after).map_err(|e| e.to_string())?;
    Ok(Undo {
        root,
        relative,
        snapshot: TargetSnapshot::written(&path, Some(&after)),
        path,
        before,
    })
}
fn undo_discard(undo: &Undo) -> Result<(), String> {
    safe_path(&undo.root, &undo.relative)?;
    undo.snapshot
        .verify(&undo.path)
        .map_err(|e| e.to_string())?;
    match &undo.before {
        Some(entry) => write::put(&undo.path, entry),
        None => write::delete(&undo.path).map(|_| ()),
    }
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        crate::git::run(dir.path(), &["init", "-q"]).unwrap();
        std::fs::write(dir.path().join("file.txt"), b"head\n").unwrap();
        crate::git::run(dir.path(), &["add", "file.txt"]).unwrap();
        crate::git::run(
            dir.path(),
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.test",
                "commit",
                "-qm",
                "initial",
            ],
        )
        .unwrap();
        std::fs::write(dir.path().join("file.txt"), b"staged\r\n").unwrap();
        crate::git::run(dir.path(), &["add", "file.txt"]).unwrap();
        std::fs::write(dir.path().join("file.txt"), b"working\n\xff").unwrap();
        dir
    }
    #[test]
    fn discard_restores_exact_index_bytes_preserves_staging_and_is_undoable() {
        let dir = repo();
        let path = dir.path().join("file.txt");
        let pending = prepare(dir.path(), "file.txt").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"working\n\xff");
        let undo = discard(pending).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"staged\r\n");
        assert_eq!(
            crate::git::run(dir.path(), &["show", ":file.txt"]).unwrap(),
            b"staged\r\n"
        );
        undo_discard(&undo).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"working\n\xff");
    }
    #[test]
    fn discard_and_undo_refuse_external_changes_and_index_changes() {
        let dir = repo();
        let path = dir.path().join("file.txt");
        let pending = prepare(dir.path(), "file.txt").unwrap();
        std::fs::write(&path, b"external").unwrap();
        assert!(discard(pending).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"external");
        let pending = prepare(dir.path(), "file.txt").unwrap();
        crate::git::run(dir.path(), &["add", "file.txt"]).unwrap();
        assert!(discard(pending).is_err());
        std::fs::write(&path, b"another edit").unwrap();
        let undo = discard(prepare(dir.path(), "file.txt").unwrap()).unwrap();
        std::fs::write(&path, b"later edit").unwrap();
        assert!(undo_discard(&undo).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"later edit");
    }
    #[test]
    fn deleted_files_and_symlinks_are_restored_and_undone_without_following_links() {
        let dir = repo();
        let path = dir.path().join("file.txt");
        std::fs::remove_file(&path).unwrap();
        let undo = discard(prepare(dir.path(), "file.txt").unwrap()).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"staged\r\n");
        undo_discard(&undo).unwrap();
        assert!(!path.exists());
        let target = dir.path().join("target");
        std::fs::write(&target, b"do not touch").unwrap();
        std::os::unix::fs::symlink("target", &path).unwrap();
        let undo = discard(prepare(dir.path(), "file.txt").unwrap()).unwrap();
        assert!(!std::fs::symlink_metadata(&path).unwrap().is_symlink());
        undo_discard(&undo).unwrap();
        assert_eq!(
            std::fs::read_link(&path).unwrap(),
            std::path::PathBuf::from("target")
        );
        assert_eq!(std::fs::read(&target).unwrap(), b"do not touch");
    }
    #[test]
    fn untracked_and_parent_symlink_paths_are_never_discarded() {
        let dir = repo();
        std::fs::write(dir.path().join("new"), b"keep").unwrap();
        assert!(prepare(dir.path(), "new").is_err());
        assert!(prepare(dir.path(), "../file.txt").is_err());
        std::fs::create_dir(dir.path().join("real")).unwrap();
        std::fs::write(dir.path().join("real/f"), b"original").unwrap();
        crate::git::run(dir.path(), &["add", "real/f"]).unwrap();
        std::fs::rename(dir.path().join("real"), dir.path().join("elsewhere")).unwrap();
        std::os::unix::fs::symlink("elsewhere", dir.path().join("real")).unwrap();
        assert!(prepare(dir.path(), "real/f").is_err());
    }
}
