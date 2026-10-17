//! Safe writing of the user's files: a file's new content goes back exactly as the file was
//! (mode, encoding, BOM, dominant line ending), atomically, or not at all with a reason.
//!
//! Content writes use exclusively created random temps beside the target, then `fsync` and rename.
//! A symlinked path writes its target. In place instead when the file has other hard links, isn't
//! the user's, or its directory isn't writable. Only temps created by this save are cleaned up.
//! Unix only.
#![allow(dead_code)] // ponytail: wired into the Merge View, Version Picker and 2-way moves in tickets 03, 08, 10

use std::{
    fmt,
    fs::{self, File, OpenOptions},
    hash::{DefaultHasher, Hasher},
    io::{self, Read, Seek, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Component, Path, PathBuf},
    time::SystemTime,
};

use encoding_rs::{Encoding, UTF_16BE, UTF_16LE, UTF_8};

use crate::encoding::{Decoded, decode};

/// Why a write didn't happen. The file on disk is as it was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteError {
    /// The file's content is not what Lenz read.
    ChangedOnDisk,
    /// A character on this line (1-based) has no form in the file's encoding.
    Unrepresentable { line: usize, encoding: &'static str },
    /// The file had undecodable bytes; writing the text back would replace them with U+FFFD.
    Lossy,
    /// The OS refused, with its message.
    Io(String),
}

impl fmt::Display for WriteError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            WriteError::ChangedOnDisk => f.write_str("the file changed on disk"),
            WriteError::Unrepresentable { line, encoding } => write!(f, "line {line} has a character {encoding} can't represent"),
            WriteError::Lossy => f.write_str("the file has bytes Lenz couldn't decode; writing would replace them"),
            WriteError::Io(msg) => f.write_str(msg),
        }
    }
}

fn io_err(what: &str, path: &Path) -> impl FnOnce(io::Error) -> WriteError {
    let msg = format!("{what} {}", path.display());
    move |e| WriteError::Io(format!("{msg}: {e}"))
}

/// A file as Lenz read it: how to write it back, and how to tell it changed meanwhile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    /// As given; a symlink is followed at write time.
    pub path: PathBuf,
    /// Of the bytes read; `None` for a file that didn't exist. `DefaultHasher` is per process, never persisted.
    hash: Option<u64>,
    size: u64,
    mtime: Option<SystemTime>,
    pub encoding: &'static Encoding,
    pub bom: bool,
    /// The dominant line ending is CRLF (majority; ties and no line ends at all are LF).
    pub crlf: bool,
    lossy: bool,
    repository_root: Option<PathBuf>,
}

impl Snapshot {
    /// Reads and decodes the file at `path` (`declared` as in [`decode`]).
    pub fn read(path: &Path, declared: Option<&str>) -> Result<(Snapshot, Decoded), WriteError> {
        let target = fs::canonicalize(path).map_err(io_err("reading", path))?;
        // stat first: a change after it shows as "changed", never hides
        let meta = fs::metadata(&target).map_err(io_err("reading", path))?;
        let bytes = fs::read(&target).map_err(io_err("reading", path))?;
        let decoded = decode(&bytes, declared);
        let snapshot = Snapshot { path: path.to_path_buf(), hash: Some(hash(&bytes)), size: meta.len(), mtime: meta.modified().ok(), ..Self::like(path, &decoded) };
        Ok((snapshot, decoded))
    }

    /// A file that doesn't exist yet, to be written like `like` (Ours, when the working-tree file is gone).
    pub fn missing(path: &Path, like: &Decoded) -> Snapshot {
        Snapshot { lossy: false, ..Self::like(path, like) }
    }

    fn like(path: &Path, d: &Decoded) -> Snapshot {
        let crlf = d.text.matches("\r\n").count();
        Snapshot {
            path: path.to_path_buf(),
            hash: None,
            size: 0,
            mtime: None,
            encoding: Encoding::for_label(d.encoding.as_bytes()).unwrap_or(UTF_8),
            bom: d.had_bom,
            crlf: crlf > d.text.matches('\n').count() - crlf,
            lossy: d.lossy,
            repository_root: None,
        }
    }

    /// The file as it is now, to write over a change on disk ("Overwrite"): read afresh, or, deleted meanwhile,
    /// to be created again in this snapshot's form.
    pub fn reread(&self) -> Result<(Snapshot, Option<Decoded>), WriteError> {
        self.check_repository()?;
        match Snapshot::read(&self.path, None) {
            Err(_) if fs::symlink_metadata(&self.path).is_err() => Ok((Snapshot { hash: None, size: 0, mtime: None, ..self.clone() }, None)),
            read => read.map(|(mut s, d)| { s.repository_root = self.repository_root.clone(); (s, Some(d)) }),
        }
    }

    /// Reload an edit draft in the encoding used when it was read. Attribute-declared
    /// UTF-16 without a BOM must not become an autodetected UTF-8 string with NULs.
    pub(crate) fn reread_as_read(&self)->Result<(Snapshot,Option<Decoded>),WriteError> {
        self.check_repository()?;
        match Snapshot::read(&self.path,Some(self.encoding.name())) {
            Err(_) if fs::symlink_metadata(&self.path).is_err()=>Ok((Snapshot{hash:None,size:0,mtime:None,..self.clone()},None)),
            read=>read.map(|(mut s,d)|{s.repository_root=self.repository_root.clone();(s,Some(d))}),
        }
    }

    pub(crate) fn in_repository(mut self, root: &Path) -> Result<Self, WriteError> {
        check_repository_path(root, &self.path, false)?;
        self.repository_root = Some(root.to_path_buf());
        Ok(self)
    }

    fn check_repository(&self) -> Result<(), WriteError> {
        if let Some(root) = &self.repository_root {
            check_repository_path(root, &self.path, false)?;
        }
        Ok(())
    }

    /// The cheap check (for window activation): size, mtime or existence differ from the read.
    pub fn changed(&self) -> bool {
        match fs::metadata(&self.path) {
            Ok(meta) => self.hash.is_none() || meta.len() != self.size || meta.modified().ok() != self.mtime,
            Err(_) => self.hash.is_some() || fs::symlink_metadata(&self.path).is_ok(),
        }
    }

    /// The thorough check: re-reads the file and compares its content with the read.
    pub fn verify(&self) -> Result<(), WriteError> {
        self.current().map(|_| ())
    }

    /// The write target (symlinks resolved) and its bytes, if the content is still what was read.
    fn current(&self) -> Result<(PathBuf, Option<Vec<u8>>), WriteError> {
        self.check_repository()?;
        let Some(expected) = self.hash else {
            return match fs::symlink_metadata(&self.path) {
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok((self.path.clone(), None)),
                Err(e) => Err(io_err("reading", &self.path)(e)),
                Ok(_) => Err(WriteError::ChangedOnDisk),
            };
        };
        let target = match fs::canonicalize(&self.path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(WriteError::ChangedOnDisk),
            other => other.map_err(io_err("reading", &self.path))?,
        };
        let bytes = fs::read(&target).map_err(io_err("reading", &self.path))?;
        if hash(&bytes) != expected {
            return Err(WriteError::ChangedOnDisk);
        }
        Ok((target, Some(bytes)))
    }
}

fn hash(bytes: &[u8]) -> u64 {
    let mut h = DefaultHasher::new();
    h.write(bytes);
    h.finish()
}

/// Writes `text` (`\n` line ends) as the snapshot's file was: its encoding, BOM and dominant line
/// ending, keeping its current mode. Refuses when the file changed since the snapshot.
/// To overwrite a changed file anyway, take a fresh [`Snapshot::read`] and write with that.
/// Check a pending save before either side is written.
pub(crate) fn validate_text(snapshot: &Snapshot, text: &str) -> Result<(), WriteError> {
    if snapshot.lossy { return Err(WriteError::Lossy); }
    encode(text, snapshot.encoding, snapshot.bom, snapshot.crlf)?;
    snapshot.verify()
}

pub fn write_text(snapshot: &Snapshot, text: &str) -> Result<Snapshot, WriteError> {
    if snapshot.lossy {
        return Err(WriteError::Lossy);
    }
    let bytes = encode(text, snapshot.encoding, snapshot.bom, snapshot.crlf)?;
    let (target, old) = snapshot.current()?;
    match &old {
        Some(old) => replace(&target, &bytes, None, Some(old))?,
        None => create(&target, |tmp| new_temp(tmp, None, &bytes))?,
    }
    let meta = fs::metadata(&target).map_err(io_err("reading", &target))?;
    Ok(Snapshot { hash: Some(hash(&bytes)), size: meta.len(), mtime: meta.modified().ok(), ..snapshot.clone() })
}

/// The text in the file's form; the 1-based line of the first character the encoding can't hold.
fn encode(text: &str, encoding: &'static Encoding, bom: bool, crlf: bool) -> Result<Vec<u8>, WriteError> {
    let text = if crlf { with_crlf(text) } else { text.to_string() };
    let mut out = Vec::with_capacity(text.len() + 3);
    // encoding_rs encodes UTF-16 as UTF-8 (WHATWG), so UTF-16 is done here
    if encoding == UTF_16LE || encoding == UTF_16BE {
        let le = encoding == UTF_16LE;
        if bom {
            out.extend_from_slice(if le { &[0xFF, 0xFE] } else { &[0xFE, 0xFF] });
        }
        out.extend(text.encode_utf16().flat_map(|u| if le { u.to_le_bytes() } else { u.to_be_bytes() }));
        return Ok(out);
    }
    if bom && encoding == UTF_8 {
        out.extend_from_slice(b"\xEF\xBB\xBF");
    }
    let (bytes, _, had_errors) = encoding.encode(&text);
    if had_errors {
        // the output has `&#NNNN;` in place of the character: never write it
        let line = text.split('\n').position(|l| encoding.encode(l).2).unwrap_or(0) + 1;
        return Err(WriteError::Unrepresentable { line, encoding: encoding.name() });
    }
    out.extend_from_slice(&bytes);
    Ok(out)
}

/// `\n` → `\r\n`, leaving an existing `\r\n` alone.
pub(crate) fn with_crlf(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + text.len() / 16);
    let mut prev = '\0';
    for c in text.chars() {
        if c == '\n' && prev != '\r' {
            out.push('\r');
        }
        out.push(c);
        prev = c;
    }
    out
}

/// What a path holds, for the Version Picker's whole-file writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    /// A regular file; `mode: None` keeps the current file's mode (a new file gets the umask's).
    File { bytes: Vec<u8>, mode: Option<u32> },
    /// A symlink and its target.
    Link(PathBuf),
}

/// Makes `path` hold exactly `entry`, acting on the path itself: a link there is replaced, not followed.
/// A symlink is created under the temp name and renamed over (atomic). Undo of [`delete`] is `put` of what it returned.
pub fn put(path: &Path, entry: &Entry) -> Result<(), WriteError> {
    let meta = match fs::symlink_metadata(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        other => Some(other.map_err(io_err("reading", path))?),
    };
    match entry {
        // a regular file is replaced like any content write: hard links and ownership respected
        Entry::File { bytes, mode } if meta.as_ref().is_some_and(|m| m.is_file()) => replace(path, bytes, *mode, None),
        Entry::File { bytes, mode } => create(path, |tmp| new_temp(tmp, *mode, bytes)),
        Entry::Link(target) => create(path, |tmp| std::os::unix::fs::symlink(target, tmp)),
    }
}

/// Restores a private file backup without loading its bytes into memory.
pub(crate) fn put_file_from(path: &Path, source: &mut File, mode: u32) -> Result<(), WriteError> {
    let meta = match fs::symlink_metadata(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        other => Some(other.map_err(io_err("reading", path))?),
    };
    source.rewind().map_err(io_err("reading backup for", path))?;
    let Some(meta) = meta.filter(|m| m.is_file()) else {
        return create(path, |tmp| {
            source.rewind()?;
            let mut file = OpenOptions::new().write(true).create_new(true).mode(0o600).open(tmp)?;
            let result = (|| {
                file.set_permissions(fs::Permissions::from_mode(mode))?;
                io::copy(source, &mut file)?;
                file.sync_all()
            })();
            if result.is_err() { let _ = fs::remove_file(tmp); }
            result
        });
    };
    if meta.nlink() == 1 {
        match tempfile::Builder::new().prefix(".lenz-").suffix(".lenz~").tempfile_in(parent(path)) {
            Ok(tmp) => {
                if atomic_from(tmp.as_file(), tmp.path(), path, &meta, mode, source)
                    .map_err(io_err("writing", path))? { return Ok(()); }
            }
            Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {}
            Err(e) => return Err(io_err("writing", path)(e)),
        }
    }
    // Preserve hard links and ownership, with a streamed rollback for failed writes.
    let mut restore = || -> io::Result<()> {
        let mut original = File::open(path)?;
        let mut rollback = tempfile::tempfile()?;
        io::copy(&mut original, &mut rollback)?;
        let mut file = OpenOptions::new().write(true).open(path)?;
        let copy = |from: &mut File, to: &mut File| -> io::Result<()> {
            from.rewind()?;
            to.rewind()?;
            let size = io::copy(from, to)?;
            to.set_len(size)?;
            to.sync_all()
        };
        file.set_permissions(fs::Permissions::from_mode(mode))?;
        copy(source, &mut file).inspect_err(|_| {
            let _ = copy(&mut rollback, &mut file);
            let _ = file.set_permissions(fs::Permissions::from_mode(meta.mode() & 0o7777));
        })
    };
    restore().map_err(io_err("writing", path))
}

/// Removes an entry when the caller already owns its undo backup.
pub(crate) fn remove_entry(path: &Path) -> Result<(), WriteError> {
    match fs::remove_file(path) {
        Ok(()) => sync_dir(path),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {},
        Err(e) => return Err(io_err("deleting", path)(e)),
    }
    Ok(())
}

/// Removes the file or symlink at `path`, returning what re-creates it with [`put`].
pub fn delete(path: &Path) -> Result<Entry, WriteError> {
    let meta = fs::symlink_metadata(path).map_err(io_err("deleting", path))?;
    let entry = if meta.is_symlink() {
        Entry::Link(fs::read_link(path).map_err(io_err("reading", path))?)
    } else {
        Entry::File { bytes: fs::read(path).map_err(io_err("reading", path))?, mode: Some(meta.mode() & 0o7777) }
    };
    fs::remove_file(path).map_err(io_err("deleting", path))?;
    sync_dir(path);
    Ok(entry)
}

/// Repository operations must not traverse a working-tree symlink. Whole-entry
/// writes may replace the final link itself; text writes must reject it too.
pub(crate) fn check_repository_path(root: &Path, path: &Path, allow_leaf_link: bool) -> Result<(), WriteError> {
    let relative = path.strip_prefix(root).map_err(|_| WriteError::Io("Invalid repository file path".into()))?;
    if relative.as_os_str().is_empty() || relative.components().any(|c| !matches!(c, Component::Normal(_))) {
        return Err(WriteError::Io("Invalid repository file path".into()));
    }
    let end = if allow_leaf_link { path.parent().unwrap_or(root) } else { path };
    for part in end.ancestors() {
        match fs::symlink_metadata(part) {
            Ok(meta) if meta.is_symlink() => return Err(WriteError::Io("Cannot write a repository file through a symlink".into())),
            Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(io_err("reading", part)(e)),
            _ => {}
        }
        if part == root { break; }
    }
    Ok(())
}

fn parent(path: &Path) -> &Path {
    path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."))
}

/// Creates `path` (or replaces what's there) using a temp owned by this operation.
fn create(path: &Path, make: impl FnMut(&Path) -> io::Result<()>) -> Result<(), WriteError> {
    // git drops a deleted file's empty directories
    fs::create_dir_all(parent(path)).map_err(io_err("creating", parent(path)))?;
    let tmp = tempfile::Builder::new().prefix(".lenz-").suffix(".lenz~")
        .make_in(parent(path), make).map_err(io_err("writing", path))?;
    tmp.persist(path).map_err(|e| io_err("writing", path)(e.error))?;
    sync_dir(path);
    Ok(())
}

/// Writes a new temp file: private until its mode is set, then the bytes, then `fsync`.
fn new_temp(tmp: &Path, mode: Option<u32>, bytes: &[u8]) -> io::Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).mode(if mode.is_some() { 0o600 } else { 0o666 }).open(tmp)?;
    let result = (|| {
        if let Some(mode) = mode { file.set_permissions(fs::Permissions::from_mode(mode))?; }
        file.write_all(bytes)?;
        file.sync_all()
    })();
    if result.is_err() { let _ = fs::remove_file(tmp); }
    result
}

/// Replaces the regular file `target`'s content: atomically via a temp, else in place.
/// `old` is its content, when already read, for restoring after a failed in-place write.
fn replace(target: &Path, bytes: &[u8], mode: Option<u32>, old: Option<&[u8]>) -> Result<(), WriteError> {
    let meta = fs::metadata(target).map_err(io_err("reading", target))?;
    let mode = mode.unwrap_or(meta.mode() & 0o7777);
    if meta.nlink() == 1 {
        let opened = tempfile::Builder::new().prefix(".lenz-").suffix(".lenz~").tempfile_in(parent(target));
        match opened {
            Ok(tmp) => {
                if atomic(tmp.as_file(), tmp.path(), target, &meta, mode, bytes).map_err(io_err("writing", target))? {
                    return Ok(());
                }
                // Not ours to recreate: the owned temp drops before the in-place fallback.
            }
            // the directory isn't writable: the file may still be
            Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {}
            Err(e) => return Err(io_err("writing", target)(e)),
        }
    }
    let old = match old {
        Some(old) => old.to_vec(),
        None => fs::read(target).map_err(io_err("reading", target))?,
    };
    in_place(target, &meta, mode, bytes, &old).map_err(io_err("writing", target))
}

/// The temp-and-rename write. `Ok(false)`: the temp can't take the original's owner, so a rename would change it.
fn atomic(file: &File, tmp: &Path, target: &Path, meta: &fs::Metadata, mode: u32, mut bytes: &[u8]) -> io::Result<bool> {
    atomic_from(file, tmp, target, meta, mode, &mut bytes)
}

fn atomic_from(file: &File, tmp: &Path, target: &Path, meta: &fs::Metadata, mode: u32, source: &mut impl Read) -> io::Result<bool> {
    let ours = file.metadata()?;
    if ours.uid() != meta.uid() {
        return Ok(false);
    }
    if ours.gid() != meta.gid() && std::os::unix::fs::fchown(file, None, Some(meta.gid())).is_err() {
        return Ok(false);
    }
    // mode before content: a private file's bytes are never readable under the temp name
    file.set_permissions(fs::Permissions::from_mode(mode))?;
    io::copy(source, &mut &*file)?;
    file.sync_all()?;
    fs::rename(tmp, target)?;
    sync_dir(target);
    Ok(true)
}

// ponytail: the one non-atomic path (other hard links, foreign owner, unwritable directory); a failure
// mid-write restores `old` best-effort. A copy of the original elsewhere would survive a crash too.
fn in_place(target: &Path, meta: &fs::Metadata, mode: u32, bytes: &[u8], old: &[u8]) -> io::Result<()> {
    let mut file = OpenOptions::new().write(true).open(target)?;
    if meta.mode() & 0o7777 != mode {
        file.set_permissions(fs::Permissions::from_mode(mode))?;
    }
    let write = |file: &mut File, bytes: &[u8]| -> io::Result<()> {
        file.rewind()?;
        file.write_all(bytes)?;
        file.set_len(bytes.len() as u64)?;
        file.sync_all()
    };
    write(&mut file, bytes).inspect_err(|_| {
        let _ = write(&mut file, old);
    })
}

/// `fsync` of `path`'s directory, so a rename survives a crash. Best effort.
fn sync_dir(path: &Path) {
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    if let Ok(dir) = File::open(dir) {
        let _ = dir.sync_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{os::unix::fs::symlink, time::Duration};

    fn mode(path: &Path) -> u32 {
        fs::metadata(path).unwrap().mode() & 0o7777
    }

    fn set_mode(path: &Path, mode: u32) {
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
    }

    fn utf16(s: &str, le: bool) -> Vec<u8> {
        let mut out = if le { vec![0xFF, 0xFE] } else { vec![0xFE, 0xFF] };
        out.extend(s.encode_utf16().flat_map(|u| if le { u.to_le_bytes() } else { u.to_be_bytes() }));
        out
    }

    /// Writes `bytes`, snapshots the file, writes `text` back through it; returns the file's new bytes.
    fn round_trip(bytes: &[u8], text: &str) -> Vec<u8> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f.txt");
        fs::write(&path, bytes).unwrap();
        let (snap, _) = Snapshot::read(&path, None).unwrap();
        write_text(&snap, text).unwrap();
        fs::read(&path).unwrap()
    }

    fn no_temps(dir: &Path) -> bool {
        fs::read_dir(dir).unwrap().flatten().all(|e| !e.file_name().to_string_lossy().ends_with(".lenz~"))
    }

    /// Root ignores permission bits; tests about them can't run as root.
    fn permissions_bite(dir: &Path) -> bool {
        let probe = dir.join("probe");
        fs::write(&probe, "").unwrap();
        set_mode(&probe, 0o000);
        let bites = File::open(&probe).is_err();
        fs::remove_file(&probe).unwrap();
        bites
    }

    #[test]
    fn writes_atomically_keeping_the_mode() {
        let dir = tempfile::tempdir().unwrap();
        for m in [0o755, 0o600, 0o640] {
            let path = dir.path().join(format!("run-{m:o}.sh"));
            fs::write(&path, "echo a\n").unwrap();
            set_mode(&path, m);
            let ino = fs::metadata(&path).unwrap().ino();
            let (snap, d) = Snapshot::read(&path, None).unwrap();
            assert_eq!(d.text, "echo a\n");
            let next = write_text(&snap, "echo b\n").unwrap();
            assert_eq!(fs::read_to_string(&path).unwrap(), "echo b\n");
            assert_eq!(mode(&path), m);
            assert_ne!(fs::metadata(&path).unwrap().ino(), ino, "renamed over, not written in place");
            // the returned snapshot is current: writable again, not "changed"
            assert!(!next.changed());
            write_text(&next, "echo c\n").unwrap();
        }
        assert!(no_temps(dir.path()));
    }

    #[test]
    fn a_symlinked_path_writes_its_target_and_keeps_the_link() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("real")).unwrap();
        fs::create_dir_all(dir.path().join("links")).unwrap();
        let target = dir.path().join("real/f.txt");
        let link = dir.path().join("links/f.txt");
        fs::write(&target, "one\n").unwrap();
        symlink("../real/f.txt", &link).unwrap();
        let (snap, _) = Snapshot::read(&link, None).unwrap();
        write_text(&snap, "two\n").unwrap();
        assert!(fs::symlink_metadata(&link).unwrap().is_symlink());
        assert_eq!(fs::read_link(&link).unwrap(), Path::new("../real/f.txt"));
        assert_eq!(fs::read_to_string(&target).unwrap(), "two\n");
        assert!(no_temps(&dir.path().join("real")) && no_temps(&dir.path().join("links")));
    }

    #[test]
    fn a_hard_linked_file_is_written_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        fs::write(&a, "old\n").unwrap();
        fs::hard_link(&a, &b).unwrap();
        let (snap, _) = Snapshot::read(&a, None).unwrap();
        write_text(&snap, "new, and longer\n").unwrap();
        assert_eq!(fs::read_to_string(&a).unwrap(), "new, and longer\n");
        assert_eq!(fs::read_to_string(&b).unwrap(), "new, and longer\n");
        assert_eq!(fs::metadata(&a).unwrap().ino(), fs::metadata(&b).unwrap().ino());
        // shorter content truncates
        let (snap, _) = Snapshot::read(&b, None).unwrap();
        write_text(&snap, "x\n").unwrap();
        assert_eq!(fs::read_to_string(&a).unwrap(), "x\n");
    }

    #[test]
    fn utf16_with_bom_round_trips() {
        for le in [true, false] {
            let bytes = round_trip(&utf16("añb\n", le), "añ€b\nc😀\n");
            assert_eq!(bytes, utf16("añ€b\nc😀\n", le));
            let d = decode(&bytes, None);
            assert_eq!((d.text.as_str(), d.encoding, d.had_bom), ("añ€b\nc😀\n", if le { "UTF-16LE" } else { "UTF-16BE" }, true));
        }
    }

    #[test]
    fn utf16_declared_without_bom_stays_without() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f");
        fs::write(&path, &utf16("a\n", true)[2..]).unwrap();
        let (snap, _) = Snapshot::read(&path, Some("UTF-16LE")).unwrap();
        write_text(&snap, "b\n").unwrap();
        assert_eq!(fs::read(&path).unwrap(), utf16("b\n", true)[2..]);
    }

    #[test]
    fn utf8_bom_is_kept() {
        assert_eq!(round_trip(b"\xEF\xBB\xBFa\n", "b\n"), b"\xEF\xBB\xBFb\n");
        assert_eq!(round_trip(b"a\n", "b\n"), b"b\n");
    }

    #[test]
    fn line_endings_follow_the_majority() {
        assert_eq!(round_trip(b"a\r\nb\r\n", "x\ny\n"), b"x\r\ny\r\n");
        assert_eq!(round_trip(b"a\r\nb\r\nc\n", "x\ny\nz"), b"x\r\ny\r\nz");
        assert_eq!(round_trip(b"a\r\nb\nc\n", "x\ny\n"), b"x\ny\n");
        // a tie, and no line ends at all, are LF
        assert_eq!(round_trip(b"a\r\nb\n", "x\ny\n"), b"x\ny\n");
        assert_eq!(round_trip(b"a", "x\ny\n"), b"x\ny\n");
        // a CRLF already in the text isn't doubled
        assert_eq!(round_trip(b"a\r\n", "x\r\ny\n"), b"x\r\ny\r\n");
    }

    #[test]
    fn legacy_encodings_round_trip() {
        let (latin, _, _) = encoding_rs::WINDOWS_1252.encode("café\n");
        assert_eq!(round_trip(&latin, "déjà vu\n"), encoding_rs::WINDOWS_1252.encode("déjà vu\n").0.as_ref());
    }

    #[test]
    fn an_unrepresentable_character_is_refused_naming_its_line() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f");
        let (latin, _, _) = encoding_rs::WINDOWS_1252.encode("café\n");
        fs::write(&path, &latin).unwrap();
        let (snap, _) = Snapshot::read(&path, Some("windows-1252")).unwrap();
        let err = write_text(&snap, "one\ntwo\nthree 😀\n").unwrap_err();
        assert_eq!(err, WriteError::Unrepresentable { line: 3, encoding: "windows-1252" });
        assert_eq!(err.to_string(), "line 3 has a character windows-1252 can't represent");
        assert_eq!(fs::read(&path).unwrap(), latin.as_ref());
        assert!(no_temps(dir.path()));
    }

    #[test]
    fn a_lossy_read_is_never_written_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f");
        fs::write(&path, b"ab\xFFcd\n").unwrap();
        let (snap, d) = Snapshot::read(&path, Some("utf-8")).unwrap();
        assert!(d.lossy);
        assert_eq!(write_text(&snap, "x\n"), Err(WriteError::Lossy));
        assert_eq!(fs::read(&path).unwrap(), b"ab\xFFcd\n");
    }

    #[test]
    fn a_change_on_disk_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f");
        fs::write(&path, "one\n").unwrap();
        let (snap, _) = Snapshot::read(&path, None).unwrap();
        assert!(!snap.changed());
        assert_eq!(snap.verify(), Ok(()));
        fs::write(&path, "someone else's, longer\n").unwrap();
        assert!(snap.changed());
        assert_eq!(snap.verify(), Err(WriteError::ChangedOnDisk));
        assert_eq!(write_text(&snap, "mine\n"), Err(WriteError::ChangedOnDisk));
        assert_eq!(fs::read_to_string(&path).unwrap(), "someone else's, longer\n");
        // same size, same mtime, different bytes: only the hash sees it
        let mtime = fs::metadata(&path).unwrap().modified().unwrap();
        let (snap, _) = Snapshot::read(&path, None).unwrap();
        fs::write(&path, "someone else's, LONGER\n").unwrap();
        File::options().write(true).open(&path).unwrap().set_modified(mtime).unwrap();
        assert!(!snap.changed());
        assert_eq!(write_text(&snap, "mine\n"), Err(WriteError::ChangedOnDisk));
        // deleted meanwhile
        fs::remove_file(&path).unwrap();
        assert!(snap.changed());
        assert_eq!(write_text(&snap, "mine\n"), Err(WriteError::ChangedOnDisk));
        assert!(!path.exists());
    }

    #[test]
    fn a_missing_file_is_written_like_ours() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gone/deeper/f.txt");
        let ours = decode(&utf16("a\r\nb\r\n", false), None);
        let snap = Snapshot::missing(&path, &ours);
        assert!(!snap.changed());
        let next = write_text(&snap, "x\ny\n").unwrap();
        assert_eq!(fs::read(&path).unwrap(), utf16("x\r\ny\r\n", false));
        assert!(!next.changed());
        // created meanwhile: refused
        let other = dir.path().join("other");
        let snap = Snapshot::missing(&other, &ours);
        fs::write(&other, "theirs").unwrap();
        assert!(snap.changed());
        assert_eq!(write_text(&snap, "x\n"), Err(WriteError::ChangedOnDisk));
        assert_eq!(fs::read_to_string(&other).unwrap(), "theirs");
    }

    #[test]
    fn an_unwritable_directory_falls_back_to_writing_in_place() {
        let dir = tempfile::tempdir().unwrap();
        if !permissions_bite(dir.path()) {
            eprintln!("skipped: running as root");
            return;
        }
        let sub = dir.path().join("ro");
        fs::create_dir(&sub).unwrap();
        let path = sub.join("f");
        fs::write(&path, "one\n").unwrap();
        set_mode(&path, 0o640);
        // a crash's leftover temp that can't be removed there doesn't block reading or writing
        let tmp = sub.join(".f.lenz~");
        fs::write(&tmp, "leftover").unwrap();
        File::options().write(true).open(&tmp).unwrap().set_modified(SystemTime::now() - Duration::from_secs(60)).unwrap();
        set_mode(&sub, 0o555);
        let (snap, _) = Snapshot::read(&path, None).unwrap();
        let written = write_text(&snap, "two\n");
        // and a read-only file there too: a clear error, the original intact
        set_mode(&path, 0o440);
        let (snap2, _) = Snapshot::read(&path, None).unwrap();
        let refused = write_text(&snap2, "three\n");
        set_mode(&sub, 0o755);
        written.unwrap();
        assert!(matches!(&refused, Err(WriteError::Io(msg)) if msg.contains("Permission denied")), "{refused:?}");
        assert_eq!(fs::read_to_string(&path).unwrap(), "two\n");
        assert_eq!(fs::read_to_string(&tmp).unwrap(), "leftover");
    }

    #[test]
    fn a_failed_rename_leaves_the_original_and_no_temp() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d");
        fs::create_dir(&path).unwrap();
        fs::write(path.join("inside"), "keep").unwrap();
        let err = put(&path, &Entry::File { bytes: b"x".to_vec(), mode: Some(0o644) }).unwrap_err();
        assert!(matches!(&err, WriteError::Io(msg) if msg.starts_with("writing ")), "{err:?}");
        assert_eq!(fs::read_to_string(path.join("inside")).unwrap(), "keep");
        assert!(no_temps(dir.path()));
    }

    #[test]
    fn streamed_backup_restores_atomically_and_preserves_hard_links() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("result");
        let alias = dir.path().join("alias");
        let mut backup = tempfile::tempfile().unwrap();
        backup.write_all(b"restored\0bytes").unwrap();
        for hard_link in [false, true] {
            fs::write(&path, b"current").unwrap();
            set_mode(&path, 0o644);
            if hard_link { fs::hard_link(&path, &alias).unwrap(); }
            let ino = fs::metadata(&path).unwrap().ino();
            put_file_from(&path, &mut backup, 0o751).unwrap();
            assert_eq!(fs::read(&path).unwrap(), b"restored\0bytes");
            assert_eq!(mode(&path), 0o751);
            if hard_link {
                assert_eq!(fs::read(&alias).unwrap(), b"restored\0bytes");
                assert_eq!(fs::metadata(&path).unwrap().ino(), ino);
                // A source that refuses reading must leave the old bytes and mode intact.
                let mut unreadable = OpenOptions::new().write(true).open(dir.path().join("alias")).unwrap();
                assert!(put_file_from(&path, &mut unreadable, 0o600).is_err());
                assert_eq!(fs::read(&path).unwrap(), b"restored\0bytes");
                assert_eq!(mode(&path), 0o751);
            } else { assert_ne!(fs::metadata(&path).unwrap().ino(), ino); }
        }
        assert!(no_temps(dir.path()));
    }

    #[test]
    fn put_writes_exact_bytes_keeping_or_setting_the_mode() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bin");
        fs::write(&path, b"\x00old").unwrap();
        set_mode(&path, 0o755);
        put(&path, &Entry::File { bytes: b"\x00\xFFnew\r\n".to_vec(), mode: None }).unwrap();
        assert_eq!((fs::read(&path).unwrap(), mode(&path)), (b"\x00\xFFnew\r\n".to_vec(), 0o755));
        put(&path, &Entry::File { bytes: b"plain".to_vec(), mode: Some(0o644) }).unwrap();
        assert_eq!((fs::read(&path).unwrap(), mode(&path)), (b"plain".to_vec(), 0o644));
    }

    #[test]
    fn delete_then_put_restores_bytes_and_mode() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tool.sh");
        fs::write(&path, "#!/bin/sh\n").unwrap();
        set_mode(&path, 0o750);
        let removed = delete(&path).unwrap();
        assert_eq!(removed, Entry::File { bytes: b"#!/bin/sh\n".to_vec(), mode: Some(0o750) });
        assert!(!path.exists());
        put(&path, &removed).unwrap();
        assert_eq!((fs::read_to_string(&path).unwrap(), mode(&path)), ("#!/bin/sh\n".into(), 0o750));
        // a link deletes and comes back as a link
        let link = dir.path().join("link");
        symlink("tool.sh", &link).unwrap();
        let removed = delete(&link).unwrap();
        assert_eq!(removed, Entry::Link("tool.sh".into()));
        assert!(fs::symlink_metadata(&link).is_err() && path.exists());
        put(&link, &removed).unwrap();
        assert_eq!(fs::read_link(&link).unwrap(), Path::new("tool.sh"));
    }

    #[test]
    fn put_link_replaces_a_link_or_a_file_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let link = dir.path().join("link");
        fs::write(dir.path().join("a"), "a").unwrap();
        symlink("a", &link).unwrap();
        put(&link, &Entry::Link("b".into())).unwrap();
        assert_eq!(fs::read_link(&link).unwrap(), Path::new("b"));
        assert_eq!(fs::read_to_string(dir.path().join("a")).unwrap(), "a", "the old target is untouched");
        // a file becomes a link
        let file = dir.path().join("file");
        fs::write(&file, "x").unwrap();
        put(&file, &Entry::Link("a".into())).unwrap();
        assert_eq!(fs::read_link(&file).unwrap(), Path::new("a"));
        assert!(no_temps(dir.path()));
    }

    #[test]
    fn put_file_replaces_a_link_with_a_file_of_that_mode() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target");
        fs::write(&target, "target").unwrap();
        let link = dir.path().join("link");
        symlink("target", &link).unwrap();
        put(&link, &Entry::File { bytes: b"now a file\n".to_vec(), mode: Some(0o755) }).unwrap();
        let meta = fs::symlink_metadata(&link).unwrap();
        assert!(meta.is_file());
        assert_eq!(meta.mode() & 0o7777, 0o755);
        assert_eq!(fs::read_to_string(&link).unwrap(), "now a file\n");
        assert_eq!(fs::read_to_string(&target).unwrap(), "target");
    }

    #[test]
    fn reading_and_saving_preserve_unowned_temp_names() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f.txt");
        let tmp = dir.path().join(".f.txt.lenz~");
        fs::write(&path, "one\n").unwrap();
        fs::write(&tmp, "unrelated user file").unwrap();
        for age in [0, 60] {
            File::options().write(true).open(&tmp).unwrap()
                .set_modified(SystemTime::now() - Duration::from_secs(age)).unwrap();
            let (snap, _) = Snapshot::read(&path, None).unwrap();
            assert_eq!(fs::read_to_string(&tmp).unwrap(), "unrelated user file");
            write_text(&snap, "two\n").unwrap();
            assert_eq!(fs::read_to_string(&tmp).unwrap(), "unrelated user file");
        }
        assert_eq!(fs::read_to_string(&path).unwrap(), "two\n");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
    }
}
