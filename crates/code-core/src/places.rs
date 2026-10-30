//! A project's bookmarks, recent files and recent locations. Each mark keeps its line's text, so it finds the line
//! again after the file changes; everything is saved outside the checkout, one file per project root.
use std::{
    collections::{HashMap, VecDeque},
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, mpsc},
    time::Duration,
};

use crate::{
    text::TextSnapshot,
    tabs::Location,
};

/// Recent files kept.
pub const RECENT_FILES: usize = 50;
/// Recent locations kept.
pub const RECENT_LOCATIONS: usize = 30;
/// A new location this many lines or fewer from a recent one in the same file replaces it.
pub const MERGE_LINES: usize = 5;
/// The largest places file read.
pub const PLACES_BYTES: u64 = 4 << 20;
/// A change is saved once no newer one arrived for this long.
pub const DEBOUNCE: Duration = Duration::from_secs(1);
/// How long a save waits for another window's write before it writes anyway.
pub const LOCK_WAIT: Duration = Duration::from_secs(2);
/// The saved format's version; a file of another one is left alone.
const VERSION: u32 = 1;
/// The largest line or byte offset read from a places file: past any document, and clear of overflow.
pub const MAX_POSITION: usize = i32::MAX as usize;

/// A place in a file: its line and the line's text (its first [`crate::tabs::ANCHOR_CHARS`] characters) to find it again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mark {
    pub source: Arc<Path>,
    pub line: usize,
    pub anchor: Arc<str>,
    /// The byte within the line.
    pub offset: usize,
    /// A bookmark's key (0–9, A–Z).
    pub mnemonic: Option<char>,
    /// The line's text was not found when the file last changed: the mark kept its line number.
    pub moved: bool,
}

impl Mark {
    /// Byte `offset` of line `line` of `doc` (both clamped), shown from `source`.
    pub fn at(source: &Path, doc: &TextSnapshot, line: usize, offset: usize) -> Self {
        let line = line.min(doc.line_count() - 1);
        let range = doc.reader_line_range(line);
        Self { source: source.into(), line, anchor: crate::tabs::anchor(doc, line).into(), offset: doc.floor_char_boundary(range.start.saturating_add(offset).min(range.end)) - range.start, mnemonic: None, moved: false }
    }

    pub fn from_location(at: &Location) -> Self {
        Self { source: at.source.as_path().into(), line: at.line, anchor: at.anchor.as_ref().into(), offset: at.offset, mnemonic: None, moved: false }
    }

    fn key(&self) -> (&Path, usize) {
        (&self.source, self.line)
    }
}

/// Moves `source`'s marks to the line of `doc` whose text is their anchor, nearest the old line (a tie goes up); a
/// mark without one keeps its line, even past the end (a file cut short and restored gets each mark back, key and
/// all), and is `moved`; going there stops at the last line. Whether any mark changed.
fn relocate(marks: &mut [Mark], source: &Path, doc: &TextSnapshot) -> bool {
    let last = doc.line_count() - 1;
    let anchored = |l: usize, m: &Mark| crate::tabs::anchor_matches(doc, l, &m.anchor);
    // ponytail: one String per line, built only when some mark's line changed; an index per document if this shows up.
    let mut lines: Option<HashMap<String, Vec<usize>>> = None;
    let mut changed = false;
    for m in marks.iter_mut().filter(|m| *m.source == *source) {
        let before = (m.line, m.offset, m.moved);
        if m.line <= last && anchored(m.line, m) {
            m.moved = false;
        } else {
            let lines = lines.get_or_insert_with(|| {
                let mut map: HashMap<String, Vec<usize>> = HashMap::new();
                for l in 0..=last {
                    map.entry(crate::tabs::anchor(doc, l).into()).or_default().push(l);
                }
                map
            });
            match lines.get(&*m.anchor).and_then(|ls| nearest(ls, m.line)) {
                Some(l) => (m.line, m.moved) = (l, false),
                None => m.moved = true,
            }
        }
        if m.line <= last {
            m.offset = { let r = doc.reader_line_range(m.line); doc.floor_char_boundary(r.start.saturating_add(m.offset).min(r.end)) - r.start };
        }
        changed |= before != (m.line, m.offset, m.moved);
    }
    changed
}

/// The entry of sorted `lines` nearest `old`; a tie goes to the smaller.
fn nearest(lines: &[usize], old: usize) -> Option<usize> {
    let i = lines.partition_point(|&l| l < old);
    match (i.checked_sub(1).map(|i| lines[i]), lines.get(i).copied()) {
        (Some(up), Some(down)) => Some(if old - up <= down - old { up } else { down }),
        (up, down) => up.or(down),
    }
}

/// One project's places.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Places {
    pub(crate) workspace: Option<Box<crate::workspace::Workspace>>,
    /// Sorted by source, then line; one per line.
    bookmarks: Vec<Mark>,
    /// Most recently left first.
    recent_files: VecDeque<PathBuf>,
    /// Most recent first.
    recent_locations: VecDeque<Mark>,
}

impl Places {
    pub fn bookmarks(&self) -> &[Mark] {
        &self.bookmarks
    }

    pub fn recent_files(&self) -> &VecDeque<PathBuf> {
        &self.recent_files
    }

    pub fn recent_locations(&self) -> &VecDeque<Mark> {
        &self.recent_locations
    }

    fn find(&self, source: &Path, line: usize) -> Result<usize, usize> {
        self.bookmarks.binary_search_by(|m| m.key().cmp(&(source, line)))
    }

    /// The bookmark on line `line` of `source`.
    pub fn bookmark(&self, source: &Path, line: usize) -> Option<&Mark> {
        self.find(source, line).ok().map(|i| &self.bookmarks[i])
    }

    /// Add an anonymous bookmark at `mark`, or remove the one on its line: whether one was added.
    pub fn toggle(&mut self, mark: Mark) -> bool {
        match self.find(&mark.source, mark.line) {
            Ok(i) => {
                self.bookmarks.remove(i);
                false
            }
            Err(i) => {
                self.bookmarks.insert(i, Mark { mnemonic: None, moved: false, ..mark });
                true
            }
        }
    }

    /// Key `key` on `mark`'s line: the line's own key removes its bookmark; another key is set there (adding the
    /// bookmark), taken from any other bookmark that had it.
    pub fn set_mnemonic(&mut self, mark: Mark, key: char) {
        let key = key.to_ascii_uppercase();
        match self.find(&mark.source, mark.line) {
            Ok(i) if self.bookmarks[i].mnemonic == Some(key) => {
                self.bookmarks.remove(i);
            }
            found => {
                for m in self.bookmarks.iter_mut().filter(|m| m.mnemonic == Some(key)) {
                    m.mnemonic = None;
                }
                match found {
                    Ok(i) => self.bookmarks[i].mnemonic = Some(key),
                    Err(i) => self.bookmarks.insert(i, Mark { mnemonic: Some(key), moved: false, ..mark }),
                }
            }
        }
    }

    pub fn mnemonic(&self, key: char) -> Option<&Mark> {
        let key = key.to_ascii_uppercase();
        self.bookmarks.iter().find(|m| m.mnemonic == Some(key))
    }

    /// The bookmark after (or before) line `line` of `source`, by path then line, wrapping. A bookmark of `source`
    /// past its `last` line counts as on it (going there stops there).
    pub fn step(&self, source: &Path, line: usize, last: usize, forward: bool) -> Option<&Mark> {
        let n = self.bookmarks.len();
        if n == 0 {
            return None;
        }
        // Clamping keeps `source`'s marks in order, so the list stays sorted by this key.
        let shown = |m: &Mark| if *m.source == *source { m.line.min(last) } else { m.line };
        let i = if forward {
            self.bookmarks.partition_point(|m| (&*m.source, shown(m)) <= (source, line)) % n
        } else {
            (self.bookmarks.partition_point(|m| (&*m.source, shown(m)) < (source, line)) + n - 1) % n
        };
        Some(&self.bookmarks[i])
    }

    pub fn remove_bookmark(&mut self, source: &Path, line: usize) {
        if let Ok(i) = self.find(source, line) {
            self.bookmarks.remove(i);
        }
    }

    /// The reader left `path`: it is the most recent file.
    pub fn left_file(&mut self, path: &Path) {
        self.recent_files.retain(|p| p != path);
        self.recent_files.push_front(path.to_owned());
        self.recent_files.truncate(RECENT_FILES);
    }

    /// `from` was renamed to `to`: its bookmarks, recent locations and place among recent files follow. Whether any did.
    pub fn rename(&mut self, from: &Path, to: &Path) -> bool {
        if from == to {
            return false;
        }
        let mut changed = false;
        for m in self.bookmarks.iter_mut().chain(self.recent_locations.iter_mut()).filter(|m| *m.source == *from) {
            m.source = to.into();
            changed = true;
        }
        if self.recent_files.iter().any(|f| f == from) {
            // Where it was in the list, once (a file once at `to` may be listed too).
            self.recent_files.retain(|f| f != to);
            if let Some(f) = self.recent_files.iter_mut().find(|f| *f == from) {
                *f = to.to_owned();
            }
            changed = true;
        }
        // One bookmark per line, as everywhere (a file once at `to` may have left some); a key either had stays.
        self.bookmarks.sort_by(|a, b| a.key().cmp(&b.key()));
        self.bookmarks.dedup_by(|later, kept| {
            let same = later.key() == kept.key();
            if same {
                kept.mnemonic = kept.mnemonic.or(later.mnemonic);
            }
            same
        });
        changed
    }

    /// `from` (a file or a folder) was moved to `to`: the places of everything inside it follow. Whether any did.
    pub fn rename_tree(&mut self, from: &Path, to: &Path) -> bool {
        let mut sources: Vec<PathBuf> = self.bookmarks.iter().chain(self.recent_locations.iter()).map(|m| PathBuf::from(&*m.source))
            .chain(self.recent_files.iter().cloned()).filter(|p| p.starts_with(from)).collect();
        sources.sort();
        sources.dedup();
        let mut changed = false;
        for source in sources {
            let Ok(rest) = source.strip_prefix(from) else { continue };
            let target = if rest.as_os_str().is_empty() { to.to_path_buf() } else { to.join(rest) };
            changed |= self.rename(&source, &target);
        }
        changed
    }

    pub fn forget_file(&mut self, path: &Path) {
        self.recent_files.retain(|p| p != path);
    }

    /// The reader was at `mark`: the most recent location, replacing one within [`MERGE_LINES`] in its file.
    pub fn visited(&mut self, mark: Mark) {
        self.recent_locations.retain(|m| !(m.source == mark.source && m.line.abs_diff(mark.line) <= MERGE_LINES));
        self.recent_locations.push_front(Mark { mnemonic: None, moved: false, ..mark });
        self.recent_locations.truncate(RECENT_LOCATIONS);
    }

    pub fn forget_location(&mut self, source: &Path, line: usize) {
        self.recent_locations.retain(|m| m.key() != (source, line));
    }

    /// `source` now reads as `doc`: its marks move with their lines. Whether any changed.
    pub fn relocate(&mut self, source: &Path, doc: &TextSnapshot) -> bool {
        let changed = relocate(&mut self.bookmarks, source, doc) | relocate(self.recent_locations.make_contiguous(), source, doc);
        if changed {
            self.coalesce();
        }
        changed
    }

    /// Move existing source anchors through a validated transaction without searching the document.
    pub(crate) fn edited(&mut self, paths: &std::collections::HashSet<PathBuf>, previous: &TextSnapshot, next: &TextSnapshot, edits: &[crate::text::TextEdit]) -> bool {
        let mut changed = false;
        for mark in self.bookmarks.iter_mut().chain(self.recent_locations.iter_mut()).filter(|mark| paths.contains(mark.source.as_ref())) {
            if mark.line >= previous.line_count() || !crate::tabs::anchor_matches(previous, mark.line, &mark.anchor) { continue; }
            let range = previous.reader_line_range(mark.line);
            let byte = previous.floor_char_boundary(range.start.saturating_add(mark.offset).min(range.end));
            let byte = crate::text::edited_byte(byte, edits).min(next.len_bytes());
            let line = next.line_of(byte);
            let offset = byte - next.reader_line_range(line).start;
            let anchor: Arc<str> = crate::tabs::anchor(next, line).into();
            changed |= mark.line != line || mark.offset != offset || mark.anchor != anchor || mark.moved;
            mark.line = line; mark.offset = offset; mark.anchor = anchor; mark.moved = false;
        }
        if changed { self.coalesce(); }
        changed
    }

    fn coalesce(&mut self) {
        self.bookmarks.sort_by(|a, b| a.key().cmp(&b.key()));
        // Two marks on one line become one; a key either had stays.
        self.bookmarks.dedup_by(|later, kept| {
            let same = later.key() == kept.key();
            if same {
                kept.mnemonic = kept.mnemonic.or(later.mnemonic);
            }
            same
        });
        // Locations that now lie close together become one, as when visited: the most recent stays.
        let mut kept: VecDeque<Mark> = VecDeque::with_capacity(self.recent_locations.len());
        for m in self.recent_locations.drain(..) {
            if !kept.iter().any(|k| k.source == m.source && k.line.abs_diff(m.line) <= MERGE_LINES) {
                kept.push_back(m);
            }
        }
        self.recent_locations = kept;
    }

    /// Add `saved` (read from disk) behind what is here: entries here win their line, key and order.
    pub fn merge(&mut self, saved: Places) {
        if self.workspace.is_none() { self.workspace = saved.workspace.clone(); }
        for m in saved.bookmarks {
            if let Err(i) = self.find(&m.source, m.line) {
                let taken = m.mnemonic.is_some_and(|k| self.mnemonic(k).is_some());
                self.bookmarks.insert(i, Mark { mnemonic: if taken { None } else { m.mnemonic }, ..m });
            }
        }
        for f in saved.recent_files {
            if self.recent_files.len() < RECENT_FILES && !self.recent_files.contains(&f) {
                self.recent_files.push_back(f);
            }
        }
        for m in saved.recent_locations {
            if self.recent_locations.len() < RECENT_LOCATIONS && !self.near(&m) {
                self.recent_locations.push_back(m);
            }
        }
    }

    /// These places (one window's) written over `disk`, the file as it is now, given `base`, the file as this window
    /// last knew it: `disk + (self − base) − (base − self)`. What was added here since is added (winning its line and
    /// key), what was removed here is removed, and `disk`'s own changes (another window's) stay. Entries match as in
    /// [`Places::merge`]: bookmarks and locations by file and line, recent files by path.
    pub fn rebase(&self, base: &Places, mut disk: Places) -> Places {
        // Each key's line in the file, before the merge below drops keys it finds taken here.
        let holder = |p: &Places, k: char| p.mnemonic(k).map(|m| (m.source.clone(), m.line));
        let keys: std::collections::BTreeSet<char> = [self, base, &disk].iter().flat_map(|p| p.bookmarks.iter().filter_map(|m| m.mnemonic)).collect();
        let disk_keys: Vec<_> = keys.iter().map(|&k| (k, holder(&disk, k))).collect();
        let mut out = self.clone();
        // Gone from the file since base: another window removed it.
        out.bookmarks.retain(|m| disk.bookmark(&m.source, m.line).is_some() || base.bookmark(&m.source, m.line).is_none());
        out.recent_files.retain(|f| disk.recent_files.contains(f) || !base.recent_files.contains(f));
        out.recent_locations.retain(|m| disk.has_location(m) || !base.has_location(m));
        // Gone from here since base: removed in this window.
        disk.bookmarks.retain(|m| self.bookmark(&m.source, m.line).is_some() || base.bookmark(&m.source, m.line).is_none());
        disk.recent_files.retain(|f| self.recent_files.contains(f) || !base.recent_files.contains(f));
        disk.recent_locations.retain(|m| self.has_location(m) || !base.has_location(m));
        // Recent places: no times are kept, so the other window's new ones go right after this window's (which sit
        // before every older entry), not behind the oldest where the cap drops them first.
        let at = out.recent_files.iter().rposition(|f| !base.recent_files.contains(f)).map_or(0, |i| i + 1);
        let theirs: Vec<PathBuf> = disk.recent_files.iter().filter(|f| !base.recent_files.contains(f) && !out.recent_files.contains(f)).cloned().collect();
        for (n, f) in theirs.into_iter().enumerate() {
            out.recent_files.insert(at + n, f);
        }
        out.recent_files.truncate(RECENT_FILES);
        // Locations also move with their file's text, so only the leading run counts as new on each side (visits go to
        // the front; a moved one keeps its place deeper and goes behind, through `merge`, as before).
        let at = out.recent_locations.iter().position(|m| base.has_location(m)).unwrap_or(out.recent_locations.len());
        let theirs: Vec<Mark> = disk.recent_locations.iter().take_while(|m| !base.has_location(m)).filter(|m| !out.near(m)).cloned().collect();
        for (n, m) in theirs.into_iter().enumerate() {
            out.recent_locations.insert(at + n, m);
        }
        out.recent_locations.truncate(RECENT_LOCATIONS);
        let workspace = if self.workspace == base.workspace { disk.workspace.clone() } else { self.workspace.clone() };
        out.merge(disk);
        out.workspace = workspace;
        // Keys: where this window left a key as it was, the file's assignment of it stands (another window's), except
        // on a line holding a key this window changed: that one wins its line.
        let changed_here = |k: char| holder(self, k) != holder(base, k);
        for (k, want) in disk_keys {
            if changed_here(k) {
                continue;
            }
            for m in out.bookmarks.iter_mut().filter(|m| m.mnemonic == Some(k)) {
                m.mnemonic = None;
            }
            if let Some((source, line)) = want
                && let Ok(i) = out.find(&source, line)
                && !out.bookmarks[i].mnemonic.is_some_and(changed_here)
            {
                out.bookmarks[i].mnemonic = Some(k);
            }
        }
        out
    }

    fn has_location(&self, m: &Mark) -> bool {
        self.recent_locations.iter().any(|h| h.key() == m.key())
    }

    /// Whether a recent location lies within [`MERGE_LINES`] of `m` in its file: the same one, as when visited.
    fn near(&self, m: &Mark) -> bool {
        self.recent_locations.iter().any(|h| h.source == m.source && h.line.abs_diff(m.line) <= MERGE_LINES)
    }

    /// Whether any bookmark or recent location is in `source` (before copying places to move them).
    pub fn has_marks(&self, source: &Path) -> bool {
        self.bookmarks.get(self.bookmarks.partition_point(|m| &*m.source < source)).is_some_and(|m| *m.source == *source) || self.recent_locations.iter().any(|m| *m.source == *source)
    }

    /// The saved form: paths relative to `root` when under it.
    pub fn to_bytes(&self, root: &Path) -> Vec<u8> {
        let rel = |p: &Path| p.strip_prefix(root).map_or_else(|_| p.to_owned(), Path::to_owned);
        let saved = |m: &Mark| SavedMark { path: rel(&m.source), line: m.line, anchor: m.anchor.to_string(), offset: m.offset, mnemonic: m.mnemonic };
        let mut s = Saved {
            v: VERSION,
            root: root.to_owned(),
            workspace: self.workspace.clone().map(|mut w| { w.relativize(root); w }),
            bookmarks: self.bookmarks.iter().map(saved).collect(),
            recent_files: self.recent_files.iter().map(|p| rel(p)).collect(),
            recent_locations: self.recent_locations.iter().map(saved).collect(),
        };
        let bytes = serde_json::to_vec(&s).unwrap_or_default();
        if bytes.len() as u64 > PLACES_BYTES && s.workspace.is_some() {
            s.workspace = None; // Oversized UI metadata must not make bookmarks unreadable.
            return serde_json::to_vec(&s).unwrap_or_default();
        }
        bytes
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Saved {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    workspace: Option<Box<crate::workspace::Workspace>>,
    v: u32,
    root: PathBuf,
    bookmarks: Vec<SavedMark>,
    recent_files: Vec<PathBuf>,
    recent_locations: Vec<SavedMark>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct SavedMark {
    path: PathBuf,
    line: usize,
    anchor: String,
    offset: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    mnemonic: Option<char>,
}

/// Only the version, read first: another version's file may not parse as this one's.
#[derive(serde::Deserialize)]
struct Versioned {
    v: u32,
}

/// Why a places file was not read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReadError {
    /// Another version of the reader wrote it: it is left alone, and nothing is saved over it.
    OtherVersion,
    /// It cannot be read or parsed, or it holds another root's places.
    Broken(String),
}

/// `root`'s places file in `dir`: a stable hash of the root's path (FNV-1a), so any path makes a plain name.
pub fn file_for(dir: &Path, root: &Path) -> PathBuf {
    let hash = root.as_os_str().as_encoded_bytes().iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3));
    dir.join(format!("{hash:016x}.json"))
}

/// `root`'s places file in `dir`, named for the canonical root so every spelling of a folder shares it, and the file to
/// read: that one, or, when only the one named for `root` as given exists (named before names were canonical), that.
pub fn files_for(dir: &Path, root: &Path) -> (PathBuf, PathBuf) {
    let file = file_for(dir, &std::fs::canonicalize(root).unwrap_or_else(|_| root.to_owned()));
    let old = file_for(dir, root);
    let from = if file != old && !file.exists() && old.exists() { old } else { file.clone() };
    (file, from)
}

/// `root`'s places from `file`: none when there is no file; an error when it is another version's, cannot be read or
/// parsed, or is another root's.
pub fn read(file: &Path, root: &Path) -> Result<Places, ReadError> {
    let broken = ReadError::Broken;
    let meta = match std::fs::metadata(file) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Places::default()),
        Err(e) => return Err(broken(e.to_string())),
        Ok(m) => m,
    };
    if !meta.is_file() {
        return Err(broken("not a regular file".into()));
    }
    let mut bytes = Vec::new();
    std::fs::File::open(file).and_then(|f| f.take(PLACES_BYTES + 1).read_to_end(&mut bytes)).map_err(|e| broken(e.to_string()))?;
    if bytes.len() as u64 > PLACES_BYTES {
        return Err(broken("larger than 4 MiB".into()));
    }
    let v: Versioned = serde_json::from_slice(&bytes).map_err(|e| broken(e.to_string()))?;
    if v.v != VERSION {
        return Err(ReadError::OtherVersion);
    }
    let s: Saved = serde_json::from_slice(&bytes).map_err(|e| broken(e.to_string()))?;
    // Saved under another spelling of this folder: both spellings resolve to one folder, or the saved one is not a
    // folder's own path (a symlink since removed or pointed elsewhere) and the file's own name is this folder's. A file
    // copied from another folder keeps that folder's real path and is set aside.
    let canonical = std::fs::canonicalize(root);
    let saved = std::fs::canonicalize(&s.root);
    let same = s.root == root
        || saved.as_ref().is_ok_and(|a| canonical.as_ref().is_ok_and(|b| a == b))
        || (saved.as_ref().map_or(true, |a| *a != s.root) && canonical.is_ok_and(|b| file_for(Path::new(""), &b).file_name() == file.file_name()));
    if !same {
        return Err(broken("written for another folder".into()));
    }
    let abs = |p: PathBuf| if p.is_absolute() { p } else { root.join(p) };
    // Positions past any document are clamped; keys other than 0–9 and A–Z are dropped (the bookmark stays).
    // An environment file's mark keeps no line text (an older file may hold some): it stays on its line number.
    let private = |p: &Path| lenz_content::highlight::detect(p, "") == Some(lenz_content::highlight::Language::Env);
    let mark = |m: SavedMark| Mark { anchor: if private(&abs(m.path.clone())) { "".into() } else { m.anchor.into() }, source: abs(m.path).into(), line: m.line.min(MAX_POSITION), offset: m.offset.min(MAX_POSITION), mnemonic: m.mnemonic.map(|k| k.to_ascii_uppercase()).filter(char::is_ascii_alphanumeric), moved: false };
    let mut p = Places { workspace: s.workspace.map(|mut w| { w.resolve(root); w }), ..Default::default() };
    // Through the same rules as typing them: sorted, one per line and key.
    for m in s.bookmarks.into_iter().map(mark) {
        match m.mnemonic {
            Some(k) => p.set_mnemonic(m, k),
            None if p.bookmark(&m.source, m.line).is_none() => {
                p.toggle(m);
            }
            None => {}
        }
    }
    // A hand-edited file may repeat an entry: the first stays.
    for f in s.recent_files.into_iter().map(abs) {
        if p.recent_files.len() == RECENT_FILES {
            break;
        }
        if !p.recent_files.contains(&f) {
            p.recent_files.push_back(f);
        }
    }
    for m in s.recent_locations.into_iter().map(mark) {
        if p.recent_locations.len() == RECENT_LOCATIONS {
            break;
        }
        if !p.near(&m) {
            p.recent_locations.push_back(m);
        }
    }
    Ok(p)
}

/// Best effort, atomically: a failed save loses state, never a project file.
pub fn write_atomic(file: &Path, bytes: &[u8]) {
    use std::sync::atomic::{AtomicU64, Ordering};
    let Some(dir) = file.parent() else { return };
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let temp = file.with_extension(format!("tmp-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    let ok = std::fs::create_dir_all(dir).is_ok() && std::fs::write(&temp, bytes).is_ok() && std::fs::rename(&temp, file).is_ok();
    if !ok {
        let _ = std::fs::remove_file(&temp);
    }
}

/// A save of one project's places.
pub struct Save {
    pub file: PathBuf,
    /// The project root: paths under it are saved relative to it.
    pub root: PathBuf,
    /// Returned with a merge, so the window can tell a stale one from the current one.
    pub generation: u64,
    pub seq: u64,
    pub places: Arc<Places>,
}

enum Job {
    /// A file as the window now knows it.
    Base(PathBuf, Arc<Places>),
    Save(Save),
    Flush(mpsc::Sender<()>),
}

/// Writes places off the UI thread. A file is written once no newer save has arrived for the debounce. A save for
/// another file, or a new base, writes the pending one first. Dropping the saver writes what is pending and waits for
/// it. Each write merges with the file as it is then ([`Places::rebase`]). When the merge brings in another window's
/// changes, `merged` gets the save's generation, sequence and the places written.
pub struct Saver {
    tx: Option<mpsc::Sender<Job>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Saver {
    pub fn new(debounce: Duration, merged: impl Fn(u64, u64, Places) + Send + 'static) -> Self {
        let (tx, rx) = mpsc::channel::<Job>();
        let thread = std::thread::Builder::new()
            .name("reader-places".into())
            .spawn(move || {
                // The file as the window knows it: as read, as it last saved it, or as it took a merge back.
                let mut base: (PathBuf, Arc<Places>) = Default::default();
                let mut pending: Option<Save> = None;
                let write = |save: Save, base: &mut (PathBuf, Arc<Places>)| {
                    if let Some(p) = write_merged(&save, base) {
                        merged(save.generation, save.seq, p);
                    }
                };
                loop {
                    let next = match pending {
                        Some(_) => rx.recv_timeout(debounce),
                        None => rx.recv().map_err(|_| mpsc::RecvTimeoutError::Disconnected),
                    };
                    match next {
                        Ok(job) => {
                            if let Some(old) = pending.take_if(|p| match &job {
                                Job::Base(..) | Job::Flush(_) => true,
                                Job::Save(s) => s.file != p.file,
                            }) {
                                write(old, &mut base);
                            }
                            match job {
                                Job::Base(file, places) => base = (file, places),
                                Job::Save(save) => pending = Some(save),
                                Job::Flush(done) => { let _ = done.send(()); }
                            }
                        }
                        Err(e) => {
                            if let Some(save) = pending.take() {
                                write(save, &mut base);
                            }
                            if e == mpsc::RecvTimeoutError::Disconnected {
                                return;
                            }
                        }
                    }
                }
            })
            .ok();
        Self { tx: Some(tx), thread }
    }

    /// `file` as the window now knows it (read, or a merge taken back): later saves to it merge from there.
    pub fn base(&self, file: PathBuf, places: Arc<Places>) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(Job::Base(file, places));
        }
    }

    /// A background reader waits for pending saves before reopening a directory.
    pub fn flush(&self) -> mpsc::Receiver<()> {
        let (done, ready) = mpsc::channel();
        if let Some(tx) = &self.tx { let _ = tx.send(Job::Flush(done)); }
        ready
    }

    pub fn save(&self, save: Save) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(Job::Save(save));
        }
    }
}

/// Write `save` over its file as it is now, under an advisory lock beside it so two windows' writes do not interleave.
/// The save becomes the base. Returns the places written when they differ from the save's. Nothing is written when
/// the file cannot be read (another version's, or broken).
fn write_merged(save: &Save, base: &mut (PathBuf, Arc<Places>)) -> Option<Places> {
    let _ = std::fs::create_dir_all(save.file.parent()?);
    let lock = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(save.file.with_extension("json.lock"));
    // ponytail: advisory and best effort; where locks are unsupported, or another window holds one past LOCK_WAIT
    // (a reader stopped or hung while saving), two windows' saves can still interleave.
    if let Ok(l) = &lock {
        let until = std::time::Instant::now() + LOCK_WAIT;
        while matches!(l.try_lock(), Err(std::fs::TryLockError::WouldBlock)) && std::time::Instant::now() < until {
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    let disk = read(&save.file, &save.root).ok()?;
    let empty = Places::default();
    let was = if base.0 == save.file { &*base.1 } else { &empty };
    let merged = save.places.rebase(was, disk);
    write_atomic(&save.file, &merged.to_bytes(&save.root));
    // What the window holds; a merge it takes back is sent as the next base.
    *base = (save.file.clone(), save.places.clone());
    (merged != *save.places).then_some(merged)
} // the lock is released here, before `merged` is called

impl Drop for Saver {
    fn drop(&mut self) {
        self.tx = None;
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn doc(text: &str) -> TextSnapshot {
        TextSnapshot::from_document(crate::text::BufferId(1), &crate::document::Document::decode(PathBuf::from("/p/a.rs"), 1, text.as_bytes()))
    }
    fn a() -> &'static Path {
        Path::new("/p/a.rs")
    }
    fn mark(d: &TextSnapshot, line: usize) -> Mark {
        Mark::at(a(), d, line, 0)
    }

    #[test]
    fn a_mark_finds_its_line_after_lines_are_inserted_above() {
        let mut p = Places::default();
        p.toggle(mark(&doc("a\nfn f() {}\nb\n"), 1));
        assert!(p.relocate(a(), &doc("x\ny\na\nfn f() {}\nb\n")));
        assert_eq!((p.bookmarks()[0].line, p.bookmarks()[0].moved), (3, false));
    }

    #[test]
    fn duplicate_lines_go_to_the_nearest_and_a_tie_goes_up() {
        let mut p = Places::default();
        p.toggle(mark(&doc("a\n}\nb\nc\nd\ne\n}\n"), 1));
        p.relocate(a(), &doc("}\nq\na\nb\nc\n}\n"));
        assert_eq!(p.bookmarks()[0].line, 0, "lines 0 and 5 are 1 and 4 away from line 1");
        let mut p = Places::default();
        p.toggle(mark(&doc("}\nx\n}\n"), 1)); // anchored to "x"
        p.relocate(a(), &doc("}\nx\n}\n"));
        assert_eq!(p.bookmarks()[0].line, 1, "unchanged text keeps it");
        let mut p = Places::default();
        p.toggle(mark(&doc("a\n}\nb\n"), 1));
        p.relocate(a(), &doc("}\nz\n}\n"));
        assert_eq!(p.bookmarks()[0].line, 0, "1 away up and 1 away down: up wins");
    }

    #[test]
    fn a_mark_whose_line_is_gone_keeps_its_place_and_says_moved() {
        let mut p = Places::default();
        p.toggle(mark(&doc("a\nb\nc\nd\n"), 3));
        p.relocate(a(), &doc("x\ny\n"));
        assert_eq!((p.bookmarks()[0].line, p.bookmarks()[0].moved), (3, true), "its own line, past the end");
    }

    #[test]
    fn crlf_text_anchors_without_its_line_ending() {
        let mut p = Places::default();
        p.toggle(mark(&doc("a\r\nfn f()\r\n"), 1));
        assert_eq!(&*p.bookmarks()[0].anchor, "fn f()");
        p.relocate(a(), &doc("z\nfn f()\n"));
        assert_eq!((p.bookmarks()[0].line, p.bookmarks()[0].moved), (1, false));
    }

    #[test]
    fn two_marks_landing_on_one_line_become_one_keeping_a_mnemonic() {
        let d = doc("x\nx\n");
        let mut p = Places::default();
        p.toggle(mark(&d, 0));
        p.set_mnemonic(mark(&d, 1), '7');
        p.relocate(a(), &doc("x\n"));
        assert_eq!(p.bookmarks().len(), 1);
        assert_eq!(p.bookmarks()[0].mnemonic, Some('7'));
    }

    #[test]
    fn toggling_adds_then_removes_whatever_its_mnemonic() {
        let d = doc("a\nb\n");
        let mut p = Places::default();
        assert!(p.toggle(mark(&d, 1)));
        p.set_mnemonic(mark(&d, 1), 'k');
        assert_eq!(p.bookmark(a(), 1).unwrap().mnemonic, Some('K'), "letters are kept upper-case");
        assert!(!p.toggle(mark(&d, 1)));
        assert!(p.bookmarks().is_empty());
    }

    #[test]
    fn a_mnemonic_sets_moves_and_removes() {
        let d = doc("a\nb\nc\n");
        let mut p = Places::default();
        p.set_mnemonic(mark(&d, 0), '3');
        p.set_mnemonic(mark(&d, 2), '3');
        assert_eq!(p.mnemonic('3').unwrap().line, 2, "moved here");
        assert_eq!(p.bookmark(a(), 0).unwrap().mnemonic, None, "the old line keeps an anonymous bookmark");
        p.set_mnemonic(mark(&d, 2), '3');
        assert!(p.bookmark(a(), 2).is_none(), "the line's own key removes it");
        assert!(p.mnemonic('3').is_none());
    }

    #[test]
    fn stepping_goes_by_path_then_line_and_wraps() {
        let d = doc("a\nb\nc\nd\n");
        let b = Path::new("/p/b.rs");
        let mut p = Places::default();
        for (s, l) in [(b, 0), (a(), 3), (a(), 1)] {
            p.toggle(Mark::at(s, &d, l, 0));
        }
        let at = |m: Option<&Mark>| m.map(|m| (m.source.clone(), m.line));
        assert_eq!(at(p.step(a(), 1, usize::MAX, true)), Some((a().into(), 3)));
        assert_eq!(at(p.step(a(), 3, usize::MAX, true)), Some((b.into(), 0)));
        assert_eq!(at(p.step(b, 0, usize::MAX, true)), Some((a().into(), 1)), "wraps to the first");
        assert_eq!(at(p.step(a(), 1, usize::MAX, false)), Some((b.into(), 0)), "wraps to the last");
        assert_eq!(at(p.step(a(), 2, usize::MAX, false)), Some((a().into(), 1)), "from between two marks");
        assert!(Places::default().step(a(), 0, usize::MAX, true).is_none());
    }

    #[test]
    fn recent_files_are_most_recently_left_first_and_capped() {
        let mut p = Places::default();
        for i in 0..RECENT_FILES + 5 {
            p.left_file(&PathBuf::from(format!("/p/{i}")));
        }
        p.left_file(Path::new("/p/3"));
        assert_eq!(p.recent_files().len(), RECENT_FILES);
        assert_eq!(p.recent_files()[0], PathBuf::from("/p/3"));
        assert_eq!(p.recent_files().iter().filter(|f| *f == Path::new("/p/3")).count(), 1);
    }

    // a file that shrinks and comes back keeps each bookmark's key.
    #[test]
    fn keys_survive_a_file_that_shrinks_and_returns() {
        let d = doc("0\n1\n2\n3\n4\n");
        let mut p = Places::default();
        p.set_mnemonic(mark(&d, 3), '1');
        p.set_mnemonic(mark(&d, 4), '2');
        p.relocate(a(), &doc("x\n"));
        p.relocate(a(), &d);
        assert_eq!(p.mnemonic('1').map(|m| m.line), Some(3), "key 1");
        assert_eq!(p.mnemonic('2').map(|m| m.line), Some(4), "key 2");
    }

    // a key this window set wins its line over another window's key there.
    #[test]
    fn a_key_set_here_wins_its_line_in_a_rebase() {
        let d = doc("a\nb\n");
        let mut base = Places::default();
        base.toggle(mark(&d, 0));
        let mut mine = base.clone();
        mine.set_mnemonic(mark(&d, 0), 'A');
        let mut disk = base.clone();
        disk.set_mnemonic(mark(&d, 0), 'B');
        let out = mine.rebase(&base, disk);
        assert_eq!(out.bookmark(a(), 0).unwrap().mnemonic, Some('A'));
    }

    #[test]
    fn read_positions_are_clamped() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("x.json");
        let huge = r#"{"path":"a.rs","line":18446744073709551615,"anchor":"","offset":18446744073709551615}"#;
        std::fs::write(&file, format!(r#"{{"v":1,"root":"/p","bookmarks":[{huge}],"recent_files":[],"recent_locations":[{huge}]}}"#)).unwrap();
        let p = read(&file, Path::new("/p")).unwrap();
        let (b, l) = (&p.bookmarks()[0], &p.recent_locations()[0]);
        assert_eq!((b.line, b.offset, l.line, l.offset), (MAX_POSITION, MAX_POSITION, MAX_POSITION, MAX_POSITION));
        assert_eq!(b.line + 1, MAX_POSITION + 1, "a row label's line number does not overflow");
    }

    #[test]
    fn only_digit_and_letter_mnemonics_are_read() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("x.json");
        let at = |line: usize, key: &str| format!(r#"{{"path":"a.rs","line":{line},"anchor":"","offset":0,"mnemonic":"{key}"}}"#);
        let marks = [at(0, "é"), at(1, "!"), at(2, "q")].join(",");
        std::fs::write(&file, format!(r#"{{"v":1,"root":"/p","bookmarks":[{marks}],"recent_files":[],"recent_locations":[]}}"#)).unwrap();
        let p = read(&file, Path::new("/p")).unwrap();
        let keys: Vec<Option<char>> = p.bookmarks().iter().map(|m| m.mnemonic).collect();
        assert_eq!(keys, [None, None, Some('Q')], "other keys are dropped; their bookmarks stay");
    }

    #[test]
    fn recent_locations_that_move_together_collapse_to_one() {
        let mut text = String::from("a\n");
        text += &"x\n".repeat(19);
        text += "b\n";
        let d = doc(&text);
        let mut p = Places::default();
        p.visited(mark(&d, 0));
        p.visited(mark(&d, 20));
        assert!(p.relocate(a(), &doc("a\nx\nb\n")));
        let lines: Vec<usize> = p.recent_locations().iter().map(|m| m.line).collect();
        assert_eq!(lines, [2], "\"b\" moved within 5 lines of \"a\": the most recent of the two stays");
    }

    #[test]
    fn a_nearby_location_replaces_the_older_one() {
        let d = doc(&"x\n".repeat(40));
        let mut p = Places::default();
        p.visited(mark(&d, 10));
        p.visited(mark(&d, 30));
        p.visited(mark(&d, 14));
        let lines: Vec<usize> = p.recent_locations().iter().map(|m| m.line).collect();
        assert_eq!(lines, [14, 30], "within 5 lines of 10: replaced and moved to the front");
        for l in 0..40 {
            p.visited(Mark::at(&PathBuf::from(format!("/p/{l}")), &d, 0, 0));
        }
        assert_eq!(p.recent_locations().len(), RECENT_LOCATIONS);
    }

    #[test]
    fn merging_keeps_what_is_here_first() {
        let d = doc("a\nb\nc\n");
        let mut here = Places::default();
        here.set_mnemonic(mark(&d, 0), '1');
        here.left_file(Path::new("/p/new"));
        let mut saved = Places::default();
        saved.set_mnemonic(mark(&d, 2), '1');
        saved.toggle(mark(&d, 1));
        saved.left_file(Path::new("/p/old"));
        here.merge(saved);
        assert_eq!(here.mnemonic('1').unwrap().line, 0, "the newer mnemonic wins");
        assert_eq!(here.bookmarks().len(), 3, "line 2 stays, anonymous");
        assert_eq!(here.recent_files().iter().map(|p| p.to_str().unwrap()).collect::<Vec<_>>(), ["/p/new", "/p/old"]);
    }

    #[test]
    fn places_round_trip_relative_to_the_root() {
        let dir = tempfile::tempdir().unwrap();
        let root = Path::new("/p");
        let d = doc("a\nb\n");
        let mut p = Places::default();
        p.set_mnemonic(mark(&d, 1), 'Q');
        p.toggle(Mark::at(Path::new("/elsewhere/z.rs"), &d, 0, 0));
        p.left_file(a());
        p.visited(mark(&d, 0));
        let bytes = p.to_bytes(root);
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["bookmarks"][0]["path"], "/elsewhere/z.rs", "absolute outside it (sorted by path: first)");
        assert_eq!(json["bookmarks"][1]["path"], "a.rs", "relative inside the root");
        let file = file_for(dir.path(), root);
        write_atomic(&file, &bytes);
        assert_eq!(read(&file, root).unwrap(), p);
    }

    #[cfg(unix)]
    #[test]
    fn a_folder_has_one_places_file_whatever_its_spelling() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real");
        std::fs::create_dir(&real).unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let projects = dir.path().join("projects");
        let (file, from) = files_for(&projects, &link);
        assert_eq!(file, files_for(&projects, &real).0, "named for the target");
        assert_eq!(from, file, "nothing under the old name: read the new one");
        write_atomic(&file_for(&projects, &link), &Places::default().to_bytes(&link));
        assert_eq!(files_for(&projects, &link), (file.clone(), file_for(&projects, &link)), "only the old name exists: read it");
        write_atomic(&file, &Places::default().to_bytes(&link));
        assert_eq!(read(&file, &real), Ok(Places::default()), "saved through the link, read through the target");
        std::fs::remove_file(&link).unwrap();
        assert_eq!(read(&file, &real), Ok(Places::default()), "the link is gone: the file's name is still the folder's");
    }

    #[cfg(unix)]
    #[test]
    fn places_saved_through_a_link_that_now_points_elsewhere_are_still_the_targets() {
        let dir = tempfile::tempdir().unwrap();
        let (one, two) = (dir.path().join("one"), dir.path().join("two"));
        std::fs::create_dir(&one).unwrap();
        std::fs::create_dir(&two).unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&one, &link).unwrap();
        let file = files_for(dir.path(), &link).0;
        let mut p = Places::default();
        p.left_file(&link.join("x.rs"));
        write_atomic(&file, &p.to_bytes(&link));
        std::fs::remove_file(&link).unwrap();
        std::os::unix::fs::symlink(&two, &link).unwrap();
        let read_back = read(&file, &one).unwrap();
        assert_eq!(read_back.recent_files(), &[one.join("x.rs")], "the file's name is still the folder's");
    }

    #[test]
    fn a_file_for_another_folder_is_set_aside_though_that_folder_is_gone() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        std::fs::create_dir(&root).unwrap();
        let gone = dir.path().join("gone");
        let file = file_for(dir.path(), &gone);
        write_atomic(&file, &Places::default().to_bytes(&gone));
        assert!(matches!(read(&file, &root), Err(ReadError::Broken(_))), "neither its root nor its name is this folder's");
    }

    #[test]
    fn a_places_file_that_repeats_an_entry_reads_it_once() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("x.json");
        let at = |line: usize| format!(r#"{{"path":"a.rs","line":{line},"anchor":"","offset":0}}"#);
        let (three, five) = (at(3), at(3 + MERGE_LINES));
        std::fs::write(&file, format!(r#"{{"v":1,"root":"/p","bookmarks":[],"recent_files":["a.rs","b.rs","./a.rs","a.rs"],"recent_locations":[{three},{three},{five}]}}"#)).unwrap();
        let p = read(&file, Path::new("/p")).unwrap();
        assert_eq!(p.recent_files(), &[PathBuf::from("/p/a.rs"), PathBuf::from("/p/b.rs")], "the first stays, however it is spelled");
        let lines: Vec<usize> = p.recent_locations().iter().map(|m| m.line).collect();
        assert_eq!(lines, [3], "a location within MERGE_LINES of an earlier one in its file is the same one, as when visited");
    }

    #[test]
    fn places_of_another_version_are_left_alone_and_broken_ones_say_so() {
        let dir = tempfile::tempdir().unwrap();
        let root = Path::new("/p");
        let file = file_for(dir.path(), root);
        assert_eq!(read(&file, root), Ok(Places::default()), "no file yet");
        std::fs::write(&file, b"{ not json").unwrap();
        assert!(matches!(read(&file, root), Err(ReadError::Broken(_))));
        std::fs::write(&file, br#"{"v":2,"root":"/p","bookmarks":[],"recent_files":[],"recent_locations":[]}"#).unwrap();
        assert_eq!(read(&file, root), Err(ReadError::OtherVersion), "a newer version is left alone");
        std::fs::write(&file, br#"{"v":2,"marks":{}}"#).unwrap();
        assert_eq!(read(&file, root), Err(ReadError::OtherVersion), "whatever its shape");
        std::fs::write(&file, Places::default().to_bytes(dir.path())).unwrap(); // a folder that exists
        assert!(matches!(read(&file, root), Err(ReadError::Broken(_))), "another root's file (a hash collision) is set aside like a broken one");
        std::fs::write(&file, vec![b' '; PLACES_BYTES as usize + 1]).unwrap();
        assert!(read(&file, root).is_err(), "over the cap");
        assert_ne!(file_for(dir.path(), root), file_for(dir.path(), Path::new("/q")));
        assert_eq!(file_for(dir.path(), root).file_name().unwrap().len(), 16 + ".json".len());
    }

    #[cfg(unix)]
    #[test]
    fn a_fifo_is_not_read() {
        let dir = tempfile::tempdir().unwrap();
        let file = file_for(dir.path(), Path::new("/p"));
        assert!(std::process::Command::new("mkfifo").arg(&file).status().unwrap().success());
        assert!(read(&file, Path::new("/p")).is_err(), "returns instead of blocking");
    }

    /// The gutter's per-frame cost: one lookup per visible row (60) with 10,000 bookmarks in the file.
    #[test]
    #[ignore = "timing; run with --release --ignored --nocapture"]
    fn gutter_lookups_with_ten_thousand_bookmarks() {
        let d = doc(&"x\n".repeat(50_000));
        let mut p = Places::default();
        for l in (0..50_000).step_by(5) {
            p.toggle(mark(&d, l));
        }
        let frames = 10_000;
        let start = std::time::Instant::now();
        let mut found = 0;
        for f in 0..frames {
            let top = (f * 7) % 49_000;
            found += (top..top + 60).filter(|&l| p.bookmark(a(), l).is_some()).count();
        }
        let per = start.elapsed().as_secs_f64() * 1000.0 / frames as f64;
        println!("gutter: {per:.4} ms per frame of 60 rows, {found} marks found");
        assert!(per < 1.0);
    }

    /// The UI thread's cost of a change with 10,000 bookmarks. Before this change, all of them were serialized for the
    /// saver (`to_bytes`). Now the saver gets a snapshot, and the next change copies the places while the saver still
    /// holds it (the worst case).
    #[test]
    #[ignore = "timing; run with --release --ignored --nocapture"]
    fn a_change_with_ten_thousand_bookmarks() {
        let d = doc(&"x\n".repeat(50_000));
        let mut p = Places::default();
        for l in (0..50_000).step_by(5) {
            p.toggle(mark(&d, l));
        }
        let root = Path::new("/p");
        let rounds = 200;
        let start = std::time::Instant::now();
        for i in 0..rounds {
            p.toggle(mark(&d, 1 + i % 4));
            std::hint::black_box(p.to_bytes(root));
        }
        let serialized = start.elapsed().as_secs_f64() * 1000.0 / rounds as f64;
        let dir = tempfile::tempdir().unwrap();
        let saver = Saver::new(Duration::from_secs(3600), |_, _, _| {});
        let mut shared = Arc::new(p);
        let start = std::time::Instant::now();
        for i in 0..rounds {
            Arc::make_mut(&mut shared).toggle(mark(&d, 1 + i % 4));
            saver.save(Save { file: dir.path().join("x.json"), root: root.to_owned(), generation: 1, seq: i as u64, places: shared.clone() });
        }
        let snapshot = start.elapsed().as_secs_f64() * 1000.0 / rounds as f64;
        println!("a change with 10,000 bookmarks on the UI thread: {serialized:.3} ms serializing (before), {snapshot:.3} ms with a snapshot (now)");
        assert!(snapshot < serialized);
        drop(saver);
    }

    /// A save of bookmarks on `lines` of /p/a.rs, under root /p.
    fn save_of(file: &Path, lines: &[usize]) -> Save {
        let d = doc("0\n1\n2\n3\n");
        let mut p = Places::default();
        for &l in lines {
            p.toggle(mark(&d, l));
        }
        Save { file: file.to_owned(), root: PathBuf::from("/p"), generation: 1, seq: 1, places: Arc::new(p) }
    }

    fn lines_in(file: &Path) -> Vec<usize> {
        read(file, Path::new("/p")).unwrap().bookmarks().iter().map(|m| m.line).collect()
    }

    #[test]
    fn dropping_the_saver_writes_what_is_pending() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("x.json");
        let saver = Saver::new(Duration::from_secs(60), |_, _, _| {});
        saver.save(save_of(&file, &[1]));
        saver.save(save_of(&file, &[2]));
        assert!(!file.exists(), "still waiting");
        drop(saver);
        assert_eq!(lines_in(&file), [2]);
    }

    #[test]
    fn the_saver_writes_once_changes_stop() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("x.json");
        let saver = Saver::new(Duration::from_millis(50), |_, _, _| {});
        saver.save(save_of(&file, &[1]));
        let until = std::time::Instant::now() + Duration::from_secs(5);
        while !file.exists() {
            assert!(std::time::Instant::now() < until, "never written");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(lines_in(&file), [1]);
    }

    #[test]
    fn a_save_for_another_file_writes_the_pending_one_first() {
        let dir = tempfile::tempdir().unwrap();
        let (one, two) = (dir.path().join("1.json"), dir.path().join("2.json"));
        let saver = Saver::new(Duration::from_secs(60), |_, _, _| {});
        saver.save(save_of(&one, &[1]));
        saver.save(save_of(&two, &[2]));
        drop(saver);
        assert_eq!((lines_in(&one), lines_in(&two)), (vec![1], vec![2]));
    }

    #[test]
    fn two_savers_on_one_file_keep_each_others_changes() {
        let d = doc("x\ny\nz\n");
        let lines = |p: &Places| p.bookmarks().iter().map(|m| m.line).collect::<Vec<_>>();
        for a_first in [true, false] {
            let dir = tempfile::tempdir().unwrap();
            let root = Path::new("/p");
            let file = file_for(dir.path(), root);
            let mut start = Places::default();
            start.toggle(mark(&d, 2)); // Z, which both windows read
            write_atomic(&file, &start.to_bytes(root));
            let (tx, rx) = mpsc::channel();
            let saver = |tx: mpsc::Sender<Places>| Saver::new(Duration::from_secs(60), move |_, _, p| {
                let _ = tx.send(p);
            });
            let (a_saver, b_saver) = (saver(tx.clone()), saver(tx));
            let mut a = read(&file, root).unwrap();
            let mut b = a.clone();
            a_saver.base(file.clone(), Arc::new(a.clone()));
            b_saver.base(file.clone(), Arc::new(b.clone()));
            a.toggle(mark(&d, 0)); // A adds X
            b.toggle(mark(&d, 1)); // B adds Y
            b.toggle(mark(&d, 2)); // and removes Z
            let save = |p: Places| Save { file: file.clone(), root: root.to_owned(), generation: 1, seq: 1, places: Arc::new(p) };
            let (first, second) = if a_first { ((a_saver, a), (b_saver, b)) } else { ((b_saver, b), (a_saver, a)) };
            first.0.save(save(first.1));
            drop(first.0);
            second.0.save(save(second.1));
            drop(second.0);
            assert_eq!(lines(&read(&file, root).unwrap()), [0, 1], "X and Y, not Z (A first: {a_first})");
            assert_eq!(rx.try_iter().map(|p| lines(&p)).collect::<Vec<_>>(), [vec![0, 1]], "the second window gets the first's change back");
        }
    }

    #[test]
    fn another_windows_recent_places_come_right_after_this_ones() {
        let files = |p: &Places| p.recent_files().iter().map(|f| f.to_string_lossy().into_owned()).collect::<Vec<_>>();
        let mut base = Places::default();
        for f in (0..RECENT_FILES).rev() {
            base.left_file(Path::new(&format!("/p/{f}.rs")));
        }
        let (mut mine, mut disk) = (base.clone(), base.clone());
        mine.left_file(Path::new("/p/mine.rs"));
        disk.left_file(Path::new("/p/theirs.rs"));
        let out = mine.rebase(&base, disk);
        assert_eq!(files(&out)[..3], ["/p/mine.rs", "/p/theirs.rs", "/p/0.rs"], "the other window's file is not cut by the cap");
        assert_eq!(out.recent_files().len(), RECENT_FILES);
        let d = doc(&"x\n".repeat(300));
        let lines = |p: &Places| p.recent_locations().iter().map(|m| m.line).collect::<Vec<_>>();
        let mut base = Places::default();
        for l in (0..RECENT_LOCATIONS).rev() {
            base.visited(mark(&d, l * 6 + 40));
        }
        assert_eq!(base.recent_locations().len(), RECENT_LOCATIONS);
        let (mut mine, mut disk) = (base.clone(), base.clone());
        mine.visited(mark(&d, 0));
        disk.visited(mark(&d, 20));
        let out = mine.rebase(&base, disk);
        assert_eq!(lines(&out)[..3], [0, 20, 40]);
        assert_eq!(out.recent_locations().len(), RECENT_LOCATIONS);
    }

    #[test]
    fn moved_recent_locations_keep_their_place_in_a_rebase() {
        let d = doc(&"x\n".repeat(300));
        let lines = |p: &Places| p.recent_locations().iter().map(|m| m.line).collect::<Vec<_>>();
        let mut base = Places::default();
        for l in (0..RECENT_LOCATIONS).rev() {
            base.visited(mark(&d, l * 6 + 40)); // 40 newest … 214 oldest
        }
        let olds: Vec<usize> = (0..RECENT_LOCATIONS).map(|l| l * 6 + 40).collect();
        // The other window's file changed under its five oldest places: they moved a line down.
        let mut disk = base.clone();
        for m in disk.recent_locations.iter_mut().skip(RECENT_LOCATIONS - 5) {
            m.line += 1;
        }
        let mut mine = base.clone();
        mine.visited(mark(&d, 0));
        let mut want = vec![0];
        want.extend(olds.iter().take(RECENT_LOCATIONS - 5));
        want.extend(olds.iter().skip(RECENT_LOCATIONS - 5).take(4).map(|l| l + 1));
        assert_eq!(lines(&mine.rebase(&base, disk)), want, "moved places keep their age; the oldest is the one dropped");
        // This window's own oldest moved: the other window's new place still comes right after this window's.
        let mut mine = base.clone();
        mine.recent_locations.back_mut().unwrap().line += 1;
        mine.visited(mark(&d, 0));
        let mut disk = base.clone();
        disk.visited(mark(&d, 20));
        assert_eq!(lines(&mine.rebase(&base, disk))[..3], [0, 20, 40]);
    }

    #[test]
    fn another_windows_key_changes_survive_a_rebase() {
        let d = doc("x\ny\nz\n");
        let keys = |p: &Places| p.bookmarks().iter().map(|m| (m.line, m.mnemonic)).collect::<Vec<_>>();
        let mut base = Places::default();
        base.set_mnemonic(mark(&d, 0), 'A'); // X holds A in both windows
        base.set_mnemonic(mark(&d, 2), 'C'); // Z holds C
        // The other window moved A to Y and removed Z's key; the file holds that.
        let mut disk = base.clone();
        disk.set_mnemonic(mark(&d, 1), 'A');
        disk.set_mnemonic(mark(&d, 2), 'C');
        disk.toggle(mark(&d, 2));
        // This window changed nothing about keys; it only left a file.
        let mut mine = base.clone();
        mine.left_file(Path::new("/p/other.rs"));
        assert_eq!(keys(&mine.rebase(&base, disk.clone())), [(0, None), (1, Some('A')), (2, None)], "the file's keys stand");
        // This window moved A itself (to Z): its own change wins over the file's.
        let mut mine = base.clone();
        mine.set_mnemonic(mark(&d, 2), 'A');
        assert_eq!(keys(&mine.rebase(&base, disk)), [(0, None), (1, None), (2, Some('A'))]);
    }

    #[test]
    fn a_save_waits_for_another_windows_write() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("x.json");
        let held = std::fs::File::create(file.with_extension("json.lock")).unwrap();
        held.lock().unwrap();
        let saver = Saver::new(Duration::from_millis(10), |_, _, _| {});
        saver.save(save_of(&file, &[1]));
        std::thread::sleep(Duration::from_millis(200));
        assert!(!file.exists(), "waits for the lock");
        held.unlock().unwrap();
        drop(saver);
        assert_eq!(lines_in(&file), [1]);
    }

    #[test]
    fn a_lock_held_forever_delays_a_save_but_never_hangs_it() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("x.json");
        let held = std::fs::File::create(file.with_extension("json.lock")).unwrap();
        held.lock().unwrap();
        let saver = Saver::new(Duration::from_millis(10), |_, _, _| {});
        saver.save(save_of(&file, &[1]));
        // Dropping the saver waits for its write, as at exit: it must return though the lock never comes free.
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            drop(saver);
            let _ = tx.send(());
        });
        assert!(rx.recv_timeout(LOCK_WAIT + Duration::from_secs(5)).is_ok(), "exit hangs on another window's lock");
        assert_eq!(lines_in(&file), [1], "written without the lock");
        drop(held);
    }

    #[test]
    fn a_rename_onto_a_bookmarked_line_keeps_its_key() {
        let d = doc("x\ny\n");
        let z = Path::new("/p/z.rs");
        let mut p = Places::default();
        p.toggle(mark(&d, 1));
        p.set_mnemonic(Mark::at(z, &d, 1, 0), 'A');
        p.rename(z, a());
        assert!(p.mnemonic('A').is_some(), "key A survives the rename: {:?}", p.bookmarks());
    }

    #[test]
    fn a_rename_onto_a_recent_file_lists_it_once() {
        let mut p = Places::default();
        p.left_file(Path::new("/p/z.rs"));
        p.left_file(a());
        p.rename(Path::new("/p/z.rs"), a());
        assert_eq!(p.recent_files().iter().filter(|f| *f == a()).count(), 1, "{:?}", p.recent_files());
    }
}
