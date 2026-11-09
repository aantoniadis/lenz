//! Archive explorer rows. Physical project inventories never contain these virtual locations.
use crate::{archive::Location, cache::Stamp, project::TreeRow};
use lenz_archive::{Index, MemberKind};
use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

const ROOTS: usize = crate::workspace::MAX_ARCHIVES;
const LISTINGS: usize = 16;
const ITEMS: usize = 100_000;
const TREE_BYTES: usize = 64 << 20;
const CACHE_ENTRIES: usize = 4;
const CACHE_BYTES: usize = 256 << 20;

fn key(location: &Location) -> Arc<str> {
    location.to_uri().to_string_lossy().as_ref().into()
}
fn child(location: &Location, path: &str) -> Location {
    let mut next = location.clone();
    next.members.push(path.to_owned());
    next
}
fn label(location: &Location) -> String {
    location
        .members
        .last()
        .and_then(|p| p.rsplit('/').next())
        .map(str::to_owned)
        .or_else(|| {
            location
                .container
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| location.display_path())
}
fn identity_location(identity: &str) -> Option<Location> {
    if let Some(rest) = identity.strip_prefix("lenz-archive-tree://") {
        let hex = rest.split('/').next()?;
        if hex.len() % 2 != 0 {
            return None;
        }
        let bytes: Option<Vec<u8>> = hex
            .as_bytes()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| {
                std::str::from_utf8(b)
                    .ok()
                    .and_then(|s| u8::from_str_radix(s, 16).ok())
            })
            .collect();
        let uri = String::from_utf8(bytes?).ok()?;
        Location::from_uri(Path::new(&uri))
    } else {
        Location::from_uri(Path::new(identity))
    }
}
fn tree_key(location: &Location, kind: &str, path: &str) -> Arc<str> {
    fn hex(bytes: &[u8]) -> String {
        const DIGITS: &[u8; 16] = b"0123456789abcdef";
        let mut result = String::with_capacity(bytes.len() * 2);
        for byte in bytes {
            result.push(DIGITS[(byte >> 4) as usize] as char);
            result.push(DIGITS[(byte & 15) as usize] as char);
        }
        result
    }
    format!(
        "lenz-archive-tree://{}/{kind}/{}",
        hex(key(location).as_bytes()),
        hex(path.as_bytes())
    )
    .into()
}
pub(crate) fn directory_key(location: &Location, path: &str) -> Arc<str> {
    tree_key(location, "directory", path)
}
fn directory_path(identity: &str) -> Option<String> {
    let mut parts = identity.strip_prefix("lenz-archive-tree://")?.split('/');
    let _location = parts.next()?;
    if parts.next()? != "directory" {
        return None;
    }
    let value = parts.next()?;
    if value.len() % 2 != 0 || parts.next().is_some() {
        return None;
    }
    let bytes: Option<Vec<u8>> = value
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| {
            std::str::from_utf8(b)
                .ok()
                .and_then(|s| u8::from_str_radix(s, 16).ok())
        })
        .collect();
    String::from_utf8(bytes?).ok()
}

/// Dirty member ownership follows virtual paths even when their directory is collapsed.
pub(crate) fn row_contains_source(row: &TreeRow, source: &Location) -> bool {
    let Some(location) = identity_location(&row.path) else { return false };
    if location.container != source.container { return false; }
    if let Some(directory) = directory_path(&row.path) {
        return source.members.starts_with(&location.members)
            && source.members.get(location.members.len()).is_some_and(|member|
                member.strip_prefix(directory.as_str()).is_some_and(|rest| rest.is_empty() || rest.starts_with('/')));
    }
    if row.dir.is_some() { source.members.starts_with(&location.members) }
    else { source.members == location.members }
}

/// Validate a persisted explorer identity without consulting or allocating from unbounded input.
pub(crate) fn valid_identity(identity: &str) -> bool {
    if identity.len() > 64 << 10 {
        return false;
    }
    let Some(location) = identity_location(identity) else {
        return false;
    };
    let Some(rest) = identity.strip_prefix("lenz-archive-tree://") else {
        return location.to_uri() == Path::new(identity);
    };
    let mut parts = rest.split('/');
    let Some(_location) = parts.next() else {
        return false;
    };
    let Some(kind) = parts.next() else {
        return false;
    };
    let Some(encoded) = parts.next() else {
        return false;
    };
    if parts.next().is_some() || encoded.len() % 2 != 0 {
        return false;
    }
    let bytes: Option<Vec<u8>> = encoded
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| {
            std::str::from_utf8(b)
                .ok()
                .and_then(|s| u8::from_str_radix(s, 16).ok())
        })
        .collect();
    let Some(path) = bytes.and_then(|bytes| String::from_utf8(bytes).ok()) else {
        return false;
    };
    let valid_path =
        |path: &str| Location::new(location.container.clone(), vec![path.to_owned()]).is_ok();
    let valid = match kind {
        "up" | "status" => path.is_empty(),
        "directory" | "depth-limit" => valid_path(&path),
        "unavailable" => path.rsplit_once(':').is_some_and(|(path, ordinal)| {
            valid_path(path)
                && ordinal
                    .parse::<usize>()
                    .is_ok_and(|value| value.to_string() == ordinal)
        }),
        _ => false,
    };
    valid && tree_key(&location, kind, &path).as_ref() == identity
}

#[derive(Clone, Debug)]
pub(crate) enum Target {
    Archive(Location),
    Directory,
    Member(Location),
    Up(Arc<str>),
    Retry(Location),
    Unavailable(String),
    Status,
}

#[derive(Clone, Debug)]
struct Entry {
    path: String,
    name: Arc<str>,
    directory: bool,
    archive: bool,
    unavailable: Option<String>,
    ordinal: usize,
}

#[derive(Default)]
struct Entries {
    children: BTreeMap<String, Vec<Entry>>,
    count: usize,
    skipped: usize,
    bytes: usize,
    identity_bytes: usize,
}
impl Entries {
    fn of(index: &Index, location: &Location) -> Result<Self, String> {
        let mut counts = HashMap::new();
        for member in index.members() {
            if matches!(
                member.kind,
                MemberKind::File | MemberKind::Symlink | MemberKind::HardLink
            ) && let Some(path) = &member.path
            {
                *counts.entry(path.as_str()).or_insert(0usize) += 1;
            }
        }
        let mut result = Self {
            identity_bytes: key(location).len(),
            ..Self::default()
        };
        let mut directories = HashSet::new();
        for member in index.members() {
            if matches!(member.kind, MemberKind::Metadata | MemberKind::Other) {
                continue;
            }
            let Some(path) = member.path.as_deref() else {
                result.skipped += 1;
                continue;
            };
            let components: Vec<_> = path.trim_end_matches('/').split('/').collect();
            if components.is_empty()
                || components.len() > 256
                || components
                    .iter()
                    .any(|s| s.is_empty() || matches!(*s, "." | ".."))
            {
                result.skipped += 1;
                continue;
            }
            let mut parent = String::new();
            let dirs = if member.kind == MemberKind::Directory {
                components.len()
            } else {
                components.len() - 1
            };
            for component in components.iter().take(dirs) {
                let path = if parent.is_empty() {
                    (*component).to_owned()
                } else {
                    format!("{parent}/{component}")
                };
                if directories.insert(path.clone()) {
                    result.push(
                        &parent,
                        Entry {
                            path: path.clone(),
                            name: (*component).into(),
                            directory: true,
                            archive: false,
                            unavailable: None,
                            ordinal: 0,
                        },
                    )?;
                }
                parent = path;
            }
            if member.kind != MemberKind::Directory {
                // A persistent URI names a path, so duplicate physical members cannot be opened ambiguously.
                let unavailable = (counts.get(path).copied().unwrap_or(0) > 1)
                    .then(|| "Several archive members have this path; this entry cannot be opened unambiguously".into());
                let name = components.last().copied().unwrap_or_default();
                result.push(
                    &parent,
                    Entry {
                        path: path.to_owned(),
                        name: name.into(),
                        directory: false,
                        archive: member.kind == MemberKind::File
                            && lenz_archive::Kind::of(path).is_some(),
                        unavailable,
                        ordinal: member.id.ordinal(),
                    },
                )?;
            }
        }
        for entries in result.children.values_mut() {
            entries.sort_by(|a, b| {
                b.directory
                    .cmp(&a.directory)
                    .then(a.name.cmp(&b.name))
                    .then(a.path.cmp(&b.path))
            });
        }
        Ok(result)
    }
    fn push(&mut self, parent: &str, entry: Entry) -> Result<(), String> {
        if self.count == ITEMS {
            return Err(format!("Archive tree exceeds {ITEMS} entries"));
        }
        // Includes the row URI, target location, parent identity, metadata strings and map/row overhead.
        let bytes = self
            .identity_bytes
            .saturating_add(parent.len())
            .saturating_add(entry.path.len())
            .saturating_add(entry.name.len())
            .saturating_mul(8)
            .saturating_add(512);
        self.bytes = self.bytes.saturating_add(bytes);
        if self.bytes > TREE_BYTES {
            return Err("Archive tree exceeds the 64 MiB metadata budget".into());
        }
        self.children
            .entry(parent.to_owned())
            .or_default()
            .push(entry);
        self.count += 1;
        Ok(())
    }
}

enum Status {
    Loading(u64),
    Ready(Entries),
    Error(String),
}
struct Listing {
    location: Location,
    status: Status,
}

/// A bounded overlay over the normal project and library trees. Rows hold metadata, never member contents.
#[derive(Default)]
pub struct Tree {
    roots: Vec<Location>,
    listings: HashMap<Arc<str>, Listing>,
    expanded: HashSet<Arc<str>>,
    rows: Vec<TreeRow>,
    targets: HashMap<Arc<str>, Target>,
    parents: HashMap<Arc<str>, Arc<str>>,
    requested_reveal: Option<Arc<str>>,
}
impl Tree {
    pub fn rows(&self) -> &[TreeRow] {
        &self.rows
    }
    pub fn roots(&self) -> &[Location] {
        &self.roots
    }
    pub fn title(&self) -> Option<String> {
        self.roots.last().map(label)
    }
    pub fn display_path(&self, path: &str) -> Option<String> {
        identity_location(path).map(|l| match directory_path(path) {
            Some(dir) => format!("{}!/{dir}", l.display_path()),
            None => l.display_path(),
        })
    }
    pub fn is_virtual(&self, path: &str) -> bool {
        self.targets.contains_key(path)
    }
    pub fn is_member(&self, path: &str) -> bool {
        matches!(self.targets.get(path), Some(Target::Member(_)))
    }
    pub fn is_archive(&self, path: &str) -> bool {
        matches!(self.targets.get(path), Some(Target::Archive(_)))
    }
    pub fn parent(&self, path: &str) -> Option<Arc<str>> {
        self.parents.get(path).cloned()
    }
    pub fn hint(&self, path: &str) -> Option<&'static str> {
        Some(match self.targets.get(path)? {
            Target::Archive(_) | Target::Directory => "Enter expands or collapses · Left goes up",
            Target::Member(_) => "Enter opens · Left goes up",
            Target::Retry(_) => "Enter retries · Left goes up",
            Target::Up(_) => "Enter goes up or closes archive",
            Target::Unavailable(_) => "Entry unavailable · Enter explains · Left goes up",
            Target::Status => "Left goes up",
        })
    }
    /// Cached immediate children, including those inside a currently collapsed archive directory.
    /// Navigation menus never consult the filesystem for an opaque archive path.
    pub fn children(&self, path: &str) -> Vec<TreeRow> {
        let Some(location) = identity_location(path) else {
            return Vec::new();
        };
        let directory = directory_path(path).unwrap_or_default();
        let Some(Status::Ready(entries)) = self.listings.get(&key(&location)).map(|l| &l.status)
        else {
            return Vec::new();
        };
        entries
            .children
            .get(&directory)
            .into_iter()
            .flatten()
            .filter_map(|entry| {
                let (row, target) = self.entry_row(&location, entry, 0);
                (!matches!(target, Target::Unavailable(_))).then_some(row)
            })
            .collect()
    }
    pub(crate) fn target(&self, path: &str) -> Option<Target> {
        self.targets.get(path).cloned()
    }
    pub(crate) fn expanded(&self, path: &str) -> bool {
        self.expanded.contains(path)
    }
    pub(crate) fn contains_source(&self, path: &Path) -> bool {
        path.to_str().is_some_and(|p| self.known(p))
    }
    fn known(&self, path: &str) -> bool {
        if self.targets.contains_key(path) {
            return true;
        }
        let Some(location) = identity_location(path) else {
            return false;
        };
        if !self.owns(&location) {
            return false;
        }
        if let Some(directory) = directory_path(path) {
            return self
                .listings
                .get(&key(&location))
                .is_some_and(|l| match &l.status {
                    Status::Ready(entries) => entries
                        .children
                        .values()
                        .flatten()
                        .any(|e| e.directory && e.path == directory),
                    _ => false,
                });
        }
        if self.listings.contains_key(&key(&location)) {
            return true;
        }
        let Some(member) = location.members.last() else {
            return false;
        };
        let parent = Location {
            container: location.container.clone(),
            members: location.members[..location.members.len() - 1].to_vec(),
        };
        self.listings
            .get(&key(&parent))
            .is_some_and(|l| match &l.status {
                Status::Ready(entries) => entries
                    .children
                    .values()
                    .flatten()
                    .any(|e| !e.directory && e.path == *member && e.unavailable.is_none()),
                _ => false,
            })
    }
    /// Reveal a known member/directory through every collapsed virtual ancestor.
    pub(crate) fn reveal(&mut self, path: &str) -> bool {
        if !self.known(path) {
            return false;
        }
        let Some(mut location) = identity_location(path) else {
            return false;
        };
        let directory = if let Some(directory) = directory_path(path) {
            Some(directory)
        } else if !self.listings.contains_key(&key(&location)) {
            let member = location.members.pop().unwrap_or_default();
            member.rsplit_once('/').map(|(dir, _)| dir.to_owned())
        } else {
            None
        };
        for n in 0..=location.members.len() {
            let ancestor = Location {
                container: location.container.clone(),
                members: location.members[..n].to_vec(),
            };
            self.expanded.insert(key(&ancestor));
            if let Some(member) = location.members.get(n)
                && let Some((dir, _)) = member.rsplit_once('/')
            {
                self.expand_directories(&ancestor, dir);
            }
        }
        if let Some(directory) = directory {
            self.expand_directories(&location, &directory);
        }
        self.rebuild();
        true
    }
    fn expand_directories(&mut self, location: &Location, directory: &str) {
        let mut prefix = String::new();
        for part in directory.split('/') {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(part);
            self.expanded.insert(directory_key(location, &prefix));
        }
    }
    pub(crate) fn open(&mut self, location: Location) -> Arc<str> {
        let identity = key(&location);
        if !self.owns(&location) {
            // Opening an outer archive subsumes standalone roots previously opened inside it.
            self.roots.retain(|l| {
                !(l.container == location.container && l.members.starts_with(&location.members))
            });
            if self.roots.len() == ROOTS {
                let removed = self.roots.remove(0);
                self.remove_container_root(&removed);
            }
            self.roots.push(location.clone());
        }
        for n in 0..=location.members.len() {
            let ancestor = Location {
                container: location.container.clone(),
                members: location.members[..n].to_vec(),
            };
            self.expanded.insert(key(&ancestor));
            if let Some(member) = location.members.get(n)
                && let Some((directory, _)) = member.rsplit_once('/')
            {
                self.expand_directories(&ancestor, directory);
            }
        }
        self.expanded.insert(identity.clone());
        self.requested_reveal = Some(identity.clone());
        self.rebuild();
        identity
    }
    pub(crate) fn take_reveal(&mut self) -> Option<Arc<str>> {
        let path = self.requested_reveal.clone()?;
        if !self.targets.contains_key(&path) {
            self.reveal(&path);
        }
        if self.targets.contains_key(&path) {
            self.requested_reveal.take()
        } else {
            None
        }
    }
    pub(crate) fn request_reveal(&mut self, path: Arc<str>) {
        self.requested_reveal = Some(path);
    }
    pub(crate) fn clear_reveal(&mut self) {
        self.requested_reveal = None;
    }
    pub(crate) fn set_expanded(&mut self, path: Arc<str>, expanded: bool) {
        if expanded {
            self.expanded.insert(path);
        } else {
            self.expanded.remove(&path);
        }
        self.rebuild();
    }
    pub(crate) fn needs_listing(&self, location: &Location) -> bool {
        !matches!(
            self.listings.get(&key(location)).map(|l| &l.status),
            Some(Status::Ready(_))
        )
    }
    fn begin(&mut self, location: Location, request: u64) -> bool {
        let identity = key(&location);
        let already_active = self
            .listings
            .get(&identity)
            .is_some_and(|l| !matches!(l.status, Status::Error(_)));
        if self
            .listings
            .values()
            .filter(|l| !matches!(l.status, Status::Error(_)))
            .count()
            >= LISTINGS
            && !already_active
        {
            // Collapsed nested indexes are reloadable; visible roots stay represented.
            let victim = self
                .listings
                .iter()
                .find(|(k, l)| !self.expanded.contains(*k) && !matches!(l.status, Status::Error(_)))
                .map(|(k, _)| k.clone());
            if let Some(victim) = victim {
                self.listings.remove(&victim);
            } else {
                self.listings.insert(
                    identity,
                    Listing {
                        location,
                        status: Status::Error(
                            "Close an archive before expanding another one (16 listing limit)"
                                .into(),
                        ),
                    },
                );
                self.rebuild();
                return false;
            }
        }
        self.listings.insert(
            identity,
            Listing {
                location,
                status: Status::Loading(request),
            },
        );
        self.rebuild();
        true
    }
    fn apply(&mut self, location: &Location, request: u64, result: Result<&Index, &str>) -> bool {
        let identity = key(location);
        if !matches!(self.listings.get(&identity).map(|l| &l.status), Some(Status::Loading(current)) if *current == request)
        {
            return false;
        }
        let status = match result {
            Err(error) => Status::Error(error.to_owned()),
            Ok(index) => match Entries::of(index, location) {
                Ok(entries) => {
                    self.prune_children(location, &entries);
                    let others = self
                        .listings
                        .iter()
                        .filter(|(k, _)| **k != identity)
                        .filter_map(|(_, l)| match &l.status {
                            Status::Ready(e) => Some(e),
                            _ => None,
                        });
                    let (count, bytes) = others.fold((0usize, 0usize), |(count, bytes), e| {
                        (count.saturating_add(e.count), bytes.saturating_add(e.bytes))
                    });
                    if count.saturating_add(entries.count) > ITEMS
                        || bytes.saturating_add(entries.bytes) > TREE_BYTES
                    {
                        Status::Error(format!(
                            "Close another archive before listing this one ({ITEMS} entries / 64 MiB total tree limit)"
                        ))
                    } else {
                        Status::Ready(entries)
                    }
                }
                Err(error) => Status::Error(error),
            },
        };
        if let Some(listing) = self.listings.get_mut(&identity) {
            listing.status = status;
        }
        self.rebuild();
        true
    }
    fn prune_children(&mut self, location: &Location, entries: &Entries) {
        let archives: HashSet<_> = entries
            .children
            .values()
            .flatten()
            .filter(|e| e.archive && e.unavailable.is_none())
            .map(|e| e.path.as_str())
            .collect();
        let directories: HashSet<_> = entries
            .children
            .values()
            .flatten()
            .filter(|e| e.directory)
            .map(|e| e.path.as_str())
            .collect();
        let prefix = location.members.len();
        let survives = |nested: &Location| {
            nested.container != location.container
                || !nested.members.starts_with(&location.members)
                || nested
                    .members
                    .get(prefix)
                    .is_none_or(|member| archives.contains(member.as_str()))
        };
        self.listings
            .retain(|_, listing| survives(&listing.location));
        self.expanded.retain(|identity| {
            let Some(nested) = identity_location(identity) else {
                return false;
            };
            if !survives(&nested) {
                return false;
            }
            nested != *location
                || directory_path(identity).is_none_or(|dir| directories.contains(dir.as_str()))
        });
    }
    pub(crate) fn invalidate(&mut self, container: &Path) -> Vec<Location> {
        let mut refresh = Vec::new();
        self.listings.retain(|identity, listing| {
            if listing.location.container != container {
                return true;
            }
            if self.expanded.contains(identity) {
                refresh.push(listing.location.clone());
            }
            false
        });
        self.rebuild();
        refresh
    }
    fn remove_container_root(&mut self, root: &Location) {
        self.listings.retain(|_, listing| {
            !(listing.location.container == root.container
                && listing.location.members.starts_with(&root.members))
        });
        self.expanded.retain(|identity| {
            identity_location(identity).is_none_or(|l| {
                !(l.container == root.container && l.members.starts_with(&root.members))
            })
        });
    }
    pub(crate) fn close_root(&mut self, identity: &str) {
        if let Some(at) = self.roots.iter().position(|l| key(l).as_ref() == identity) {
            let root = self.roots.remove(at);
            self.remove_container_root(&root);
            self.rebuild();
        }
    }
    fn owns(&self, location: &Location) -> bool {
        self.roots
            .iter()
            .any(|l| l.container == location.container && location.members.starts_with(&l.members))
    }
    fn row(
        &mut self,
        depth: usize,
        name: Arc<str>,
        path: Arc<str>,
        dir: Option<bool>,
        target: Target,
        parent: Option<&Arc<str>>,
    ) {
        if let Some(parent) = parent {
            self.parents.insert(path.clone(), parent.clone());
        }
        self.targets.insert(path.clone(), target);
        self.rows.push(TreeRow {
            depth,
            name,
            path,
            dir,
        });
    }
    fn rebuild(&mut self) {
        self.rows.clear();
        self.targets.clear();
        self.parents.clear();
        let roots = self.roots.clone();
        for location in roots {
            let identity = key(&location);
            let name: Arc<str> = label(&location).into();
            self.row(
                0,
                name,
                identity.clone(),
                Some(self.expanded.contains(&identity)),
                Target::Archive(location.clone()),
                None,
            );
            self.archive_rows(&location, 1, &identity);
        }
    }
    fn archive_rows(&mut self, location: &Location, depth: usize, identity: &Arc<str>) {
        if !self.expanded.contains(identity) {
            return;
        }
        self.row(
            depth,
            "↑ Up / close archive".into(),
            tree_key(location, "up", ""),
            None,
            Target::Up(identity.clone()),
            Some(identity),
        );
        let status = self.listings.get(identity).map(|l| &l.status);
        match status {
            None | Some(Status::Loading(_)) => self.row(
                depth,
                "Loading archive…".into(),
                tree_key(location, "status", ""),
                None,
                Target::Status,
                Some(identity),
            ),
            Some(Status::Error(error)) => {
                let label = format!("Cannot open archive: {error} · Enter to retry");
                self.row(
                    depth,
                    label.into(),
                    tree_key(location, "status", ""),
                    None,
                    Target::Retry(location.clone()),
                    Some(identity),
                );
            }
            Some(Status::Ready(entries)) if entries.count == 0 => {
                let label = if entries.skipped == 0 {
                    "(Empty archive)".to_owned()
                } else {
                    format!(
                        "(No browseable entries; {} unsafe paths skipped)",
                        entries.skipped
                    )
                };
                self.row(
                    depth,
                    label.into(),
                    tree_key(location, "status", ""),
                    None,
                    Target::Status,
                    Some(identity),
                );
            }
            Some(Status::Ready(_)) => self.directory_rows(location, "", depth, identity),
        }
    }
    fn directory_rows(
        &mut self,
        location: &Location,
        directory: &str,
        depth: usize,
        parent: &Arc<str>,
    ) {
        let entries = self
            .listings
            .get(&key(location))
            .and_then(|l| match &l.status {
                Status::Ready(e) => e.children.get(directory).cloned(),
                _ => None,
            })
            .unwrap_or_default();
        for entry in entries {
            let (row, target) = self.entry_row(location, &entry, depth);
            let identity = row.path.clone();
            self.row(
                row.depth,
                row.name,
                row.path,
                row.dir,
                target.clone(),
                Some(parent),
            );
            match target {
                Target::Directory if self.expanded.contains(&identity) => {
                    self.directory_rows(location, &entry.path, depth + 1, &identity)
                }
                Target::Archive(next) => self.archive_rows(&next, depth + 1, &identity),
                _ => {}
            }
        }
    }
    fn entry_row(&self, location: &Location, entry: &Entry, depth: usize) -> (TreeRow, Target) {
        let (name, identity, dir, target) = if entry.directory {
            let identity = directory_key(location, &entry.path);
            (
                entry.name.clone(),
                identity.clone(),
                Some(self.expanded.contains(&identity)),
                Target::Directory,
            )
        } else if let Some(reason) = &entry.unavailable {
            (
                format!("{} (ambiguous duplicate)", entry.name).into(),
                tree_key(
                    location,
                    "unavailable",
                    &format!("{}:{}", entry.path, entry.ordinal),
                ),
                None,
                Target::Unavailable(reason.clone()),
            )
        } else {
            let next = child(location, &entry.path);
            let identity = key(&next);
            if next.members.len() > crate::archive::MAX_DEPTH
                || (entry.archive && next.members.len() == crate::archive::MAX_DEPTH)
            {
                (
                    format!("{} (nesting limit)", entry.name).into(),
                    tree_key(location, "depth-limit", &entry.path),
                    None,
                    Target::Unavailable("Archive nesting limit reached".into()),
                )
            } else if entry.archive {
                (
                    entry.name.clone(),
                    identity.clone(),
                    Some(self.expanded.contains(&identity)),
                    Target::Archive(next),
                )
            } else {
                (entry.name.clone(), identity, None, Target::Member(next))
            }
        };
        (
            TreeRow {
                depth,
                name,
                path: identity,
                dir,
            },
            target,
        )
    }
}

struct Flight {
    request: u64,
    current: Arc<AtomicBool>,
    location: Location,
}
struct Cached {
    location: Location,
    stamp: Stamp,
    index: Arc<Index>,
}

/// Listing coordination is separate from the row metadata and from member document ownership.
#[derive(Default)]
pub(crate) struct Browser {
    next: u64,
    flights: HashMap<Arc<str>, Flight>,
    cache: VecDeque<Cached>,
}
pub(crate) struct Request {
    location: Location,
    request: u64,
    current: Arc<AtomicBool>,
    cached: Option<(Stamp, Arc<Index>)>,
}
pub struct Listed {
    location: Location,
    request: u64,
    stamp: Option<Stamp>,
    result: Result<Arc<Index>, String>,
}
impl Request {
    pub(crate) fn read(self) -> Listed {
        let wanted = || self.current.load(Ordering::Acquire);
        let stamp = Stamp::of(&self.location.container);
        let result = if !wanted() {
            Err("Archive listing cancelled".into())
        } else if let Some((_, index)) = self.cached.filter(|(s, _)| Some(*s) == stamp) {
            Ok(index)
        } else {
            crate::archive::open_index(&self.location, &|| !wanted())
        };
        let result = if !wanted() {
            Err("Archive listing cancelled".into())
        } else if stamp != Stamp::of(&self.location.container) {
            Err("Archive changed while it was being listed; Enter to retry".into())
        } else {
            result
        };
        Listed {
            location: self.location,
            request: self.request,
            stamp,
            result,
        }
    }
}
impl Browser {
    pub(crate) fn request(&mut self, tree: &mut Tree, location: Location) -> Option<Request> {
        self.flights.retain(|_, f| {
            if tree.owns(&f.location) {
                true
            } else {
                f.current.store(false, Ordering::Release);
                false
            }
        });
        let identity = key(&location);
        if self.flights.contains_key(&identity) {
            return None;
        }
        self.next = self.next.wrapping_add(1);
        let request = self.next;
        let current = Arc::new(AtomicBool::new(true));
        let cached = self
            .cache
            .iter()
            .position(|c| key(&c.location) == identity)
            .and_then(|at| self.cache.remove(at));
        let cached = cached.map(|c| (c.stamp, c.index));
        if !tree.begin(location.clone(), request) {
            return None;
        }
        self.flights.insert(
            identity,
            Flight {
                request,
                current: current.clone(),
                location: location.clone(),
            },
        );
        Some(Request {
            location,
            request,
            current,
            cached,
        })
    }
    pub(crate) fn cancel_branch(&mut self, location: &Location) {
        self.flights.retain(|_, f| {
            if f.location.container == location.container
                && f.location.members.starts_with(&location.members)
            {
                f.current.store(false, Ordering::Release);
                false
            } else {
                true
            }
        });
    }
    pub(crate) fn invalidate(&mut self, container: &Path) {
        self.cache.retain(|c| c.location.container != container);
        self.flights.retain(|_, f| {
            if f.location.container == container {
                f.current.store(false, Ordering::Release);
                false
            } else {
                true
            }
        });
    }
    pub(crate) fn apply(&mut self, tree: &mut Tree, mut listed: Listed) -> bool {
        let identity = key(&listed.location);
        if !self
            .flights
            .get(&identity)
            .is_some_and(|f| f.request == listed.request && f.current.load(Ordering::Acquire))
        {
            return false;
        }
        self.flights.remove(&identity);
        if listed.stamp != Stamp::of(&listed.location.container) {
            listed.result = Err("Archive changed after listing; Enter to retry".into());
        }
        let changed = tree.apply(
            &listed.location,
            listed.request,
            listed.result.as_deref().map_err(String::as_str),
        );
        self.flights.retain(|identity, flight| {
            if matches!(tree.listings.get(identity).map(|l| &l.status), Some(Status::Loading(request)) if *request == flight.request) { true }
            else { flight.current.store(false, Ordering::Release); false }
        });
        if changed
            && let (Some(stamp), Ok(index)) = (listed.stamp, listed.result)
            && index.resident_bytes() <= CACHE_BYTES
        {
            self.cache.push_back(Cached {
                location: listed.location,
                stamp,
                index,
            });
            while self.cache.len() > CACHE_ENTRIES
                || self
                    .cache
                    .iter()
                    .map(|c| c.index.resident_bytes())
                    .sum::<usize>()
                    > CACHE_BYTES
            {
                self.cache.pop_front();
            }
        }
        changed
    }
}
impl Drop for Browser {
    fn drop(&mut self) {
        for flight in self.flights.values() {
            flight.current.store(false, Ordering::Release);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Small USTAR fixtures keep these explorer tests independent of archive writing APIs.
    fn tar(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut bytes = Vec::new();
        for (name, payload) in entries {
            assert!(name.len() < 100);
            let mut header = [0u8; 512];
            header[..name.len()].copy_from_slice(name.as_bytes());
            header[100..108].copy_from_slice(b"0000644\0");
            header[108..116].copy_from_slice(b"0000000\0");
            header[116..124].copy_from_slice(b"0000000\0");
            header[124..136].copy_from_slice(format!("{:011o}\0", payload.len()).as_bytes());
            header[136..148].copy_from_slice(b"00000000000\0");
            header[148..156].fill(b' ');
            header[156] = if name.ends_with('/') { b'5' } else { b'0' };
            header[257..263].copy_from_slice(b"ustar\0");
            header[263..265].copy_from_slice(b"00");
            let sum: u64 = header.iter().map(|b| u64::from(*b)).sum();
            header[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
            bytes.extend_from_slice(&header);
            bytes.extend_from_slice(payload);
            bytes.resize(bytes.len().next_multiple_of(512), 0);
        }
        bytes.resize(bytes.len() + 1024, 0);
        bytes
    }
    fn location(path: &Path) -> Location {
        Location::new(path.to_owned(), Vec::new()).unwrap()
    }
    fn list(browser: &mut Browser, tree: &mut Tree, location: Location) {
        let request = browser.request(tree, location).unwrap();
        assert!(browser.apply(tree, request.read()));
    }

    #[test]
    fn persisted_tree_identities_validate_every_segment_with_a_bounded_parser() {
        let location = Location::new("/tmp/bundle.zip".into(), vec!["nested.tar".into()]).unwrap();
        let directory = directory_key(&location, "src/ exact folder ");
        assert!(valid_identity(&directory));
        assert!(valid_identity(location.to_uri().to_str().unwrap()));
        assert!(valid_identity(&tree_key(&location, "up", "")));
        for invalid in [
            format!("{directory}/ignored"),
            tree_key(&location, "unknown", "src").to_string(),
            tree_key(&location, "directory", "../outside").to_string(),
            tree_key(&location, "status", "unexpected").to_string(),
            tree_key(&location, "unavailable", "src/file:NaN").to_string(),
            format!("lenz-archive-tree://{}", "a".repeat(65 << 10)),
        ] {
            assert!(!valid_identity(&invalid), "{invalid}");
        }
        let mut tree = Tree::default();
        tree.open(location.clone());
        tree.clear_reveal();
        assert!(tree.take_reveal().is_none());
    }

    #[test]
    fn archive_tree_reads_nested_members_and_reveals_collapsed_directories_without_extraction() {
        let dir = tempfile::tempdir().unwrap();
        let inner = tar(&[("nested/é name.txt", b"nested text"), ("empty/", b"")]);
        let outer = dir.path().join("bundle.tar");
        std::fs::write(
            &outer,
            tar(&[
                ("src/ keep.txt ", b"exact bytes"),
                ("inner.tar", &inner),
                ("../escape", b"never extracted"),
            ]),
        )
        .unwrap();
        let root = location(&outer);
        let mut tree = Tree::default();
        let mut browser = Browser::default();
        tree.open(root.clone());
        list(&mut browser, &mut tree, root.clone());
        assert_eq!(tree.title().as_deref(), Some("bundle.tar"));
        let src = directory_key(&root, "src");
        assert_eq!(
            tree.children(&src)
                .iter()
                .map(|r| r.name.as_ref())
                .collect::<Vec<_>>(),
            [" keep.txt "]
        );
        let file = child(&root, "src/ keep.txt ");
        assert!(tree.contains_source(&file.to_uri()));
        assert!(!tree.rows().iter().any(|r| r.name.as_ref() == " keep.txt "));
        assert!(tree.reveal(file.to_uri().to_str().unwrap()));
        let row = tree
            .rows()
            .iter()
            .find(|r| r.name.as_ref() == " keep.txt ")
            .unwrap();
        assert_eq!(Location::from_uri(Path::new(row.path.as_ref())), Some(file));
        assert_eq!(tree.parent(&row.path), Some(src));
        let nested = child(&root, "inner.tar");
        let nested_key = key(&nested);
        tree.set_expanded(nested_key.clone(), true);
        list(&mut browser, &mut tree, nested.clone());
        let nested_file = child(&nested, "nested/é name.txt");
        assert!(tree.reveal(nested_file.to_uri().to_str().unwrap()));
        let row = tree
            .rows()
            .iter()
            .find(|r| r.name.as_ref() == "é name.txt")
            .unwrap();
        assert!(matches!(tree.target(&row.path), Some(Target::Member(ref l)) if l == &nested_file));
        assert_eq!(
            tree.children(&nested_key)
                .iter()
                .map(|r| r.name.as_ref())
                .collect::<Vec<_>>(),
            ["empty", "nested"]
        );
        assert!(!dir.path().join("src").exists());
        assert!(!dir.path().join("nested").exists());
    }

    #[test]
    fn archive_listing_cancel_and_retry_reject_old_results_and_keep_error_actions_visible() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("retry.tar");
        std::fs::write(&path, b"bad archive").unwrap();
        let root = location(&path);
        let mut tree = Tree::default();
        let mut browser = Browser::default();
        tree.open(root.clone());
        let old = browser.request(&mut tree, root.clone()).unwrap();
        browser.cancel_branch(&root);
        std::fs::write(&path, tar(&[("current.txt", b"current")])).unwrap();
        let fresh = browser.request(&mut tree, root.clone()).unwrap();
        assert!(!browser.apply(&mut tree, old.read()));
        assert!(
            tree.rows()
                .iter()
                .any(|r| r.name.as_ref() == "Loading archive…")
        );
        assert!(browser.apply(&mut tree, fresh.read()));
        assert!(tree.rows().iter().any(|r| r.name.as_ref() == "current.txt"));
        browser.invalidate(&path);
        tree.invalidate(&path);
        std::fs::write(&path, b"still invalid").unwrap();
        list(&mut browser, &mut tree, root.clone());
        let error = tree
            .rows()
            .iter()
            .find(|r| r.name.contains("Cannot open archive"))
            .unwrap();
        assert!(matches!(tree.target(&error.path), Some(Target::Retry(_))));
        assert_eq!(tree.hint(&error.path), Some("Enter retries · Left goes up"));
        assert!(
            tree.rows()
                .iter()
                .any(|r| matches!(tree.target(&r.path), Some(Target::Up(_))))
        );
        browser.invalidate(&path);
        tree.invalidate(&path);
        std::fs::write(&path, tar(&[])).unwrap();
        list(&mut browser, &mut tree, root);
        assert!(
            tree.rows()
                .iter()
                .any(|r| r.name.as_ref() == "(Empty archive)")
        );
    }

    #[test]
    fn duplicate_member_paths_are_disabled_and_cache_and_overlay_roots_stay_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let mut tree = Tree::default();
        let mut browser = Browser::default();
        for i in 0..12 {
            let path = dir.path().join(format!("bundle-{i}.tar"));
            std::fs::write(&path, tar(&[("same", b"one"), ("same", b"two")])).unwrap();
            let root = location(&path);
            tree.open(root.clone());
            list(&mut browser, &mut tree, root.clone());
            let duplicates: Vec<_> = tree
                .rows()
                .iter()
                .filter(|r| matches!(tree.target(&r.path), Some(Target::Unavailable(_))))
                .collect();
            assert!(duplicates.len() >= 2);
            assert!(
                tree.children(&key(&root)).is_empty(),
                "ambiguous rows never become navigable sibling targets"
            );
            assert!(browser.cache.len() <= CACHE_ENTRIES);
            assert!(
                browser
                    .cache
                    .iter()
                    .map(|c| c.index.resident_bytes())
                    .sum::<usize>()
                    <= CACHE_BYTES
            );
            assert!(tree.roots.len() <= ROOTS);
            assert!(tree.listings.len() <= ROOTS);
        }
        let root = key(tree.roots.last().unwrap());
        tree.close_root(&root);
        assert!(!tree.is_virtual(&root));
    }
}
