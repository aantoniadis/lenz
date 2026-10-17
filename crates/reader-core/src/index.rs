//! The project's declaration index: compact syntactic records per file, built progressively in background work
//! within a byte budget. No parse tree survives a file's indexing; files that change are re-indexed; coverage
//! (indexed, still to go, too large, over budget, with syntax errors) is always visible.

use std::{
    collections::{HashMap, VecDeque},
    path::Path,
    sync::Arc,
};

use lenz_content::highlight::{self, Declaration};

use crate::cache::Stamp;

/// Bytes of declaration records kept at most; indexing stops there and says so.
pub const INDEX_BYTES: usize = 32 << 20;
/// Files indexed per background job, so visible work interleaves.
pub const INDEX_CHUNK: usize = 32;
/// Declaration results kept, best first.
pub const DECL_LIMIT: usize = 200;

/// One indexed declaration: its record and where it sits (line, and the line's start byte).
#[derive(Clone, Debug)]
pub struct Indexed {
    pub decl: Declaration,
    pub line: u32,
    pub line_start: u32,
    lower: Box<str>,
}

impl Indexed {
    pub fn new(decl: Declaration, line: u32, line_start: u32) -> Self {
        let lower = decl.name.to_lowercase().into();
        Self { decl, line, line_start, lower }
    }

    fn bytes(&self) -> usize {
        std::mem::size_of::<Self>() - std::mem::size_of::<Declaration>() + self.decl.bytes() + self.lower.len()
    }
}

/// What indexing one file found.
pub enum FileIndex {
    Done { stamp: Option<Stamp>, items: Arc<[Indexed]>, syntax_errors: bool },
    /// Over the large-file thresholds: not parsed.
    TooLarge,
    Unreadable,
}

/// How much of the project the index covers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Coverage {
    /// Files with a language that has declarations.
    pub candidates: usize,
    pub indexed: usize,
    pub too_large: usize,
    pub unreadable: usize,
    pub syntax_errors: usize,
    /// Indexing stopped at the byte budget.
    pub limited: bool,
    /// Still indexing (or files changed and wait to be indexed again).
    pub pending: usize,
}

impl Coverage {
    pub fn complete(&self) -> bool {
        self.pending == 0 && !self.limited
    }
}

/// One file's records: the stamp it was read as, its declarations, and whether it had syntax errors.
type Records = (Option<Stamp>, Arc<[Indexed]>, bool);
/// Accounted per indexed file beyond its records: the map entry, the path and the shared slice's header.
const FILE_OVERHEAD: usize = std::mem::size_of::<(Arc<str>, Records)>() + 2 * std::mem::size_of::<usize>();
/// Ranked declarations (file and record) and how many matched in all.
pub type Ranked = (Vec<(Arc<str>, Indexed)>, usize);

pub struct DeclIndex {
    files: HashMap<Arc<str>, Records>,
    queue: VecDeque<Arc<str>>,
    /// What `queue` holds, for constant-time membership.
    queued: std::collections::HashSet<Arc<str>>,
    known: std::collections::HashSet<Arc<str>>,
    /// Files that could not be indexed, and whether because they were too large.
    failed: HashMap<Arc<str>, bool>,
    bytes: usize,
    limit: usize,
    limited: bool,
    /// A chunk is being indexed.
    pub(crate) running: bool,
}

impl DeclIndex {
    pub fn new(limit: usize) -> Self {
        Self {
            files: HashMap::new(),
            queue: VecDeque::new(),
            queued: Default::default(),
            known: Default::default(),
            failed: HashMap::new(),
            bytes: 0,
            limit,
            limited: false,
            running: false,
        }
    }

    /// Whether `path` is in a language the index understands.
    pub fn wants(path: &str) -> bool {
        highlight::detect(Path::new(path), "").is_some_and(|l| l != highlight::Language::Markdown && highlight::has_declarations(l))
    }

    /// Queue files the inventory found (those with declarations); `first` jumps the queue (a visible document).
    pub fn enqueue(&mut self, paths: impl IntoIterator<Item = Arc<str>>, first: bool) {
        for p in paths {
            if !Self::wants(&p) {
                continue;
            }
            self.known.insert(p.clone());
            if first {
                if !self.queued.insert(p.clone()) {
                    self.queue.retain(|q| *q != p);
                }
                self.queue.push_front(p);
            } else if !self.has(&p) && self.queued.insert(p.clone()) {
                self.queue.push_back(p);
            }
        }
    }

    /// Whether `path` has been indexed (or found unindexable).
    pub fn has(&self, path: &str) -> bool {
        self.files.contains_key(path) || self.failed.contains_key(path)
    }

    /// Forget `path`'s records (it changed or went) and index it again if it was a candidate and still exists.
    pub fn invalidate(&mut self, path: &Arc<str>, exists: bool) {
        if let Some((_, items, _)) = self.files.remove(path) {
            self.bytes -= file_bytes(path, &items);
            self.limited = self.limited && self.bytes >= self.limit;
        }
        self.failed.remove(path);
        if exists && self.known.contains(path) {
            self.enqueue([path.clone()], false);
        } else {
            self.known.remove(path);
            if self.queued.remove(path) {
                self.queue.retain(|q| q != path);
            }
        }
    }

    /// Keep only candidates still in the inventory `now` (after a rescan), and queue the new ones.
    pub fn retain(&mut self, now: &std::collections::HashSet<Arc<str>>) {
        let gone: Vec<Arc<str>> = self.known.iter().filter(|p| !now.contains(*p)).cloned().collect();
        for p in &gone {
            self.invalidate(p, false);
        }
        self.enqueue(now.iter().cloned(), false);
    }

    /// The next files to index, unless indexing is paused at its budget or already running.
    pub fn next_chunk(&mut self) -> Vec<Arc<str>> {
        if self.running || self.limited {
            return Vec::new();
        }
        let n = self.queue.len().min(INDEX_CHUNK);
        let chunk: Vec<_> = self.queue.drain(..n).collect();
        for p in &chunk {
            self.queued.remove(p);
        }
        self.running = !chunk.is_empty();
        chunk
    }

    pub fn add(&mut self, path: Arc<str>, result: FileIndex) {
        if !self.known.contains(&path) || self.queued.contains(&path) {
            return; // removed, or changed and queued again, while it was being indexed
        }
        self.invalidate(&path, false); // replaced records (re-indexing a visible file) are no longer counted
        self.known.insert(path.clone());
        match result {
            FileIndex::Done { stamp, items, syntax_errors } => {
                let bytes = file_bytes(&path, &items);
                if self.limited || self.bytes + bytes > self.limit {
                    // Over budget: stop here, keep what fits, and say the index is limited.
                    self.limited = true;
                    self.queued.insert(path.clone());
                    self.queue.push_front(path);
                    return;
                }
                self.bytes += bytes;
                self.files.insert(path, (stamp, items, syntax_errors));
            }
            FileIndex::TooLarge => _ = self.failed.insert(path, true),
            FileIndex::Unreadable => _ = self.failed.insert(path, false),
        }
    }

    pub fn coverage(&self) -> Coverage {
        Coverage {
            candidates: self.known.len(),
            indexed: self.files.len(),
            too_large: self.failed.values().filter(|t| **t).count(),
            unreadable: self.failed.values().filter(|t| !**t).count(),
            syntax_errors: self.files.values().filter(|f| f.2).count(),
            limited: self.limited,
            pending: self.queue.len() + usize::from(self.running),
        }
    }

    /// Bytes held by the records, by their accounting.
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// The records as they are now, shared, for a ranking job.
    pub fn snapshot(&self) -> Vec<(Arc<str>, Arc<[Indexed]>)> {
        self.files.iter().map(|(p, (_, items, _))| (p.clone(), items.clone())).collect()
    }
}

fn file_bytes(path: &str, items: &[Indexed]) -> usize {
    items.iter().map(Indexed::bytes).sum::<usize>() + path.len() + FILE_OVERHEAD
}

/// Index one file off the UI thread: read, decode, extract declarations, keep only records. A cancelled job's
/// `Unreadable` is never counted: cancellation only follows a new root, whose index discards this one's results.
pub fn index_file(root: &Path, rel: &str, current: impl Fn() -> bool + Send + Clone + 'static) -> FileIndex {
    let path = root.join(rel);
    let Ok(meta) = std::fs::metadata(&path) else { return FileIndex::Unreadable };
    if meta.len() as usize > crate::document::LARGE_BYTES {
        return FileIndex::TooLarge;
    }
    let stamp = Some(Stamp::from(&meta));
    let Ok(bytes) = std::fs::read(&path) else { return FileIndex::Unreadable };
    let doc = crate::document::Document::decode(path, 0, &bytes);
    drop(bytes);
    if doc.is_large() {
        return FileIndex::TooLarge;
    }
    let Some(lang) = doc.language() else { return FileIndex::Unreadable };
    let Some(found) = highlight::declarations_while(lang, doc.text().clone(), current) else { return FileIndex::Unreadable };
    // `impl` blocks and Markdown sections are scopes for sticky lines and breadcrumbs, not names to search for.
    let items = found
        .items
        .into_iter()
        .filter(|d| !matches!(d.kind, highlight::DeclKind::Impl | highlight::DeclKind::Heading))
        .map(|d| {
            let line = doc.line_of(d.name_range.start as usize);
            let start = doc.line_range(line).start;
            Indexed::new(d, line as u32, start as u32)
        })
        .collect();
    FileIndex::Done { stamp, items, syntax_errors: found.syntax_errors }
}

/// The best [`DECL_LIMIT`] declarations for `query` (Lenz's fuzzy ranking over names), with how many matched.
/// `None` once `current` says it was superseded.
pub fn rank(files: &[(Arc<str>, Arc<[Indexed]>)], query: &str, current: impl Fn() -> bool) -> Option<Ranked> {
    let q = query.to_lowercase();
    let mut hits: Vec<(i64, &Arc<str>, &Indexed)> = Vec::new();
    let mut matched = 0;
    for (i, (path, items)) in files.iter().enumerate() {
        if i % 256 == 0 && !current() {
            return None;
        }
        for item in items.iter() {
            if let Some(s) = lenz_content::fuzzy::score_lowered(&q, &item.lower, item.decl.name.len()) {
                matched += 1;
                hits.push((s, path, item));
                if hits.len() >= DECL_LIMIT * 8 {
                    keep_best(&mut hits);
                }
            }
        }
    }
    keep_best(&mut hits);
    Some((hits.into_iter().map(|(_, p, i)| (p.clone(), i.clone())).collect(), matched))
}

fn keep_best(hits: &mut Vec<(i64, &Arc<str>, &Indexed)>) {
    // Best score first; ties by path and position, so results are stable across runs.
    hits.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(b.1)).then(a.2.decl.name_range.start.cmp(&b.2.decl.name_range.start)));
    hits.truncate(DECL_LIMIT);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexing_is_bounded_invalidated_and_honest_about_coverage() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("a.rs"), "fn alpha() {}\nstruct Beta;\n").unwrap();
        std::fs::write(d.path().join("b.py"), "def gamma(): pass\n").unwrap();
        std::fs::write(d.path().join("c.txt"), "not code\n").unwrap();
        std::fs::write(d.path().join("big.rs"), "fn x() {}\n".repeat(crate::document::LARGE_LINES + 1)).unwrap();
        let mut ix = DeclIndex::new(INDEX_BYTES);
        ix.enqueue(["a.rs", "b.py", "c.txt", "big.rs"].map(Arc::<str>::from), false);
        let chunk = ix.next_chunk();
        assert_eq!(chunk.len(), 3, "only files with declarations are queued");
        assert!(ix.next_chunk().is_empty(), "one chunk at a time");
        for p in chunk {
            let r = index_file(d.path(), &p, || true);
            ix.add(p, r);
        }
        ix.running = false;
        let c = ix.coverage();
        assert_eq!((c.candidates, c.indexed, c.too_large, c.pending, c.complete()), (3, 2, 1, 0, true));
        let (hits, matched) = rank(&ix.snapshot(), "bet", || true).unwrap();
        assert_eq!((matched, hits[0].1.decl.name.as_ref(), hits[0].1.line), (1, "Beta", 1));
        // A change: records go and the file waits to be indexed again.
        let before = ix.bytes();
        ix.invalidate(&Arc::from("a.rs"), true);
        assert!(ix.bytes() < before);
        assert_eq!(ix.coverage().pending, 1);
        assert!(rank(&ix.snapshot(), "alpha", || true).unwrap().0.is_empty());
        ix.invalidate(&Arc::from("b.py"), false);
        assert_eq!(ix.coverage().candidates, 2, "a deleted file leaves the index");
        // A large file that changes is counted once; one that goes leaves every count.
        ix.invalidate(&Arc::from("big.rs"), true);
        for p in ix.next_chunk() {
            let r = index_file(d.path(), &p, || true);
            ix.add(p, r);
        }
        ix.running = false;
        let c = ix.coverage();
        assert_eq!((c.candidates, c.indexed, c.too_large, c.pending), (2, 1, 1, 0));
        ix.retain(&[Arc::<str>::from("a.rs")].into_iter().collect());
        let c = ix.coverage();
        assert_eq!((c.candidates, c.indexed, c.too_large), (1, 1, 0));
        // A file outside the inventory is not indexed because it changed.
        ix.invalidate(&Arc::from("new.rs"), true);
        assert_eq!(ix.coverage().pending, 0);
    }

    #[test]
    fn the_budget_stops_indexing_and_says_so() {
        let d = tempfile::tempdir().unwrap();
        for i in 0..4 {
            std::fs::write(d.path().join(format!("f{i}.rs")), (0..50).map(|j| format!("fn f{i}_{j}() {{}}\n")).collect::<String>()).unwrap();
        }
        let mut ix = DeclIndex::new(5_000);
        ix.enqueue((0..4).map(|i| Arc::<str>::from(format!("f{i}.rs"))), false);
        for p in ix.next_chunk() {
            let r = index_file(d.path(), &p, || true);
            ix.add(p, r);
        }
        ix.running = false;
        let c = ix.coverage();
        assert!(c.limited && !c.complete() && c.indexed < 4, "{c:?}");
        assert!(ix.bytes() <= 5_000);
        assert!(ix.next_chunk().is_empty(), "paused at the budget");
    }
}
