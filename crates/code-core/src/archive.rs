//! Archive locations are document identities, never filesystem extraction paths.
use std::{borrow::Cow, collections::BTreeMap, os::unix::ffi::{OsStrExt, OsStringExt}, path::{Path, PathBuf}, sync::Arc};
use lenz_archive::{Index, Kind, Limits, Member, Replacement};

const PREFIX: &str = "lenz-archive://";
pub const MAX_DEPTH: usize = 4;
const NESTED_BYTES: usize = 2 << 30;

/// The outer physical file and the successive member names inside it. Empty members names the
/// archive itself in the browser; a document has at least one member. Names are normalized virtual
/// paths, not paths that are ever joined to a directory on disk.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Location {
    pub container: PathBuf,
    pub members: Vec<String>,
}
impl Location {
    pub fn new(container: PathBuf, members: Vec<String>) -> Result<Self, String> {
        if !container.is_absolute() { return Err("an archive needs an absolute container path".into()); }
        if members.len() > MAX_DEPTH { return Err("archive nesting limit reached".into()); }
        if members.iter().any(|name| name.is_empty() || name.starts_with('/') || name.contains('\0')
            || name.split('/').any(|part| matches!(part, "" | "." | ".."))) {
            return Err("archive member names must be normalized relative paths".into());
        }
        let mut normalized = PathBuf::new();
        for part in container.components() {
            match part {
                std::path::Component::CurDir => {},
                std::path::Component::ParentDir => { normalized.pop(); },
                part => normalized.push(part.as_os_str()),
            }
        }
        if normalized.as_os_str().as_bytes().contains(&0) { return Err("invalid archive container path".into()); }
        Ok(Self { container: normalized, members })
    }
    pub fn to_uri(&self) -> PathBuf {
        let mut uri = format!("{PREFIX}{}", hex(self.container.as_os_str().as_bytes()));
        for member in &self.members { uri.push('/'); uri.push_str(&hex(member.as_bytes())); }
        uri.into()
    }
    pub fn from_uri(path: &Path) -> Option<Self> {
        let mut parts = path.to_str()?.strip_prefix(PREFIX)?.split('/');
        let container = PathBuf::from(std::ffi::OsString::from_vec(unhex(parts.next()?)?));
        let members = parts.map(|part| String::from_utf8(unhex(part)?).ok()).collect::<Option<Vec<_>>>()?;
        Self::new(container, members).ok()
    }
    pub fn canonicalized(&self) -> Result<Self, String> {
        Self::new(std::fs::canonicalize(&self.container).map_err(|e| e.to_string())?, self.members.clone())
    }
    pub fn canonical_source(&self) -> Result<PathBuf, String> { Ok(self.canonicalized()?.to_uri()) }
    pub fn outer_path(&self) -> &Path { &self.container }
    pub fn member_name(&self) -> Option<&str> { self.members.last().map(String::as_str) }
    pub fn display_path(&self) -> String {
        let mut shown = self.container.display().to_string();
        for member in &self.members { shown.push_str("!/"); shown.push_str(member); }
        shown
    }
    pub fn estimated_bytes(&self) -> usize {
        std::mem::size_of::<Self>() + self.container.capacity() + self.members.capacity() * std::mem::size_of::<String>()
            + self.members.iter().map(String::capacity).sum::<usize>()
    }
    pub fn parent(&self) -> Option<Self> {
        if self.members.is_empty() { return None; }
        let mut parent = self.clone(); parent.members.pop(); Some(parent)
    }
}
impl serde::Serialize for Location {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_uri().to_string_lossy())
    }
}
impl<'de> serde::Deserialize<'de> for Location {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = <String as serde::Deserialize>::deserialize(deserializer)?;
        Self::from_uri(Path::new(&value)).ok_or_else(|| serde::de::Error::custom("invalid archive location"))
    }
}
fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &byte in bytes { out.push(DIGITS[(byte >> 4) as usize] as char); out.push(DIGITS[(byte & 15) as usize] as char); }
    out
}
fn unhex(value: &str) -> Option<Vec<u8>> {
    if !value.len().is_multiple_of(2) { return None; }
    value.as_bytes().as_chunks::<2>().0.iter().map(|pair| Some((digit(pair[0])? << 4) | digit(pair[1])?)).collect()
}
fn digit(value: u8) -> Option<u8> { match value { b'0'..=b'9' => Some(value - b'0'), b'a'..=b'f' => Some(value - b'a' + 10), _ => None } }
pub(crate) fn physical_path(path: &Path) -> Cow<'_, Path> {
    Location::from_uri(path).map_or(Cow::Borrowed(path), |location| Cow::Owned(location.container))
}

/// Small exact outer-container identity shared by its member baselines and write lane. Raw bytes
/// are worker/browser-cache owned, so released clean tabs cannot pin many whole archives.
#[derive(Debug)]
pub(crate) struct ContainerSnapshot {
    pub snapshot: lenz_content::write::Snapshot,
    pub target: lenz_content::write::TargetIdentity,
    pub fingerprint: [u8; 32],
}
pub(crate) struct ContainerRead {
    pub baseline: Arc<ContainerSnapshot>,
    pub bytes: Arc<[u8]>,
}
impl ContainerSnapshot {
    pub fn same(&self, other: &Self) -> bool { self.target == other.target && self.fingerprint == other.fingerprint }
    pub fn stamp(&self) -> crate::cache::Stamp {
        crate::cache::Stamp { len: self.snapshot.len_bytes(), modified: self.snapshot.modified(), file: Some((self.target.device, self.target.inode)) }
    }
}
/// A member's confirmed source bytes belong to this exact container, separately from disk-text save targets.
#[derive(Clone, Debug)]
pub(crate) struct Baseline {
    pub location: Location,
    pub container: Arc<ContainerSnapshot>,
    pub crlf: bool,
    pub encoding: &'static str,
    pub bom: bool,
}

pub(crate) fn read_container(path: &Path, cancel: &dyn Fn() -> bool) -> Result<ContainerRead, String> {
    for _ in 0..3 {
        if cancel() { return Err("archive operation cancelled".into()); }
        let before = crate::cache::Stamp::of(path).ok_or_else(|| format!("Cannot read {}", path.display()))?;
        if before.len > Limits::default().max_archive { return Err("archive exceeds the stored bytes limit".into()); }
        let read = lenz_content::write::read_exact_bytes_bounded(path, Limits::default().max_archive, cancel).map_err(|e| e.to_string())?;
        let snapshot = read.bytes_snapshot();
        let target = read.target.clone();
        let stamp = crate::cache::Stamp::from(read.metadata());
        if stamp == before && crate::cache::Stamp::of(path) == Some(before) {
            use sha2::Digest;
            let fingerprint = sha2::Sha256::digest(read.bytes()).into();
            return Ok(ContainerRead { baseline: Arc::new(ContainerSnapshot { snapshot, target, fingerprint }), bytes: read.into_bytes().into() });
        }
    }
    Err("the archive kept changing while it was read; try Reload".into())
}

/// Read a root or nested archive index with the same member naming and nesting policy as entry loads.
pub fn open_index(location: &Location, cancel: &dyn Fn() -> bool) -> Result<Arc<Index>, String> {
    let read = read_container(&location.container, cancel)?;
    open_index_from_bytes(location, &read.bytes, cancel)
}
pub(crate) fn open_index_from_bytes(location: &Location, bytes: &Arc<[u8]>, cancel: &dyn Fn() -> bool) -> Result<Arc<Index>, String> {
    open_index_with_capability(location, bytes, cancel).map(|(index, _)| index)
}
fn open_index_with_capability(location: &Location, bytes: &Arc<[u8]>, cancel: &dyn Fn() -> bool) -> Result<(Arc<Index>, Option<String>), String> {
    Location::new(location.container.clone(), location.members.clone())?;
    let kind = Kind::detect(&location.container.to_string_lossy(), bytes).ok_or("unsupported archive format")?;
    let mut index = Index::open(kind, bytes.clone(), Limits::default(), cancel).map_err(|e| e.to_string())?;
    let mut used = index.resident_bytes();
    let mut reason = index.rewrite_blocked().map(str::to_owned);
    if used > NESTED_BYTES { return Err("archive nesting exceeds the retained bytes limit".into()); }
    for member in &location.members {
        let entry = member_named(&index, member)?;
        if !entry.editable() { reason.get_or_insert_with(|| entry.read_only_reason.clone().unwrap_or_else(|| "an enclosing archive is read-only".into())); }
        let next = index.read_member(entry.id, cancel).map_err(|e| e.to_string())?;
        let kind = Kind::detect(member, &next).ok_or("the selected member is not an archive")?;
        drop(index);
        index = Index::open(kind, next.into(), Limits::default(), cancel).map_err(|e| e.to_string())?;
        if let Some(blocked) = index.rewrite_blocked() { reason.get_or_insert_with(|| blocked.to_owned()); }
        used = used.saturating_add(index.resident_bytes());
        if used > NESTED_BYTES { return Err("archive nesting exceeds the retained bytes limit".into()); }
    }
    Ok((Arc::new(index), reason))
}
pub(crate) fn member_named<'a>(index: &'a Index, name: &str) -> Result<&'a Member, String> {
    let mut matches = index.members().iter().filter(|member| member.kind != lenz_archive::MemberKind::Metadata && member.path.as_deref() == Some(name));
    let member = matches.next().ok_or_else(|| format!("{name} is not in the archive"))?;
    if matches.next().is_some() { return Err(format!("{name} names more than one archive member")); }
    Ok(member)
}

pub(crate) struct ReadEntry {
    pub document: crate::document::Document,
    pub baseline: Arc<Baseline>,
}
pub(crate) fn read_entry(location: &Location, version: u64, cancel: &dyn Fn() -> bool) -> Result<ReadEntry, String> {
    let requested = location.clone();
    let read = read_container(&location.container, cancel)?;
    let location = Location::new(read.baseline.target.canonical.clone(), location.members.clone())?;
    let parent = location.parent().ok_or("choose a file inside the archive")?;
    let (index, inherited) = open_index_with_capability(&parent, &read.bytes, cancel)?;
    let name = location.member_name().ok_or("choose a file inside the archive")?;
    let member = member_named(&index, name)?;
    let mut reason = inherited.or_else(|| member.read_only_reason.clone()).or_else(|| index.rewrite_blocked().map(str::to_owned));
    if !member.editable() && reason.is_none() { reason = Some("only regular archive files can be edited".into()); }
    let bytes = index.read_member(member.id, cancel).map_err(|e| e.to_string())?;
    let document = crate::document::Document::decode(PathBuf::from(name), version, &bytes);
    let crlf = document.text().matches("\r\n").count();
    let crlf = crlf > document.text().matches('\n').count() - crlf;
    let baseline = Arc::new(Baseline { location: location.clone(), container: read.baseline, crlf, encoding: document.source_meta().original_encoding(), bom: document.source_meta().had_bom() });
    let document = document.with_archive(requested, reason, Some(baseline.clone()));
    Ok(ReadEntry { document, baseline })
}
pub fn read_location(location: &Location, version: u64) -> Result<crate::document::Document, String> {
    read_entry(location, version, &|| false).map(|entry| entry.document)
}

/// Replace one or more nested leaves, rebuilding each shared ancestor once. Every update is addressed
/// against this immutable container snapshot; untouched siblings remain codec-owned original records.
pub(crate) fn replace_locations(location: &Location, bytes: &Arc<[u8]>, updates: &[(Vec<String>, Vec<u8>)], cancel: &dyn Fn() -> bool) -> Result<Vec<u8>, String> {
    let kind = Kind::detect(&location.container.to_string_lossy(), bytes).ok_or("unsupported archive format")?;
    let updates = updates.iter().map(|(members, bytes)| (members.as_slice(), bytes.as_slice())).collect::<Vec<_>>();
    rewrite(kind, bytes.clone(), &updates, 0, &mut 0, cancel)
}
fn rewrite(kind: Kind, bytes: Arc<[u8]>, updates: &[(&[String], &[u8])], depth: usize, used: &mut usize, cancel: &dyn Fn() -> bool) -> Result<Vec<u8>, String> {
    if depth >= MAX_DEPTH { return Err("archive nesting limit reached".into()); }
    let index = Index::open(kind, bytes, Limits::default(), cancel).map_err(|e| e.to_string())?;
    *used = used.saturating_add(index.resident_bytes());
    if *used > NESTED_BYTES { return Err("archive nesting exceeds the retained bytes limit".into()); }
    let mut groups = BTreeMap::<&str, Vec<(&[String], &[u8])>>::new();
    for (members, bytes) in updates {
        let first = members.first().ok_or("a replacement must name an archive member")?;
        groups.entry(first).or_default().push((&members[1..], bytes));
    }
    let mut owned = Vec::new();
    for (name, group) in groups {
        let member = member_named(&index, name)?;
        let bytes = if group.len() == 1 && group[0].0.is_empty() { Cow::Borrowed(group[0].1) }
            else {
                if group.iter().any(|(rest, _)| rest.is_empty()) { return Err("an archive and its member cannot both be replaced in one save".into()); }
                let bytes = index.read_member(member.id, cancel).map_err(|e| e.to_string())?;
                let kind = Kind::detect(name, &bytes).ok_or("the selected parent is not an archive")?;
                Cow::Owned(rewrite(kind, bytes.into(), &group, depth + 1, used, cancel)?)
            };
        owned.push((member.id, bytes));
    }
    let updates = owned.iter().map(|(id, bytes)| Replacement { id: *id, bytes }).collect::<Vec<_>>();
    index.replace_members(&updates, cancel).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn archive_locations_round_trip_non_utf8_container_names_without_path_ambiguity() {
        let path = PathBuf::from(std::ffi::OsString::from_vec(b"/tmp/a\xff !.zip".to_vec()));
        let location = Location::new(path, vec!["nested a.tar.gz".into(), "src/hello!/界.rs".into()]).unwrap();
        assert_eq!(Location::from_uri(&location.to_uri()), Some(location.clone()));
        assert_eq!(serde_json::from_str::<Location>(&serde_json::to_string(&location).unwrap()).unwrap(), location);
        for members in [vec!["../escape".into()], vec!["/absolute".into()], vec!["x//y".into()]] {
            assert!(Location::new(PathBuf::from("/tmp/a.zip"), members).is_err());
        }
    }
}
