//! Disposable, content-validated declaration records. Only the current inventory supplies file paths.
use std::{collections::HashMap, io::{Read, Write}, path::{Path, PathBuf}, sync::Arc};

use lenz_content::highlight::{DeclKind, Declaration};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::index::{INDEX_BYTES, Indexed};

const DISK_BYTES: u64 = 64 << 20;
const PROJECTS: usize = 8;

#[derive(Serialize, Deserialize)]
struct SavedDecl {
    kind: String, name: Box<str>, name_range: std::ops::Range<u32>, range: std::ops::Range<u32>,
    first_line: u32, last_line: u32, depth: u16, parent: Option<u32>, line: u32, line_start: u32,
}

impl SavedDecl {
    fn save(i: &Indexed) -> Self {
        let d = &i.decl;
        Self { kind: d.kind.label().into(), name: d.name.clone(), name_range: d.name_range.clone(), range: d.range.clone(),
            first_line: d.first_line, last_line: d.last_line, depth: d.depth, parent: d.parent, line: i.line, line_start: i.line_start }
    }

    fn load(self) -> Option<Indexed> {
        let kind = match self.kind.as_str() {
            "module" => DeclKind::Module, "type" => DeclKind::Type, "interface" => DeclKind::Interface,
            "function" => DeclKind::Function, "method" => DeclKind::Method, "component" => DeclKind::Component,
            "constant" => DeclKind::Constant, "macro" => DeclKind::Macro, "element" => DeclKind::Element,
            "selector" => DeclKind::Selector, _ => return None,
        };
        Some(Indexed::new(Declaration { kind, name: self.name, name_range: self.name_range, range: self.range,
            first_line: self.first_line, last_line: self.last_line, depth: self.depth, parent: self.parent }, self.line, self.line_start))
    }
}

#[derive(Serialize, Deserialize)]
struct SavedEntry { key: String, used: u64, errors: bool, items: Vec<SavedDecl> }

#[derive(Serialize, Deserialize)]
struct Saved { extractor: String, epoch: String, entries: Vec<SavedEntry> }

struct Entry { used: u64, errors: bool, items: Arc<[Indexed]>, bytes: usize }

pub(crate) struct Cache {
    dir: PathBuf,
    epoch: String,
    entries: HashMap<String, Entry>,
    loaded: bool,
    dirty: bool,
    bytes: usize,
    clock: u64,
}

pub(crate) fn digest(bytes: &[u8]) -> String { hex(&Sha256::digest(bytes)) }

/// Lowercase hex, as `{:x}` printed sha2 0.10's digests (the persisted keys keep their form).
pub(crate) fn hex(bytes: &[u8]) -> String { bytes.iter().map(|b| format!("{b:02x}")).collect() }

impl Cache {
    pub fn new(base: &Path, root: &Path) -> Self {
        Self { dir: base.join("declarations-v1").join(digest(root.as_os_str().as_encoded_bytes())), epoch: String::new(),
            entries: HashMap::new(), loaded: false, dirty: false, bytes: 0, clock: 0 }
    }

    /// Once per background chunk; another window's rebuild invalidates this window's reusable records too.
    pub fn load(&mut self) {
        let epoch = self.disk_epoch();
        if self.loaded && epoch == self.epoch { return; }
        self.entries.clear(); self.bytes = 0; self.dirty = false; self.loaded = true; self.epoch = epoch;
        let Some(saved) = self.read() else { return };
        for item in saved.entries {
            if item.key.len() != 64 || !item.key.bytes().all(|c| c.is_ascii_hexdigit()) { continue; }
            let Some(items) = item.items.into_iter().map(SavedDecl::load).collect::<Option<Vec<_>>>() else { continue };
            let bytes = entry_bytes(&items);
            if bytes > INDEX_BYTES || self.bytes + bytes > INDEX_BYTES { continue; }
            self.clock = self.clock.max(item.used);
            if self.entries.contains_key(&item.key) { continue; }
            self.bytes += bytes;
            self.entries.insert(item.key, Entry { used: item.used, errors: item.errors, items: items.into(), bytes });
        }
    }

    fn disk_epoch(&self) -> String {
        std::fs::File::open(self.dir.join("epoch")).ok().and_then(|f| {
            let mut s = String::new(); f.take(128).read_to_string(&mut s).ok().map(|_| s)
        }).unwrap_or_default()
    }

    fn read(&self) -> Option<Saved> {
        let file = std::fs::File::open(self.dir.join("records")).ok()?;
        if file.metadata().ok()?.len() > DISK_BYTES { return None; }
        let mut bytes = Vec::new();
        file.take(DISK_BYTES + 1).read_to_end(&mut bytes).ok()?;
        if bytes.len() > DISK_BYTES as usize || bytes.len() < 65 || bytes[64] != b'\n' || digest(&bytes[65..]).as_bytes() != &bytes[..64] { return None; }
        let saved: Saved = serde_json::from_slice(&bytes[65..]).ok()?;
        (saved.extractor == env!("LENZ_INDEX_EXTRACTOR") && saved.epoch == self.epoch).then_some(saved)
    }

    pub fn get(&mut self, key: &str) -> Option<(Arc<[Indexed]>, bool)> {
        let entry = self.entries.get_mut(key)?;
        self.clock += 1; entry.used = self.clock;
        Some((entry.items.clone(), entry.errors))
    }

    pub fn insert(&mut self, key: String, items: Arc<[Indexed]>, errors: bool) {
        let bytes = entry_bytes(&items);
        if bytes > INDEX_BYTES { return; }
        if let Some(old) = self.entries.remove(&key) { self.bytes -= old.bytes; }
        while self.bytes + bytes > INDEX_BYTES { self.evict(); }
        self.clock += 1; self.bytes += bytes; self.dirty = true;
        self.entries.insert(key, Entry { used: self.clock, errors, items, bytes });
    }

    fn evict(&mut self) {
        if let Some(key) = self.entries.iter().min_by_key(|(_, e)| e.used).map(|(k, _)| k.clone())
            && let Some(old) = self.entries.remove(&key) { self.bytes -= old.bytes; }
    }

    /// Only when a background indexing pass settles; warm reads do not rewrite the snapshot.
    pub fn flush(&mut self, current: impl Fn() -> bool) {
        if !self.dirty || !current() || self.disk_epoch() != self.epoch { return; }
        loop {
            let saved = Saved { extractor: env!("LENZ_INDEX_EXTRACTOR").into(), epoch: self.epoch.clone(),
                entries: self.entries.iter().map(|(key, e)| SavedEntry { key: key.clone(), used: e.used,
                    errors: e.errors, items: e.items.iter().map(SavedDecl::save).collect() }).collect() };
            let Ok(bytes) = serde_json::to_vec(&saved) else { return };
            if bytes.len() + 65 > DISK_BYTES as usize {
                let target = self.bytes / 2;
                while self.bytes > target { self.evict(); }
                continue;
            }
            if !current() || self.disk_epoch() != self.epoch { return; }
            if atomic_write(&self.dir.join("records"), &[digest(&bytes).as_bytes(), b"\n", &bytes]).is_ok() {
                self.dirty = false; self.prune_projects();
            }
            return;
        }
    }

    /// The separate epoch prevents an older process's late atomic write being reused after a rebuild.
    pub fn clear(&mut self) -> Result<(), String> {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let epoch = format!("{}-{:?}-{}", std::process::id(), std::time::SystemTime::now(), NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
        self.entries.clear(); self.bytes = 0; self.dirty = false; self.loaded = true;
        atomic_write(&self.dir.join("epoch"), &[epoch.as_bytes()]).map_err(|e| e.to_string())?;
        self.epoch = epoch;
        match std::fs::remove_file(self.dir.join("records")) {
            Ok(()) => Ok(()), Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()), Err(e) => Err(e.to_string()),
        }
    }

    fn prune_projects(&self) {
        let Some(base) = self.dir.parent() else { return };
        let Ok(dirs) = std::fs::read_dir(base) else { return };
        let mut dirs: Vec<_> = dirs.filter_map(Result::ok).filter(|e| {
            let name = e.file_name(); let name = name.to_string_lossy();
            name.len() == 64 && name.bytes().all(|c| c.is_ascii_hexdigit()) && e.file_type().is_ok_and(|t| t.is_dir()) && e.path() != self.dir
        }).filter_map(|e| e.metadata().ok().and_then(|m| m.modified().ok()).map(|t| (t, e.path()))).collect();
        dirs.sort_by_key(|(t, _)| *t);
        for (_, dir) in dirs.iter().take(dirs.len().saturating_sub(PROJECTS - 1)) { let _ = std::fs::remove_dir_all(dir); }
    }
}

fn entry_bytes(items: &[Indexed]) -> usize { items.iter().map(Indexed::bytes).sum::<usize>() + 192 }

fn atomic_write(path: &Path, chunks: &[&[u8]]) -> std::io::Result<()> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let dir = path.parent().ok_or_else(|| std::io::Error::other("cache has no parent"))?;
    std::fs::create_dir_all(dir)?;
    let temp = path.with_extension(format!("{}-{}.tmp", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&temp)?;
        for bytes in chunks { file.write_all(bytes)?; }
        drop(file);
        std::fs::rename(&temp, path)
    })();
    if result.is_err() { let _ = std::fs::remove_file(temp); }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::{self, FileIndex};

    fn read(cache: &mut Cache, root: &Path, path: &str) -> Arc<[Indexed]> {
        cache.load();
        match index::index_file_cached(root, path, || true, Some(cache)) {
            FileIndex::Done { items, .. } => items, _ => panic!("fixture must index"),
        }
    }

    #[test]
    fn content_versions_survive_reopen_rename_and_preserved_metadata_changes() {
        let d = tempfile::tempdir().unwrap(); let root = d.path().join("project"); std::fs::create_dir(&root).unwrap();
        let path = root.join("a.rb"); std::fs::write(&path, "class Alpha; end\n").unwrap();
        let time = std::fs::metadata(&path).unwrap().modified().unwrap();
        let mut cache = Cache::new(d.path(), &root);
        let first = read(&mut cache, &root, "a.rb");
        assert_eq!(first[0].decl.name.as_ref(), "Alpha");
        assert!(Arc::ptr_eq(&first, &read(&mut cache, &root, "a.rb")));
        std::fs::write(&path, "class Omega; end\n").unwrap();
        std::fs::File::options().write(true).open(&path).unwrap().set_modified(time).unwrap();
        assert_eq!(read(&mut cache, &root, "a.rb")[0].decl.name.as_ref(), "Omega");
        std::fs::write(&path, "class Alpha; end\n").unwrap();
        std::fs::rename(&path, root.join("renamed.rb")).unwrap();
        assert!(Arc::ptr_eq(&first, &read(&mut cache, &root, "renamed.rb")));
        cache.flush(|| true);
        let mut reopened = Cache::new(d.path(), &root); reopened.load();
        assert_eq!(reopened.entries.len(), 2);
        assert_eq!(read(&mut reopened, &root, "renamed.rb")[0].decl, first[0].decl);
        assert!(matches!(index::index_file_cached(&root, "a.rb", || true, Some(&mut reopened)), FileIndex::Unreadable));
    }

    #[test]
    fn corruption_rebuild_and_late_writers_cannot_restore_cleared_records() {
        let d = tempfile::tempdir().unwrap(); let root = d.path(); std::fs::write(root.join("a.rb"), "class Alpha; end\n").unwrap();
        let mut cache = Cache::new(d.path(), root); read(&mut cache, root, "a.rb"); cache.flush(|| true);
        let old = std::fs::read(cache.dir.join("records")).unwrap();
        let mut other = Cache::new(d.path(), root); other.load(); assert_eq!(other.entries.len(), 1);
        cache.clear().unwrap();
        std::fs::write(cache.dir.join("records"), &old).unwrap(); // old process won a race after the clear
        other.load(); assert!(other.entries.is_empty());
        let mut reopened = Cache::new(d.path(), root); reopened.load(); assert!(reopened.entries.is_empty());
        read(&mut reopened, root, "a.rb"); reopened.flush(|| true);
        let mut bytes = std::fs::read(cache.dir.join("records")).unwrap();
        let last = bytes.len() - 2; bytes[last] ^= 1;
        std::fs::write(cache.dir.join("records"), bytes).unwrap();
        let mut corrupt = Cache::new(d.path(), root); corrupt.load(); assert!(corrupt.entries.is_empty());
        assert_eq!(read(&mut corrupt, root, "a.rb")[0].decl.name.as_ref(), "Alpha");
    }

    #[test]
    fn storage_is_bounded_and_cancellation_or_a_different_extractor_is_a_miss() {
        let d = tempfile::tempdir().unwrap(); let root = d.path(); std::fs::write(root.join("a.rb"), "class Alpha; end\n").unwrap();
        let mut cache = Cache::new(d.path(), root); let items = read(&mut cache, root, "a.rb");
        cache.flush(|| false); assert!(!cache.dir.join("records").exists());
        cache.flush(|| true);
        let bytes = std::fs::read(cache.dir.join("records")).unwrap();
        let mut saved: Saved = serde_json::from_slice(&bytes[65..]).unwrap(); saved.extractor = "older extractor".into();
        let body = serde_json::to_vec(&saved).unwrap();
        atomic_write(&cache.dir.join("records"), &[digest(&body).as_bytes(), b"\n", &body]).unwrap();
        let mut next = Cache::new(d.path(), root); next.load(); assert!(next.entries.is_empty());
        let mut big = items[0].clone(); big.decl.name = "x".repeat(INDEX_BYTES / 4).into();
        let big: Arc<[Indexed]> = vec![big].into();
        for i in 0..10 { next.insert(digest(&[i]), big.clone(), false); }
        assert!(next.bytes <= INDEX_BYTES && next.entries.len() < 10);
        assert!(next.get(&digest(&[9])).is_some());
        assert!(next.get(&digest(&[0])).is_none());
        for i in 0..PROJECTS + 3 {
            let mut c = Cache::new(d.path(), &root.join(i.to_string())); c.load();
            c.insert(digest(b"key"), items.clone(), false); c.flush(|| true);
        }
        assert_eq!(std::fs::read_dir(d.path().join("declarations-v1")).unwrap().count(), PROJECTS);
    }
}
