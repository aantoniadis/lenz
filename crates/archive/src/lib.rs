//! Bounded archive snapshots. Members are read and replaced in memory; no archive path is extracted
//! onto the filesystem. A caller owns disk identity, atomic writes, nested limits and cache eviction.

use std::{collections::BTreeMap, fmt, io, sync::{Arc, atomic::{AtomicU64, Ordering}}};

mod codecs;
mod tar;
mod zip;

#[cfg(test)]
mod tests;
#[cfg(any(test, feature = "testing"))]
pub mod testing;

/// File-name formats shared by the Reader and Diff.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind { Zip, Tar(Compression) }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Compression { None, Gzip, Bzip2, Xz, Zstd }

impl Kind {
    pub fn of(path: &str) -> Option<Self> {
        let name = path.rsplit('/').next()?.to_ascii_lowercase();
        let ends = |suffixes: &[&str]| suffixes.iter().any(|s| name.len() > s.len() && name.ends_with(s));
        Some(if ends(&[".zip"]) { Self::Zip }
        else if ends(&[".tar"]) { Self::Tar(Compression::None) }
        else if ends(&[".tar.gz", ".tgz"]) { Self::Tar(Compression::Gzip) }
        else if ends(&[".tar.bz2", ".tbz2", ".tbz"]) { Self::Tar(Compression::Bzip2) }
        else if ends(&[".tar.xz", ".txz"]) { Self::Tar(Compression::Xz) }
        else if ends(&[".tar.zst", ".tzst"]) { Self::Tar(Compression::Zstd) }
        else { return None })
    }
    /// Resolve the format of an already selected archive layer. Canonical file identities may have
    /// a different suffix from a user's symlink alias, so supported content signatures take priority.
    /// This is a bounded prefix check, not validation; `open` diagnoses malformed content. Ordinary
    /// file discovery should continue using `of` rather than treating arbitrary files as archives.
    pub fn detect(name: &str, bytes: &[u8]) -> Option<Self> {
        if bytes.starts_with(b"PK\x03\x04") || bytes.starts_with(b"PK\x05\x06") || bytes.starts_with(b"PK\x07\x08") { return Some(Self::Zip); }
        if bytes.starts_with(&[31, 139, 8]) { return Some(Self::Tar(Compression::Gzip)); }
        if bytes.starts_with(b"BZh") { return Some(Self::Tar(Compression::Bzip2)); }
        if bytes.starts_with(b"\xfd7zXZ\0") { return Some(Self::Tar(Compression::Xz)); }
        if let Some(magic) = bytes.get(..4) {
            let magic = u32::from_le_bytes([magic[0], magic[1], magic[2], magic[3]]);
            if magic == 0xfd2fb528 || magic & 0xfffffff0 == 0x184d2a50 { return Some(Self::Tar(Compression::Zstd)); }
        }
        if let Some(block) = bytes.get(..512) {
            let header = ::tar::Header::from_byte_slice(block);
            let sum: u32 = block.iter().enumerate().map(|(i, b)| if (148..156).contains(&i) { 32 } else { *b as u32 }).sum();
            if header.cksum().ok() == Some(sum) && header.entry_size().is_ok() { return Some(Self::Tar(Compression::None)); }
            if bytes.get(..1024).is_some_and(|b| b.iter().all(|b| *b == 0)) { return Some(Self::Tar(Compression::None)); }
        }
        Self::of(name)
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Zip => "zip archive", Self::Tar(Compression::None) => "tar archive",
            Self::Tar(Compression::Gzip) => "gzip tar archive", Self::Tar(Compression::Bzip2) => "bzip2 tar archive",
            Self::Tar(Compression::Xz) => "xz tar archive", Self::Tar(Compression::Zstd) => "zstd tar archive",
        }
    }
}

/// Per-layer budgets. Callers additionally bound nested depth, aggregate bytes and cached snapshots.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_archive: u64,
    pub max_unpacked: u64,
    pub max_member: u64,
    pub max_members: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self { max_archive: 512 << 20, max_unpacked: 2 << 30, max_member: 1 << 30, max_members: 100_000 }
    }
}

#[derive(Debug)]
pub enum Error { Invalid(String), Limit(&'static str), Unsupported(String), StaleMember, Cancelled }
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(reason) => write!(f, "invalid archive: {reason}"),
            Self::Limit(what) => write!(f, "archive exceeds the {what} limit"),
            Self::Unsupported(reason) => f.write_str(reason),
            Self::StaleMember => f.write_str("archive member belongs to another snapshot"),
            Self::Cancelled => f.write_str("archive operation cancelled"),
        }
    }
}
impl std::error::Error for Error {}
impl From<io::Error> for Error {
    fn from(value: io::Error) -> Self { Self::Invalid(value.to_string()) }
}

/// An identity within one immutable snapshot; reopening an archive deliberately produces new IDs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MemberId { revision: u64, ordinal: usize }
impl MemberId { pub fn ordinal(self) -> usize { self.ordinal } }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemberKind { File, Directory, Symlink, HardLink, Metadata, Other }

#[derive(Debug, Clone)]
pub struct Member {
    pub id: MemberId,
    /// Original header name bytes. PAX/GNU path overrides are reflected in `path`.
    pub stored_path: Vec<u8>,
    /// Normalized virtual path, never a filesystem extraction destination. `None` is hidden/unsafe.
    pub path: Option<String>,
    pub kind: MemberKind,
    pub size: u64,
    /// ZIP's recorded CRC, without reading its payload. TAR leaves it absent.
    pub crc32: Option<u32>,
    pub read_only_reason: Option<String>,
}
impl Member { pub fn editable(&self) -> bool { self.read_only_reason.is_none() && self.kind == MemberKind::File } }

#[derive(Debug, Clone, Copy)]
pub struct Replacement<'a> { pub id: MemberId, pub bytes: &'a [u8] }

#[derive(Clone)]
enum Data { Zip(zip::Data), Tar(tar::Data) }

/// One shared stored snapshot and its index. ZIP listing does not decompress file payloads; TAR
/// retains one bounded unpacked stream and borrows raw record spans from it.
#[derive(Clone)]
pub struct Index {
    kind: Kind,
    bytes: Arc<[u8]>,
    limits: Limits,
    revision: u64,
    members: Arc<[Member]>,
    blocked: Option<String>,
    data: Data,
}
impl fmt::Debug for Index {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Index").field("kind", &self.kind).field("stored_bytes", &self.bytes.len())
            .field("members", &self.members.len()).field("rewrite_blocked", &self.blocked).finish()
    }
}

impl Index {
    pub fn open(kind: Kind, bytes: Arc<[u8]>, limits: Limits, cancel: &dyn Fn() -> bool) -> Result<Self, Error> {
        check(cancel)?;
        if bytes.len() as u64 > limits.max_archive { return Err(Error::Limit("stored bytes")); }
        static NEXT_REVISION: AtomicU64 = AtomicU64::new(1);
        let revision = NEXT_REVISION.try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .map_err(|_| Error::Unsupported("archive snapshot identity space exhausted".into()))?;
        let (data, mut members, blocked) = match kind {
            Kind::Zip => { let (data, members, blocked) = zip::open(bytes.clone(), limits, revision, cancel)?; (Data::Zip(data), members, blocked) }
            Kind::Tar(compression) => { let (data, members, blocked) = tar::open(compression, bytes.clone(), limits, revision, cancel)?; (Data::Tar(data), members, blocked) }
        };
        // Member paths are persistent locations for callers. Ambiguous aliases cannot be write targets.
        let mut paths = BTreeMap::new();
        for member in &members {
            if member.kind != MemberKind::Metadata && let Some(path) = &member.path { *paths.entry(path.clone()).or_insert(0usize) += 1; }
        }
        for member in &mut members {
            if let Some(reason) = &blocked { member.read_only_reason = Some(reason.clone()); }
            else if member.path.as_ref().is_some_and(|p| paths.get(p).copied().unwrap_or(0) > 1) {
                member.read_only_reason = Some("multiple archive members have the same normalized path".into());
            }
        }
        Ok(Self { kind, bytes, limits, revision, members: members.into(), blocked, data })
    }
    pub fn kind(&self) -> Kind { self.kind }
    pub fn bytes(&self) -> &Arc<[u8]> { &self.bytes }
    pub fn members(&self) -> &[Member] { &self.members }
    pub fn rewrite_blocked(&self) -> Option<&str> { self.blocked.as_deref() }
    /// Conservative retained-byte estimate for bounded caller caches, including unpacked TAR data.
    /// Allocator overhead and codec scratch space are not exact heap accounting.
    pub fn resident_bytes(&self) -> usize {
        let unpacked = match &self.data {
            Data::Tar(data) if !Arc::ptr_eq(&self.bytes, &data.raw) => data.raw.len(),
            Data::Zip(data) => data.metadata_bytes,
            _ => 0,
        };
        self.members.iter().fold(self.bytes.len().saturating_add(unpacked), |total, m| {
            total.saturating_add(std::mem::size_of::<Member>()).saturating_add(m.stored_path.len())
                .saturating_add(m.path.as_ref().map_or(0, String::len))
                .saturating_add(m.read_only_reason.as_ref().map_or(0, String::len))
                .saturating_add(if matches!(m.kind, MemberKind::Symlink | MemberKind::HardLink) { usize::try_from(m.size).unwrap_or(usize::MAX) } else { 0 })
                // Parsed ZIP metadata and raw TAR spans/link targets also retain per-entry storage.
                .saturating_add(256).saturating_add(m.stored_path.len())
        })
    }
    pub fn member(&self, id: MemberId) -> Result<&Member, Error> {
        if id.revision != self.revision { return Err(Error::StaleMember); }
        self.members.get(id.ordinal).ok_or(Error::StaleMember)
    }
    pub fn read_member(&self, id: MemberId, cancel: &dyn Fn() -> bool) -> Result<Vec<u8>, Error> {
        check(cancel)?;
        let member = self.member(id)?;
        if member.size > self.limits.max_member { return Err(Error::Limit("member bytes")); }
        match &self.data {
            Data::Zip(data) => zip::read(data, member, self.limits, cancel),
            Data::Tar(data) => tar::read(data, member, self.limits, cancel),
        }
    }
    pub fn replace_members(&self, updates: &[Replacement<'_>], cancel: &dyn Fn() -> bool) -> Result<Vec<u8>, Error> {
        check(cancel)?;
        if updates.is_empty() { return Ok(self.bytes.to_vec()); }
        if let Some(reason) = &self.blocked { return Err(Error::Unsupported(reason.clone())); }
        let mut replacements = BTreeMap::new();
        for update in updates {
            let member = self.member(update.id)?;
            if !member.editable() { return Err(Error::Unsupported(member.read_only_reason.clone().unwrap_or_else(|| "only regular archive files can be edited".into()))); }
            if update.bytes.len() as u64 > self.limits.max_member { return Err(Error::Limit("member bytes")); }
            if replacements.insert(update.id.ordinal, update.bytes).is_some() { return Err(Error::Unsupported("a member was replaced more than once in one operation".into())); }
        }
        let output = match &self.data {
            Data::Zip(data) => zip::replace(data, &self.members, &replacements, self.limits, cancel)?,
            Data::Tar(data) => tar::replace(data, &replacements, self.limits, cancel)?,
        };
        if output.len() as u64 > self.limits.max_archive { return Err(Error::Limit("stored bytes")); }
        check(cancel)?;
        Ok(output)
    }
}

pub(crate) fn check(cancel: &dyn Fn() -> bool) -> Result<(), Error> {
    if cancel() { Err(Error::Cancelled) } else { Ok(()) }
}
pub(crate) fn invalid(reason: impl fmt::Display) -> Error { Error::Invalid(reason.to_string()) }
pub(crate) fn normalize(name: &str) -> Option<String> {
    let mut parts = Vec::new();
    for part in name.split('/') { match part { "" | "." => {}, ".." => return None, part => parts.push(part) } }
    (!parts.is_empty()).then(|| parts.join("/"))
}
pub(crate) fn read_bounded(reader: &mut dyn io::Read, output: &mut Vec<u8>, maximum: u64, what: &'static str, cancel: &dyn Fn() -> bool) -> Result<(), Error> {
    let mut buffer = [0u8; 64 << 10];
    loop {
        check(cancel)?;
        // One sentinel byte discovers an overrun without allocating a decoder's claimed size.
        let left = maximum.saturating_sub(output.len() as u64);
        let count = buffer.len().min(left.saturating_add(1).min(usize::MAX as u64) as usize);
        let n = reader.read(&mut buffer[..count])?;
        if n == 0 { return Ok(()); }
        if n as u64 > left { return Err(Error::Limit(what)); }
        output.extend_from_slice(&buffer[..n]);
    }
}
