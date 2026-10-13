//! Folders in the File Navigator, for both frontends: which are collapsed (an archive starts collapsed, since it may
//! hold any number of changed files), the rows that leaves visible, file stepping that skips hidden files, and the
//! Folder Summary - a folder's changed files with their added and removed lines - shown in place of a comparison.

use std::{
    collections::{BTreeSet, HashMap},
    ops::Range,
    sync::{Arc, Weak, mpsc},
};

use crate::{
    changeset::{ChangeSet, Comparison, Status},
    navigator::NavRow,
};

/// What a summary row says about one changed file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stat {
    /// Lines added and removed, counted as `git diff --numstat` counts them.
    Lines { added: u64, removed: u64 },
    /// Binary on either Side: each Side's size in bytes (0 for an absent one).
    Binary { sizes: [u64; 2] },
    /// Too large to count, as the comparison itself is: each Side's size in bytes.
    Large { sizes: [u64; 2] },
    /// The Sides could not be read.
    Failed(String),
}

/// A command about folders, from the Files pane, the Folder Summary or the command menus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FolderAction {
    /// Show the Folder Summary of the current file's folder.
    Summary,
    /// Collapse the shown summary's folder, or the current file's folder.
    Collapse,
    /// Expand the shown summary's folder, or the current file's folder.
    Expand,
    ExpandAll,
    /// Collapse every archive again, as when the Change Set opened.
    CollapseArchives,
}

impl FolderAction {
    pub const ALL: [Self; 5] = [Self::Summary, Self::Collapse, Self::Expand, Self::ExpandAll, Self::CollapseArchives];
    pub const fn label(self) -> &'static str {
        match self {
            Self::Summary => "Show folder summary",
            Self::Collapse => "Collapse folder",
            Self::Expand => "Expand folder",
            Self::ExpandAll => "Expand all folders",
            Self::CollapseArchives => "Collapse all archives",
        }
    }
}

/// One folder's changed files, shown in place of a comparison.
#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    /// The folder (`"."` for a whole group or ungrouped Change Set).
    pub folder: String,
    /// The Change Set group the folder heading was in; its range bounds the files.
    pub range: Range<usize>,
    /// The folder's files, as Comparison indices in File Navigator order.
    pub files: Vec<usize>,
    /// The row the keyboard is on, an index into `files`.
    pub selected: usize,
}

impl Summary {
    /// The heading: the folder, its file count and the totals so far, in parts.
    pub fn heading(&self, folders: &Folders) -> Vec<(String, Tone)> {
        let (mut added, mut removed, mut counted) = (0, 0, 0);
        for stat in self.files.iter().filter_map(|i| folders.stat(*i)) {
            counted += 1;
            if let Stat::Lines { added: a, removed: r } = stat {
                added += a;
                removed += r;
            }
        }
        let n = self.files.len();
        let mut parts = vec![(format!("{} · {n} changed file{}", self.folder, if n == 1 { "" } else { "s" }), Tone::Dim)];
        parts.push((format!("  +{added}"), Tone::Added));
        parts.push((format!(" −{removed}"), Tone::Removed));
        if counted < n {
            parts.push((format!("  counting {counted}/{n}…"), Tone::Dim));
        }
        parts
    }

    /// What the keys do, for the summary's last line.
    pub const HINT: &'static str = "Enter open · ← collapse · → expand · Esc back";

    /// `path` relative to the summary's folder, for its row.
    pub fn relative<'a>(&self, path: &'a str) -> &'a str {
        if self.folder == "." { path } else { path.strip_prefix(self.folder.as_str()).and_then(|p| p.strip_prefix('/')).unwrap_or(path) }
    }
}

/// How a part of a summary's text is coloured: added and removed counts take the status colours.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Added,
    Removed,
    Dim,
}

/// A file's count as the summary shows it, in parts.
pub fn stat_parts(stat: Option<&Stat>) -> Vec<(String, Tone)> {
    match stat {
        None => vec![("…".into(), Tone::Dim)],
        Some(Stat::Lines { added: 0, removed: 0 }) => vec![("no line changes".into(), Tone::Dim)],
        Some(Stat::Lines { added, removed }) => {
            let mut parts = Vec::new();
            if *added > 0 {
                parts.push((format!("+{added}"), Tone::Added));
            }
            if *removed > 0 {
                parts.push((format!("−{removed}"), Tone::Removed));
            }
            parts
        }
        Some(Stat::Binary { sizes: [l, r] }) => vec![(format!("binary {} → {}", bytes(*l), bytes(*r)), Tone::Dim)],
        Some(Stat::Large { .. }) => vec![("large file".into(), Tone::Dim)],
        Some(Stat::Failed(_)) => vec![("unreadable".into(), Tone::Dim)],
    }
}

/// `n` bytes for people: `412 B`, `3.2 KiB`, `14 MiB`.
fn bytes(n: u64) -> String {
    match n {
        0..1024 => format!("{n} B"),
        1024..1_048_576 => format!("{:.1} KiB", n as f64 / 1024.0),
        _ => format!("{:.1} MiB", n as f64 / 1_048_576.0),
    }
}

/// One background batch's counts, by Comparison index.
type Counted = Vec<(usize, Stat)>;

/// How many files one background batch counts: small, so the shared document worker never waits long behind it.
const BATCH: usize = 8;

/// Folder state for one view: kept across refreshes and Change Sets, so an archive the user opened stays open.
#[derive(Default)]
pub struct Folders {
    set: Weak<ChangeSet>,
    /// Collapsed folders by path; a collapsed folder hides every folder under it.
    collapsed: BTreeSet<String>,
    /// Archive roots seen in any Change Set this view showed: only a new one starts collapsed.
    archives: BTreeSet<String>,
    /// Bumps on every change to the rows, for adapters that cache them.
    revision: u64,
    stats: HashMap<usize, Stat>,
    summary: Option<Summary>,
    token: Arc<()>,
    job: Option<mpsc::Receiver<Option<Counted>>>,
}

impl Folders {
    /// Adopts `set` if it is new: its archives start collapsed unless one of the same path was seen before, counts are
    /// dropped, and a shown summary keeps its folder when that still has files. True when anything changed.
    pub fn sync(&mut self, set: &Arc<ChangeSet>) -> bool {
        if self.set.upgrade().is_some_and(|s| Arc::ptr_eq(&s, set)) {
            return false;
        }
        self.set = Arc::downgrade(set);
        for root in archive_roots(&set.comparisons) {
            if self.archives.insert(root.to_string()) {
                self.collapsed.insert(root.to_string());
            }
        }
        self.stats.clear();
        self.cancel();
        if let Some(summary) = self.summary.take() {
            let range = set.groups.iter().map(|(_, r)| r.clone()).find(|r| r.start == summary.range.start).unwrap_or(0..set.comparisons.len());
            self.summary = Self::summary_of(set, &summary.folder, range);
        }
        self.revision += 1;
        true
    }

    /// For adapters that cache [`Folders::rows`].
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// The File Navigator rows: groups, then folder headings (an archive's own heading even when no file sits directly
    /// in it) and their files, without what a collapsed folder hides.
    pub fn rows(&self, set: &ChangeSet) -> Vec<NavRow> {
        let ranges: Vec<(Option<&str>, Range<usize>)> = if set.groups.is_empty() {
            vec![(None, 0..set.comparisons.len())]
        } else {
            set.groups.iter().map(|(name, range)| (Some(name.as_str()), range.clone())).collect()
        };
        let roots = archive_roots(&set.comparisons);
        let mut out = Vec::new();
        for (name, range) in ranges {
            if let Some(name) = name {
                out.push(NavRow::Group(name.to_string(), range.len()));
            }
            let mut last: Option<&str> = None;
            let mut headed = BTreeSet::new();
            for index in range.clone() {
                let c = &set.comparisons[index];
                let dir = dir_of(&c.path);
                if last != Some(dir) {
                    last = Some(dir);
                    // an archive's heading leads its folders, so collapsing it has a row to click
                    let root = c.archive_root().filter(|root| *root != dir);
                    for dir in root.into_iter().chain([dir]) {
                        if headed.insert(dir) {
                            self.heading(set, &range, dir, roots.contains(dir), &mut out);
                        }
                    }
                }
                if !self.hidden_dir(dir) {
                    let from = matches!(c.status, Status::Renamed { .. }).then(|| c.old_path.clone());
                    out.push(NavRow::File { index, name: c.path.rsplit('/').next().unwrap_or(&c.path).to_string(), from });
                }
            }
        }
        out
    }

    fn heading(&self, set: &ChangeSet, range: &Range<usize>, dir: &str, archive: bool, out: &mut Vec<NavRow>) {
        let path = if dir.is_empty() { "." } else { dir };
        // a folder under a collapsed one has no row; the collapsed one itself does
        if self.collapsed.iter().any(|c| under(dir, c) && c != dir) {
            return;
        }
        let collapsed = self.collapsed.contains(dir);
        let files = if collapsed { set.comparisons[range.clone()].iter().filter(|c| under(dir_of(&c.path), dir)).count() } else { 0 };
        out.push(NavRow::Dir { path: path.to_string(), collapsed, files, archive });
    }

    fn hidden_dir(&self, dir: &str) -> bool {
        self.collapsed.iter().any(|c| under(dir, c))
    }

    /// Is file `index` hidden in a collapsed folder?
    pub fn hidden(&self, files: &[Comparison], index: usize) -> bool {
        files.get(index).is_some_and(|c| self.hidden_dir(dir_of(&c.path)))
    }

    pub fn collapsed(&self, folder: &str) -> bool {
        self.collapsed.contains(folder_key(folder))
    }

    /// Collapses or expands `folder` (a heading's path).
    pub fn set_collapsed(&mut self, folder: &str, collapsed: bool) {
        let key = folder_key(folder).to_string();
        let changed = if collapsed { self.collapsed.insert(key) } else { self.collapsed.remove(&key) };
        if changed {
            self.revision += 1;
        }
    }

    pub fn toggle(&mut self, folder: &str) {
        let collapsed = self.collapsed(folder);
        self.set_collapsed(folder, !collapsed);
    }

    /// Expands every folder hiding file `index`, so the file shown has its row.
    pub fn reveal(&mut self, files: &[Comparison], index: usize) {
        let Some(c) = files.get(index) else { return };
        let dir = dir_of(&c.path);
        let before = self.collapsed.len();
        self.collapsed.retain(|c| !under(dir, c));
        if self.collapsed.len() != before {
            self.revision += 1;
        }
    }

    /// The next (or previous) file after `from` that a collapsed folder doesn't hide.
    pub fn step(&self, files: &[Comparison], from: usize, forward: bool) -> Option<usize> {
        let shown = |i: &usize| !self.hidden(files, *i);
        if forward { (from.saturating_add(1)..files.len()).find(shown) } else { (0..from.min(files.len())).rev().find(shown) }
    }

    /// Where a newly shown Change Set starts: its first visible file, or, when every file is in a collapsed folder, the
    /// first file with that folder's summary shown over it.
    pub fn start(&mut self, set: &ChangeSet) -> usize {
        if set.comparisons.is_empty() || !self.hidden(&set.comparisons, 0) {
            return 0;
        }
        if let Some(index) = self.step(&set.comparisons, 0, true) {
            return index;
        }
        let dir = dir_of(&set.comparisons[0].path);
        if let Some(folder) = self.collapsed.iter().find(|c| under(dir, c)).cloned() {
            let range = set.groups.first().map_or(0..set.comparisons.len(), |(_, r)| r.clone());
            self.summary = Self::summary_of(set, &folder, range);
        }
        0
    }

    /// The summary shown in place of the comparison.
    pub fn summary(&self) -> Option<&Summary> {
        self.summary.as_ref()
    }

    /// A file's count, once the background work reached it.
    pub fn stat(&self, index: usize) -> Option<&Stat> {
        self.stats.get(&index)
    }

    /// Shows `folder`'s summary: the heading at row `row` of [`Folders::rows`] picks its group.
    pub fn show_summary(&mut self, set: &ChangeSet, folder: &str, group: Option<usize>, select: Option<usize>) {
        let range = group.and_then(|g| set.groups.get(g)).map_or(0..set.comparisons.len(), |(_, r)| r.clone());
        self.summary = Self::summary_of(set, folder, range);
        if let (Some(summary), Some(index)) = (&mut self.summary, select) {
            summary.selected = summary.files.iter().position(|&i| i == index).unwrap_or(0);
        }
        self.revision += 1;
    }

    /// The summary of the folder holding file `index`.
    pub fn show_summary_of(&mut self, set: &ChangeSet, index: usize) {
        let Some(c) = set.comparisons.get(index) else { return };
        let dir = dir_of(&c.path);
        let folder = if dir.is_empty() { "." } else { dir };
        let group = set.groups.iter().position(|(_, r)| r.contains(&index));
        self.show_summary(set, folder, group, Some(index));
    }

    pub fn close_summary(&mut self) {
        if self.summary.take().is_some() {
            self.cancel();
            self.revision += 1;
        }
    }

    /// Moves the summary's keyboard row by `delta`, clamped.
    pub fn select(&mut self, delta: isize) {
        if let Some(s) = &mut self.summary
            && !s.files.is_empty()
        {
            s.selected = s.selected.saturating_add_signed(delta).min(s.files.len() - 1);
        }
    }

    /// The selected summary row's file.
    pub fn selected_file(&self) -> Option<usize> {
        self.summary.as_ref().and_then(|s| s.files.get(s.selected).copied())
    }

    /// Opening a file from the summary: it closes, and the file's folders expand.
    pub fn open(&mut self, set: &ChangeSet, index: usize) {
        self.close_summary();
        self.reveal(&set.comparisons, index);
    }

    fn summary_of(set: &ChangeSet, folder: &str, range: Range<usize>) -> Option<Summary> {
        let key = folder_key(folder);
        let files: Vec<usize> = range.clone().filter(|&i| folder == "." || under(dir_of(&set.comparisons[i].path), key)).collect();
        (!files.is_empty()).then(|| Summary { folder: folder.to_string(), range, files, selected: 0 })
    }

    /// Why `action` can't run now.
    pub fn disabled(&self, action: FolderAction, set: Option<&ChangeSet>, current: usize) -> Option<&'static str> {
        let Some(set) = set.filter(|s| !s.comparisons.is_empty()) else { return Some("No changed files") };
        let folder = self.target(set, current);
        match action {
            FolderAction::Summary if self.summary.is_some() => Some("The folder summary is shown"),
            FolderAction::Summary => None,
            FolderAction::Collapse if folder.is_none() => Some("Files at the top level have no folder"),
            FolderAction::Collapse if folder.as_deref().is_some_and(|f| self.collapsed(f)) => Some("The folder is collapsed"),
            FolderAction::Expand if folder.is_none() => Some("Files at the top level have no folder"),
            FolderAction::Expand if folder.as_deref().is_some_and(|f| !self.collapsed(f)) => Some("The folder is expanded"),
            FolderAction::Collapse | FolderAction::Expand => None,
            FolderAction::ExpandAll if self.collapsed.is_empty() => Some("No folder is collapsed"),
            FolderAction::ExpandAll => None,
            FolderAction::CollapseArchives if archive_roots(&set.comparisons).iter().all(|r| self.collapsed.contains(*r)) => {
                Some("No archive is expanded")
            }
            FolderAction::CollapseArchives => None,
        }
    }

    /// The folder Collapse and Expand act on: the shown summary's, else the current file's (`None` at the top level).
    fn target(&self, set: &ChangeSet, current: usize) -> Option<String> {
        if let Some(s) = &self.summary {
            return (s.folder != ".").then(|| s.folder.clone());
        }
        let dir = dir_of(&set.comparisons.get(current)?.path);
        (!dir.is_empty()).then(|| dir.to_string())
    }

    /// Runs `action` against the current file `current`; a Collapse that hides the current file shows its folder's
    /// summary in its place.
    pub fn execute(&mut self, action: FolderAction, set: &ChangeSet, current: usize) -> Result<(), &'static str> {
        if let Some(reason) = self.disabled(action, Some(set), current) {
            return Err(reason);
        }
        match action {
            FolderAction::Summary => self.show_summary_of(set, current),
            FolderAction::Collapse => {
                let folder = self.target(set, current).ok_or("Files at the top level have no folder")?;
                self.set_collapsed(&folder, true);
                if self.summary.is_none() {
                    let group = set.groups.iter().position(|(_, r)| r.contains(&current));
                    self.show_summary(set, &folder, group, Some(current));
                }
            }
            FolderAction::Expand => {
                let folder = self.target(set, current).ok_or("Files at the top level have no folder")?;
                self.set_collapsed(&folder, false);
            }
            FolderAction::ExpandAll => {
                self.collapsed.clear();
                self.revision += 1;
            }
            FolderAction::CollapseArchives => {
                self.collapsed.extend(archive_roots(&set.comparisons).into_iter().map(str::to_string));
                self.revision += 1;
                if self.summary.is_none() && self.hidden(&set.comparisons, current) {
                    self.show_summary_of(set, current);
                }
            }
        }
        Ok(())
    }

    /// Counts the next batch of the shown summary's files in the background; call on every poll. True when counts
    /// arrived.
    pub fn poll(&mut self, set: &Arc<ChangeSet>) -> bool {
        let mut changed = false;
        if let Some(rx) = &self.job {
            match rx.try_recv() {
                Ok(Some(stats)) => {
                    self.stats.extend(stats);
                    self.job = None;
                    changed = true;
                }
                Ok(None) | Err(mpsc::TryRecvError::Disconnected) => self.job = None,
                Err(mpsc::TryRecvError::Empty) => return false,
            }
        }
        let Some(summary) = &self.summary else { return changed };
        if !self.set.upgrade().is_some_and(|s| Arc::ptr_eq(&s, set)) {
            return changed;
        }
        // the rows around the keyboard first, then the rest in order
        let from = summary.selected.saturating_sub(BATCH);
        let todo: Vec<usize> = summary.files[from..].iter().chain(&summary.files[..from]).copied().filter(|i| !self.stats.contains_key(i)).take(BATCH).collect();
        if !todo.is_empty() {
            let set = set.clone();
            self.job = Some(crate::work::submit(Arc::downgrade(&self.token), move || todo.into_iter().map(|i| (i, count(&set, i))).collect()));
        }
        changed
    }

    /// Counting is under way.
    pub fn busy(&self) -> bool {
        self.job.is_some()
    }

    fn cancel(&mut self) {
        self.token = Arc::new(());
        self.job = None;
    }
}

/// File `index`'s added and removed lines, or why it has none.
pub fn count(set: &ChangeSet, index: usize) -> Stat {
    let sizes = match set.sizes(index) {
        Ok(sizes) => sizes,
        Err(e) => return Stat::Failed(e),
    };
    if sizes.iter().any(|&s| s > crate::document::LARGE) {
        return Stat::Large { sizes };
    }
    let [left, right] = match set.sides(index) {
        Ok(sides) => sides,
        Err(e) => return Stat::Failed(e),
    };
    if left.binary || right.binary {
        return Stat::Binary { sizes };
    }
    let (left, right) = (String::from_utf8_lossy(&left.bytes), String::from_utf8_lossy(&right.bytes));
    let (left, right) = (lines_of(&left), lines_of(&right));
    let blocks = lenz_engine::diff_lines(&left, &right);
    let added = blocks.iter().map(|b| u64::from(b.right.end - b.right.start)).sum();
    let removed = blocks.iter().map(|b| u64::from(b.left.end - b.left.start)).sum();
    Stat::Lines { added, removed }
}

/// Lines as git counts them: a final newline ends the last line rather than starting an empty one.
fn lines_of(text: &str) -> Vec<&str> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut lines = lenz_engine::split_lines(text);
    if text.ends_with('\n') {
        lines.pop();
    }
    lines
}

/// The folder part of `path` (empty at the top level).
fn dir_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(dir, _)| dir)
}

/// A heading's path as stored: `"."` is the top level.
fn folder_key(folder: &str) -> &str {
    if folder == "." { "" } else { folder }
}

/// Is folder `dir` `folder` or inside it?
fn under(dir: &str, folder: &str) -> bool {
    folder.is_empty() && dir.is_empty() || !folder.is_empty() && (dir == folder || dir.strip_prefix(folder).is_some_and(|rest| rest.starts_with('/')))
}

/// The outermost archive of each file inside one, as its rows show it.
fn archive_roots(comparisons: &[Comparison]) -> BTreeSet<&str> {
    comparisons.iter().filter_map(Comparison::archive_root).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::testing::zip;
    use crate::changeset::Spec;
    use crate::git::testing::{commit_all, repo, write};

    fn set_with_archive() -> (tempfile::TempDir, Arc<ChangeSet>) {
        let r = repo();
        write(r.path(), "pkg/app.zip", zip(&[("src/a.rs", b"one\ntwo\n"), ("b.txt", b"b\n")]));
        write(r.path(), "z.txt", "z\n");
        write(r.path(), "pkg/top.txt", "t\n");
        commit_all(r.path(), "1");
        write(r.path(), "pkg/app.zip", zip(&[("src/a.rs", b"one\n2\nthree\n"), ("c.bin", b"\0\x01")]));
        write(r.path(), "z.txt", "zz\n");
        write(r.path(), "pkg/top.txt", "tt\n");
        commit_all(r.path(), "2");
        let spec = Spec::Git { cached: false, revs: vec!["HEAD~".into(), "HEAD".into()], paths: vec![], untracked: false };
        let set = Arc::new(ChangeSet::load(&spec, r.path()).unwrap());
        (r, set)
    }

    fn paths(set: &ChangeSet) -> Vec<&str> {
        set.comparisons.iter().map(|c| c.path.as_str()).collect()
    }

    fn labels(rows: &[NavRow]) -> Vec<String> {
        rows.iter()
            .map(|r| match r {
                NavRow::Group(name, n) => format!("{name} ({n})"),
                NavRow::Dir { path, collapsed: true, files, .. } => format!("▸ {path} · {files}"),
                NavRow::Dir { path, .. } => format!("▾ {path}"),
                NavRow::File { name, .. } => name.clone(),
            })
            .collect()
    }

    #[test]
    fn archives_start_collapsed_with_their_file_count_and_ordinary_folders_open() {
        let (_r, set) = set_with_archive();
        assert_eq!(paths(&set), ["z.txt", "pkg/top.txt", "pkg/app.zip/b.txt", "pkg/app.zip/c.bin", "pkg/app.zip/src/a.rs"]);
        let mut folders = Folders::default();
        assert!(folders.sync(&set));
        assert!(!folders.sync(&set), "the same Change Set again changes nothing");
        assert_eq!(labels(&folders.rows(&set)), ["▾ .", "z.txt", "▾ pkg", "top.txt", "▸ pkg/app.zip · 3"]);
        assert!(folders.hidden(&set.comparisons, 4) && !folders.hidden(&set.comparisons, 1));
        // expanded, the archive heading leads its folders even with no file directly in it
        folders.toggle("pkg/app.zip");
        assert_eq!(labels(&folders.rows(&set)), ["▾ .", "z.txt", "▾ pkg", "top.txt", "▾ pkg/app.zip", "b.txt", "c.bin", "▾ pkg/app.zip/src", "a.rs"]);
        // collapsing an ordinary folder hides the folders under it, the archive among them
        folders.toggle("pkg");
        assert_eq!(labels(&folders.rows(&set)), ["▾ .", "z.txt", "▸ pkg · 4"]);
    }

    #[test]
    fn stepping_skips_hidden_files_and_revealing_expands_their_folders() {
        let (_r, set) = set_with_archive();
        let mut folders = Folders::default();
        folders.sync(&set);
        assert_eq!(folders.step(&set.comparisons, 0, true), Some(1));
        assert_eq!(folders.step(&set.comparisons, 1, true), None, "the archive's files are hidden");
        assert_eq!(folders.step(&set.comparisons, 4, false), Some(1));
        folders.reveal(&set.comparisons, 4);
        assert!(!folders.collapsed("pkg/app.zip"));
        assert_eq!(folders.step(&set.comparisons, 1, true), Some(2));
    }

    #[test]
    fn an_archive_the_user_expanded_stays_expanded_in_the_next_change_set() {
        let (r, set) = set_with_archive();
        let mut folders = Folders::default();
        folders.sync(&set);
        folders.set_collapsed("pkg/app.zip", false);
        let spec = Spec::Git { cached: false, revs: vec!["HEAD~".into(), "HEAD".into()], paths: vec![], untracked: false };
        let again = Arc::new(ChangeSet::load(&spec, r.path()).unwrap());
        assert!(folders.sync(&again));
        assert!(!folders.collapsed("pkg/app.zip"));
    }

    #[test]
    fn a_change_set_of_only_archive_files_starts_on_the_archive_summary() {
        let r = repo();
        write(r.path(), "a.zip", zip(&[("x.txt", b"1\n")]));
        commit_all(r.path(), "1");
        write(r.path(), "a.zip", zip(&[("x.txt", b"2\n"), ("y.txt", b"y\n")]));
        commit_all(r.path(), "2");
        let spec = Spec::Git { cached: false, revs: vec!["HEAD~".into(), "HEAD".into()], paths: vec![], untracked: false };
        let set = Arc::new(ChangeSet::load(&spec, r.path()).unwrap());
        let mut folders = Folders::default();
        folders.sync(&set);
        assert_eq!(folders.start(&set), 0);
        let summary = folders.summary().unwrap();
        assert_eq!((summary.folder.as_str(), summary.files.as_slice()), ("a.zip", &[0, 1][..]));
        assert_eq!(summary.relative(&set.comparisons[1].path), "y.txt");
        // a set with a visible file starts on it instead
        let (_r, set) = set_with_archive();
        let mut folders = Folders::default();
        folders.sync(&set);
        assert_eq!(folders.start(&set), 0);
        assert!(folders.summary().is_none());
    }

    #[test]
    fn summary_counts_lines_like_numstat_and_sizes_for_binary_files() {
        let (_r, set) = set_with_archive();
        let mut folders = Folders::default();
        folders.sync(&set);
        folders.show_summary(&set, "pkg/app.zip", None, Some(4));
        let summary = folders.summary().unwrap().clone();
        assert_eq!(summary.files, [2, 3, 4]);
        assert_eq!(summary.selected, 2);
        assert_eq!(summary.relative("pkg/app.zip/src/a.rs"), "src/a.rs");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while summary.files.iter().any(|&i| folders.stat(i).is_none()) {
            folders.poll(&set);
            assert!(std::time::Instant::now() < deadline, "counts did not arrive");
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert_eq!(folders.stat(4), Some(&Stat::Lines { added: 2, removed: 1 }));
        assert_eq!(folders.stat(2), Some(&Stat::Lines { added: 0, removed: 1 }));
        assert_eq!(folders.stat(3), Some(&Stat::Binary { sizes: [0, 2] }));
        // the summary of the top level lists every file
        folders.show_summary(&set, ".", None, None);
        assert_eq!(folders.summary().unwrap().files.len(), 5);
        folders.close_summary();
        assert!(folders.summary().is_none() && !folders.busy());
    }

    #[test]
    fn commands_act_on_the_summary_folder_or_the_current_file_with_reasons() {
        let (_r, set) = set_with_archive();
        let mut folders = Folders::default();
        folders.sync(&set);
        assert_eq!(folders.disabled(FolderAction::Collapse, Some(&set), 0), Some("Files at the top level have no folder"));
        assert_eq!(folders.disabled(FolderAction::Expand, Some(&set), 1), Some("The folder is expanded"));
        assert_eq!(folders.disabled(FolderAction::CollapseArchives, Some(&set), 1), Some("No archive is expanded"));
        assert_eq!(folders.disabled(FolderAction::Summary, None, 0), Some("No changed files"));
        // collapsing the current file's folder hides it, so its summary takes its place
        folders.execute(FolderAction::Collapse, &set, 1).unwrap();
        assert!(folders.collapsed("pkg") && folders.hidden(&set.comparisons, 1));
        assert_eq!(folders.summary().map(|s| (s.folder.as_str(), s.selected)), Some(("pkg", 0)));
        assert_eq!(folders.execute(FolderAction::Collapse, &set, 1), Err("The folder is collapsed"));
        folders.execute(FolderAction::Expand, &set, 1).unwrap();
        assert!(!folders.collapsed("pkg"));
        folders.select(1);
        assert_eq!(folders.selected_file(), Some(2));
        folders.open(&set, 4);
        assert!(folders.summary().is_none() && !folders.hidden(&set.comparisons, 4));
        folders.execute(FolderAction::CollapseArchives, &set, 1).unwrap();
        assert!(folders.collapsed("pkg/app.zip"));
        folders.execute(FolderAction::ExpandAll, &set, 1).unwrap();
        assert_eq!(folders.disabled(FolderAction::ExpandAll, Some(&set), 1), Some("No folder is collapsed"));
    }

    /// Polls until every file of the shown summary has its count.
    fn counted(folders: &mut Folders, set: &Arc<ChangeSet>) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while folders.summary().is_some_and(|s| s.files.iter().any(|&i| folders.stat(i).is_none())) {
            folders.poll(set);
            assert!(std::time::Instant::now() < deadline, "counts did not arrive");
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    #[test]
    fn counts_from_a_replaced_change_set_never_reach_the_new_one() {
        let (r, set) = set_with_archive();
        let mut folders = Folders::default();
        folders.sync(&set);
        // the shared worker is busy, so the summary's first batch waits behind it
        let owner = Arc::new(());
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let held = crate::work::submit(Arc::downgrade(&owner), move || {
            started_tx.send(()).unwrap();
            let _ = release_rx.recv_timeout(std::time::Duration::from_secs(15));
        });
        started_rx.recv().unwrap();
        folders.show_summary(&set, "pkg/app.zip", None, None);
        folders.poll(&set);
        assert!(folders.busy());
        // a refresh: the same files at the same indices, with other content
        write(r.path(), "pkg/app.zip", zip(&[("src/a.rs", b"one\n"), ("c.bin", b"\0\x01\x02")]));
        commit_all(r.path(), "3");
        let spec = Spec::Git { cached: false, revs: vec!["HEAD~2".into(), "HEAD".into()], paths: vec![], untracked: false };
        let fresh = Arc::new(ChangeSet::load(&spec, r.path()).unwrap());
        assert_eq!(paths(&fresh), paths(&set));
        assert!(folders.sync(&fresh));
        assert!(!folders.busy(), "the stale batch is cancelled");
        assert_eq!(folders.summary().map(|s| s.folder.as_str()), Some("pkg/app.zip"), "the folder still has files");
        release_tx.send(()).unwrap();
        let _ = held.recv();
        counted(&mut folders, &fresh);
        for &i in &folders.summary().unwrap().files.clone() {
            assert_eq!(folders.stat(i), Some(&count(&fresh, i)), "{}", fresh.comparisons[i].path);
        }
        assert_eq!(folders.stat(4), Some(&Stat::Lines { added: 0, removed: 1 }));
        assert_ne!(Some(&count(&set, 4)), folders.stat(4));
    }

    #[test]
    fn a_summary_covers_only_its_own_group() {
        let r = repo();
        write(r.path(), "a.zip", zip(&[("f.txt", b"1\n")]));
        commit_all(r.path(), "1");
        write(r.path(), "a.zip", zip(&[("f.txt", b"2\n")]));
        crate::git::testing::git(r.path(), &["add", "a.zip"]);
        write(r.path(), "a.zip", zip(&[("f.txt", b"3\n"), ("g.txt", b"g\n")]));
        let set = Arc::new(crate::repo::local_changes(r.path(), false).unwrap());
        let mut folders = Folders::default();
        folders.sync(&set);
        // each group has its own collapsed heading, with that group's count
        assert_eq!(labels(&folders.rows(&set)), ["Staged (1)", "▸ a.zip · 1", "Unstaged (2)", "▸ a.zip · 2"]);
        folders.show_summary(&set, "a.zip", Some(1), None);
        assert_eq!(folders.summary().map(|s| s.files.clone()), Some(vec![1, 2]));
        folders.show_summary(&set, "a.zip", Some(0), None);
        assert_eq!(folders.summary().map(|s| s.files.clone()), Some(vec![0]));
        // collapsing is by folder: one toggle opens the archive in both groups
        folders.toggle("a.zip");
        assert_eq!(labels(&folders.rows(&set)), ["Staged (1)", "▾ a.zip", "f.txt", "Unstaged (2)", "▾ a.zip", "f.txt", "g.txt"]);
    }

    #[test]
    fn under_matches_whole_path_components() {
        assert!(under("a/b", "a") && under("a", "a") && !under("ab", "a") && under("", "") && !under("a", ""));
    }
}
