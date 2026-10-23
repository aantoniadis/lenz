//! Inactive documents kept for a quick return, within a byte budget. Only reloadable (file-backed, still
//! readable) snapshots enter it: dropping one loses nothing that a re-read cannot restore. Open documents and
//! unavailable ones are owned elsewhere and never counted here.

use std::{path::Path, sync::Arc, time::SystemTime};

use crate::text::TextSnapshot;

/// The starting budget for inactive reloadable content.
pub const INACTIVE_BYTES: usize = 32 << 20;

/// What a snapshot was read from, to tell whether the file is still the same: size, modification time and, where
/// the platform has them, the device and inode (an atomic replacement is a new inode; a rename keeps it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stamp {
    pub len: u64,
    pub modified: Option<SystemTime>,
    pub file: Option<(u64, u64)>,
}

impl Stamp {
    pub fn of(path: &Path) -> Option<Stamp> {
        std::fs::metadata(path).ok().map(|m| Stamp::from(&m))
    }
}

impl From<&std::fs::Metadata> for Stamp {
    fn from(meta: &std::fs::Metadata) -> Self {
        #[cfg(unix)]
        let file = {
            use std::os::unix::fs::MetadataExt;
            Some((meta.dev(), meta.ino()))
        };
        #[cfg(not(unix))]
        let file = None;
        Stamp { len: meta.len(), modified: meta.modified().ok(), file }
    }
}

pub struct InactiveCache {
    limit: usize,
    used: usize,
    /// Least recently left first.
    entries: Vec<(Stamp, Arc<TextSnapshot>, usize)>,
}

impl InactiveCache {
    pub fn new(limit: usize) -> Self {
        Self { limit, used: 0, entries: Vec::new() }
    }

    /// Retained source/index bytes held, by [`TextSnapshot::storage_bytes`].
    pub fn used(&self) -> usize {
        self.used
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Keep `doc`, read as `stamp`, as the most recently left; evicts the least recently left until within
    /// the budget. A document larger than the whole budget is not kept.
    pub fn put(&mut self, stamp: Stamp, doc: Arc<TextSnapshot>) {
        self.put_current(stamp, doc);
    }

    /// Keep the current source and all its shared backing in the same budget.
    pub(crate) fn put_current(&mut self, stamp: Stamp, doc: Arc<TextSnapshot>) {
        self.remove(doc.source());
        let bytes = doc.storage_bytes();
        if bytes > self.limit {
            return;
        }
        self.used += bytes;
        self.entries.push((stamp, doc, bytes));
        while self.used > self.limit {
            let (_, _, freed) = self.entries.remove(0);
            self.used -= freed;
        }
    }

    #[cfg(test)]
    pub(crate) fn documents(&self) -> Vec<Arc<TextSnapshot>> {
        self.entries.iter().map(|(_, d, _)| d.clone()).collect()
    }

    /// The kept snapshot of `path` whatever its stamp: the last content of a file that has since disappeared,
    /// taken back by its tab before it can be evicted.
    pub fn take_any(&mut self, path: &Path) -> Option<Arc<TextSnapshot>> {
        self.remove(path).map(|(_, doc)| doc)
    }

    /// The kept snapshot of `path`, left in place, whatever its stamp (a preview shares it; opening checks the file).
    pub fn peek(&self, path: &Path) -> Option<Arc<TextSnapshot>> {
        self.entries.iter().find(|(_, d, _)| d.source() == path).map(|(_, d, _)| d.clone())
    }

    /// The stamp `path`'s kept snapshot was read with: its file's identity while its tab is released.
    pub fn stamp(&self, path: &Path) -> Option<Stamp> {
        self.entries.iter().find(|(_, d, _)| d.source() == path).map(|(s, ..)| *s)
    }

    /// `from`'s kept snapshot is the same file's under `to` now (a followed rename).
    pub fn rename(&mut self, from: &Path, to: &Path) {
        if let Some(e) = self.entries.iter_mut().find(|(_, d, _)| d.source() == from) {
            e.1 = Arc::new(e.1.with_source_meta(Arc::new(e.1.source_meta().renamed(to.to_path_buf()))));
        }
    }

    pub fn contains(&self, path: &Path) -> bool {
        self.entries.iter().any(|(_, d, _)| d.source() == path)
    }

    /// The kept snapshot of `path` if it was read as `stamp`, removed from the cache; a stale one is dropped.
    pub fn take(&mut self, path: &Path, stamp: Option<Stamp>) -> Option<Arc<TextSnapshot>> {
        let (kept, doc) = self.remove(path)?;
        (Some(kept) == stamp).then_some(doc)
    }

    fn remove(&mut self, path: &Path) -> Option<(Stamp, Arc<TextSnapshot>)> {
        let i = self.entries.iter().position(|(_, d, _)| d.source() == path)?;
        let (stamp, doc, bytes) = self.entries.remove(i);
        self.used -= bytes;
        Some((stamp, doc))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn doc(name: &str, len: usize) -> Arc<TextSnapshot> {
        Arc::new(TextSnapshot::from_document(crate::text::BufferId(1), &crate::document::Document::decode(PathBuf::from(name), 1, "a".repeat(len).as_bytes())))
    }
    const STAMP: Stamp = Stamp { len: 1, modified: None, file: None };

    #[test]
    fn evicts_least_recently_left_within_the_budget() {
        let (a, b, c) = (doc("a", 400), doc("b", 400), doc("c", 400));
        let limit = a.storage_bytes() * 2;
        let mut cache = InactiveCache::new(limit);
        cache.put(STAMP, a.clone());
        cache.put(STAMP, b.clone());
        assert_eq!(cache.used(), a.storage_bytes() + b.storage_bytes());
        cache.put(STAMP, c.clone());
        assert_eq!((cache.len(), cache.used()), (2, b.storage_bytes() + c.storage_bytes()));
        assert!(cache.take(Path::new("a"), Some(STAMP)).is_none(), "the oldest went first");
        assert!(Arc::ptr_eq(&cache.take(Path::new("b"), Some(STAMP)).unwrap(), &b));
        assert_eq!(cache.used(), c.storage_bytes(), "taking returns its bytes");
    }

    #[test]
    fn stale_snapshots_are_dropped_not_reused() {
        let mut cache = InactiveCache::new(INACTIVE_BYTES);
        cache.put(STAMP, doc("a", 10));
        let changed = Stamp { len: 2, ..STAMP };
        assert!(cache.take(Path::new("a"), Some(changed)).is_none());
        assert_eq!((cache.len(), cache.used()), (0, 0));
        cache.put(STAMP, doc("a", 10));
        assert!(cache.take(Path::new("a"), None).is_none(), "an unreadable file is not served from cache");
    }

    #[test]
    fn oversized_documents_are_not_kept_and_repeats_replace() {
        let mut cache = InactiveCache::new(100);
        cache.put(STAMP, doc("big", 1000));
        assert!(cache.is_empty());
        let mut cache = InactiveCache::new(INACTIVE_BYTES);
        cache.put(STAMP, doc("a", 10));
        cache.put(STAMP, doc("a", 20));
        assert_eq!((cache.len(), cache.used()), (1, doc("a", 20).storage_bytes()));
    }
}
