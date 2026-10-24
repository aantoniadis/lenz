//! Private recoverable drafts: a document's unsaved text with the confirmed baseline it was edited from, kept
//! outside the checkout so an interrupted session can offer the work again. A record is validated before it is
//! used, an explicit Discard leaves a tombstone the record cannot outlive, and no source text is ever logged.
use std::{
    collections::BTreeMap,
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, mpsc},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use crate::text::TextSnapshot;

/// The saved format's version; a record of another version is left alone and not read as this one.
const VERSION: u32 = 1;
/// The most draft and baseline text one record holds; a larger version is not written.
pub const RECORD_BYTES: u64 = 8 << 20;
/// Records read at start, at most, and the text read from them in total.
pub const MAX_RECORDS: usize = 20;
const LOADED_BYTES: u64 = 32 << 20;
/// A change is written once no newer one arrived for this long.
pub const DEBOUNCE: Duration = Duration::from_millis(500);
/// The tombstone log: keys and write order only, never text.
const TOMBSTONES_BYTES: u64 = 256 << 10;
/// The largest file whose text is compared with a record's confirmed baseline.
const COMPARE_BYTES: u64 = 64 << 20;
/// The title kept with a record, for the dialog.
const TITLE_CHARS: usize = 1024;

/// What a recovered draft belongs to.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Key {
    /// A document with this absolute path.
    File { path: PathBuf },
    /// A New File document's own token: it has no path until Save As.
    Untitled { token: String },
}

impl Key {
    /// One record per key, named for a stable hash of it.
    fn file_name(&self) -> String {
        let seed: Vec<u8> = match self {
            Self::File { path } => [b"f".as_slice(), path.as_os_str().as_encoded_bytes()].concat(),
            Self::Untitled { token } => [b"u".as_slice(), token.as_bytes()].concat(),
        };
        format!("{}.json", digest(&seed))
    }
    /// The path a file record belongs to.
    pub fn path(&self) -> Option<&Path> {
        match self {
            Self::File { path } => Some(path),
            Self::Untitled { .. } => None,
        }
    }
}

/// One recoverable version, as read back from its record.
#[derive(Clone, PartialEq, Eq)]
pub struct Record {
    key: Key,
    /// Its place in the write order: a tombstone at or above it keeps it from returning.
    seq: u64,
    title: String,
    draft: Arc<str>,
    baseline: Arc<str>,
    encoding: &'static str,
    bom: bool,
    /// When it was written, in seconds since the epoch, for the dialog.
    recorded: u64,
}

impl Record {
    pub fn key(&self) -> &Key { &self.key }
    pub fn title(&self) -> &str { &self.title }
    pub fn recorded(&self) -> u64 { self.recorded }
    /// The unsaved text.
    pub fn draft(&self) -> &str { &self.draft }
    /// The last confirmed saved text the draft was edited from.
    pub fn baseline(&self) -> &str { &self.baseline }
    /// The encoding the document had, to keep it across Save As.
    pub fn encoding(&self) -> &'static str { self.encoding }
    pub fn bom(&self) -> bool { self.bom }
    fn text_bytes(&self) -> u64 { (self.draft.len() + self.baseline.len()) as u64 }
}

/// Source text stays out of diagnostics: a record's `Debug` names its identity and size only.
impl std::fmt::Debug for Record {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Record").field("key", &self.key).field("seq", &self.seq).field("title", &self.title)
            .field("encoding", &self.encoding).field("bom", &self.bom).field("recorded", &self.recorded)
            .field("text_bytes", &self.text_bytes()).finish()
    }
}

/// What the document's file looks like now, compared with the record's confirmed baseline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Disk {
    /// Untouched since the draft's confirmed baseline.
    Matches,
    /// Changed, or unreadable in a way that cannot be compared: Save must not assume either version.
    Differs,
    /// No file there any more.
    Missing,
    /// A New File document: Save As chooses where it goes.
    Unsaved,
}

/// A recoverable version offered at start.
#[derive(Debug)]
pub struct Offer {
    id: u64,
    record: Record,
    disk: Disk,
}

impl Offer {
    pub fn id(&self) -> u64 { self.id }
    pub fn record(&self) -> &Record { &self.record }
    pub fn disk(&self) -> Disk { self.disk }
    /// The dialog for this offer; `remaining` counts the offers still undecided.
    pub fn prompt(&self, remaining: usize, restore_disabled: Option<&'static str>) -> Prompt {
        Prompt { ticket: self.id, label: self.record.title.clone(), remaining, disk: self.disk, recorded: self.record.recorded, restore_disabled, discard_disabled: None }
    }
}

/// The recovery dialog's answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    Restore,
    Discard,
    /// Keep it for this session; Recover Unsaved Draft… offers it again.
    Later,
}

impl Choice {
    pub const ALL: [Self; 3] = [Self::Restore, Self::Discard, Self::Later];
    pub fn label(self) -> &'static str {
        match self {
            Self::Restore => "Restore Draft",
            Self::Discard => "Discard Draft",
            Self::Later => "Later",
        }
    }
}

/// What the adapters show and dispatch for one recovered draft.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prompt {
    pub ticket: u64,
    pub label: String,
    /// Offers still undecided, this one included.
    pub remaining: usize,
    /// The file as it is now, against the record's confirmed baseline.
    pub disk: Disk,
    /// When the record was written, in seconds since the epoch.
    pub recorded: u64,
    pub restore_disabled: Option<&'static str>,
    pub discard_disabled: Option<&'static str>,
}

impl Prompt {
    pub fn disabled(&self, choice: Choice) -> Option<&'static str> {
        match choice {
            Choice::Restore => self.restore_disabled,
            Choice::Discard => self.discard_disabled,
            Choice::Later => None,
        }
    }
    /// The dialog's text: identity, baseline comparison and choices. Never the source itself.
    pub fn text(&self) -> String {
        let mut text = format!("{}\n\nRecorded {}.\n\n", self.label, age(self.recorded));
        text.push_str(match self.disk {
            Disk::Matches => "The file on disk still matches this draft's confirmed baseline (the last saved version).\n",
            Disk::Differs => "The file on disk has changed since this draft's confirmed baseline. Compare the restored draft with the file before saving; Save stays closed until the current file is rechecked.\n",
            Disk::Missing => "The file is no longer on disk; the recovered draft can still be restored and saved elsewhere.\n",
            Disk::Unsaved => "This New File document was never saved; Save As chooses where it goes.\n",
        });
        text.push_str("\nRestore Draft reopens the work with its own undo history; the file on disk is not changed.\nDiscard Draft deletes the recovered copy: this draft will not be offered again.\nLater keeps it for this session; Recover Unsaved Draft… offers it again.\n");
        text.push_str(&format!("\n{} (R)\n{} (D)\n{} (Esc)\n", Choice::Restore.label(), Choice::Discard.label(), Choice::Later.label()));
        if self.remaining > 1 {
            text.push_str(&format!("\n{} recovered drafts are undecided.\n", self.remaining));
        }
        if let Some(why) = self.restore_disabled {
            text.push_str(&format!("\n{}: {why}\n", Choice::Restore.label()));
        }
        if let Some(why) = self.discard_disabled {
            text.push_str(&format!("\n{}: {why}\n", Choice::Discard.label()));
        }
        text
    }
}

/// The session's recovery state: the offers read at start, newest first, and the saver keeping the records current.
pub(crate) struct Recovery {
    offers: Vec<Offer>,
    saver: Saver,
    /// The user chose Later: the dialog stays closed until they ask for it again.
    postponed: bool,
}

impl Recovery {
    pub(crate) fn open(dir: PathBuf) -> Self {
        let mut loaded = Store::new(dir.clone()).load();
        let mut offers: Vec<Offer> = Vec::new();
        for record in std::mem::take(&mut loaded.records).into_iter().rev() {
            let disk = match record.key().path() {
                Some(path) => compare(path, &record),
                None => Disk::Unsaved,
            };
            offers.push(Offer { id: offers.len() as u64 + 1, record, disk });
        }
        Self { offers, saver: Saver::new(dir, &loaded), postponed: false }
    }

    /// The recovered drafts still undecided, newest first.
    pub(crate) fn offers(&self) -> &[Offer] { &self.offers }

    pub(crate) fn offer(&self, id: u64) -> Option<&Offer> { self.offers.iter().find(|offer| offer.id == id) }

    /// The offer the dialog shows: the newest one, until the user postpones the rest.
    pub(crate) fn prompt_offer(&self) -> Option<&Offer> { (!self.postponed).then(|| self.offers.first()).flatten() }

    pub(crate) fn take(&mut self, id: u64) -> Option<Offer> {
        let at = self.offers.iter().position(|offer| offer.id == id)?;
        Some(self.offers.remove(at))
    }

    pub(crate) fn postpone(&mut self) { self.postponed = true; }

    pub(crate) fn resume(&mut self) { self.postponed = false; }

    /// Keeps the next version of a document, or forgets it when it holds no unsaved work.
    pub(crate) fn save(&self, version: Version) { self.saver.save(version); }

    /// The document's work was saved or discarded: it must not be offered again.
    pub(crate) fn discard(&self, key: Key) { self.saver.discard(key); }

    /// Waits for what is pending to reach the disk (tests, and shut-down checks).
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn flush(&self) { let _ = self.saver.flush().recv_timeout(Duration::from_secs(5)); }
}

/// A version to write: its text is materialized off the UI thread.
pub(crate) struct Version {
    pub(crate) key: Key,
    pub(crate) title: String,
    pub(crate) draft: Arc<TextSnapshot>,
    pub(crate) baseline: Arc<TextSnapshot>,
    pub(crate) encoding: &'static str,
    pub(crate) bom: bool,
}

impl std::fmt::Debug for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Version").field("key", &self.key).field("title", &self.title).field("encoding", &self.encoding).field("bom", &self.bom).finish()
    }
}

impl Version {
    /// The record to write, or `None` when the version is no unsaved work or too large to keep.
    fn record(self, seq: u64) -> Option<Record> {
        let (draft, baseline) = (self.draft.materialize(), self.baseline.materialize());
        if draft.text() == baseline.text() || (draft.text().len() + baseline.text().len()) as u64 > RECORD_BYTES {
            return None;
        }
        Some(Record { key: self.key, seq, title: self.title, draft: draft.text().clone(), baseline: baseline.text().clone(), encoding: self.encoding, bom: self.bom, recorded: now() })
    }
}

/// The records and tombstones of one directory.
pub(crate) struct Store {
    dir: PathBuf,
}

/// What a directory held: its valid records oldest first, the tombstones, and the highest write order seen.
pub(crate) struct Loaded {
    pub(crate) records: Vec<Record>,
    pub(crate) tombstones: BTreeMap<Key, u64>,
    pub(crate) high: u64,
}

impl Store {
    pub(crate) fn new(dir: PathBuf) -> Self { Self { dir } }

    /// Only a record that parses, carries this version and is not tombstoned is offered; a record the user
    /// discarded is removed. Nothing is inferred from a path alone.
    pub(crate) fn load(&self) -> Loaded {
        let (tombstones, mut high) = read_tombstones(&self.dir.join("tombstones.json")).unwrap_or_default();
        let mut records = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&self.dir) {
            let mut files: Vec<PathBuf> = entries.flatten().map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e == "json") && p.file_name().is_none_or(|n| n != "tombstones.json"))
                .filter(|p| p.symlink_metadata().is_ok_and(|m| m.is_file())).collect();
            files.sort();
            for file in files {
                let Some(record) = std::fs::File::open(&file).ok().and_then(read_record) else { continue };
                high = high.max(record.seq);
                if tombstones.get(record.key()).is_some_and(|seq| *seq >= record.seq) {
                    let _ = std::fs::remove_file(&file);
                    continue;
                }
                records.push(record);
            }
        }
        records.sort_by_key(|record| (record.recorded, record.seq));
        records.truncate(MAX_RECORDS);
        let mut text = 0;
        records.retain(|record| {
            if text + record.text_bytes() > LOADED_BYTES { return false; }
            text += record.text_bytes();
            true
        });
        Loaded { records, tombstones, high }
    }

    fn record_file(&self, key: &Key) -> PathBuf { self.dir.join(key.file_name()) }

    fn write(&self, record: &Record) {
        let Ok(bytes) = serde_json::to_vec(&Saved::from(record)) else { return };
        // A version too large to keep must not leave an older one to be offered as the current work.
        if bytes.len() as u64 > RECORD_BYTES { self.remove(record.key()); return; }
        write_private(&self.record_file(record.key()), &bytes);
    }

    fn remove(&self, key: &Key) -> bool {
        match std::fs::remove_file(self.record_file(key)) {
            Ok(()) => true,
            Err(e) => e.kind() == std::io::ErrorKind::NotFound,
        }
    }

    fn write_tombstones(&self, tombstones: &BTreeMap<Key, u64>) {
        let stored = StoredTombstones { v: VERSION, keys: tombstones.iter().map(|(key, seq)| StoredTombstone { key: key.clone(), seq: *seq }).collect() };
        if let Ok(bytes) = serde_json::to_vec(&stored) { write_private(&self.dir.join("tombstones.json"), &bytes); }
    }
}

/// A version as it is kept: text, its identity and the write order it must beat to return.
#[derive(serde::Serialize, serde::Deserialize)]
struct Saved {
    v: u32,
    seq: u64,
    key: Key,
    title: String,
    draft: String,
    baseline: String,
    encoding: String,
    bom: bool,
    recorded: u64,
}

impl From<&Record> for Saved {
    fn from(record: &Record) -> Self {
        Self { v: VERSION, seq: record.seq, key: record.key.clone(), title: record.title.clone(), draft: record.draft.to_string(),
            baseline: record.baseline.to_string(), encoding: record.encoding.into(), bom: record.bom, recorded: record.recorded }
    }
}

/// Only the version, read first: another version's record may not parse as this one's.
#[derive(serde::Deserialize)]
struct Versioned {
    v: u32,
}

/// A record from its file: `None` when it is another version's, larger than a record, or not a valid record.
fn read_record(file: std::fs::File) -> Option<Record> {
    let mut bytes = Vec::new();
    file.take(RECORD_BYTES.saturating_add(1)).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > RECORD_BYTES { return None; }
    if serde_json::from_slice::<Versioned>(&bytes).ok()?.v != VERSION { return None; }
    let saved: Saved = serde_json::from_slice(&bytes).ok()?;
    let encoding = encoding(&saved.encoding)?;
    let valid_title = !saved.title.chars().any(char::is_control) && saved.title.chars().count() <= TITLE_CHARS;
    let valid_text = saved.draft.len() + saved.baseline.len() <= RECORD_BYTES as usize && saved.draft != saved.baseline;
    (valid_key(&saved.key) && valid_title && valid_text).then(|| Record {
        key: saved.key, seq: saved.seq, title: saved.title, draft: saved.draft.into(), baseline: saved.baseline.into(),
        encoding, bom: saved.bom, recorded: saved.recorded,
    })
}

/// Whether a record's identity is one this reader could have written: an absolute file path, or a New File
/// token. A hand-edited record cannot point elsewhere or name a retrieved source.
fn valid_key(key: &Key) -> bool {
    match key {
        Key::File { path } => path.is_absolute()
            && path.to_str().is_some_and(|p| !p.contains("://"))
            && path.components().all(|c| matches!(c, std::path::Component::RootDir | std::path::Component::Normal(_) | std::path::Component::Prefix(_))),
        Key::Untitled { token } => !token.is_empty() && token.len() <= 200 && token.chars().all(|c| c.is_ascii_graphic()),
    }
}

/// The canonical WHATWG name for a stored label, when the decoder agrees: a record cannot choose another charset.
fn encoding(label: &str) -> Option<&'static str> {
    let decoded = lenz_content::encoding::decode(b"", Some(label));
    (decoded.encoding == label && !decoded.had_bom).then_some(decoded.encoding)
}

#[derive(serde::Serialize, serde::Deserialize)]
struct StoredTombstones {
    v: u32,
    #[serde(default)]
    keys: Vec<StoredTombstone>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct StoredTombstone {
    key: Key,
    seq: u64,
}

/// The discarded keys and the write order that must not return; an unreadable log is no tombstones.
fn read_tombstones(file: &Path) -> Option<(BTreeMap<Key, u64>, u64)> {
    let mut bytes = Vec::new();
    std::fs::File::open(file).ok()?.take(TOMBSTONES_BYTES + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > TOMBSTONES_BYTES { return None; }
    let stored: StoredTombstones = serde_json::from_slice(&bytes).ok()?;
    if stored.v != VERSION { return None; }
    let mut high = 0;
    let tombstones = stored.keys.into_iter().filter(|t| valid_key(&t.key)).take(MAX_RECORDS * 4).map(|t| {
        high = high.max(t.seq);
        (t.key, t.seq)
    }).collect();
    Some((tombstones, high))
}

/// Writes recoverable versions and their tombstones off the UI thread, one job at a time so a discard ordered
/// after a write always wins. Only the newest version of each document waits to be written, so a burst of edits
/// retains no earlier snapshot. Dropping the saver writes what is pending and waits for it.
pub(crate) struct Saver {
    pending: std::sync::Arc<std::sync::Mutex<BTreeMap<Key, Version>>>,
    tx: Option<mpsc::Sender<Job>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

enum Job {
    Wake,
    Discard(Key),
    #[cfg(any(test, feature = "test-support"))]
    Flush(mpsc::Sender<()>),
}

impl Saver {
    pub(crate) fn new(dir: PathBuf, loaded: &Loaded) -> Self {
        let (tx, rx) = mpsc::channel::<Job>();
        let pending = std::sync::Arc::new(std::sync::Mutex::new(BTreeMap::new()));
        let (tombstones, high) = (loaded.tombstones.clone(), loaded.high);
        let slots = pending.clone();
        let run = move || run(Store::new(dir), tombstones, high.max(now_millis()), slots, rx);
        let thread = std::thread::Builder::new().name("reader-recovery".into()).spawn(run).ok();
        Self { pending, tx: Some(tx), thread }
    }

    pub(crate) fn save(&self, version: Version) {
        // Replacing the earlier version here releases its snapshot at once, not in a queue.
        self.pending.lock().unwrap_or_else(std::sync::PoisonError::into_inner).insert(version.key.clone(), version);
        if let Some(tx) = &self.tx { let _ = tx.send(Job::Wake); }
    }

    /// The draft was saved, discarded or closed explicitly: it must not be offered again.
    pub(crate) fn discard(&self, key: Key) {
        // A waiting version is released here, as `save` releases the one it replaces: a draft that became clean or
        // was closed must not keep its snapshot alive until the writer thread next wakes.
        self.pending.lock().unwrap_or_else(std::sync::PoisonError::into_inner).remove(&key);
        if let Some(tx) = &self.tx { let _ = tx.send(Job::Discard(key)); }
    }

    /// Writes what is pending and says when it is done.
    #[cfg(any(test, feature = "test-support"))]
    pub fn flush(&self) -> mpsc::Receiver<()> {
        let (done, ready) = mpsc::channel();
        if let Some(tx) = &self.tx { let _ = tx.send(Job::Flush(done)); }
        ready
    }
}

impl Drop for Saver {
    fn drop(&mut self) {
        self.tx = None;
        if let Some(t) = self.thread.take() { let _ = t.join(); }
    }
}

fn run(store: Store, mut tombstones: BTreeMap<Key, u64>, mut next: u64, pending: std::sync::Arc<std::sync::Mutex<BTreeMap<Key, Version>>>, rx: mpsc::Receiver<Job>) {
    let waiting = |pending: &std::sync::Mutex<BTreeMap<Key, Version>>| !pending.lock().unwrap_or_else(std::sync::PoisonError::into_inner).is_empty();
    let write = |tombstones: &BTreeMap<Key, u64>, next: &mut u64| {
        let versions: Vec<Version> = std::mem::take(&mut *pending.lock().unwrap_or_else(std::sync::PoisonError::into_inner)).into_values().collect();
        for version in versions {
            *next = (*next).max(now_millis()) + 1;
            let key = version.key.clone();
            match version.record(*next) {
                Some(record) if tombstones.get(&key).is_none_or(|seq| *seq < record.seq) => store.write(&record),
                // Nothing to recover (clean, or too large): an older record must not stand in for this work.
                _ => { store.remove(&key); }
            }
        }
    };
    loop {
        let received = if waiting(&pending) { rx.recv_timeout(DEBOUNCE) } else { rx.recv().map_err(|_| mpsc::RecvTimeoutError::Disconnected) };
        match received {
            // A newer change restarts the quiet period, so a burst is written once.
            Ok(Job::Wake) => {}
            Ok(Job::Discard(key)) => {
                pending.lock().unwrap_or_else(std::sync::PoisonError::into_inner).remove(&key);
                next = next.max(now_millis()) + 1;
                tombstones.insert(key.clone(), next);
                store.write_tombstones(&tombstones);
                store.remove(&key);
            }
            #[cfg(any(test, feature = "test-support"))]
            Ok(Job::Flush(done)) => {
                write(&tombstones, &mut next);
                let _ = done.send(());
            }
            Err(mpsc::RecvTimeoutError::Timeout) => write(&tombstones, &mut next),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                write(&tombstones, &mut next);
                return;
            }
        }
    }
}

/// A private file, atomically: a failed write loses recovery state, never a project file.
fn write_private(file: &Path, bytes: &[u8]) {
    use std::sync::atomic::{AtomicU64, Ordering};
    let Some(dir) = file.parent() else { return };
    static NEXT: AtomicU64 = AtomicU64::new(0);
    if std::fs::create_dir_all(dir).is_err() { return; }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
    let temp = dir.join(format!(".{}-{}-{}", file.file_name().map_or_else(|| "record".into(), |n| n.to_string_lossy().into_owned()), std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    let mut opened = std::fs::OpenOptions::new();
    opened.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opened.mode(0o600);
    }
    let ok = opened.open(&temp).and_then(|mut f| std::io::Write::write_all(&mut f, bytes)).is_ok() && std::fs::rename(&temp, file).is_ok();
    if !ok { let _ = std::fs::remove_file(&temp); }
}

/// A 16-character hex name for arbitrary bytes (FNV-1a), as places files are named.
fn digest(bytes: &[u8]) -> String {
    format!("{:016x}", bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)))
}

/// Seconds since the epoch, for a record's age; a clock before it is zero.
fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Milliseconds since the epoch: the write order, so a tombstone written later outranks earlier records.
fn now_millis() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

/// The document's file as it is now, compared with a record's confirmed baseline: its text, or `None` when it
/// cannot be read or compared.
pub(crate) fn compare(path: &Path, record: &Record) -> Disk {
    match std::fs::metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Disk::Missing,
        Err(_) => return Disk::Differs,
        Ok(meta) if !meta.is_file() => return Disk::Differs,
        Ok(meta) if meta.len() > COMPARE_BYTES => return Disk::Differs,
        Ok(_) => {}
    }
    let mut bytes = Vec::new();
    if std::fs::File::open(path).and_then(|f| f.take(COMPARE_BYTES + 1).read_to_end(&mut bytes)).is_err() { return Disk::Differs; }
    let decoded = lenz_content::encoding::decode(&bytes, Some(record.encoding()));
    if decoded.lossy || decoded.text != record.baseline() { Disk::Differs } else { Disk::Matches }
}

/// How long ago `recorded` (seconds since the epoch) was, in words for the dialog.
pub(crate) fn age(recorded: u64) -> &'static str {
    let seconds = now().saturating_sub(recorded);
    match seconds {
        0..=90 => "moments ago",
        91..=3599 => "within the last hour",
        3600..=86_399 => "earlier today",
        86_400..=172_799 => "yesterday",
        172_800..=604_799 => "days ago",
        _ => "weeks ago",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{document::Document, text::BufferId};

    fn snapshot(text: &str) -> Arc<TextSnapshot> {
        Arc::new(TextSnapshot::from_document(BufferId(1), &Document::decode(PathBuf::from("/p/a.txt"), 1, text.as_bytes())))
    }

    fn version(draft: &str, baseline: &str) -> Version {
        Version { key: Key::File { path: PathBuf::from("/p/a.txt") }, title: "a.txt".into(), draft: snapshot(draft), baseline: snapshot(baseline), encoding: "UTF-8", bom: false }
    }

    fn saver(dir: &Path) -> Saver {
        Saver::new(dir.to_owned(), &Store::new(dir.to_owned()).load())
    }

    fn flush(saver: &Saver) { saver.flush().recv_timeout(Duration::from_secs(5)).expect("the saver answered"); }

    #[test]
    fn a_version_is_kept_privately_and_read_back_as_written() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("recovery");
        let saver = saver(&dir);
        saver.save(version("draft λ\r\nsecond\n", "saved\r\nsecond\n"));
        flush(&saver);
        let loaded = Store::new(dir.clone()).load();
        assert_eq!(loaded.records.len(), 1);
        let record = &loaded.records[0];
        assert_eq!((record.key(), record.title(), record.draft(), record.baseline(), record.encoding(), record.bom()),
            (&Key::File { path: PathBuf::from("/p/a.txt") }, "a.txt", "draft λ\r\nsecond\n", "saved\r\nsecond\n", "UTF-8", false));
        let debug = format!("{record:?}");
        assert!(!debug.contains("draft λ") && !debug.contains("saved"), "source text never reaches diagnostics: {debug}");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777, 0o700);
            let file = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.path()).find(|p| p.file_name().is_some_and(|n| n != "tombstones.json")).unwrap();
            assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
        }
        drop(saver);
    }

    #[test]
    fn a_draft_that_matches_its_baseline_or_is_too_large_is_not_kept() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("recovery");
        let saver = saver(&dir);
        saver.save(version("same\n", "same\n"));
        flush(&saver);
        assert!(Store::new(dir.clone()).load().records.is_empty(), "a clean document has nothing to recover");
        // An older record must not be offered as the current work once the newer version cannot be kept.
        saver.save(version("small draft\n", "saved\n"));
        flush(&saver);
        assert_eq!(Store::new(dir.clone()).load().records.len(), 1);
        let huge = "x".repeat(RECORD_BYTES as usize + 1);
        saver.save(version(&huge, "saved\n"));
        flush(&saver);
        assert!(Store::new(dir.clone()).load().records.is_empty());
        drop(saver);
    }

    #[test]
    fn the_last_change_of_a_burst_is_what_survives() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("recovery");
        let saver = saver(&dir);
        for draft in ["one\n", "one two\n", "one two three\n"] { saver.save(version(draft, "saved\n")); }
        flush(&saver);
        let loaded = Store::new(dir.clone()).load();
        assert_eq!(loaded.records.len(), 1);
        assert_eq!(loaded.records[0].draft(), "one two three\n");
        drop(saver);
    }

    #[test]
    fn a_discard_releases_the_waiting_version_without_waiting_for_the_writer() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("recovery");
        let saver = saver(&dir);
        let draft = version("draft\n", "saved\n");
        let waiting = Arc::downgrade(&draft.draft);
        saver.save(draft);
        saver.discard(Key::File { path: PathBuf::from("/p/a.txt") });
        assert!(waiting.upgrade().is_none(), "a discarded version's snapshot is released now, not by the writer thread");
        drop(saver);
    }

    #[test]
    fn a_discarded_record_cannot_return_even_when_its_file_survives() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("recovery");
        let saver = saver(&dir);
        saver.save(version("discarded draft\n", "saved\n"));
        flush(&saver);
        let key = Key::File { path: PathBuf::from("/p/a.txt") };
        let file = dir.join(key.file_name());
        let kept = std::fs::read(&file).unwrap();
        saver.discard(key.clone());
        flush(&saver);
        assert!(!file.exists());
        // A record written before the discard and left behind (an interrupted earlier write) is not offered.
        std::fs::write(&file, &kept).unwrap();
        let loaded = Store::new(dir.clone()).load();
        assert!(loaded.records.is_empty(), "discarded work stays discarded");
        assert!(!file.exists(), "the tombstoned record is removed");
        assert!(loaded.tombstones.contains_key(&key));
        // Work done after the discard is new work, written later.
        saver.save(version("new work\n", "saved\n"));
        flush(&saver);
        assert_eq!(Store::new(dir.clone()).load().records.len(), 1);
        drop(saver);
    }

    fn write_saved(dir: &Path, saved: &Saved) { std::fs::write(dir.join(saved.key.file_name()), serde_json::to_vec(saved).unwrap()).unwrap(); }

    fn saved(draft: &str, baseline: &str, key: Key) -> Saved {
        Saved { v: VERSION, seq: 1, key, title: "a.txt".into(), draft: draft.into(), baseline: baseline.into(), encoding: "UTF-8".into(), bom: false, recorded: 1 }
    }

    #[test]
    fn broken_foreign_or_impossible_records_are_not_offered() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("recovery");
        std::fs::create_dir_all(&dir).unwrap();
        let file = Key::File { path: PathBuf::from("/p/a.txt") };
        std::fs::write(dir.join(super::digest(b"garbage").to_string() + ".json"), b"{not json").unwrap();
        let mut future = saved("d\n", "s\n", file.clone());
        future.v = VERSION + 1;
        write_saved(&dir, &future);
        write_saved(&dir, &saved("d\n", "s\n", Key::File { path: PathBuf::from("relative/a.txt") }));
        write_saved(&dir, &saved("d\n", "s\n", Key::File { path: PathBuf::from("untitled://2") }));
        write_saved(&dir, &saved("d\n", "s\n", Key::File { path: PathBuf::from("/p/../a.txt") }));
        write_saved(&dir, &saved("d\n", "s\n", Key::Untitled { token: String::new() }));
        let mut unknown = saved("d\n", "s\n", Key::File { path: PathBuf::from("/p/b.txt") });
        unknown.encoding = "no-such-charset".into();
        write_saved(&dir, &unknown);
        write_saved(&dir, &saved("same\n", "same\n", Key::File { path: PathBuf::from("/p/c.txt") }));
        assert!(Store::new(dir.clone()).load().records.is_empty(), "nothing impossible is offered");
        // A valid record of this version is still offered beside them.
        write_saved(&dir, &saved("draft\n", "saved\n", file.clone()));
        let loaded = Store::new(dir.clone()).load();
        assert_eq!(loaded.records.iter().map(|r| r.title.as_str()).collect::<Vec<_>>(), ["a.txt"]);
    }

    #[test]
    fn an_untitled_record_keeps_its_identity_and_utf8() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("recovery");
        let saver = saver(&dir);
        saver.save(Version { key: Key::Untitled { token: "1-0000-2".into() }, title: "Untitled-2".into(), ..version("new file\n", "") });
        flush(&saver);
        let loaded = Store::new(dir).load();
        assert_eq!(loaded.records[0].key(), &Key::Untitled { token: "1-0000-2".into() });
        assert_eq!((loaded.records[0].baseline(), loaded.records[0].encoding()), ("", "UTF-8"));
        drop(saver);
    }

    #[test]
    fn a_baseline_comparison_names_the_disk_state_without_changing_it() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("recovery");
        let path = d.path().join("a.txt");
        std::fs::write(&path, "saved\n").unwrap();
        let saver = saver(&dir);
        let mut v = version("draft\n", "saved\n");
        v.key = Key::File { path: path.clone() };
        saver.save(v);
        flush(&saver);
        let record = Store::new(dir).load().records.remove(0);
        assert_eq!(compare(&path, &record), Disk::Matches);
        let before = std::fs::read(&path).unwrap();
        std::fs::write(&path, "changed\n").unwrap();
        assert_eq!(compare(&path, &record), Disk::Differs);
        std::fs::remove_file(&path).unwrap();
        assert_eq!(compare(&path, &record), Disk::Missing);
        assert_eq!(before, b"saved\n");
        drop(saver);
    }

    #[test]
    fn the_dialog_names_the_draft_its_baseline_state_and_every_choice() {
        let saver_dir = tempfile::tempdir().unwrap();
        let dir = saver_dir.path().join("recovery");
        let saver = saver(&dir);
        saver.save(version("draft-body-token\n", "baseline-body-token\n"));
        flush(&saver);
        let loaded = Store::new(dir).load();
        let record = loaded.records.into_iter().next().unwrap();
        let offer = Offer { id: 7, record, disk: Disk::Differs };
        let prompt = offer.prompt(2, Some("The file already has unsaved changes"));
        let text = prompt.text();
        assert_eq!(prompt.disabled(Choice::Restore), Some("The file already has unsaved changes"));
        assert_eq!(prompt.disabled(Choice::Discard), None);
        for part in ["a.txt", "Recorded ", "has changed since this draft's confirmed baseline", "Restore Draft (R)", "Discard Draft (D)", "Later (Esc)", "2 recovered drafts are undecided", "The file already has unsaved changes"] {
            assert!(text.contains(part), "{part}: {text}");
        }
        assert!(!text.contains("draft-body-token") && !text.contains("baseline-body-token"), "the dialog never shows source text: {text}");
        drop(saver);
    }
}
