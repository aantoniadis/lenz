//! The browsing root: its file inventory, discovered progressively, the project tree built from it and the fuzzy
//! file finder over it. Discovery coverage (how much of the root has been enumerated) is separate from a finder
//! query's completion (whether ranking finished over the files known when it ran).

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

/// Files sent per discovery batch.
pub const BATCH: usize = 1024;
/// Finder results kept, best first.
pub const FINDER_LIMIT: usize = 200;

/// One discovered file, by its `/`-separated path relative to the root.
#[derive(Clone, Debug)]
pub struct Entry {
    pub path: Arc<str>,
    lower: Box<str>,
}

impl Entry {
    pub fn new(path: &str) -> Self {
        Self { path: path.into(), lower: path.to_lowercase().into() }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Coverage {
    /// Files found so far; the walk continues.
    Scanning(usize),
    Complete(usize),
    Unavailable(String),
}

/// One row of the project tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeRow {
    pub depth: usize,
    pub name: Arc<str>,
    /// Stable identity: project-relative path, absolute library path, or a library/source URI.
    pub path: Arc<str>,
    /// `Some(expanded)` for a directory.
    pub dir: Option<bool>,
}

impl TreeRow {
    /// Hidden: its name starts with a dot, or it is inside a folder whose name does.
    pub fn hidden(&self) -> bool {
        if Path::new(self.path.as_ref()).is_absolute() { self.name.starts_with('.') } else { self.path.split('/').any(|part| part.starts_with('.')) }
    }
}

/// A finder query's state: its results over the files known when it ran.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finder {
    pub query: String,
    /// The best matches, at most [`FINDER_LIMIT`].
    pub hits: Vec<Arc<str>>,
    /// All matches, of which `hits` are the best.
    pub matched: usize,
    /// Files the shown hits were ranked over.
    pub searched: usize,
    /// Ranking finished for the current query over the current inventory.
    pub ready: bool,
}

impl Finder {
    /// The line (and column) a `name:line[:column]` query asks for; the name alone is ranked.
    pub fn at(&self) -> Option<lenz_content::goto::LineColumn> {
        lenz_content::goto::split_path_line(&self.query).1
    }
}

pub struct Project {
    root: PathBuf,
    generation: u64,
    /// The walk whose batches are accepted: the first scan, then each rescan after changes.
    pub(crate) walk: u64,
    /// A rescan's inventory, swapped in whole when its walk completes (the shown inventory stays complete meanwhile).
    incoming: Option<Incoming>,
    coverage: Coverage,
    /// Entries the walk could not read (permissions, races); a complete scan with skips says so.
    skipped: usize,
    /// Discovery order in shared batches; ranking ties keep it. A ranking job clones the list, not the files.
    entries: Vec<Arc<[Entry]>>,
    files: usize,
    /// Sorted paths for the tree, and batches not yet merged into it (merged when they reach a quarter of it,
    /// so a whole scan sorts in O(n log n)).
    sorted: Vec<Arc<str>>,
    pending: Vec<Arc<str>>,
    /// Folders created here that hold no inventoried file yet: they are shown (and watched) all the same.
    empty: HashSet<Arc<str>>,
    expanded: HashSet<Arc<str>>,
    /// Each folder lists its folders before its files (else all by name).
    folders_first: bool,
    rows: Vec<TreeRow>,
    pub(crate) finder: Option<Finder>,
    /// A ranking job runs for this query sequence number.
    pub(crate) ranking: Option<u64>,
    pub(crate) query_seq: u64,
}

impl Project {
    pub(crate) fn new(root: PathBuf, generation: u64, folders_first: bool) -> Self {
        Self {
            root,
            generation,
            walk: generation,
            incoming: None,
            coverage: Coverage::Scanning(0),
            skipped: 0,
            entries: Vec::new(),
            files: 0,
            sorted: Vec::new(),
            pending: Vec::new(),
            empty: HashSet::new(),
            expanded: HashSet::new(),
            folders_first,
            rows: Vec::new(),
            finder: None,
            ranking: None,
            query_seq: 0,
        }
    }

    pub(crate) fn expanded(&self) -> Vec<String> { let mut dirs: Vec<_> = self.expanded.iter().map(|p| p.to_string()).collect(); dirs.sort(); dirs }
    pub(crate) fn restore_expanded(&mut self, dirs: Vec<String>) { self.expanded = dirs.into_iter().map(Into::into).collect(); self.rebuild(); }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn coverage(&self) -> &Coverage {
        &self.coverage
    }

    pub fn files(&self) -> usize {
        self.files
    }

    /// Whether a file with extension `ext` is in the inventory (a pass over it).
    pub fn has_extension(&self, ext: &str) -> bool {
        self.entries.iter().flat_map(|b| b.iter()).any(|e| e.path.rsplit_once('.').is_some_and(|(_, x)| x == ext))
    }

    /// Entries the walk skipped as unreadable.
    pub fn skipped(&self) -> usize {
        self.skipped
    }

    /// The visible tree: directories expanded where toggled, everything under a collapsed one skipped.
    pub fn rows(&self) -> &[TreeRow] {
        &self.rows
    }

    pub fn finder(&self) -> Option<&Finder> {
        self.finder.as_ref()
    }

    /// Every inventoried path, in discovery order (shared, not copied).
    pub(crate) fn paths(&self) -> Vec<Arc<str>> {
        self.entries.iter().flat_map(|b| b.iter().map(|e| e.path.clone())).collect()
    }

    pub(crate) fn entries(&self) -> Vec<Arc<[Entry]>> {
        self.entries.clone()
    }

    /// The folders holding inventoried files, relative to the root (ignored folders never appear).
    pub fn dirs(&self) -> impl Iterator<Item = &str> {
        folders(&self.sorted)
    }

    /// The inventory's paths (shared, not copied) and the known empty folders as `dir/`, sorted: the tree's rows
    /// and the folders watched come from the same list.
    pub(crate) fn sorted(&self) -> Vec<Arc<str>> {
        if self.empty.is_empty() { return self.sorted.clone(); }
        let mut all: Vec<Arc<str>> = self.sorted.iter().cloned().chain(self.empty.iter().map(|d| format!("{d}/").into())).collect();
        all.sort_unstable();
        all
    }

    /// A file found without a rescan (created here): shown at once, its folders open.
    pub(crate) fn add_file(&mut self, rel: Arc<str>) {
        if let Some((dir, _)) = rel.rsplit_once('/') {
            for dir in ancestors(dir) {
                self.expanded.insert(dir.into());
            }
        }
        if let Err(at) = self.sorted.binary_search(&rel) {
            self.sorted.insert(at, rel.clone());
            self.entries.push(Arc::from([Entry::new(&rel)]));
            self.files += 1;
            if let Some(f) = &mut self.finder { f.ready = false; }
        }
        self.rebuild();
    }

    /// A folder created here: shown at once, open, and kept until a rescan finds it gone.
    pub(crate) fn add_folder(&mut self, rel: Arc<str>) {
        for dir in ancestors(&rel) {
            self.expanded.insert(dir.into());
        }
        if self.empty.insert(rel) {
            self.rebuild();
        }
    }

    /// `path`'s place in the tree: relative to the root, `/`-separated (`None` outside the root).
    pub fn relative(&self, path: &Path) -> Option<Arc<str>> {
        let rel = path.strip_prefix(&self.root).ok()?;
        Some(rel.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/").into())
    }

    /// Whether the tree holds file `rel`: a binary search.
    pub(crate) fn has_file(&self, rel: &str) -> bool {
        self.sorted.binary_search_by(|p| p.as_ref().cmp(rel)).is_ok()
    }

    /// Whether folder `rel` (relative, `/`-separated) holds a file of the tree: a binary search.
    pub(crate) fn has_dir(&self, rel: &str) -> bool {
        if self.empty.contains(rel) { return true; }
        let prefix = format!("{rel}/");
        let i = self.sorted.partition_point(|p| p.as_ref() < prefix.as_str());
        self.sorted.get(i).is_some_and(|p| p.starts_with(&prefix))
    }
}

/// `rel` and the folders above it (`/`-separated), innermost first.
fn ancestors(rel: &str) -> impl Iterator<Item = &str> {
    std::iter::successors(Some(rel), |dir| dir.rsplit_once('/').map(|(up, _)| up))
}

/// The folders of sorted relative file paths (each ancestor once).
pub(crate) fn folders(sorted: &[Arc<str>]) -> impl Iterator<Item = &str> {
    let mut last: Option<&str> = None;
    sorted.iter().filter_map(|p| p.rfind('/').map(|i| &p[..i])).flat_map(|dir| {
        // Each ancestor of a file's folder is a folder too; sorted order lets consecutive repeats be skipped.
        let ancestors: Vec<&str> = std::iter::successors(Some(dir), |d| d.rfind('/').map(|i| &d[..i])).collect();
        ancestors.into_iter().rev()
    }).filter(move |d| {
        let new = last != Some(*d);
        if new {
            last = Some(d);
        }
        new
    })
}

impl Project {
    /// Whether a rescan is collecting a fresh inventory.
    pub fn refreshing(&self) -> bool {
        self.incoming.is_some()
    }

    /// Start collecting a fresh inventory from walk `walk`; the current one stays shown until it completes.
    pub(crate) fn begin_rescan(&mut self, walk: u64) {
        self.walk = walk;
        self.incoming = Some(Incoming::default());
    }

    pub(crate) fn add(&mut self, batch: Batch) {
        if let Some(incoming) = &mut self.incoming {
            let done = batch.done;
            incoming.add(batch);
            if done && let Some(fresh) = self.incoming.take() {
                self.replace(fresh);
            }
            return;
        }
        let Batch { paths, done, skipped, error } = batch;
        let entries: Arc<[Entry]> = paths.iter().map(|p| Entry::new(p)).collect();
        self.pending.extend(entries.iter().map(|e| e.path.clone()));
        self.files += entries.len();
        if !entries.is_empty() {
            self.entries.push(entries);
        }
        self.skipped += skipped;
        if done || self.pending.len() * 4 >= self.sorted.len() {
            let mut pending = std::mem::take(&mut self.pending);
            pending.sort_unstable();
            self.sorted = merge(std::mem::take(&mut self.sorted), pending);
        }
        let n = self.files;
        self.coverage = match error {
            Some(e) if n == 0 => Coverage::Unavailable(e),
            Some(_) => {
                self.skipped += 1;
                if done { Coverage::Complete(n) } else { Coverage::Scanning(n) }
            }
            None if done => Coverage::Complete(n),
            None => Coverage::Scanning(n),
        };
        if let Some(f) = &mut self.finder {
            f.ready = false; // new files: the shown results cover fewer than are known now
        }
        self.rebuild();
    }

    fn replace(&mut self, fresh: Incoming) {
        let Incoming { entries, mut paths, files, skipped, error } = fresh;
        paths.sort_unstable();
        self.entries = entries;
        self.files = files;
        self.skipped = skipped + usize::from(error.is_some() && files > 0);
        self.sorted = paths;
        self.pending.clear();
        let root = self.root.clone();
        self.empty.retain(|d| root.join(d.as_ref()).is_dir());
        self.coverage = match error {
            Some(e) if files == 0 => Coverage::Unavailable(e),
            _ => Coverage::Complete(files),
        };
        if let Some(f) = &mut self.finder {
            f.ready = false;
        }
        self.rebuild();
    }

    pub(crate) fn toggle(&mut self, dir: &str) -> bool {
        let is_dir = self.rows.iter().any(|r| &*r.path == dir && r.dir.is_some());
        if !is_dir {
            return false;
        }
        if !self.expanded.remove(dir) {
            // Opening a folder whose only entry is a folder opens that one too, down to a folder with more.
            let mut dir: Arc<str> = dir.into();
            while let Some(only) = self.only_folder(&dir) {
                self.expanded.insert(std::mem::replace(&mut dir, only));
            }
            self.expanded.insert(dir);
        }
        self.rebuild();
        true
    }

    /// The single entry of folder `dir` when it is a folder: two binary searches per entry, at most two entries.
    fn only_folder(&self, dir: &str) -> Option<Arc<str>> {
        let prefix = format!("{dir}/");
        let start = self.sorted.partition_point(|p| p.as_ref() < prefix.as_str());
        let first = self.sorted.get(start).filter(|p| p.starts_with(&prefix))?;
        let name = first[prefix.len()..].split_once('/')?.0;
        let inner = format!("{prefix}{name}/");
        let end = start + self.sorted[start..].partition_point(|p| p.starts_with(&inner));
        let alone = !self.sorted.get(end).is_some_and(|p| p.starts_with(&prefix));
        alone.then(|| inner[..inner.len() - 1].into())
    }

    /// The direct entries of folder `dir` (project-relative, `""` for the root): names and whether each is a folder,
    /// folders first while that order is set.
    pub fn children(&self, dir: &str) -> Vec<(Arc<str>, bool)> {
        let prefix = if dir.is_empty() { String::new() } else { format!("{dir}/") };
        let start = self.sorted.partition_point(|p| p.as_ref() < prefix.as_str());
        let mut out: Vec<(Arc<str>, bool)> = Vec::new();
        for p in self.sorted.get(start..).unwrap_or_default().iter().take_while(|p| p.starts_with(&prefix)) {
            let rest = p.get(prefix.len()..).unwrap_or_default();
            let (name, dir) = rest.split_once('/').map_or((rest, false), |(n, _)| (n, true));
            if name.is_empty() {
                continue; // a folder known without files stands in for itself
            }
            if !out.last().is_some_and(|(n, _)| &**n == name) {
                out.push((name.into(), dir));
            }
        }
        if self.folders_first {
            out.sort_by_key(|(_, dir)| !dir);
        }
        out
    }

    /// Open folder `rel` and every folder above it; `false` when the tree has no such folder.
    pub(crate) fn reveal_folder(&mut self, rel: &str) -> bool {
        if !self.has_dir(rel) {
            return false;
        }
        for dir in std::iter::successors(Some(rel), |d| d.rfind('/').map(|i| &d[..i])) {
            if !self.expanded.contains(dir) {
                self.expanded.insert(dir.into());
            }
        }
        self.rebuild();
        true
    }

    /// Open every folder above `rel` so its row shows; `false` when the tree has no such file.
    pub(crate) fn reveal(&mut self, rel: &str) -> bool {
        if !self.has_file(rel) {
            return false;
        }
        for dir in std::iter::successors(rel.rfind('/').map(|i| &rel[..i]), |d| d.rfind('/').map(|i| &d[..i])) {
            if !self.expanded.contains(dir) {
                self.expanded.insert(dir.into());
            }
        }
        self.rebuild();
        true
    }

    pub(crate) fn set_folders_first(&mut self, on: bool) {
        if self.folders_first != on {
            self.folders_first = on;
            self.rebuild();
        }
    }

    fn rebuild(&mut self) {
        let mut rows = Vec::new();
        let mut i = 0;
        let paths = self.sorted();
        walk(&paths, &self.expanded, self.folders_first, "", 0, &mut i, &mut rows);
        self.rows = rows;
    }
}

/// Rows under `prefix` (which ends in `/` unless it is the root) starting at `sorted[*i]`; with `folders_first`
/// a folder's files follow its folders.
fn walk(sorted: &[Arc<str>], expanded: &HashSet<Arc<str>>, folders_first: bool, prefix: &str, depth: usize, i: &mut usize, rows: &mut Vec<TreeRow>) {
    let mut files = Vec::new();
    while *i < sorted.len() && sorted[*i].starts_with(prefix) {
        if sorted[*i].as_ref() == prefix {
            // A folder known without files stands in for itself.
            *i += 1;
            continue;
        }
        let rest = &sorted[*i][prefix.len()..];
        match rest.find('/') {
            Some(slash) => {
                let dir = format!("{prefix}{}", &rest[..slash]);
                let open = expanded.contains(dir.as_str());
                rows.push(TreeRow { depth, name: rest[..slash].into(), path: dir.as_str().into(), dir: Some(open) });
                let inner = format!("{dir}/");
                if open {
                    walk(sorted, expanded, folders_first, &inner, depth + 1, i, rows);
                } else {
                    // Everything under a collapsed directory is contiguous in sorted order.
                    *i += sorted[*i..].partition_point(|p| p.starts_with(inner.as_str()));
                }
            }
            None => {
                let row = TreeRow { depth, name: rest.into(), path: sorted[*i].clone(), dir: None };
                if folders_first { files.push(row) } else { rows.push(row) }
                *i += 1;
            }
        }
    }
    rows.append(&mut files);
}

fn merge(a: Vec<Arc<str>>, b: Vec<Arc<str>>) -> Vec<Arc<str>> {
    if a.is_empty() {
        return b;
    }
    let mut out = Vec::with_capacity(a.len() + b.len());
    let (mut a, mut b) = (a.into_iter().peekable(), b.into_iter().peekable());
    while let (Some(x), Some(y)) = (a.peek(), b.peek()) {
        out.push(if x <= y { a.next() } else { b.next() }.unwrap_or_else(|| unreachable!()));
    }
    out.extend(a);
    out.extend(b);
    out
}

#[derive(Default)]
struct Incoming {
    entries: Vec<Arc<[Entry]>>,
    paths: Vec<Arc<str>>,
    files: usize,
    skipped: usize,
    error: Option<String>,
}

impl Incoming {
    fn add(&mut self, batch: Batch) {
        let entries: Arc<[Entry]> = batch.paths.iter().map(|p| Entry::new(p)).collect();
        self.paths.extend(entries.iter().map(|e| e.path.clone()));
        self.files += entries.len();
        if !entries.is_empty() {
            self.entries.push(entries);
        }
        self.skipped += batch.skipped;
        self.error = self.error.take().or(batch.error);
    }
}

/// One discovery batch: paths relative to the root, whether the walk ended, entries it could not read, and a
/// failure to read the root itself.
#[derive(Debug, Default)]
pub struct Batch {
    pub paths: Vec<String>,
    pub done: bool,
    pub skipped: usize,
    pub error: Option<String>,
}

/// The best [`FINDER_LIMIT`] paths in `batches` for `query`, by Lenz's file ranking (ties keep discovery order),
/// and how many matched in all. `None` once `current` says the query or project was superseded.
pub fn rank(batches: &[Arc<[Entry]>], query: &str, current: impl Fn() -> bool) -> Option<(Vec<Arc<str>>, usize)> {
    let q = query.to_lowercase();
    let mut hits: Vec<(i64, &Arc<str>)> = Vec::new();
    let mut matched = 0;
    for (i, e) in batches.iter().flat_map(|b| b.iter()).enumerate() {
        if i % 4096 == 0 && !current() {
            return None;
        }
        if let Some(s) = lenz_content::fuzzy::score_lowered(&q, &e.lower, e.path.len()) {
            matched += 1;
            hits.push((s, &e.path));
            // Keep memory bounded on broad queries: drop all but the best periodically.
            if hits.len() >= FINDER_LIMIT * 8 {
                keep_best(&mut hits);
            }
        }
    }
    keep_best(&mut hits);
    Some((hits.into_iter().map(|(_, p)| p.clone()).collect(), matched))
}

/// The best [`FINDER_LIMIT`] of `hits`, best first; a stable sort keeps discovery order among equal scores.
fn keep_best(hits: &mut Vec<(i64, &Arc<str>)>) {
    hits.sort_by_key(|h| std::cmp::Reverse(h.0));
    hits.truncate(FINDER_LIMIT);
}

/// Enumerate `root` in batches: ignore rules obeyed (also outside Git), hidden files kept unless ignored, `.git`
/// skipped, symlinked directories not followed (symlinked files are listed). Stops when `generation` moves on.
pub fn scan(root: &Path, generation: u64, latest: &AtomicU64, mut send: impl FnMut(Batch)) {
    let failed = |e: String| Batch { done: true, error: Some(e), ..Batch::default() };
    match std::fs::metadata(root) {
        Ok(m) if m.is_dir() => {}
        Ok(_) => return send(failed("not a folder".into())),
        Err(e) => return send(failed(e.to_string())),
    }
    let walker = ignore::WalkBuilder::new(root)
        .hidden(false)
        .require_git(false)
        .follow_links(false)
        .filter_entry(|e| e.file_name() != ".git")
        .build();
    let mut batch = Vec::with_capacity(BATCH);
    let mut skipped = 0;
    for entry in walker {
        if latest.load(Ordering::Relaxed) != generation {
            return; // superseded: the new root's scan reports instead
        }
        let Ok(entry) = entry else {
            skipped += 1;
            continue;
        };
        let Some(kind) = entry.file_type() else { continue };
        let file = kind.is_file() || (kind.is_symlink() && entry.path().metadata().is_ok_and(|m| m.is_file()));
        if !file {
            continue;
        }
        let Ok(rel) = entry.path().strip_prefix(root) else { continue };
        let rel = rel.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/");
        batch.push(rel);
        if batch.len() == BATCH {
            send(Batch { paths: std::mem::replace(&mut batch, Vec::with_capacity(BATCH)), skipped: std::mem::take(&mut skipped), ..Batch::default() });
        }
    }
    send(Batch { paths: batch, done: true, skipped, error: None });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(paths: &[&str]) -> Project {
        let mut p = Project::new(PathBuf::from("/r"), 1, false);
        p.add(Batch { paths: paths.iter().map(|s| s.to_string()).collect(), done: true, ..Batch::default() });
        p
    }

    fn names(p: &Project) -> Vec<String> {
        p.rows().iter().map(|r| format!("{}{}{}", "  ".repeat(r.depth), r.name, match r.dir { Some(true) => "/-", Some(false) => "/+", None => "" })).collect()
    }

    #[test]
    fn the_tree_collapses_directories_and_expands_on_toggle() {
        let mut p = project(&["src/main.rs", "README.md", "src/ui/view.rs", "src/lib.rs", ".github/ci.yml"]);
        assert_eq!(names(&p), [".github/+", "README.md", "src/+"]);
        assert!(p.toggle("src"));
        assert_eq!(names(&p), [".github/+", "README.md", "src/-", "  lib.rs", "  main.rs", "  ui/+"]);
        assert!(p.toggle("src/ui"));
        assert_eq!(names(&p).last().unwrap(), "    view.rs");
        assert!(!p.toggle("src/lib.rs"), "files do not toggle");
        assert!(p.toggle("src"));
        assert_eq!(names(&p), [".github/+", "README.md", "src/+"]);
    }

    #[test]
    fn folders_can_list_before_files_and_back_by_name() {
        let mut p = project(&["b.rs", "a/x.rs", "README.md", "c/d.rs", "a.rs"]);
        assert_eq!(names(&p), ["README.md", "a.rs", "a/+", "b.rs", "c/+"], "by name, bytewise");
        p.set_folders_first(true);
        assert!(p.toggle("a"));
        assert_eq!(names(&p), ["a/-", "  x.rs", "c/+", "README.md", "a.rs", "b.rs"]);
        p.set_folders_first(false);
        assert_eq!(names(&p), ["README.md", "a.rs", "a/-", "  x.rs", "b.rs", "c/+"]);
    }

    #[test]
    fn opening_a_folder_holding_only_a_folder_opens_down_to_one_with_more() {
        let mut p = project(&["a/b/c/one.rs", "a/b/c/two.rs", "a/b/c/d/deep.rs", "x/y/only.rs", "z/w/f.rs", "z/v.rs"]);
        assert!(p.toggle("a"));
        assert_eq!(names(&p)[..5], ["a/-", "  b/-", "    c/-", "      d/+", "      one.rs"], "stops at the folder with several entries");
        assert!(p.toggle("x"));
        assert!(names(&p).contains(&"  y/-".to_string()) && names(&p).contains(&"    only.rs".to_string()), "down to a file");
        assert!(p.toggle("z"));
        assert!(names(&p).contains(&"  w/+".to_string()), "a folder beside a file stays closed");
        assert!(p.toggle("a"));
        assert!(p.toggle("a"));
        assert_eq!(names(&p)[2], "    c/-", "closing keeps the inner folders open");
    }

    #[test]
    fn revealing_a_file_opens_the_folders_above_it() {
        let mut p = project(&["src/ui/view.rs", "src/lib.rs", "docs/a.md"]);
        assert!(p.reveal("src/ui/view.rs"));
        assert_eq!(names(&p), ["docs/+", "src/-", "  lib.rs", "  ui/-", "    view.rs"]);
        assert!(!p.reveal("src/ui/missing.rs"), "a file the tree does not hold");
        assert!(!p.reveal("src/ui"), "a folder is not a file");
        assert_eq!(p.relative(Path::new("/r/src/lib.rs")).as_deref(), Some("src/lib.rs"));
        assert_eq!(p.relative(Path::new("/elsewhere/x.rs")), None);
    }

    #[test]
    fn batches_merge_into_one_sorted_tree_and_coverage_says_how_far() {
        let mut p = Project::new(PathBuf::from("/r"), 1, false);
        p.add(Batch { paths: vec!["b.rs".into(), "a/x.rs".into()], ..Batch::default() });
        assert_eq!(p.coverage(), &Coverage::Scanning(2));
        p.add(Batch { paths: vec!["a.rs".into()], done: true, skipped: 2, ..Batch::default() });
        assert_eq!(p.skipped(), 2, "unreadable entries are counted, not hidden");
        assert_eq!(p.coverage(), &Coverage::Complete(3));
        assert_eq!(names(&p), ["a.rs", "a/+", "b.rs"]);
        let mut empty = Project::new(PathBuf::from("/r"), 1, false);
        empty.add(Batch { done: true, ..Batch::default() });
        assert_eq!(empty.coverage(), &Coverage::Complete(0));
        let mut gone = Project::new(PathBuf::from("/r"), 1, false);
        gone.add(Batch { done: true, error: Some("No such file or directory".into()), ..Batch::default() });
        assert!(matches!(gone.coverage(), Coverage::Unavailable(_)));
    }

    #[test]
    fn ranking_uses_lenz_order_disambiguates_duplicates_and_stops_when_superseded() {
        let batch = |paths: &[&str]| -> Arc<[Entry]> { paths.iter().map(|p| Entry::new(p)).collect() };
        let entries = [batch(&["a/mod.rs", "src/main.rs"]), batch(&["b/mod.rs", "docs/main.md"])];
        let (hits, matched) = rank(&entries, "mod", || true).unwrap();
        assert_eq!(hits.iter().map(|h| &**h).collect::<Vec<_>>(), ["a/mod.rs", "b/mod.rs"], "same names, full paths, discovery order");
        assert_eq!(matched, 2);
        assert_eq!(rank(&entries, "MAIN.rs", || true).unwrap().0[0].as_ref(), "src/main.rs");
        assert!(rank(&entries, "zzz", || true).unwrap().0.is_empty());
        let many: Vec<String> = (0..10_000).map(|i| format!("d/f{i}.rs")).collect();
        let many = [many.iter().map(|p| Entry::new(p)).collect::<Arc<[Entry]>>()];
        let (hits, matched) = rank(&many, "f", || true).unwrap();
        assert_eq!((hits.len(), matched), (FINDER_LIMIT, 10_000), "the limit is reported against all matches");
        assert_eq!(&*hits[0], "d/f0.rs", "shortest first, then discovery order");
        assert!(rank(&many, "f", || false).is_none());
    }

    #[test]
    fn scanning_obeys_ignores_keeps_hidden_files_and_does_not_follow_directory_links() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path();
        for (p, body) in [("src/main.rs", ""), (".gitignore", "target/\n*.log\n"), ("target/out.rs", ""), ("app.log", ""), (".config/tool.toml", ""), (".git/HEAD", "ref"), ("docs/main.rs", "")] {
            std::fs::create_dir_all(root.join(p).parent().unwrap()).unwrap();
            std::fs::write(root.join(p), body).unwrap();
        }
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(root.join("src"), root.join("loop")).unwrap();
            std::os::unix::fs::symlink(root.join("src/main.rs"), root.join("linked.rs")).unwrap();
        }
        let latest = AtomicU64::new(7);
        let mut found = Vec::new();
        let mut done = false;
        scan(root, 7, &latest, |b| {
            assert!(b.error.is_none());
            found.extend(b.paths);
            done = b.done;
        });
        found.sort();
        let mut want = vec![".config/tool.toml", ".gitignore", "docs/main.rs", "src/main.rs"];
        if cfg!(unix) {
            want.push("linked.rs");
            want.sort();
        }
        assert!(done);
        assert_eq!(found, want);
        let mut errors = Vec::new();
        scan(&root.join("missing"), 7, &latest, |b| errors.push((b.done, b.error)));
        assert!(matches!(errors.as_slice(), [(true, Some(_))]));
        let mut nothing = 0;
        scan(root, 6, &latest, |b| nothing += b.paths.len());
        assert_eq!(nothing, 0, "a superseded scan sends nothing");
    }
}

#[cfg(test)]
mod rescans {
    use super::*;

    #[test]
    fn a_rescan_keeps_the_shown_tree_until_it_completes_and_keeps_expansion() {
        let mut p = Project::new(PathBuf::from("/r"), 1, false);
        p.add(Batch { paths: vec!["src/a.rs".into(), "b.rs".into()], done: true, ..Batch::default() });
        assert!(p.toggle("src"));
        p.begin_rescan(2);
        p.add(Batch { paths: vec!["src/c.rs".into()], ..Batch::default() });
        assert!(p.refreshing());
        assert_eq!((p.files(), p.coverage()), (2, &Coverage::Complete(2)), "the old inventory stays, complete");
        p.add(Batch { paths: vec!["src/a.rs".into()], done: true, ..Batch::default() });
        assert!(!p.refreshing());
        assert_eq!(p.rows().iter().map(|r| &*r.path).collect::<Vec<_>>(), ["src", "src/a.rs", "src/c.rs"], "b.rs went, src stays open");
    }
}
