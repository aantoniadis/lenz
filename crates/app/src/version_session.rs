//! Shared whole-version resolution preserving bytes, executable modes and links.
use crate::{
    document::LARGE,
    write::{self, Entry},
};
use std::{
    hash::{DefaultHasher, Hasher},
    io::Read,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    time::SystemTime,
};

#[derive(Debug, Clone, PartialEq, Eq)]
enum TargetEntry {
    Missing,
    File {
        hash: u64,
        size: u64,
        mtime: Option<SystemTime>,
        mode: u32,
    },
    Link(PathBuf),
}
/// Captured by the loader off the UI thread; whole-version writes act on a link itself.
#[derive(Debug, Clone)]
pub struct TargetSnapshot(Result<TargetEntry, String>, Option<PathBuf>);
impl TargetSnapshot {
    pub fn capture(path: &Path) -> Self {
        Self(Self::read(path), None)
    }
    fn in_repository(root: &Path, path: &Path) -> Self {
        let result = write::check_repository_path(root, path, true)
            .map_err(|e| e.to_string()).and_then(|()| Self::read(path));
        Self(result, Some(root.to_path_buf()))
    }
    fn recapture(&self, path: &Path) -> Self {
        match &self.1 {
            Some(root) => Self::in_repository(root, path),
            None => Self::capture(path),
        }
    }
    fn read(path: &Path) -> Result<TargetEntry, String> {
        let meta = match std::fs::symlink_metadata(path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(TargetEntry::Missing),
            other => other.map_err(|e| format!("reading {}: {e}", path.display()))?,
        };
        if meta.is_symlink() {
            return std::fs::read_link(path)
                .map(TargetEntry::Link)
                .map_err(|e| format!("reading {}: {e}", path.display()));
        }
        if !meta.is_file() {
            return Err(format!(
                "{} is not a regular file or symlink",
                path.display()
            ));
        }
        let mut file =
            std::fs::File::open(path).map_err(|e| format!("reading {}: {e}", path.display()))?;
        let mut hash = DefaultHasher::new();
        let mut bytes = [0; 65536];
        loop {
            let n = file
                .read(&mut bytes)
                .map_err(|e| format!("reading {}: {e}", path.display()))?;
            if n == 0 {
                break;
            }
            hash.write(&bytes[..n]);
        }
        Ok(TargetEntry::File {
            hash: hash.finish(),
            size: meta.len(),
            mtime: meta.modified().ok(),
            mode: meta.mode() & 0o7777,
        })
    }
    pub fn verify(&self, path: &Path) -> Result<(), write::WriteError> {
        if let Some(root) = &self.1 { write::check_repository_path(root, path, true)?; }
        let expected = self
            .0
            .as_ref()
            .map_err(|e| write::WriteError::Io(e.clone()))?;
        let current = Self::read(path).map_err(write::WriteError::Io)?;
        if current == *expected {
            Ok(())
        } else {
            Err(write::WriteError::ChangedOnDisk)
        }
    }
    pub(crate) fn written(path: &Path, entry: Option<&Entry>) -> Self {
        Self(match entry {
            None => Ok(TargetEntry::Missing),
            Some(Entry::Link(target)) => Ok(TargetEntry::Link(target.clone())),
            Some(Entry::File { bytes, .. }) => std::fs::symlink_metadata(path)
                .map_err(|e| format!("reading {}: {e}", path.display()))
                .and_then(|meta| {
                    if !meta.is_file() {
                        return Err(format!("{} is no longer a regular file", path.display()));
                    }
                    let mut hash = DefaultHasher::new();
                    hash.write(bytes);
                    Ok(TargetEntry::File {
                        hash: hash.finish(),
                        size: meta.len(),
                        mtime: meta.modified().ok(),
                        mode: meta.mode() & 0o7777,
                    })
                }),
        }, None)
    }
    fn changed(&self, path: &Path) -> bool {
        match (&self.0, std::fs::symlink_metadata(path)) {
            (Ok(TargetEntry::Missing), Err(e)) if e.kind() == std::io::ErrorKind::NotFound => false,
            (
                Ok(TargetEntry::File {
                    size, mtime, mode, ..
                }),
                Ok(meta),
            ) => {
                !meta.is_file()
                    || meta.len() != *size
                    || meta.modified().ok() != *mtime
                    || meta.mode() & 0o7777 != *mode
            }
            (Ok(TargetEntry::Link(target)), Ok(meta)) if meta.is_symlink() => {
                std::fs::read_link(path).ok().as_ref() != Some(target)
            }
            _ => true,
        }
    }
}
fn megabytes(n: u64) -> String {
    format!("{:.1} MB", n as f64 / (1 << 20) as f64)
}
fn kilobytes(n: u64) -> String {
    if n < 1024 {
        format!("{n} bytes")
    } else {
        format!("{:.1} KB", n as f64 / 1024.0)
    }
}
/// One version a Conflicted File can be resolved to.
#[derive(Debug, Clone)]
pub struct Version {
    /// "Base", "Ours · main", "Theirs · feature".
    pub label: String,
    /// What taking it writes; `None` removes the file.
    pub entry: Option<Entry>,
    /// Size, "same bytes as …", a link's target.
    pub note: String,
    /// Couldn't be read (e.g. an LFS object offline): shown on the row, and it can't be taken.
    pub error: Option<String>,
}

/// A Version Picker ready to show.
pub struct Opened {
    /// The merged file's name as shown.
    pub path: String,
    /// Where the Result is written.
    pub target: PathBuf,
    pub snapshot: TargetSnapshot,
    pub versions: Vec<Version>,
    /// Why the file can't be merged line by line: "binary", "not valid UTF-8 …", "large (34 MB)".
    pub why: String,
    /// Text that is merely too large: offer "Merge anyway".
    pub large_text: bool,
    /// BASE, LOCAL, REMOTE, MERGED under `git mergetool`, for "Merge anyway".
    pub files: Option<[PathBuf; 4]>,
    /// Lines shown instead of rows when nothing can be taken here (a submodule); a line starting with `$ ` is a
    /// command, copied when clicked.
    pub info: Vec<String>,
}

pub struct PickerState {
    pub o: Opened,
    /// One `git mergetool` file in its own window, rather than a file of the Conflict List.
    pub mergetool: bool,
    /// The version taken, if any.
    pub chosen: Option<usize>,
    undo: Vec<Option<usize>>,
    redo: Vec<Option<usize>>,
    pub dirty: bool,
    pub saved: bool,
    pub ask: Option<crate::merge_session::Ask>,
    disk_kept: bool,
}

/// Why `bytes` can't be merged line by line, if so.
pub fn unmergeable(bytes: &[u8]) -> Option<String> {
    if crate::changeset::looks_binary(bytes) {
        return Some("binary".into());
    }
    if bytes.len() as u64 > LARGE {
        return Some(format!("large ({})", megabytes(bytes.len() as u64)));
    }
    let d = crate::encoding::decode(bytes, None);
    d.lossy.then(|| format!("not valid {}", d.encoding))
}

fn size(n: usize) -> String {
    if n as u64 >= 1 << 20 {
        megabytes(n as u64)
    } else {
        kilobytes(n as u64)
    }
}

/// Rows for versions given as bytes (`None`: the version doesn't exist), noting sizes and identical bytes.
pub fn versions(named: Vec<(String, Option<Vec<u8>>)>) -> Vec<Version> {
    let bytes: Vec<Option<Vec<u8>>> = named.iter().map(|(_, b)| b.clone()).collect();
    named
        .into_iter()
        .enumerate()
        .map(|(i, (label, b))| {
            let note = match &b {
                None => "absent: taking it deletes the file".to_string(),
                Some(b) => {
                    let same = (0..i)
                        .find(|&j| bytes[j].as_ref() == Some(b))
                        .map(|j| format!(" · same bytes as row {}", j + 1));
                    format!("{}{}", size(b.len()), same.unwrap_or_default())
                }
            };
            Version {
                label,
                entry: b.map(|bytes| Entry::File { bytes, mode: None }),
                note,
                error: None,
            }
        })
        .collect()
}

/// `lenz merge BASE LOCAL REMOTE MERGED` for a file that can't be merged line by line; `None` when it can.
pub fn open_files(
    base: &Path,
    ours: &Path,
    theirs: &Path,
    merged: &Path,
    cwd: &Path,
) -> Result<Option<Opened>, String> {
    let read = |p: &Path| std::fs::read(p).map_err(|e| format!("lenz merge: {}: {e}", p.display()));
    let (b, o, t) = (read(base)?, read(ours)?, read(theirs)?);
    let Some(why) = [&o, &t, &b].iter().find_map(|x| unmergeable(x)) else {
        return Ok(None);
    };
    let large_text = why.starts_with("large")
        && ![&o, &t, &b]
            .iter()
            .any(|x| crate::changeset::looks_binary(x));
    let root = crate::changeset::toplevel(cwd).ok();
    let rel = root.as_ref().and_then(|r| {
        let merged = std::fs::canonicalize(merged).ok()?;
        merged
            .strip_prefix(std::fs::canonicalize(r).ok()?)
            .ok()
            .map(|p| p.to_string_lossy().into_owned())
    });
    let (ours_label, theirs_label, has_base) = match (&root, &rel) {
        (Some(root), Some(rel)) => {
            let (o, t) = match crate::conflicts::in_progress(root) {
                Ok(Some(p)) => (p.ours, p.theirs),
                _ => ("Local".to_string(), "Remote".to_string()),
            };
            (o, t, crate::conflicts::has_base(root, rel).unwrap_or(true))
        }
        _ => ("Local".to_string(), "Remote".to_string(), !b.is_empty()),
    };
    let mut named = vec![
        (format!("Ours · {ours_label}"), Some(o)),
        (format!("Theirs · {theirs_label}"), Some(t)),
    ];
    if has_base {
        named.push(("Base".to_string(), Some(b)));
    }
    let path = rel.unwrap_or_else(|| {
        merged
            .strip_prefix(cwd)
            .unwrap_or(merged)
            .display()
            .to_string()
    });
    let files = Some([
        base.to_path_buf(),
        ours.to_path_buf(),
        theirs.to_path_buf(),
        merged.to_path_buf(),
    ]);
    Ok(Some(Opened {
        path,
        target: merged.to_path_buf(),
        snapshot: TargetSnapshot::capture(merged),
        versions: versions(named),
        why,
        large_text,
        files,
        info: Vec::new(),
    }))
}

fn submodule_command(path: &str, sha: &str) -> String {
    let path = format!("'{}'", path.replace('\'', r"'\''"));
    format!("git -C {path} checkout {sha} && git add -- {path}")
}

/// A Conflicted File of the Conflict List that is resolved whole: binary, undecodable or large text, deleted on one
/// side, a symlink, or a submodule (shown, resolved with git).
pub fn open_conflict(root: &Path, file: &crate::conflicts::ConflictedFile) -> Opened {
    use crate::conflicts::{Kind, Side};
    let (ours, theirs) = match crate::conflicts::in_progress(root) {
        Ok(Some(p)) => (p.ours, p.theirs),
        _ => ("Local".to_string(), "Remote".to_string()),
    };
    let names = [
        ("Base".to_string(), 1u8),
        (format!("Ours · {ours}"), 2),
        (format!("Theirs · {theirs}"), 3),
    ];
    let target = root.join(&file.path);
    let mut o = Opened {
        path: file.path.clone(),
        snapshot: TargetSnapshot::in_repository(root, &target),
        target,
        versions: Vec::new(),
        why: file.wording().to_string(),
        large_text: false,
        files: None,
        info: Vec::new(),
    };
    // a stage as what taking it writes, keeping a link a link and an executable executable
    let version = |label: &str, n: u8| -> Version {
        let stage = file.stages[n as usize - 1].as_ref();
        let mode = stage.map_or(0, |s| s.mode);
        match crate::conflicts::read_stage(root, &file.path, n) {
            Ok(bytes) if mode == 0o120000 => {
                let to = String::from_utf8_lossy(&bytes).into_owned();
                Version {
                    label: label.to_string(),
                    note: format!("symlink → {to}"),
                    entry: Some(Entry::Link(to.into())),
                    error: None,
                }
            }
            Ok(bytes) => {
                let note = format!(
                    "{}{}",
                    size(bytes.len()),
                    if mode & 0o111 != 0 {
                        " · executable"
                    } else {
                        ""
                    }
                );
                Version {
                    label: label.to_string(),
                    note,
                    entry: Some(Entry::File {
                        bytes,
                        mode: Some(mode & 0o777),
                    }),
                    error: None,
                }
            }
            Err(e) => Version {
                label: label.to_string(),
                note: String::new(),
                entry: None,
                error: Some(e),
            },
        }
    };
    match file.kind {
        Kind::Submodule => {
            let sha = |n: usize| file.stages[n].as_ref().map(|s| s.sha.clone());
            let sub = root.join(&file.path);
            let subject = |sha: &str| {
                crate::git::run(&sub, &["log", "-1", "--format=%s", sha])
                    .map(|b| String::from_utf8_lossy(&b).trim().to_string())
                    .unwrap_or_default()
            };
            o.why = format!("a submodule ({}) — resolve it with git", file.wording());
            for (label, n) in [(&names[1].0, 1), (&names[2].0, 2)] {
                if let Some(sha) = sha(n) {
                    o.info.push(format!(
                        "{label}: {} {}",
                        &sha[..sha.len().min(10)],
                        subject(&sha)
                    ));
                }
            }
            if let (Some(a), Some(b)) = (sha(1), sha(2)) {
                let ancestor = |x: &str, y: &str| {
                    crate::git::run(&sub, &["merge-base", "--is-ancestor", x, y]).is_ok()
                };
                if ancestor(&a, &b) {
                    o.info.push("Theirs is ahead of Ours: fast-forward".into());
                } else if ancestor(&b, &a) {
                    o.info.push("Ours is ahead of Theirs".into());
                }
                if let Ok(log) =
                    crate::git::run(&sub, &["log", "--oneline", "-8", &format!("{a}...{b}")])
                {
                    o.info.extend(
                        String::from_utf8_lossy(&log)
                            .lines()
                            .map(|l| format!("  {l}")),
                    );
                }
                for (label, sha) in [("Theirs", &b), ("Ours", &a)] {
                    o.info.push(format!("$ {}   ({label})", submodule_command(&file.path, sha)));
                }
            }
        }
        Kind::DeletedBy(side) => {
            let keep = match side {
                Side::Us => 3,
                _ => 2,
            };
            if file.stages[keep as usize - 1].is_some() {
                let mut v = version(&format!("Keep — {}", names[keep as usize - 1].0), keep);
                v.note = format!("{} · the diff below is from the Base", v.note);
                o.versions.push(v);
            }
            o.versions.push(Version {
                label: "Delete".into(),
                entry: None,
                note: "removes the file".into(),
                error: None,
            });
        }
        _ => {
            for (label, n) in [&names[1], &names[2], &names[0]] {
                if file.stages[*n as usize - 1].is_some() {
                    o.versions.push(version(label, *n));
                }
            }
            if file.kind != Kind::Symlink {
                if let Some(why) = o
                    .versions
                    .iter()
                    .filter_map(|v| match &v.entry {
                        Some(Entry::File { bytes, .. }) => unmergeable(bytes),
                        _ => None,
                    })
                    .next()
                {
                    o.why = why;
                }
                // identical versions, as for mergetool's rows
                for i in 1..o.versions.len() {
                    let same = (0..i).find(|&j| {
                        o.versions[j].entry.is_some() && o.versions[j].entry == o.versions[i].entry
                    });
                    if let Some(j) = same {
                        o.versions[i].note += &format!(" · same bytes as row {}", j + 1);
                    }
                }
            }
        }
    }
    o
}

impl PickerState {
    pub fn new(o: Opened, mergetool: bool) -> Self {
        PickerState {
            o,
            mergetool,
            chosen: None,
            undo: Vec::new(),
            redo: Vec::new(),
            dirty: false,
            saved: false,
            ask: None,
            disk_kept: false,
        }
    }

    pub fn take(&mut self, i: usize) -> bool {
        if self.o.versions.get(i).is_none_or(|v| v.error.is_some()) || self.chosen == Some(i) {
            return false;
        }
        self.undo.push(self.chosen);
        self.redo.clear();
        self.chosen = Some(i);
        self.dirty = true;
        self.ask = None;
        true
    }

    pub fn undo(&mut self) -> bool {
        let Some(prev) = self.undo.pop() else {
            return false;
        };
        self.redo.push(std::mem::replace(&mut self.chosen, prev));
        self.dirty = true;
        self.ask = None;
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(next) = self.redo.pop() else {
            return false;
        };
        self.undo.push(std::mem::replace(&mut self.chosen, next));
        self.dirty = true;
        self.ask = None;
        true
    }

    /// Write the version taken; nothing taken leaves the file as it is.
    pub fn save(&mut self) -> Result<(), String> {
        self.save_checked().map_err(|e| e.to_string())
    }
    fn save_checked(&mut self) -> Result<(), write::WriteError> {
        let Some(i) = self.chosen else {
            self.dirty = false;
            self.ask = None;
            return Ok(());
        };
        let target = &self.o.target;
        self.o.snapshot.verify(target)?;
        match &self.o.versions[i].entry {
            Some(entry) => write::put(target, entry)?,
            None if std::fs::symlink_metadata(target).is_ok() => {
                write::delete(target)?;
            }
            None => {}
        }
        self.dirty = false;
        self.saved = true;
        self.ask = None;
        self.disk_kept = false;
        self.o.snapshot = TargetSnapshot(
            TargetSnapshot::written(target, self.o.versions[i].entry.as_ref()).0,
            self.o.snapshot.1.clone(),
        );
        Ok(())
    }
    fn overwrite(&mut self) -> Result<(), write::WriteError> {
        let snapshot = self.o.snapshot.recapture(&self.o.target);
        snapshot
            .0
            .as_ref()
            .map_err(|e| write::WriteError::Io(e.clone()))?;
        self.o.snapshot = snapshot;
        self.save_checked()
    }
    pub fn reload(&mut self) -> Result<(), String> {
        let snapshot = self.o.snapshot.recapture(&self.o.target);
        snapshot.0.as_ref().map_err(Clone::clone)?;
        self.o.snapshot = snapshot;
        self.chosen = None;
        self.undo.clear();
        self.redo.clear();
        self.dirty = false;
        self.ask = None;
        self.disk_kept = false;
        Ok(())
    }
    pub fn check_disk(&mut self) -> bool {
        if !self.o.versions.is_empty()
            && self.ask.is_none()
            && !self.disk_kept
            && self.o.snapshot.changed(&self.o.target)
        {
            self.ask = Some(crate::merge_session::Ask::ChangedOnDisk);
            true
        } else {
            false
        }
    }
    fn refused(&mut self, error: write::WriteError, done: bool, out: &mut Outcome) {
        if error == write::WriteError::ChangedOnDisk {
            self.ask = Some(crate::merge_session::Ask::Overwrite { done });
        } else {
            self.ask = None;
            out.error = Some(format!("Not saved (the choice is kept): {error}"));
        }
    }
}

/// The Version Picker's toolbar buttons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickAct {
    Undo,
    Redo,
    Save,
    Done,
    Cancel,
    /// Repository Window only: stage the saved file (`git rm` after a Delete).
    MarkResolved,
    Reload,
}

#[derive(Default)]
pub struct Outcome {
    pub toast: Option<String>,
    pub error: Option<String>,
    pub done: bool,
    pub back: bool,
    pub mark: Option<String>,
    pub quit: Option<u8>,
}
impl PickerState {
    pub fn execute(&mut self, act: PickAct) -> Outcome {
    crate::diagnostics::breadcrumb(act.label());
        use crate::merge_session::Ask;
        let mut out = Outcome::default();
        match act {
            PickAct::Undo => {
                if !self.undo() {
                    out.toast = Some("Nothing to undo".into());
                }
            }
            PickAct::Redo => {
                if !self.redo() {
                    out.toast = Some("Nothing to redo".into());
                }
            }
            PickAct::Save => match if self.ask == Some(Ask::Overwrite { done: false }) {
                self.overwrite()
            } else {
                self.save_checked()
            } {
                Ok(()) => out.toast = Some(format!("Saved {}", self.o.path)),
                Err(e) => self.refused(e, false, &mut out),
            },
            PickAct::Done => {
                if self.chosen.is_none() && self.ask != Some(Ask::DoneWithConflicts) {
                    self.ask = Some(Ask::DoneWithConflicts);
                } else {
                    match if self.ask == Some(Ask::Overwrite { done: true }) {
                        self.overwrite()
                    } else {
                        self.save_checked()
                    } {
                        Ok(()) if self.mergetool => out.quit = Some(0),
                        Ok(()) => {
                            self.ask = None;
                            out.done = true;
                        }
                        Err(e) => self.refused(e, true, &mut out),
                    }
                }
            }
            PickAct::Cancel => {
                let warn = self.dirty || (self.saved && self.mergetool);
                if warn && self.ask != Some(Ask::Close) {
                    self.ask = Some(Ask::Close);
                } else if self.mergetool {
                    out.quit = Some(1);
                } else {
                    out.back = true;
                }
            }
            PickAct::MarkResolved => {
                if self.dirty {
                    out.toast = Some(format!(
                        "Unsaved choice in {} — save it first (ctrl-s)",
                        self.o.path
                    ));
                } else {
                    out.mark = Some(self.o.path.clone());
                }
            }
            PickAct::Reload => match self.reload() {
                Ok(()) => out.toast = Some(format!("Reloaded {} — choices discarded", self.o.path)),
                Err(e) => out.error = Some(format!("Not reloaded: {e}")),
            },
        }
        out
    }
    pub fn may_close(&mut self) -> bool {
        use crate::merge_session::Ask;
        if !(self.dirty || (self.saved && self.mergetool)) || self.ask == Some(Ask::Close) {
            true
        } else {
            self.ask = Some(Ask::Close);
            false
        }
    }
}

impl PickerState {
    pub fn question(&self) -> Option<&'static str> {
        use crate::merge_session::Ask;
        self.ask.map(|a| match a {
            Ask::DoneWithConflicts => {
                "No version taken — Done again keeps the file as it is; Esc goes back"
            }
            Ask::Close if self.dirty => {
                "Unsaved choice — Save keeps it; Esc again closes without saving"
            }
            Ask::Close => "git restores its original — Done keeps saved work; Esc again discards",
            Ask::ChangedOnDisk => {
                "Changed on disk — r reloads (discards choices); Esc keeps the choice"
            }
            Ask::Overwrite { done: false } => {
                "Changed on disk — Save again overwrites; r reloads (discards choices); Esc cancels"
            }
            Ask::Overwrite { done: true } => {
                "Changed on disk — Done again overwrites; r reloads (discards choices); Esc cancels"
            }
        })
    }
}

impl PickerState {
    pub fn dismiss(&mut self) -> bool {
        if self
            .ask
            .is_some_and(|ask| ask != crate::merge_session::Ask::Close)
        {
            self.disk_kept |= self.ask == Some(crate::merge_session::Ask::ChangedOnDisk);
            self.ask = None;
            true
        } else {
            false
        }
    }
}

impl PickAct {
    pub const ALL: [Self; 7] = [
        Self::Undo,
        Self::Redo,
        Self::Save,
        Self::Done,
        Self::Cancel,
        Self::MarkResolved,
        Self::Reload,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Undo => "Undo version choice",
            Self::Redo => "Redo version choice",
            Self::Save => "Save chosen version",
            Self::Done => "Done",
            Self::Cancel => "Cancel",
            Self::MarkResolved => "Mark resolved (stage saved file)",
            Self::Reload => "Reload file (discard version choices)",
        }
    }
}
impl PickerState {
    pub fn disabled(&self, action: PickAct) -> Option<&'static str> {
        match action {
            PickAct::Undo if self.undo.is_empty() => Some("Nothing to undo"),
            PickAct::Redo if self.redo.is_empty() => Some("Nothing to redo"),
            PickAct::MarkResolved if self.mergetool => Some("Only available in the Conflict List"),
            PickAct::MarkResolved if self.dirty => Some("Save chosen version first"),
            _ => None,
        }
    }
}
impl PickerState {
    pub fn command(&self, i: usize) -> Option<String> {
        Some(
            self.o
                .info
                .get(i)?
                .strip_prefix("$ ")?
                .rsplit_once("   (")?
                .0
                .to_owned(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copied_submodule_commands_keep_paths_literal() {
        let dir = tempfile::tempdir().unwrap();
        let capture = dir.path().join("args");
        let sha = "0123456789abcdef";
        for path in ["module$(touch INJECTED)", "-module's   (space)"] {
            let command = submodule_command(path, sha);
            let target = dir.path().join("unused");
            let picker = PickerState::new(Opened {
                path: path.into(), target: target.clone(), snapshot: TargetSnapshot::capture(&target),
                versions: Vec::new(), why: "submodule".into(), large_text: false, files: None,
                info: vec![format!("$ {command}   (Ours)")],
            }, false);
            assert_eq!(picker.command(0).as_deref(), Some(command.as_str()));
            let status = std::process::Command::new("sh").current_dir(dir.path())
                .env("CAPTURE", &capture)
                .args(["-c", &format!("git() {{ printf '%s\\0' \"$@\" >> \"$CAPTURE\"; }}; {command}")])
                .status().unwrap();
            assert!(status.success());
            let expected = ["-C", path, "checkout", sha, "add", "--", path].join("\0") + "\0";
            assert_eq!(std::fs::read(&capture).unwrap(), expected.as_bytes());
            assert!(!dir.path().join("INJECTED").exists());
            std::fs::remove_file(&capture).unwrap();
        }
    }

    #[test]
    fn rows_note_sizes_and_identical_versions() {
        let v = versions(vec![
            ("Ours".into(), Some(vec![1, 2])),
            ("Theirs".into(), Some(vec![3])),
            ("Base".into(), Some(vec![1, 2])),
            ("Gone".into(), None),
        ]);
        assert_eq!(v[0].note, size(2));
        assert!(v[2].note.ends_with("same bytes as row 1"));
        assert!(v[3].entry.is_none());
    }

    #[test]
    fn binary_undecodable_and_huge_files_need_the_picker() {
        assert_eq!(unmergeable(b"plain\ntext\n"), None);
        assert_eq!(unmergeable(b"\x89PNG\0\0").as_deref(), Some("binary"));
        assert!(
            unmergeable(&vec![b'a'; (LARGE + 1) as usize])
                .unwrap()
                .starts_with("large")
        );
    }

    #[test]
    fn conflicts_resolved_whole_offer_their_versions() {
        use crate::git::testing::{commit_all, git, repo, try_git, write};
        let dir = repo();
        let d = dir.path();
        write(d, "gone.txt", "keep\n");
        std::os::unix::fs::symlink("one", d.join("link")).unwrap();
        commit_all(d, "base");
        git(d, &["checkout", "-q", "-b", "feature"]);
        write(d, "gone.txt", "changed\n");
        std::fs::remove_file(d.join("link")).unwrap();
        std::os::unix::fs::symlink("three", d.join("link")).unwrap();
        commit_all(d, "theirs");
        git(d, &["checkout", "-q", "main"]);
        git(d, &["rm", "-q", "gone.txt"]);
        std::fs::remove_file(d.join("link")).unwrap();
        std::os::unix::fs::symlink("two", d.join("link")).unwrap();
        commit_all(d, "ours");
        assert!(!try_git(d, &["merge", "-q", "feature"]));
        let files = crate::conflicts::list(d).unwrap();
        let open = |path: &str| open_conflict(d, files.iter().find(|f| f.path == path).unwrap());
        let gone = open("gone.txt");
        let labels: Vec<_> = gone.versions.iter().map(|v| v.label.as_str()).collect();
        assert_eq!(labels, ["Keep — Theirs · feature", "Delete"]);
        assert!(
            matches!(&gone.versions[0].entry, Some(Entry::File { bytes, .. }) if bytes == b"changed\n")
        );
        let link = open("link");
        let targets: Vec<_> = link.versions.iter().map(|v| v.entry.clone()).collect();
        assert_eq!(
            targets,
            [
                Some(Entry::Link("two".into())),
                Some(Entry::Link("three".into())),
                Some(Entry::Link("one".into()))
            ]
        );
        // taking Theirs re-points the link; Delete removes the file
        let mut s = PickerState::new(link, false);
        s.take(1);
        s.save().unwrap();
        assert_eq!(
            std::fs::read_link(d.join("link")).unwrap(),
            PathBuf::from("three")
        );
        let mut s = PickerState::new(gone, false);
        s.take(0);
        s.save().unwrap();
        assert_eq!(std::fs::read(d.join("gone.txt")).unwrap(), b"changed\n");
    }

    #[test]
    fn repository_picker_rejects_parent_symlinks_even_after_save_and_reload() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("nested");
        std::fs::create_dir(&folder).unwrap();
        let target = folder.join("binary");
        std::fs::write(&target, b"old").unwrap();
        let opened = Opened {
            path: "nested/binary".into(), target: target.clone(),
            snapshot: TargetSnapshot::in_repository(dir.path(), &target),
            versions: versions(vec![("Ours".into(), Some(b"new".to_vec())), ("Delete".into(), None)]),
            why: "binary".into(), large_text: false, files: None, info: Vec::new(),
        };
        let mut picker = PickerState::new(opened, false);
        picker.take(0);
        picker.save().unwrap();
        picker.reload().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("binary"), b"keep").unwrap();
        std::fs::rename(&folder, dir.path().join("original")).unwrap();
        std::os::unix::fs::symlink(outside.path(), &folder).unwrap();
        assert!(TargetSnapshot::in_repository(dir.path(), &target).0.is_err());
        picker.take(0);
        assert!(picker.save().is_err());
        assert!(picker.overwrite().is_err());
        assert!(picker.reload().is_err());
        picker.take(1);
        assert!(picker.save().is_err());
        assert_eq!(std::fs::read(outside.path().join("binary")).unwrap(), b"keep");
    }

    #[test]
    fn taking_versions_undoes_and_saves_exact_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("img.bin");
        std::fs::write(&target, [0u8, 1, 2]).unwrap();
        let o = Opened {
            path: "img.bin".into(),
            target: target.clone(),
            snapshot: TargetSnapshot::capture(&target),
            versions: versions(vec![
                ("Ours".into(), Some(vec![0, 1, 2])),
                ("Theirs".into(), Some(vec![9, 9])),
                ("Delete".into(), None),
            ]),
            why: "binary".into(),
            large_text: false,
            files: None,
            info: Vec::new(),
        };
        let mut s = PickerState::new(o, true);
        assert!(s.take(1));
        assert!(s.take(2));
        assert!(s.undo());
        assert_eq!(s.chosen, Some(1));
        s.save().unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), [9, 9]);
        s.take(2);
        s.save().unwrap();
        assert!(!target.exists());
    }

    #[test]
    fn stale_whole_versions_refuse_save_and_delete_until_same_action_confirms() {
        use crate::merge_session::Ask;
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("binary");
        std::fs::write(&target, b"\0ours").unwrap();
        let mut picker = PickerState::new(
            Opened {
                path: "binary".into(),
                target: target.clone(),
                snapshot: TargetSnapshot::capture(&target),
                versions: versions(vec![
                    ("Ours".into(), Some(b"\0ours".to_vec())),
                    ("Theirs".into(), Some(b"\0theirs".to_vec())),
                    ("Delete".into(), None),
                ]),
                why: "binary".into(),
                large_text: false,
                files: None,
                info: Vec::new(),
            },
            true,
        );
        picker.take(0);
        picker.take(1);
        std::fs::write(&target, b"\0external").unwrap();
        assert!(picker.check_disk());
        picker.execute(PickAct::Save);
        assert_eq!(picker.ask, Some(Ask::Overwrite { done: false }));
        assert_eq!(std::fs::read(&target).unwrap(), b"\0external");
        picker.execute(PickAct::Undo);
        picker.execute(PickAct::Save);
        assert_eq!(std::fs::read(&target).unwrap(), b"\0external");
        assert_eq!(picker.ask, Some(Ask::Overwrite { done: false }));
        picker.execute(PickAct::Redo);
        picker.execute(PickAct::Save);
        assert_eq!(std::fs::read(&target).unwrap(), b"\0external");
        // Switching Save to Done is not overwrite confirmation for Save.
        assert!(picker.execute(PickAct::Done).quit.is_none());
        assert_eq!(std::fs::read(&target).unwrap(), b"\0external");
        assert_eq!(picker.ask, Some(Ask::Overwrite { done: true }));
        picker.execute(PickAct::Reload);
        assert_eq!(picker.chosen, None);
        assert!(!picker.dirty);
        assert_eq!(picker.disabled(PickAct::Undo), Some("Nothing to undo"));
        assert_eq!(picker.ask, None);
        picker.take(1);
        picker.execute(PickAct::Save);
        assert_eq!(std::fs::read(&target).unwrap(), b"\0theirs");
        picker.take(2);
        std::fs::write(&target, b"\0new external").unwrap();
        picker.execute(PickAct::Save);
        assert_eq!(std::fs::read(&target).unwrap(), b"\0new external");
        picker.execute(PickAct::Save);
        assert!(!target.exists());
        std::fs::write(&target, b"\0appeared").unwrap();
        assert!(picker.execute(PickAct::Done).quit.is_none());
        assert_eq!(std::fs::read(&target).unwrap(), b"\0appeared");
        assert_eq!(picker.execute(PickAct::Done).quit, Some(0));
        assert!(!target.exists());
    }

    #[test]
    fn whole_version_streaming_snapshot_matches_large_written_bytes_on_repeated_save() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("large.bin");
        std::fs::write(&target, vec![17; 131_089]).unwrap();
        let chosen: Vec<u8> = (0..196_623).map(|i| (i % 251) as u8).collect();
        let mut picker = PickerState::new(
            Opened {
                path: "large.bin".into(),
                target: target.clone(),
                snapshot: TargetSnapshot::capture(&target),
                versions: versions(vec![("Theirs".into(), Some(chosen.clone()))]),
                why: "binary".into(),
                large_text: false,
                files: None,
                info: Vec::new(),
            },
            false,
        );
        picker.take(0);
        picker.save().unwrap();
        picker.save().unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), chosen);
        assert!(!picker.check_disk());
    }

    #[test]
    fn whole_version_guard_checks_symlink_itself_modes_and_unreadable_target() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("link");
        symlink("missing-one", &target).unwrap();
        let mut picker = PickerState::new(
            Opened {
                path: "link".into(),
                target: target.clone(),
                snapshot: TargetSnapshot::capture(&target),
                versions: vec![Version {
                    label: "Theirs".into(),
                    entry: Some(Entry::Link("missing-three".into())),
                    note: String::new(),
                    error: None,
                }],
                why: "symlink".into(),
                large_text: false,
                files: None,
                info: Vec::new(),
            },
            false,
        );
        picker.take(0);
        std::fs::remove_file(&target).unwrap();
        symlink("missing-two", &target).unwrap();
        picker.execute(PickAct::Save);
        assert_eq!(
            std::fs::read_link(&target).unwrap(),
            PathBuf::from("missing-two")
        );
        picker.execute(PickAct::Save);
        assert_eq!(
            std::fs::read_link(&target).unwrap(),
            PathBuf::from("missing-three")
        );
        std::fs::remove_file(&target).unwrap();
        std::fs::write(&target, b"same").unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755)).unwrap();
        picker.execute(PickAct::Reload);
        picker.o.versions[0].entry = Some(Entry::File {
            bytes: b"chosen".to_vec(),
            mode: Some(0o755),
        });
        picker.take(0);
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600)).unwrap();
        picker.execute(PickAct::Save);
        assert_eq!(std::fs::read(&target).unwrap(), b"same");
        picker.execute(PickAct::Save);
        assert_eq!(
            std::fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o755
        );
        std::fs::remove_file(&target).unwrap();
        std::fs::create_dir(&target).unwrap();
        assert!(picker.execute(PickAct::Save).error.is_some());
        assert!(target.is_dir());
    }
}
