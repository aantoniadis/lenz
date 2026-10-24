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

    pub(crate) fn bytes(&self) -> usize {
        std::mem::size_of::<Self>() - std::mem::size_of::<Declaration>() + self.decl.bytes() + self.lower.len()
    }
}

/// What indexing one file found.
#[derive(Clone)]
pub enum FileIndex {
    Done { stamp: Option<Stamp>, items: Arc<[Indexed]>, syntax_errors: bool, fingerprint: Option<[u8; 32]>, decoded_digest: [u8; 32] },
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
type Records = (Option<Stamp>, Arc<[Indexed]>, bool, Option<[u8; 32]>, [u8; 32]);
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
            let new = self.known.insert(p.clone());
            if first {
                if !self.queued.insert(p.clone()) {
                    self.queue.retain(|q| *q != p);
                }
                self.queue.push_front(p);
            } else if new && !self.has(&p) && self.queued.insert(p.clone()) {
                self.queue.push_back(p);
            }
        }
    }

    /// Whether `path` has been indexed (or found unindexable).
    pub fn has(&self, path: &str) -> bool {
        self.files.contains_key(path) || self.failed.contains_key(path)
    }

    pub(crate) fn fingerprint(&self, path: &str) -> Option<[u8; 32]> {
        self.files.get(path).and_then(|record| record.3)
    }

    /// None while a fresh read is pending; unknown or unavailable contents require notification.
    pub(crate) fn changed_since(&self, path: &str, fingerprint: [u8; 32]) -> Option<bool> {
        (self.has(path) || self.limited || !self.known.contains(path))
            .then(|| self.fingerprint(path) != Some(fingerprint))
    }

    /// Only current, successfully indexed files can authorize suppressing an overflow notification.
    pub(crate) fn fingerprints(&self) -> HashMap<Arc<str>, [u8; 32]> {
        self.files.iter().filter_map(|(path, record)| record.3.map(|hash| (path.clone(), hash))).collect()
    }

    /// Forget `path`'s records (it changed or went) and index it again if it was a candidate and still exists.
    pub fn invalidate(&mut self, path: &Arc<str>, exists: bool) {
        if let Some((_, items, _, _, _)) = self.files.remove(path) {
            self.bytes -= file_bytes(path, &items);
            self.limited = self.limited && self.bytes >= self.limit;
        }
        self.failed.remove(path);
        if exists && self.known.remove(path) {
            self.enqueue([path.clone()], false);
        } else {
            self.known.remove(path);
            if self.queued.remove(path) {
                self.queue.retain(|q| q != path);
            }
        }
    }

    /// Rebuild affected descendants, including queued/in-flight files. The rescan removes vanished paths.
    pub(crate) fn invalidate_changed(&mut self, root: &Path, change: &crate::watch::Change) {
        let affected: Vec<_> = self.known.iter().filter(|p| change.touches(&root.join(p.as_ref()))).cloned().collect();
        for path in affected {
            self.invalidate(&path, true);
        }
    }

    /// Check completed reads made before a watch was registered; in-flight/failed reads must try again.
    pub(crate) fn registration_checks(&mut self, root: &Path, folders: &std::collections::BTreeSet<std::path::PathBuf>) -> Vec<(std::path::PathBuf, Stamp)> {
        let mut checks = Vec::new();
        let mut retry = Vec::new();
        for rel in &self.known {
            let path = root.join(rel.as_ref());
            if !path.ancestors().any(|p| folders.contains(p)) { continue; }
            match self.files.get(rel).and_then(|(stamp, _, _, _, _)| *stamp) {
                Some(stamp) => checks.push((path, stamp)),
                None if !self.queued.contains(rel) => retry.push(rel.clone()),
                None => {}
            }
        }
        for rel in retry { self.invalidate(&rel, true); }
        checks
    }

    /// Keep only candidates still in the inventory `now` (after a rescan), and queue the new ones.
    pub fn retain(&mut self, now: &std::collections::HashSet<Arc<str>>) {
        // A removed directory can discard thousands of queued files; compact the queue once.
        self.queue.retain(|p| now.contains(p));
        self.queued.retain(|p| now.contains(p));
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
            FileIndex::Done { stamp, items, syntax_errors, fingerprint, decoded_digest } => {
                let bytes = file_bytes(&path, &items);
                if self.limited || self.bytes + bytes > self.limit {
                    // Over budget: stop here, keep what fits, and say the index is limited.
                    self.limited = true;
                    self.queued.insert(path.clone());
                    self.queue.push_front(path);
                    return;
                }
                self.bytes += bytes;
                self.files.insert(path, (stamp, items, syntax_errors, fingerprint, decoded_digest));
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

    pub(crate) fn source(&self, path: &str) -> Option<crate::search::DiskSource> {
        let record = self.files.get(path)?;
        Some(crate::search::DiskSource { stamp: record.0?, digest: record.4 })
    }

    /// The records as they are now, shared, for a ranking job.
    pub fn snapshot(&self) -> Vec<(Arc<str>, Arc<[Indexed]>)> {
        self.files.iter().map(|(p, (_, items, _, _, _))| (p.clone(), items.clone())).collect()
    }
}

fn file_bytes(path: &str, items: &[Indexed]) -> usize {
    items.iter().map(Indexed::bytes).sum::<usize>() + path.len() + FILE_OVERHEAD
}

/// Index one file off the UI thread: read, decode, extract declarations, keep only records. A cancelled job's
/// `Unreadable` is never counted: cancellation only follows a new root, whose index discards this one's results.
pub fn index_file(root: &Path, rel: &str, current: impl Fn() -> bool + Send + Clone + 'static) -> FileIndex {
    index_file_cached(root, rel, current, None)
}

pub(crate) fn index_file_cached(root: &Path, rel: &str, current: impl Fn() -> bool + Send + Clone + 'static, mut cache: Option<&mut crate::index_cache::Cache>) -> FileIndex {
    if !current() { return FileIndex::Unreadable; }
    let path = root.join(rel);
    let Ok(meta) = std::fs::metadata(&path) else { return FileIndex::Unreadable };
    if meta.len() as usize > crate::document::LARGE_BYTES {
        return FileIndex::TooLarge;
    }
    let stamp = Some(Stamp::from(&meta));
    let Ok(bytes) = std::fs::read(&path) else { return FileIndex::Unreadable };
    if !current() || Stamp::of(&path) != stamp { return FileIndex::Unreadable; }
    let doc = crate::document::Document::decode(path, 0, &bytes);
    let decoded_digest = crate::search::decoded_digest(doc.text().as_bytes());
    let fingerprint = cache.as_ref().and_then(|_| doc.language()).map(|language| {
        use sha2::{Digest, Sha256};
        let mut hash = Sha256::new(); hash.update(format!("{language:?}\0")); hash.update(&bytes);
        hash.finalize()
    });
    let key = fingerprint.as_ref().map(|hash| format!("{hash:x}"));
    let fingerprint = fingerprint.map(Into::into);
    drop(bytes);
    if doc.is_large() {
        return FileIndex::TooLarge;
    }
    if let Some((items, syntax_errors)) = key.as_ref().and_then(|key| cache.as_mut()?.get(key))
        && items.iter().all(|i| {
            let d = &i.decl;
            d.range.start <= d.range.end && d.name_range.start <= d.name_range.end
                && d.range.end as usize <= doc.text().len() && d.name_range.end as usize <= doc.text().len()
                && doc.text().is_char_boundary(d.name_range.start as usize) && doc.text().is_char_boundary(d.name_range.end as usize)
                && (i.line as usize) < doc.line_count() && doc.line_range(i.line as usize).start == i.line_start as usize
                && doc.line_of(d.name_range.start as usize) == i.line as usize
        }) {
        return FileIndex::Done { stamp, items, syntax_errors, fingerprint, decoded_digest };
    }
    let Some((items, syntax_errors)) = declaration_records(&doc, current.clone()) else { return FileIndex::Unreadable };
    if current() && let (Some(cache), Some(key)) = (cache, key) { cache.insert(key, items.clone(), syntax_errors); }
    FileIndex::Done { stamp, items, syntax_errors, fingerprint, decoded_digest }
}

/// Current-buffer declarations reuse the admitted Document and never persist a draft in the disk cache.
pub(crate) fn index_document(doc: &crate::document::Document, stamp: Option<Stamp>, current: impl Fn() -> bool + Send + Clone + 'static) -> FileIndex {
    if doc.is_large() { return FileIndex::TooLarge; }
    let decoded_digest = crate::search::decoded_digest(doc.text().as_bytes());
    let Some((items, syntax_errors)) = declaration_records(doc, current.clone()) else { return FileIndex::Unreadable };
    let fingerprint = stamp.filter(|_| !doc.source_meta().had_bom() && !doc.source_meta().is_lossy() && doc.source_meta().original_encoding() == "UTF-8")
        .and_then(|_| doc.language()).map(|language| {
            use sha2::{Digest, Sha256};
            let mut hash = Sha256::new(); hash.update(format!("{language:?}\0")); hash.update(doc.text().as_bytes()); hash.finalize().into()
        });
    FileIndex::Done { stamp, items, syntax_errors, fingerprint, decoded_digest }
}

fn declaration_records(doc: &crate::document::Document, current: impl Fn() -> bool + Send + Clone + 'static) -> Option<(Arc<[Indexed]>, bool)> {
    let lang = doc.language()?;
    let found = highlight::declarations_while(lang, doc.text().clone(), current)?;
    // `impl` blocks and Markdown sections are scopes for sticky lines and breadcrumbs, not names to search for.
    let items: Arc<[Indexed]> = found
        .items
        .into_iter()
        .filter(|d| !matches!(d.kind, highlight::DeclKind::Impl | highlight::DeclKind::Heading))
        .map(|d| {
            let line = doc.line_of(d.name_range.start as usize);
            let start = doc.line_range(line).start;
            Indexed::new(d, line as u32, start as u32)
        })
        .collect();
    Some((items, found.syntax_errors))
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

/// Merge already-ranked chunks without retaining every draft declaration.
pub(crate) fn merge_ranked(previous: Ranked, next: Ranked, query: &str) -> Ranked {
    let matched = previous.1 + next.1;
    let mut items = previous.0;
    items.extend(next.0);
    let lower = query.to_lowercase();
    let mut scored: Vec<_> = items.iter().filter_map(|(path, item)|
        lenz_content::fuzzy::score_lowered(&lower, &item.lower, item.decl.name.len()).map(|score| (score, path, item))).collect();
    keep_best(&mut scored);
    (scored.into_iter().map(|(_, path, item)| (path.clone(), item.clone())).collect(), matched)
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
    fn overflow_content_checks_wait_for_current_reads_and_fall_back_at_limits() {
        let d = tempfile::tempdir().unwrap();
        let file = d.path().join("a.rb");
        std::fs::write(&file, "class Alpha; end\n").unwrap();
        let time = std::fs::metadata(&file).unwrap().modified().unwrap();
        let mut cache = crate::index_cache::Cache::new(d.path(), d.path()); cache.load();
        let read = |cache: &mut crate::index_cache::Cache| index_file_cached(d.path(), "a.rb", || true, Some(cache));
        let mut ix = DeclIndex::new(INDEX_BYTES);
        let path: Arc<str> = "a.rb".into();
        ix.enqueue([path.clone()], false); ix.next_chunk(); ix.add(path.clone(), read(&mut cache)); ix.running = false;
        let old = ix.fingerprint(&path).unwrap();
        assert_eq!(ix.changed_since(&path, old), Some(false));
        ix.invalidate(&path, true); ix.next_chunk();
        let stale = read(&mut cache);
        std::fs::write(&file, "class Omega; end\n").unwrap();
        std::fs::File::options().write(true).open(&file).unwrap().set_modified(time).unwrap();
        ix.invalidate(&path, true); ix.add(path.clone(), stale); ix.running = false;
        assert_eq!(ix.changed_since(&path, old), None, "a requeued read cannot settle recovery");
        ix.next_chunk(); ix.add(path.clone(), read(&mut cache)); ix.running = false;
        assert_eq!(ix.changed_since(&path, old), Some(true), "same size and mtime do not establish equality");
        ix.invalidate(&path, true); ix.next_chunk(); ix.add(path.clone(), FileIndex::Unreadable); ix.running = false;
        assert_eq!(ix.changed_since(&path, old), Some(true));
        let mut limited = DeclIndex::new(0); limited.enqueue([path.clone()], false); limited.next_chunk();
        limited.add(path.clone(), read(&mut cache)); limited.running = false;
        assert_eq!(limited.changed_since(&path, old), Some(true), "a byte-budget stop must not wait forever");
        ix.invalidate(&path, true); ix.next_chunk(); ix.add(path.clone(), index_file(d.path(), "a.rb", || true)); ix.running = false;
        assert!(ix.fingerprints().is_empty(), "reuse off has no suppression baseline");
        assert_eq!(ix.changed_since(&path, old), Some(true));
    }

    #[test]
    fn an_unchanged_inventory_does_not_queue_the_running_chunk_again() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("a.rb"), "class Alpha; end\n").unwrap();
        let mut ix = DeclIndex::new(INDEX_BYTES);
        let path: Arc<str> = "a.rb".into();
        ix.enqueue([path.clone()], false);
        assert_eq!(ix.next_chunk().len(), 1);
        ix.retain(&[path.clone()].into());
        assert_eq!(ix.coverage().pending, 1, "only the running job, no duplicate queued read");
        ix.add(path, index_file(d.path(), "a.rb", || true));
        ix.running = false;
        assert_eq!(ix.coverage().indexed, 1);
        assert!(ix.coverage().complete());
    }

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
