//! Change Sets: list the Comparisons to show and read each Side's bytes, from git or from plain files.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fs,
    ops::Range,
    path::{Path, PathBuf},
    sync::{Mutex, PoisonError},
};

use crate::{
    archive,
    git::{self, CatFile},
};

/// What to load, as the command line asked for it.
#[derive(Debug, Clone, PartialEq)]
pub enum Spec {
    /// No repository or files; used by the standalone About launcher.
    Empty,
    /// `git diff [--cached] [<rev>...] [-- <path>...]`; `untracked` adds untracked files as added.
    Git { cached: bool, revs: Vec<String>, paths: Vec<String>, untracked: bool },
    /// Two files, or two directories paired by relative path; relative paths resolve against the cwd.
    Plain { left: PathBuf, right: PathBuf },
}

/// Is the right Side the working tree for `git diff [--cached] <revs>`?
pub fn right_is_worktree(cached: bool, revs: &[String]) -> bool {
    !cached && (revs.is_empty() || revs.len() == 1 && !revs[0].contains(".."))
}

#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    Added,
    Deleted,
    Modified,
    /// `old_path` holds where it came from; plain directories only pair identical files (100 %).
    Renamed { similarity: u8 },
    /// Same content, different file mode (e.g. `0o100644` → `0o100755`).
    ModeChanged { old: u32, new: u32 },
    /// Unmerged: the working tree (with conflict markers) against HEAD.
    Conflicted,
    /// Not tracked by git; shown as added.
    Untracked,
}

/// Where a Side's bytes come from.
#[derive(Debug, Clone, PartialEq)]
pub enum Content {
    /// The Side doesn't exist (added or deleted file).
    Empty,
    /// A git object name for `cat-file`: a blob sha, or `HEAD:<path>` (missing → empty).
    Blob(String),
    /// A file on disk, symlinks followed.
    File(PathBuf),
    /// A working-tree symlink in a repo: its target is the content, as git sees it.
    Symlink(PathBuf),
    /// A submodule at a commit; shown as `Subproject commit <sha>`, as `git difftool` does.
    Submodule(String),
    /// A file inside a zip or tar archive, read in memory from the archive's own Content: read-only.
    Archived { archive: Box<Content>, kind: archive::Kind, member: String },
}

/// One pairing of a left and a right version of a file.
#[derive(Debug, Clone, PartialEq)]
pub struct Comparison {
    /// The left Side's path; differs from `path` for renames and two plain files.
    pub old_path: String,
    /// The right Side's path (a deleted file's old path). Repo-root relative for git.
    pub path: String,
    pub status: Status,
    pub left: Content,
    pub right: Content,
}

impl Comparison {
    /// Is this a file inside an archive (see [`crate::archive`])?
    pub fn in_archive(&self) -> bool {
        matches!(self.left, Content::Archived { .. }) || matches!(self.right, Content::Archived { .. })
    }

    /// The outermost archive's folder in `path`, for a file inside an archive: where its rows sit.
    pub fn archive_root(&self) -> Option<&str> {
        let mut content = if matches!(self.right, Content::Empty) { &self.left } else { &self.right };
        let mut path = self.path.as_str();
        let mut root = None;
        while let Content::Archived { archive, member, .. } = content {
            path = path.strip_suffix(member.as_str()).and_then(|p| p.strip_suffix('/'))?;
            root = Some(path);
            content = archive;
        }
        root
    }

    /// The repository path git knows: the outermost archive's path for a file inside an archive, else `path`.
    pub fn repository_path(&self) -> &str {
        let mut content = if matches!(self.right, Content::Empty) { &self.left } else { &self.right };
        let mut path = if matches!(self.right, Content::Empty) { self.old_path.as_str() } else { self.path.as_str() };
        while let Content::Archived { archive, member, .. } = content {
            path = path.strip_suffix(member.as_str()).and_then(|p| p.strip_suffix('/')).unwrap_or(path);
            content = archive;
        }
        path
    }
}

/// The Comparisons opened at once, in File Navigator order (directory, then file name; within each group).
#[derive(Clone)]
pub struct ChangeSet {
    pub comparisons: Vec<Comparison>,
    /// Named runs of `comparisons` (e.g. Staged, Unstaged), none empty, in order; empty when ungrouped.
    pub groups: Vec<(String, Range<usize>)>,
    source: Source,
    /// Each listed path's `diff` attribute; paths without one aren't here.
    attrs: HashMap<String, DiffAttr>,
    /// Each listed path's `working-tree-encoding`, where set.
    encodings: HashMap<String, String>,
    /// The last compressed tar streams read whole, decompressed, so reading their members one by one doesn't
    /// decompress them again; see [`ChangeSet::unpacked`].
    unpacked: std::sync::Arc<Mutex<UnpackedCache>>,
    /// Why a path may not look as `git diff` shows it: a repository-config textconv or filter driver Lenz did not
    /// run, or an archive Lenz could not show as a folder.
    notes: HashMap<String, String>,
}

/// A compressed tar archive's Content and its decompressed stream.
type Unpacked = (Content, std::sync::Arc<[u8]>);

#[derive(Default)]
struct UnpackedCache {
    generation: u64,
    streams: Vec<Unpacked>,
}

/// The adapters behind a Change Set.
#[derive(Clone)]
enum Source {
    Git {
        root: PathBuf,
        /// Long-lived `git cat-file --batch` and `--batch-check`, each started on first use.
        cat_file: std::sync::Arc<Mutex<Option<CatFile>>>,
        check: std::sync::Arc<Mutex<Option<CatFile>>>,
    },
    Plain,
}

/// What a path's gitattributes (and its diff driver's config) say about showing it.
#[derive(Debug, Clone, PartialEq)]
enum DiffAttr {
    /// `diff` set, or a driver with `binary = false`: text even with a NUL.
    Text,
    /// `-diff` or `binary`, or a driver with `binary = true`.
    Binary,
    /// A driver with `textconv = <command>`: the command's output is diffed as text.
    Textconv(String),
}

/// One Side read for display by [`ChangeSet::sides`].
#[derive(Debug, Clone, PartialEq)]
pub struct SideBytes {
    pub bytes: Vec<u8>,
    /// As git judges it: the `diff` / `binary` attributes, else a NUL in the first 8000 bytes (a UTF-16 BOM aside).
    pub binary: bool,
    /// `bytes` are the output of the path's textconv driver, not what is stored.
    pub textconv: bool,
    /// The `working-tree-encoding` of a working-tree Side (git stores blobs as UTF-8).
    pub encoding: Option<String>,
}

/// Stable comparison identity within a refreshed selection; groups distinguish staged/unstaged.
pub(crate) fn comparison_identity(set:&ChangeSet,index:usize)->Option<(&str,&str,&str)>{let c=set.comparisons.get(index)?;let group=set.groups.iter().find(|(_,range)|range.contains(&index)).map_or("",|(name,_)|name.as_str());Some((group,c.old_path.as_str(),c.path.as_str()))}

impl ChangeSet {
    /// Retained applied/undone files stay navigable when a fresh Git diff omits them.
    pub(crate) fn retain_comparison(&mut self,old:&Self,index:usize){
        let Some(c)=old.comparisons.get(index)else{return;};let at=self.comparisons.len();self.comparisons.push(c.clone());
        for path in [&c.old_path,&c.path]{if let Some(attr)=old.attrs.get(path){self.attrs.insert(path.clone(),attr.clone());}
            if let Some(encoding)=old.encodings.get(path){self.encodings.insert(path.clone(),encoding.clone());}
            if let Some(note)=old.notes.get(path){self.notes.insert(path.clone(),note.clone());}}
        if let Some((group,_))=old.groups.iter().find(|(_,range)|range.contains(&index)){if let Some((last,range))=self.groups.last_mut().filter(|(_,range)|range.end==at)&&last==group{range.end+=1;}else{self.groups.push((group.clone(),at..at+1));}}
    }

    /// Lists the Change Set; `cwd` is where the user ran Lenz. Synchronous: call it off the UI thread.
    pub fn load(spec: &Spec, cwd: &Path) -> Result<ChangeSet, String> {
        match spec {
            Spec::Empty => Ok(ChangeSet{comparisons:Vec::new(),groups:Vec::new(),source:Source::Plain,attrs:Default::default(),encodings:Default::default(),unpacked:Default::default(),notes:Default::default()}),
            Spec::Git { cached, revs, paths, untracked } => {
                let root = toplevel(cwd)?;
                let list = list_git(cwd, &root, *cached, revs, paths, *untracked)?;
                let mut set = ChangeSet::grouped(root, vec![(String::new(), list)])?;
                set.groups.clear();
                Ok(set)
            }
            // gitattributes are git's: plain files are binary by content alone
            Spec::Plain { left, right } => {
                let mut comparisons = list_plain(cwd, left, right)?;
                sort_for_navigator(&mut comparisons);
                let (attrs, encodings, notes) = Default::default();
                let mut set = ChangeSet { comparisons, groups: Vec::new(), source: Source::Plain, attrs, encodings, unpacked: Default::default(), notes };
                set.expand_archives();
                Ok(set)
            }
        }
    }

    /// A git Change Set in the repository (or worktree) at `root`, of named groups each in File Navigator order;
    /// empty groups are dropped.
    /// The repository's root, for a git Change Set.
    pub fn root(&self) -> Option<&Path> {
        match &self.source {
            Source::Git { root, .. } => Some(root),
            Source::Plain => None,
        }
    }

    pub(crate) fn grouped(root: PathBuf, lists: Vec<(String, Vec<Comparison>)>) -> Result<ChangeSet, String> {
        let (mut comparisons, mut groups) = (Vec::new(), Vec::new());
        for (name, mut list) in lists.into_iter().filter(|(_, list)| !list.is_empty()) {
            sort_for_navigator(&mut list);
            let start = comparisons.len();
            comparisons.extend(list);
            groups.push((name, start..comparisons.len()));
        }
        let (attrs, encodings, notes) = diff_attrs(&root, &comparisons)?;
        let source = Source::Git { root, cat_file: std::sync::Arc::new(Mutex::new(None)), check: std::sync::Arc::new(Mutex::new(None)) };
        let mut set = ChangeSet { comparisons, groups, source, attrs, encodings, unpacked: Default::default(), notes };
        set.expand_archives();
        Ok(set)
    }

    /// Why comparison `index` may not look as `git diff` shows it: a repository-config driver Lenz did not run, or
    /// an archive Lenz could not show as a folder.
    pub fn note(&self, index: usize) -> Option<&str> {
        let c = self.comparisons.get(index)?;
        self.notes.get(&c.path).or_else(|| self.notes.get(&c.old_path)).map(String::as_str)
    }

    /// One Side's bytes as stored (no textconv).
    pub fn read(&self, content: &Content) -> Result<Vec<u8>, String> {
        match content {
            Content::Empty => Ok(Vec::new()),
            Content::File(path) => fs::read(path).map_err(io_error(path)),
            Content::Symlink(path) => {
                fs::read_link(path).map(|target| target.into_os_string().into_encoded_bytes()).map_err(io_error(path))
            }
            Content::Submodule(sha) => Ok(format!("Subproject commit {sha}\n").into_bytes()),
            Content::Blob(name) => self.blob(name, "--batch", |p| p.get(name)),
            Content::Archived { archive, kind, member } => match self.unpacked(archive, *kind)? {
                Some(tar) => archive::extract(archive::Kind::Tar(archive::Compression::None), &tar, member),
                None => archive::extract(*kind, &self.read(archive)?, member),
            },
        }
    }

    /// Explicit Reload must reread mutable archive inputs, including reads already in flight.
    pub(crate) fn invalidate_archive_cache(&self) {
        let mut cache = self.unpacked.lock().unwrap_or_else(PoisonError::into_inner);
        cache.generation += 1;
        cache.streams.clear();
    }

    /// A compressed tar archive's stream, decompressed once and kept for its next member (two archives, up to
    /// 64 MiB each); `None` for any other archive, or one too large to keep.
    fn unpacked(&self, archive: &Content, kind: archive::Kind) -> Result<Option<std::sync::Arc<[u8]>>, String> {
        const KEEP: u64 = 64 << 20;
        let archive::Kind::Tar(compression) = kind else { return Ok(None) };
        if compression == archive::Compression::None {
            return Ok(None);
        }
        let generation = {
            let cache = self.unpacked.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some((_, tar)) = cache.streams.iter().find(|(key, _)| key == archive) {
                return Ok(Some(tar.clone()));
            }
            cache.generation
        };
        let Some(tar) = archive::unpack(compression, &self.read(archive)?, KEEP)? else { return Ok(None) };
        let tar: std::sync::Arc<[u8]> = tar.into();
        let mut cache = self.unpacked.lock().unwrap_or_else(PoisonError::into_inner);
        if cache.generation == generation {
            if cache.streams.len() == 2 {
                cache.streams.remove(0);
            }
            cache.streams.push((archive.clone(), tar.clone()));
        }
        Ok(Some(tar))
    }

    /// The repository's top directory, for a git Change Set.
    pub fn git_root(&self) -> Option<&Path> {
        match &self.source {
            Source::Git { root, .. } => Some(root),
            Source::Plain => None,
        }
    }

    /// Comparison `index`'s two Sides as text (textconv applied, invalid UTF-8 replaced).
    pub fn texts(&self, index: usize) -> Result<(String, String), String> {
        let [l, r] = self.sides(index)?;
        let text = |s: SideBytes| String::from_utf8_lossy(&s.bytes).into_owned();
        Ok((text(l), text(r)))
    }

    /// Comparison `index`'s Sides read for display: through the path's textconv driver if it has one, and judged
    /// binary as git does (the plain adapter by NUL alone). Reads every byte: check [`ChangeSet::sizes`] first.
    pub fn sides(&self, index: usize) -> Result<[SideBytes; 2], String> {
        let c = &self.comparisons[index];
        Ok([self.side(&c.left, &c.old_path)?, self.side(&c.right, &c.path)?])
    }

    /// Each Side's size in bytes as stored (before textconv), without reading it.
    #[allow(dead_code)] // for the UI's Large-file gate and binary placeholder
    pub fn sizes(&self, index: usize) -> Result<[u64; 2], String> {
        let c = &self.comparisons[index];
        Ok([self.size(&c.left)?, self.size(&c.right)?])
    }

    /// Are the two Sides' stored bytes the same (a mode-only change, a pure rename)?
    #[allow(dead_code)] // for the UI's binary placeholder ("bytes differ")
    pub fn identical(&self, index: usize) -> Result<bool, String> {
        let c = &self.comparisons[index];
        if let (Content::Blob(l), Content::Blob(r)) = (&c.left, &c.right)
            && !l.contains(':')
            && !r.contains(':')
        {
            return Ok(l == r); // object ids
        }
        // ponytail: equal sizes read both Sides whole; hash with `git hash-object` if huge equal-size binaries matter.
        Ok(self.size(&c.left)? == self.size(&c.right)? && self.read(&c.left)? == self.read(&c.right)?)
    }

    fn side(&self, content: &Content, path: &str) -> Result<SideBytes, String> {
        let attr = match content {
            Content::Blob(_) | Content::File(_) => self.attrs.get(path),
            _ => None,
        };
        if let (Some(DiffAttr::Textconv(command)), Source::Git { root, .. }) = (attr, &self.source) {
            // Not through the long-lived `--batch --textconv`: its header gives the unconverted size, and a failing
            // driver kills it.
            let bytes = match content {
                Content::File(file) => textconv_file(root, command, file)?,
                Content::Blob(name) => git::run(root, &["cat-file", "--textconv", &format!("--path={path}"), name])?,
                _ => unreachable!("attributes apply to blobs and files only"),
            };
            return Ok(SideBytes { bytes, binary: false, textconv: true, encoding: None });
        }
        let bytes = self.read(content)?;
        let encoding = matches!(content, Content::File(_)).then(|| self.encodings.get(path).cloned()).flatten();
        let binary = match attr {
            Some(DiffAttr::Text) => false,
            Some(DiffAttr::Binary) => true,
            // a declared encoding makes it text (UTF-16 without a BOM is full of NULs)
            _ => encoding.is_none() && looks_binary(&bytes),
        };
        Ok(SideBytes { bytes, binary, textconv: false, encoding })
    }

    fn size(&self, content: &Content) -> Result<u64, String> {
        match content {
            Content::File(path) => fs::metadata(path).map(|m| m.len()).map_err(io_error(path)),
            Content::Blob(name) => self.blob(name, "--batch-check", |p| p.size(name)).map(|s| s as u64),
            Content::Archived { archive, kind, member } => match self.unpacked(archive, *kind)? {
                Some(tar) => archive::size(archive::Kind::Tar(archive::Compression::None), &tar, member),
                None => archive::size(*kind, &self.read(archive)?, member),
            },
            _ => self.read(content).map(|b| b.len() as u64),
        }
    }

    /// Asks the long-lived `cat-file <mode>` about blob `name`; a missing `HEAD:<path>` is empty.
    fn blob<T: Default>(&self, name: &str, mode: &str, ask: impl Fn(&mut CatFile) -> std::io::Result<Option<T>>) -> Result<T, String> {
        let Source::Git { root, cat_file, check, .. } = &self.source else {
            return Err(format!("lenz: {name}: not a git Change Set"));
        };
        let mut process = if mode == "--batch" { cat_file } else { check }.lock().unwrap_or_else(PoisonError::into_inner);
        let p = match process.as_mut() {
            Some(p) => p,
            None => process.insert(CatFile::spawn(root, mode)?),
        };
        match ask(p) {
            Ok(Some(answer)) => Ok(answer),
            // A conflicted file HEAD doesn't have (e.g. deleted on our side).
            Ok(None) if name.starts_with("HEAD:") => Ok(T::default()),
            Ok(None) => Err(format!("lenz: git object {name} is missing")),
            Err(e) => {
                *process = None; // out of step with its output; respawn on the next use
                Err(format!("lenz: git cat-file: {e}"))
            }
        }
    }
}

/// How deep archives inside archives are opened as folders.
const MAX_NESTING: usize = 3;

impl ChangeSet {
    /// Replaces each changed archive with one Comparison per changed member, archives inside it too, keeping every
    /// group in File Navigator order. An archive that can't be read, or whose members are all unchanged, stays one
    /// binary Comparison with a note saying why.
    fn expand_archives(&mut self) {
        if !self.comparisons.iter().any(|c| archive_kinds(c).is_some()) {
            return;
        }
        let spans: Vec<(Option<String>, Range<usize>)> = if self.groups.is_empty() {
            vec![(None, 0..self.comparisons.len())]
        } else {
            self.groups.iter().map(|(name, range)| (Some(name.clone()), range.clone())).collect()
        };
        let old = std::mem::take(&mut self.comparisons);
        let (mut comparisons, mut groups) = (Vec::with_capacity(old.len()), Vec::new());
        for (name, range) in spans {
            let start = comparisons.len();
            let mut list = Vec::new();
            for c in &old[range] {
                self.expand(c.clone(), 0, &mut list);
            }
            sort_for_navigator(&mut list);
            comparisons.extend(list);
            if let Some(name) = name {
                groups.push((name, start..comparisons.len()));
            }
        }
        self.comparisons = comparisons;
        self.groups = groups;
    }

    fn expand(&mut self, c: Comparison, depth: usize, out: &mut Vec<Comparison>) {
        let Some(kinds) = archive_kinds(&c) else {
            out.push(c);
            return;
        };
        match self.archive_members(&c, kinds) {
            Ok(members) if !members.is_empty() => {
                for member in members {
                    if depth < MAX_NESTING {
                        self.expand(member, depth + 1, out);
                    } else {
                        out.push(member);
                    }
                }
            }
            Ok(_) => {
                let note = "no file inside the archive changed: only its packing differs";
                self.notes.entry(c.path.clone()).or_insert_with(|| note.into());
                out.push(c);
            }
            Err(e) => {
                self.notes.entry(c.path.clone()).or_insert_with(|| format!("not shown as a folder: {e}"));
                out.push(c);
            }
        }
    }

    /// Comparison `c`'s changed members: paired by path, and files only on one side with the same content paired as
    /// renames, as plain directories are.
    fn archive_members(&self, c: &Comparison, [left_kind, right_kind]: [Option<archive::Kind>; 2]) -> Result<Vec<Comparison>, String> {
        let list = |content: &Content, kind: Option<archive::Kind>| -> Result<BTreeMap<String, archive::Fingerprint>, String> {
            let Some(kind) = kind else { return Ok(BTreeMap::new()) };
            if self.size(content)? > archive::MAX_ARCHIVE {
                return Err(format!("larger than {} MiB", archive::MAX_ARCHIVE >> 20));
            }
            archive::members(kind, &self.read(content)?)
        };
        let (left, right) = (list(&c.left, left_kind)?, list(&c.right, right_kind)?);
        let member = |content: &Content, kind: Option<archive::Kind>, name: &str| match kind {
            Some(kind) => Content::Archived { archive: Box::new(content.clone()), kind, member: name.to_string() },
            None => Content::Empty,
        };
        let comparison = |old: &str, new: &str, status: Status, left: Content, right: Content| Comparison {
            old_path: format!("{}/{old}", c.old_path),
            path: format!("{}/{new}", c.path),
            status,
            left,
            right,
        };
        let added = if c.status == Status::Untracked { Status::Untracked } else { Status::Added };
        let mut out = Vec::new();
        let mut deleted: HashMap<archive::Fingerprint, Vec<&String>> = HashMap::new();
        let mut gone = Vec::new();
        for (name, fingerprint) in &left {
            match right.get(name) {
                Some(other) if other != fingerprint => out.push(comparison(
                    name,
                    name,
                    Status::Modified,
                    member(&c.left, left_kind, name),
                    member(&c.right, right_kind, name),
                )),
                Some(_) => {}
                None => {
                    gone.push(name);
                    // Like git, never pair empty files as renames.
                    if fingerprint.size > 0 {
                        deleted.entry(*fingerprint).or_default().push(name);
                    }
                }
            }
        }
        let mut renamed = BTreeSet::new();
        for (name, fingerprint) in right.iter().filter(|(name, _)| !left.contains_key(*name)) {
            let right_side = member(&c.right, right_kind, name);
            match deleted.get_mut(fingerprint).and_then(Vec::pop) {
                Some(old) => {
                    renamed.insert(old);
                    let status = Status::Renamed { similarity: 100 };
                    out.push(comparison(old, name, status, member(&c.left, left_kind, old), right_side));
                }
                None => out.push(comparison(name, name, added.clone(), Content::Empty, right_side)),
            }
        }
        for name in gone.into_iter().filter(|name| !renamed.contains(name)) {
            out.push(comparison(name, name, Status::Deleted, member(&c.left, left_kind, name), Content::Empty));
        }
        Ok(out)
    }
}

/// Each Side's archive format when comparison `c` is a changed archive Lenz may open as a folder (`None` for an
/// absent Side).
fn archive_kinds(c: &Comparison) -> Option<[Option<archive::Kind>; 2]> {
    // conflicts belong to the merge workflow; a mode-only change or a pure rename has no changed member
    if matches!(c.status, Status::Conflicted | Status::ModeChanged { .. } | Status::Renamed { similarity: 100 }) {
        return None;
    }
    let side = |content: &Content, path: &str| match content {
        Content::Empty => Some(None),
        Content::Blob(_) | Content::File(_) | Content::Archived { .. } => archive::Kind::of(path).map(Some),
        Content::Symlink(_) | Content::Submodule(_) => None,
    };
    let kinds = [side(&c.left, &c.old_path)?, side(&c.right, &c.path)?];
    kinds.iter().any(Option::is_some).then_some(kinds)
}

pub(crate) use lenz_content::encoding::looks_binary;

/// Runs a textconv `command` on a working-tree file as git does: through the shell, the file's path appended.
fn textconv_file(root: &Path, command: &str, file: &Path) -> Result<Vec<u8>, String> {
    // ponytail: needs `sh`, as git itself does; on Windows that means Git for Windows' sh on PATH.
    let out = std::process::Command::new("sh")
        .args(["-c", &format!("{command} \"$@\""), command])
        .arg(file)
        .current_dir(root)
        .output()
        .map_err(|e| format!("lenz: textconv {command}: {e}"))?;
    if out.status.success() {
        Ok(out.stdout)
    } else {
        Err(format!("lenz: textconv {command}: {}", String::from_utf8_lossy(&out.stderr).trim_end()))
    }
}

type Attrs = (HashMap<String, DiffAttr>, HashMap<String, String>, HashMap<String, String>);

/// The `diff` attribute of every Side's path, resolved against the diff drivers in git's config, and its
/// `working-tree-encoding`.
fn diff_attrs(root: &Path, list: &[Comparison]) -> Result<Attrs, String> {
    let paths: BTreeSet<&str> = list.iter().flat_map(|c| [c.old_path.as_str(), c.path.as_str()]).collect();
    if paths.is_empty() {
        return Ok(Default::default());
    }
    // `diff.<driver>.textconv` / `.binary`; git exits 1 when there are none.
    let config = git::command(root)?
        .args(["config", "-z", "--get-regexp", r"^diff\..+\.(textconv|binary)$"])
        .output()
        .map_err(|e| format!("lenz: git config: {e}"))?;
    let (mut textconv, mut binary) = (HashMap::new(), HashMap::new());
    for entry in config.stdout.split(|&b| b == 0).filter(|e| !e.is_empty()) {
        let entry = String::from_utf8_lossy(entry);
        // A key without `=` is a boolean true.
        let (key, value) = entry.split_once('\n').unwrap_or((&entry, "true"));
        if let Some(driver) = key.strip_prefix("diff.").and_then(|k| k.strip_suffix(".textconv")) {
            textconv.insert(driver.to_string(), value.to_string());
        } else if let Some(driver) = key.strip_prefix("diff.").and_then(|k| k.strip_suffix(".binary")) {
            let off = ["false", "no", "off", "0", ""].contains(&value.to_ascii_lowercase().as_str());
            binary.insert(driver.to_string(), !off);
        }
    }

    // `binary` is a macro for `-diff`, so `diff` alone says it all.
    let input: Vec<u8> = paths.iter().flat_map(|p| p.bytes().chain([0])).collect();
    let out = git::run_with_input(root, &["check-attr", "-z", "--stdin", "diff", "working-tree-encoding", "filter"], input)?;
    let fields: Vec<&[u8]> = out.split(|&b| b == 0).collect();
    let (mut attrs, mut encodings, mut skipped) = (HashMap::new(), HashMap::new(), HashMap::new());
    let untrusted = git::untrusted(root);
    let not_run = |kind: &str, driver: &str| format!("{kind} `{driver}` from this repository's .git/config was not run");
    for [path, name, value] in fields.as_chunks().0 {
        let (path, value) = (git::path_text(path)?, String::from_utf8_lossy(value));
        if name == b"filter" {
            if untrusted.filters.iter().any(|f| *f == value) {
                skipped.insert(path.to_owned(), not_run("filter", &value));
            }
            continue;
        }
        if name == b"working-tree-encoding" {
            if !["unspecified", "set", "unset"].contains(&value.as_ref()) {
                encodings.insert(path.to_owned(), value.into_owned());
            }
            continue;
        }
        let attr = match value.as_ref() {
            "set" => DiffAttr::Text,
            "unset" => DiffAttr::Binary,
            "unspecified" => continue,
            driver => match (textconv.get(driver), binary.get(driver)) {
                (Some(command), _) if !command.is_empty() => DiffAttr::Textconv(command.clone()),
                _ if untrusted.textconv.iter().any(|t| t == driver) => {
                    skipped.insert(path.to_owned(), not_run("textconv driver", driver));
                    match binary.get(driver) {
                        Some(true) => DiffAttr::Binary,
                        Some(false) => DiffAttr::Text,
                        None => continue,
                    }
                }
                (Some(_), _) => continue, // set empty: no textconv
                (None, Some(true)) => DiffAttr::Binary,
                (None, Some(false)) => DiffAttr::Text,
                (None, None) => continue,
            },
        };
        attrs.insert(path.to_owned(), attr);
    }
    Ok((attrs, encodings, skipped))
}

/// Directory, then file name.
fn sort_for_navigator(list: &mut [Comparison]) {
    list.sort_by_cached_key(|c| {
        let (dir, name) = c.path.rsplit_once('/').unwrap_or(("", &c.path));
        (dir.to_string(), name.to_string())
    });
}

/// The top directory of the repository or worktree around `cwd`.
pub fn toplevel(cwd: &Path) -> Result<PathBuf, String> {
    let top = git::run(cwd, &["rev-parse", "--show-toplevel"])?;
    Ok(PathBuf::from(git::path_text(top.strip_suffix(b"\n").unwrap_or(&top))?))
}

fn io_error(path: &Path) -> impl Fn(std::io::Error) -> String + '_ {
    move |e| format!("lenz: {}: {e}", path.display())
}

/// `git diff --raw`, plus untracked and unmerged files from `git status` when the right Side is the working tree.
/// Runs in `cwd` so pathspecs mean what they mean to `git diff`; both commands print root-relative paths.
pub(crate) fn list_git(
    cwd: &Path,
    root: &Path,
    cached: bool,
    revs: &[String],
    paths: &[String],
    untracked: bool,
) -> Result<Vec<Comparison>, String> {
    let pathspec: Vec<&str> =
        if paths.is_empty() { Vec::new() } else { ["--"].into_iter().chain(paths.iter().map(String::as_str)).collect() };
    let fields = |out: &[u8]| -> Result<Vec<String>, String> {
        out.split(|&b| b == 0).filter(|f| !f.is_empty())
            .map(|f| git::path_text(f).map(str::to_owned)).collect()
    };

    let mut diff = vec!["diff", "--raw", "-z", "--no-abbrev", "-M", "--no-relative", "--no-color"];
    if cached {
        diff.push("--cached");
    }
    diff.extend(revs.iter().map(String::as_str));
    diff.extend(&pathspec);
    let raw = fields(&git::run(cwd, &diff)?)?;

    let side = |mode: &str, sha: &str, path: &str| match mode {
        "000000" => Content::Empty,
        "160000" => Content::Submodule(sha.to_string()),
        _ if sha.bytes().all(|b| b == b'0') && mode == "120000" => Content::Symlink(root.join(path)),
        _ if sha.bytes().all(|b| b == b'0') => Content::File(root.join(path)),
        _ => Content::Blob(sha.to_string()),
    };
    let mode = |m: &str| u32::from_str_radix(m, 8).unwrap_or(0);
    let mut list = Vec::new();
    let mut unmerged = BTreeSet::new();
    // Working-tree files whose mode changed: same content as `old sha`?
    let mut mode_checks = Vec::new();
    let mut raw = raw.into_iter();
    while let Some(meta) = raw.next() {
        let bad = || format!("lenz: unexpected `git diff --raw` output: {meta:?}");
        let [old_mode, new_mode, old_sha, new_sha, status] = meta.trim_start_matches(':').split(' ').collect::<Vec<_>>()[..]
        else {
            return Err(bad());
        };
        let old = raw.next().ok_or_else(bad)?;
        let new = if status.starts_with(['R', 'C']) { raw.next().ok_or_else(bad)? } else { old.clone() };
        let (old_path, left, mut status) = match status.as_bytes()[0] {
            b'U' => {
                unmerged.insert(new);
                continue;
            }
            // A copy only appears when the user's config asks for copies; Lenz shows it as added.
            b'A' | b'C' => (new.clone(), Content::Empty, Status::Added),
            b'D' => (old.clone(), side(old_mode, old_sha, &old), Status::Deleted),
            b'R' => {
                let similarity = status[1..].parse().unwrap_or(0);
                (old.clone(), side(old_mode, old_sha, &old), Status::Renamed { similarity })
            }
            _ => (old.clone(), side(old_mode, old_sha, &old), Status::Modified),
        };
        let right = side(new_mode, new_sha, &new);
        if old_mode != new_mode && status == Status::Modified {
            let mode_only = Status::ModeChanged { old: mode(old_mode), new: mode(new_mode) };
            if old_sha == new_sha {
                status = mode_only;
            } else if matches!(right, Content::File(_)) {
                // The working tree's sha isn't listed; hashed below.
                mode_checks.push((list.len(), old_sha.to_string(), mode_only));
            }
        }
        list.push(Comparison { old_path, path: new, status, left, right });
    }

    if !mode_checks.is_empty() {
        let mut hash = vec!["hash-object", "--"];
        hash.extend(mode_checks.iter().map(|(i, ..)| list[*i].path.as_str()));
        let out = git::run(root, &hash)?;
        for ((i, old_sha, mode_only), new_sha) in mode_checks.into_iter().zip(String::from_utf8_lossy(&out).lines()) {
            if old_sha == new_sha {
                list[i].status = mode_only;
            }
        }
    }

    if right_is_worktree(cached, revs) {
        let untracked_files = if untracked { "--untracked-files=all" } else { "--untracked-files=no" };
        let mut args = vec!["status", "--porcelain=v2", "-z", untracked_files];
        args.extend(&pathspec);
        let status = fields(&git::run(cwd, &args)?)?;
        let mut status = status.into_iter();
        while let Some(entry) = status.next() {
            match entry.as_bytes()[0] {
                // A rename carries its original path as an extra field.
                b'2' => {
                    status.next();
                }
                b'u' => {
                    if let Some(path) = entry.splitn(11, ' ').nth(10) {
                        unmerged.insert(path.to_string());
                    }
                }
                // A trailing slash marks a nested repository.
                b'?' if !entry.ends_with('/') => {
                    let path = entry[2..].to_string();
                    let right = Content::File(root.join(&path));
                    list.push(Comparison { old_path: path.clone(), path, status: Status::Untracked, left: Content::Empty, right });
                }
                _ => {}
            }
        }
    }

    list.retain(|c| !unmerged.contains(&c.path));
    for path in unmerged {
        let file = root.join(&path);
        let right = if file.symlink_metadata().is_ok() { Content::File(file) } else { Content::Empty };
        let left = Content::Blob(format!("HEAD:{path}"));
        list.push(Comparison { old_path: path.clone(), path, status: Status::Conflicted, left, right });
    }
    Ok(list)
}

/// Two files, or two directories paired by relative path.
fn list_plain(cwd: &Path, left: &Path, right: &Path) -> Result<Vec<Comparison>, String> {
    let (l, r) = (cwd.join(left), cwd.join(right));
    let (lm, rm) = (fs::metadata(&l).map_err(io_error(left))?, fs::metadata(&r).map_err(io_error(right))?);
    match (lm.is_dir(), rm.is_dir()) {
        (true, true) => diff_dirs(&l, &r),
        (false, false) => {
            if fs::read(&l).map_err(io_error(left))? == fs::read(&r).map_err(io_error(right))? {
                return Ok(Vec::new());
            }
            // `git difftool` hands a missing Side over as /dev/null.
            let null = Path::new("/dev/null");
            let (old_path, path) = (left.display().to_string(), right.display().to_string());
            Ok(vec![if left == null {
                Comparison { old_path: path.clone(), path, status: Status::Added, left: Content::Empty, right: Content::File(r) }
            } else if right == null {
                Comparison { path: old_path.clone(), old_path, status: Status::Deleted, left: Content::File(l), right: Content::Empty }
            } else {
                Comparison { old_path, path, status: Status::Modified, left: Content::File(l), right: Content::File(r) }
            }])
        }
        _ => Err(format!("lenz: cannot compare a file with a directory: {} {}", left.display(), right.display())),
    }
}

/// Pairs by relative path; files only on one side with byte-identical content become renames.
fn diff_dirs(l: &Path, r: &Path) -> Result<Vec<Comparison>, String> {
    let (left, right) = (files(l)?, files(r)?);
    let read = |p: &PathBuf| fs::read(p).map_err(io_error(p));
    let file = |p: &PathBuf| Content::File(p.clone());
    let mut list = Vec::new();
    // ponytail: keyed by whole contents, so every deleted file is held in memory; hash + verify if trees get huge.
    let mut deleted: HashMap<Vec<u8>, Vec<&String>> = HashMap::new();
    let mut gone = Vec::new();
    for (rel, lp) in &left {
        match right.get(rel) {
            Some(rp) if read(lp)? != read(rp)? => list.push(Comparison {
                old_path: rel.clone(),
                path: rel.clone(),
                status: Status::Modified,
                left: file(lp),
                right: file(rp),
            }),
            Some(_) => {}
            None => {
                gone.push(rel);
                let bytes = read(lp)?;
                // Like git, never pair empty files as renames.
                if !bytes.is_empty() {
                    deleted.entry(bytes).or_default().push(rel);
                }
            }
        }
    }
    let mut renamed = BTreeSet::new();
    for (rel, rp) in right.iter().filter(|(rel, _)| !left.contains_key(*rel)) {
        let from = if deleted.is_empty() { None } else { deleted.get_mut(&read(rp)?).and_then(|v| v.pop()) };
        let (old_path, status, left_side) = match from {
            Some(old) => {
                renamed.insert(old);
                (old.clone(), Status::Renamed { similarity: 100 }, file(&left[old]))
            }
            None => (rel.clone(), Status::Added, Content::Empty),
        };
        list.push(Comparison { old_path, path: rel.clone(), status, left: left_side, right: file(rp) });
    }
    for rel in gone.into_iter().filter(|rel| !renamed.contains(rel)) {
        list.push(Comparison {
            old_path: rel.clone(),
            path: rel.clone(),
            status: Status::Deleted,
            left: file(&left[rel]),
            right: Content::Empty,
        });
    }
    Ok(list)
}

/// Every file of a compared folder by `/`-separated relative path: inside a git work tree (and not itself ignored)
/// the files git lists, tracked and untracked but not ignored; else every file under it.
fn files(root: &Path) -> Result<BTreeMap<String, PathBuf>, String> {
    // `git difftool -d` exports GIT_DIR and GIT_WORK_TREE, and git would then list that repository's files for
    // its temporary folders.
    let env = |name| std::env::var_os(name).is_some();
    if env("GIT_DIR") || env("GIT_WORK_TREE") || !git::may_be_in_repository(root) {
        return walk(root);
    }
    // Fails outside a work tree (and inside `.git`); lists nothing in an ignored folder such as `target/`.
    let out = match git::run(root, &["ls-files", "-co", "--exclude-standard", "-z"]) {
        Ok(out) if !out.is_empty() || git::run(root, &["check-ignore", "-q", "."]).is_err() => out,
        _ => return walk(root),
    };
    let mut files = BTreeMap::new();
    for entry in out.split(|&b| b == 0).filter(|e| !e.is_empty()) {
        let rel = git::path_text(entry)?.to_string();
        let path = root.join(&rel);
        // Deleted tracked files, submodules and nested repositories (`x/`) are not files here.
        if path.is_file() {
            files.insert(rel, path);
        }
    }
    Ok(files)
}

/// Every file under `root` by `/`-separated relative path, `.git` aside. Symlinks are read through, never descended
/// (`git difftool -d` links files into the working tree; directory links could loop).
fn walk(root: &Path) -> Result<BTreeMap<String, PathBuf>, String> {
    let mut files = BTreeMap::new();
    let mut dirs = vec![(root.to_path_buf(), String::new())];
    while let Some((dir, prefix)) = dirs.pop() {
        for entry in fs::read_dir(&dir).map_err(io_error(&dir))? {
            let entry = entry.map_err(io_error(&dir))?;
            if entry.file_name() == ".git" {
                continue;
            }
            let rel = format!("{prefix}{}", git::path_text(entry.file_name().as_encoded_bytes())?);
            let kind = entry.file_type().map_err(io_error(&entry.path()))?;
            if kind.is_dir() {
                dirs.push((entry.path(), rel + "/"));
            } else if !(kind.is_symlink() && entry.path().is_dir()) {
                files.insert(rel, entry.path());
            }
        }
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::testing::{commit_all, git, repo, write};

    // APFS refuses non-UTF-8 file names (EILSEQ), so the fixture can't exist on macOS.
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn repository_paths_fail_closed_instead_of_colliding_after_utf8_replacement() {
        use std::os::unix::ffi::OsStringExt;
        let dir = repo();
        let invalid = dir.path().join(std::ffi::OsString::from_vec(b"f\xff".to_vec()));
        let valid = dir.path().join("f�");
        std::fs::write(&invalid, "invalid base\n").unwrap();
        std::fs::write(&valid, "valid base\n").unwrap();
        commit_all(dir.path(), "two distinct paths");
        std::fs::write(&invalid, "invalid draft\n").unwrap();
        std::fs::write(&valid, "valid draft\n").unwrap();
        // Both ordinary local changes (including Git discard) and HEAD comparisons
        // must fail before exposing an editable/discardable Comparison for the wrong file.
        for revs in [vec![], vec!["HEAD".to_string()]] {
            let spec = Spec::Git { cached: false, revs, paths: vec![], untracked: true };
            let error = ChangeSet::load(&spec, dir.path()).err().expect("invalid path rejected");
            assert!(error.contains("non-UTF-8 file paths are unsupported"), "{error}");
        }
        git(dir.path(), &["add", "-A"]);
        let staged = Spec::Git { cached: true, revs: vec![], paths: vec![], untracked: false };
        assert!(ChangeSet::load(&staged, dir.path()).is_err());
        assert_eq!(std::fs::read_to_string(&invalid).unwrap(), "invalid draft\n");
        assert_eq!(std::fs::read_to_string(&valid).unwrap(), "valid draft\n");
        // Untracked paths and plain directory comparisons have the same boundary.
        let other = repo();
        std::fs::write(other.path().join(std::ffi::OsString::from_vec(b"u\xff".to_vec())), "keep").unwrap();
        assert!(list_git(other.path(), other.path(), false, &[], &[], true).is_err());
        assert!(walk(dir.path()).is_err());
        assert!(files(dir.path()).is_err() && files(other.path()).is_err());
    }

    #[test]
    fn repository_root_keeps_its_trailing_newline() {
        let dir = tempfile::tempdir().unwrap();
        // git reports the resolved path (macOS temp dirs sit behind the /var -> /private/var link)
        let newline = dir.path().canonicalize().unwrap().join("repo\n");
        std::fs::create_dir(&newline).unwrap();
        git(&newline, &["init", "-q"]);
        assert_eq!(toplevel(&newline).unwrap(), newline);
    }

    // APFS refuses non-UTF-8 file names (EILSEQ), so the fixture can't exist on macOS.
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn repository_root_rejects_invalid_utf8() {
        use std::os::unix::ffi::OsStringExt;
        let dir = tempfile::tempdir().unwrap();
        let invalid = dir.path().join(std::ffi::OsString::from_vec(b"repo\xff".to_vec()));
        std::fs::create_dir(&invalid).unwrap();
        git(&invalid, &["init", "-q"]);
        assert!(toplevel(&invalid).unwrap_err().contains("non-UTF-8 file paths are unsupported"));
    }

    fn strings(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    fn git_spec(cached: bool, revs: &[&str], paths: &[&str], untracked: bool) -> Spec {
        Spec::Git { cached, revs: strings(revs), paths: strings(paths), untracked }
    }

    fn load(cwd: &Path, spec: Spec) -> ChangeSet {
        ChangeSet::load(&spec, cwd).unwrap()
    }

    fn listing(set: &ChangeSet) -> Vec<(&str, Status)> {
        set.comparisons.iter().map(|c| (c.path.as_str(), c.status.clone())).collect()
    }

    fn texts(set: &ChangeSet, i: usize) -> (String, String) {
        set.texts(i).unwrap()
    }

    fn pair(l: &str, r: &str) -> (String, String) {
        (l.to_string(), r.to_string())
    }

    #[test]
    fn a_change_set_can_be_shared_with_background_threads() {
        fn shareable<T: Send + Sync>() {}
        shareable::<ChangeSet>();
    }

    // --- git ---

    #[test]
    fn no_arguments_show_unstaged_changes_against_the_index() {
        let r = repo();
        write(r.path(), "a.txt", "one\n");
        commit_all(r.path(), "1");
        write(r.path(), "a.txt", "staged\n");
        git(r.path(), &["add", "a.txt"]);
        write(r.path(), "a.txt", "worktree\n");
        let set = load(r.path(), git_spec(false, &[], &[], true));
        assert_eq!(listing(&set), [("a.txt", Status::Modified)]);
        assert_eq!(texts(&set, 0), pair("staged\n", "worktree\n"));
    }

    #[test]
    fn staged_shows_the_index_against_head() {
        let r = repo();
        write(r.path(), "a.txt", "one\n");
        commit_all(r.path(), "1");
        write(r.path(), "a.txt", "staged\n");
        write(r.path(), "new.txt", "added\n");
        git(r.path(), &["add", "-A"]);
        write(r.path(), "a.txt", "worktree\n");
        write(r.path(), "untracked.txt", "u\n");
        let set = load(r.path(), git_spec(true, &[], &[], false));
        assert_eq!(listing(&set), [("a.txt", Status::Modified), ("new.txt", Status::Added)]);
        assert_eq!(texts(&set, 0), pair("one\n", "staged\n"));
        assert_eq!(texts(&set, 1), pair("", "added\n"));
    }

    #[test]
    fn revisions_and_ranges_compare_commits() {
        let r = repo();
        write(r.path(), "a.txt", "base\n");
        commit_all(r.path(), "1");
        git(r.path(), &["checkout", "-q", "-b", "topic"]);
        write(r.path(), "a.txt", "topic\n");
        commit_all(r.path(), "2");
        git(r.path(), &["checkout", "-q", "main"]);
        write(r.path(), "b.txt", "main only\n");
        commit_all(r.path(), "3");
        write(r.path(), "a.txt", "dirty\n");

        let set = load(r.path(), git_spec(false, &["main", "topic"], &[], false));
        assert_eq!(listing(&set), [("a.txt", Status::Modified), ("b.txt", Status::Deleted)]);
        assert_eq!(texts(&set, 0), pair("base\n", "topic\n"));
        assert_eq!(texts(&set, 1), pair("main only\n", ""));
        let set = load(r.path(), git_spec(false, &["main..topic"], &[], false));
        assert_eq!(listing(&set).len(), 2);
        // A...B: from the merge base, so main's own commit doesn't show.
        let set = load(r.path(), git_spec(false, &["main...topic"], &[], false));
        assert_eq!(listing(&set), [("a.txt", Status::Modified)]);
        assert_eq!(texts(&set, 0), pair("base\n", "topic\n"));
        // One revision: the working tree against it.
        let set = load(r.path(), git_spec(false, &["HEAD~1"], &[], true));
        assert_eq!(listing(&set), [("a.txt", Status::Modified), ("b.txt", Status::Added)]);
        assert_eq!(texts(&set, 0), pair("base\n", "dirty\n"));
    }

    #[test]
    fn untracked_files_are_added_when_the_right_side_is_the_working_tree() {
        let r = repo();
        write(r.path(), "a.txt", "a\n");
        write(r.path(), ".gitignore", "*.log\n");
        commit_all(r.path(), "1");
        write(r.path(), "new/deep/u.txt", "fresh\n");
        write(r.path(), "skip.log", "ignored\n");
        let set = load(r.path(), git_spec(false, &[], &[], true));
        assert_eq!(listing(&set), [("new/deep/u.txt", Status::Untracked)]);
        assert_eq!(texts(&set, 0), pair("", "fresh\n"));
        assert_eq!(listing(&load(r.path(), git_spec(false, &["HEAD"], &[], true))).len(), 1);
        assert!(listing(&load(r.path(), git_spec(false, &[], &[], false))).is_empty());
    }

    #[test]
    fn renames_carry_the_old_path_and_similarity() {
        let r = repo();
        let body: String = (0..20).map(|i| format!("line {i}\n")).collect();
        write(r.path(), "old/same.txt", &body);
        write(r.path(), "edited.txt", &body);
        commit_all(r.path(), "1");
        std::fs::create_dir(r.path().join("new")).unwrap();
        git(r.path(), &["mv", "old/same.txt", "new/same.txt"]);
        git(r.path(), &["mv", "edited.txt", "renamed.txt"]);
        write(r.path(), "renamed.txt", format!("{body}one more\n"));
        commit_all(r.path(), "2");
        let set = load(r.path(), git_spec(false, &["HEAD~1", "HEAD"], &[], false));
        let c = &set.comparisons;
        assert_eq!((c[0].old_path.as_str(), c[0].path.as_str()), ("edited.txt", "renamed.txt"));
        assert!(matches!(c[0].status, Status::Renamed { similarity } if (50..100).contains(&similarity)));
        assert_eq!(texts(&set, 0), (body.clone(), format!("{body}one more\n")));
        assert_eq!((c[1].old_path.as_str(), c[1].path.as_str()), ("old/same.txt", "new/same.txt"));
        assert_eq!(c[1].status, Status::Renamed { similarity: 100 });
    }

    #[test]
    fn deletions_have_an_empty_right_side() {
        let r = repo();
        write(r.path(), "gone.txt", "bye\n");
        commit_all(r.path(), "1");
        std::fs::remove_file(r.path().join("gone.txt")).unwrap();
        let set = load(r.path(), git_spec(false, &[], &[], true));
        assert_eq!(listing(&set), [("gone.txt", Status::Deleted)]);
        assert_eq!(set.comparisons[0].right, Content::Empty);
        assert_eq!(texts(&set, 0), pair("bye\n", ""));
    }

    #[cfg(unix)]
    #[test]
    fn a_mode_only_change_is_flagged_and_an_edit_with_it_is_modified() {
        use std::os::unix::fs::PermissionsExt;
        let r = repo();
        write(r.path(), "run.sh", "echo\n");
        write(r.path(), "edit.sh", "echo\n");
        commit_all(r.path(), "1");
        for f in ["run.sh", "edit.sh"] {
            std::fs::set_permissions(r.path().join(f), std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        write(r.path(), "edit.sh", "echo hi\n");
        let mode_only = Status::ModeChanged { old: 0o100644, new: 0o100755 };
        let set = load(r.path(), git_spec(false, &[], &[], true));
        assert_eq!(listing(&set), [("edit.sh", Status::Modified), ("run.sh", mode_only.clone())]);
        git(r.path(), &["add", "-A"]);
        let set = load(r.path(), git_spec(true, &[], &[], false));
        assert_eq!(listing(&set), [("edit.sh", Status::Modified), ("run.sh", mode_only)]);
    }

    /// main and topic both edit `both.txt`; main deletes `gone.txt`, topic edits it.
    fn conflicted_repo() -> tempfile::TempDir {
        let r = repo();
        write(r.path(), "both.txt", "base\n");
        write(r.path(), "gone.txt", "base\n");
        commit_all(r.path(), "1");
        git(r.path(), &["checkout", "-q", "-b", "topic"]);
        write(r.path(), "both.txt", "topic\n");
        write(r.path(), "gone.txt", "topic\n");
        commit_all(r.path(), "topic");
        git(r.path(), &["checkout", "-q", "main"]);
        write(r.path(), "both.txt", "main\n");
        std::fs::remove_file(r.path().join("gone.txt")).unwrap();
        commit_all(r.path(), "main");
        let merge = std::process::Command::new("git")
            .current_dir(r.path())
            .args(["-c", "user.name=t", "-c", "user.email=t@t", "merge", "-q", "topic"])
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .output()
            .unwrap();
        assert!(!merge.status.success(), "the merge must conflict");
        r
    }

    #[test]
    fn conflicted_files_show_the_working_tree_against_head() {
        let r = conflicted_repo();
        for spec in [git_spec(false, &[], &[], true), git_spec(false, &["HEAD"], &[], false), git_spec(true, &[], &[], false)] {
            let set = load(r.path(), spec.clone());
            assert_eq!(listing(&set), [("both.txt", Status::Conflicted), ("gone.txt", Status::Conflicted)], "{spec:?}");
            let (head, worktree) = texts(&set, 0);
            assert_eq!(head, "main\n");
            assert!(worktree.starts_with("<<<<<<<") && worktree.contains("topic\n"), "{worktree}");
            // Deleted on main: HEAD has no such file.
            assert_eq!(texts(&set, 1), pair("", "topic\n"));
        }
    }

    #[test]
    fn working_tree_sides_carry_their_declared_working_tree_encoding() {
        let r = repo();
        write(r.path(), ".gitattributes", "*.txt working-tree-encoding=UTF-16LE\n");
        commit_all(r.path(), "attrs");
        let utf16 = |s: &str| s.encode_utf16().flat_map(u16::to_le_bytes).collect::<Vec<u8>>();
        write(r.path(), "a.txt", utf16("hello\n"));
        commit_all(r.path(), "a");
        write(r.path(), "a.txt", utf16("hello world\n"));
        let set = load(r.path(), git_spec(false, &[], &[], false));
        let [left, right] = set.sides(0).unwrap();
        // git keeps the index copy in UTF-8; only the working tree is UTF-16
        assert_eq!((left.encoding.as_deref(), left.bytes.as_slice()), (None, "hello\n".as_bytes()));
        assert_eq!(right.encoding.as_deref(), Some("UTF-16LE"));
        assert!(!right.binary);
        assert_eq!(crate::encoding::decode(&right.bytes, right.encoding.as_deref()).text, "hello world\n");
    }

    #[test]
    fn binary_files_are_listed_and_read_byte_for_byte() {
        let r = repo();
        write(r.path(), "img.bin", [0u8, 159, 146, 150, 0, 1]);
        commit_all(r.path(), "1");
        let new = [0u8, 255, 254, 0, 10, 13, 0];
        write(r.path(), "img.bin", new);
        commit_all(r.path(), "2");
        let set = load(r.path(), git_spec(false, &["HEAD~1", "HEAD"], &[], false));
        assert_eq!(listing(&set), [("img.bin", Status::Modified)]);
        let c = &set.comparisons[0];
        assert_eq!(set.read(&c.left).unwrap(), [0u8, 159, 146, 150, 0, 1]);
        assert_eq!(set.read(&c.right).unwrap(), new);
    }

    /// `(path, left binary, right binary, left textconv, right textconv)` per Comparison.
    fn verdicts(set: &ChangeSet) -> Vec<(&str, bool, bool, bool, bool)> {
        (0..set.comparisons.len())
            .map(|i| {
                let [l, r] = set.sides(i).unwrap();
                (set.comparisons[i].path.as_str(), l.binary, r.binary, l.textconv, r.textconv)
            })
            .collect()
    }

    /// The paths `git diff` calls binary (`--numstat` would ignore textconv).
    fn git_binary(dir: &Path, args: &[&str]) -> BTreeSet<String> {
        let out = git(dir, &[&["diff", "--no-prefix"], args].concat());
        let pair = |l: &str| l.strip_prefix("Binary files ")?.strip_suffix(" differ")?.split_once(" and ").map(|(_, r)| r.into());
        out.lines().filter_map(pair).collect()
    }

    #[test]
    fn binary_is_a_nul_in_the_first_8000_bytes_unless_gitattributes_say_otherwise() {
        let r = repo();
        let nul_at = |at: usize, fill: u8| {
            let mut bytes = vec![fill; 8100];
            bytes[at] = 0;
            bytes
        };
        write(r.path(), ".gitattributes", "forced.txt binary\nnodiff.txt -diff\ntext.dat diff\n");
        let files = [("early.bin", 7999), ("late.txt", 8000), ("text.dat", 10)];
        for (name, at) in files {
            write(r.path(), name, nul_at(at, b'a'));
        }
        write(r.path(), "forced.txt", "plain\n");
        write(r.path(), "nodiff.txt", "plain\n");
        write(r.path(), "plain.txt", "plain\n");
        commit_all(r.path(), "1");
        for (name, at) in files {
            write(r.path(), name, nul_at(at, b'b'));
        }
        for name in ["forced.txt", "nodiff.txt", "plain.txt"] {
            write(r.path(), name, "edited\n");
        }
        // the left Side is a blob, the right Side a working-tree file
        let set = load(r.path(), git_spec(false, &[], &[], true));
        let got = verdicts(&set);
        assert_eq!(
            got,
            [
                ("early.bin", true, true, false, false),
                ("forced.txt", true, true, false, false),
                ("late.txt", false, false, false, false),
                ("nodiff.txt", true, true, false, false),
                ("plain.txt", false, false, false, false),
                ("text.dat", false, false, false, false),
            ]
        );
        let binary: BTreeSet<String> = got.iter().filter(|v| v.1 || v.2).map(|v| v.0.to_string()).collect();
        assert_eq!(binary, git_binary(r.path(), &[]));
        // reading a binary Side still gives its bytes
        assert_eq!(set.sides(0).unwrap()[1].bytes, nul_at(7999, b'b'));
    }

    #[test]
    fn a_utf16_file_with_a_bom_is_text_though_git_calls_it_binary() {
        let r = repo();
        write(r.path(), "u16.txt", [0xFF, 0xFE, b'h', 0, b'i', 0]);
        write(r.path(), "u32.txt", [0xFF, 0xFE, 0, 0, b'h', 0, 0, 0]);
        let set = load(r.path(), git_spec(false, &[], &[], true));
        assert_eq!(verdicts(&set), [("u16.txt", false, false, false, false), ("u32.txt", false, true, false, false)]);
    }

    #[test]
    fn a_repository_config_textconv_driver_is_not_run_and_says_so() {
        let r = repo();
        let ran = r.path().join("ran");
        write(r.path(), ".gitattributes", "*.up diff=upper\n");
        git(r.path(), &["config", "diff.upper.textconv", &format!("touch {}; tr a-z A-Z <", ran.display())]);
        write(r.path(), "doc.up", "abc\n");
        commit_all(r.path(), "1");
        write(r.path(), "doc.up", "abd\n");
        for spec in [git_spec(false, &[], &[], true), git_spec(false, &["HEAD"], &[], false)] {
            let set = load(r.path(), spec);
            assert_eq!(texts(&set, 0), pair("abc\n", "abd\n"));
            assert_eq!(set.note(0), Some("textconv driver `upper` from this repository's .git/config was not run"));
            let doc = crate::document::TextDocument::from_sides(&set, 0, Path::new("doc.up"), Path::new("doc.up"), set.sides(0).unwrap());
            assert!(doc.notes.iter().any(|n| n.contains("was not run")), "{:?}", doc.notes);
        }
        assert!(!ran.exists());
    }

    #[test]
    fn a_textconv_driver_turns_a_side_into_text() {
        let r = repo();
        crate::git::trust(r.path()); // as if the driver were in the user's global config
        write(r.path(), ".gitattributes", "*.up diff=upper\n*.raw diff=nodriver\n*.blob diff=blobby\n");
        git(r.path(), &["config", "diff.upper.textconv", "tr a-z A-Z <"]);
        git(r.path(), &["config", "diff.blobby.binary", "true"]);
        write(r.path(), "doc.up", "abc\0\n");
        write(r.path(), "doc.raw", "abc\0\n");
        write(r.path(), "doc.blob", "abc\n");
        commit_all(r.path(), "1");
        for name in ["doc.up", "doc.raw", "doc.blob"] {
            write(r.path(), name, "abd\0\n");
        }
        commit_all(r.path(), "2");
        write(r.path(), "doc.up", "xyz\n");
        let expected = [
            ("doc.blob", true, true, false, false),
            ("doc.raw", true, true, false, false),
            ("doc.up", false, false, true, true),
        ];
        // blob against blob, and blob against the working tree
        let commits = load(r.path(), git_spec(false, &["HEAD~1", "HEAD"], &[], false));
        assert_eq!(verdicts(&commits), expected);
        assert_eq!(git_binary(r.path(), &["HEAD~1", "HEAD"]), BTreeSet::from(["doc.blob".into(), "doc.raw".into()]));
        assert_eq!(texts(&commits, 2), pair("ABC\0\n", "ABD\0\n"));
        let worktree = load(r.path(), git_spec(false, &[], &[], true));
        assert_eq!(verdicts(&worktree), [("doc.up", false, false, true, true)]);
        assert_eq!(texts(&worktree, 0), pair("ABD\0\n", "XYZ\n"));
        // the stored bytes stay readable
        assert_eq!(worktree.read(&worktree.comparisons[0].right).unwrap(), b"xyz\n");
    }

    #[test]
    fn sizes_are_known_without_reading_and_identical_bytes_are_told_apart() {
        let r = repo();
        write(r.path(), "a.bin", [1u8, 0, 2]);
        write(r.path(), "mode.sh", "echo\n");
        commit_all(r.path(), "1");
        write(r.path(), "a.bin", [1u8, 0, 2, 3, 4]);
        write(r.path(), "new.txt", "fresh\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(r.path().join("mode.sh"), std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let set = load(r.path(), git_spec(false, &[], &[], true));
        let all: Vec<_> = (0..set.comparisons.len())
            .map(|i| (set.comparisons[i].path.as_str(), set.sizes(i).unwrap(), set.identical(i).unwrap()))
            .collect();
        let mut expected = vec![("a.bin", [3, 5], false)];
        if cfg!(unix) {
            expected.push(("mode.sh", [5, 5], true));
        }
        expected.push(("new.txt", [0, 6], false));
        assert_eq!(all, expected);
        commit_all(r.path(), "2");
        let set = load(r.path(), git_spec(false, &["HEAD~1", "HEAD"], &[], false));
        assert_eq!((set.sizes(0).unwrap(), set.identical(0).unwrap()), ([3, 5], false));
    }

    #[test]
    fn paths_are_relative_to_where_lenz_runs_and_listed_from_the_root() {
        let r = repo();
        write(r.path(), "sub/file.txt", "a\n");
        write(r.path(), "file.txt", "a\n");
        commit_all(r.path(), "1");
        write(r.path(), "sub/file.txt", "b\n");
        write(r.path(), "file.txt", "b\n");
        write(r.path(), "sub/new.txt", "n\n");
        write(r.path(), "new.txt", "n\n");
        let sub = r.path().join("sub");
        let set = load(&sub, git_spec(false, &[], &["file.txt", "new.txt"], true));
        assert_eq!(listing(&set), [("sub/file.txt", Status::Modified), ("sub/new.txt", Status::Untracked)]);
        assert_eq!(texts(&set, 0), pair("a\n", "b\n"));
        assert_eq!(texts(&set, 1), pair("", "n\n"));
        // No paths: the whole repository, as git diff does from a subdirectory.
        assert_eq!(listing(&load(&sub, git_spec(false, &[], &[], true))).len(), 4);
    }

    #[test]
    fn comparisons_are_ordered_by_directory_then_file_name() {
        let r = repo();
        for p in ["b.txt", "a/z.txt", "a-c.txt", "a/b/c.txt", "A.txt"] {
            write(r.path(), p, "x\n");
        }
        let set = load(r.path(), git_spec(false, &[], &[], true));
        let paths: Vec<_> = set.comparisons.iter().map(|c| c.path.as_str()).collect();
        assert_eq!(paths, ["A.txt", "a-c.txt", "b.txt", "a/z.txt", "a/b/c.txt"]);
    }

    #[test]
    fn nothing_changed_is_an_empty_change_set() {
        let r = repo();
        write(r.path(), "a.txt", "a\n");
        commit_all(r.path(), "1");
        assert!(load(r.path(), git_spec(false, &[], &[], true)).comparisons.is_empty());
    }

    #[test]
    fn git_failures_are_errors_not_panics() {
        let outside = tempfile::tempdir().unwrap();
        let err = ChangeSet::load(&git_spec(false, &[], &[], true), outside.path()).err().unwrap();
        assert!(err.contains("not a git repository"), "{err}");
        let r = repo();
        write(r.path(), "a.txt", "a\n");
        commit_all(r.path(), "1");
        assert!(ChangeSet::load(&git_spec(false, &["no-such-rev"], &[], true), r.path()).is_err());
    }

    // --- plain ---

    fn plain(cwd: &Path, left: &str, right: &str) -> Result<ChangeSet, String> {
        ChangeSet::load(&Spec::Plain { left: left.into(), right: right.into() }, cwd)
    }

    #[test]
    fn two_files_are_one_comparison() {
        let d = tempfile::tempdir().unwrap();
        write(d.path(), "old.txt", "a\n");
        write(d.path(), "new.txt", "b\n");
        write(d.path(), "same.txt", "a\n");
        let set = plain(d.path(), "old.txt", "new.txt").unwrap();
        let c = &set.comparisons[0];
        assert_eq!((c.old_path.as_str(), c.path.as_str(), &c.status), ("old.txt", "new.txt", &Status::Modified));
        assert_eq!(texts(&set, 0), pair("a\n", "b\n"));
        assert!(plain(d.path(), "old.txt", "same.txt").unwrap().comparisons.is_empty());
        let set = plain(d.path(), "/dev/null", "new.txt").unwrap();
        assert_eq!(listing(&set), [("new.txt", Status::Added)]);
        assert_eq!(texts(&set, 0), pair("", "b\n"));
        let set = plain(d.path(), "old.txt", "/dev/null").unwrap();
        assert_eq!(listing(&set), [("old.txt", Status::Deleted)]);
    }

    #[test]
    fn directories_pair_files_by_relative_path() {
        let d = tempfile::tempdir().unwrap();
        write(d.path(), "l/same.txt", "s\n");
        write(d.path(), "r/same.txt", "s\n");
        write(d.path(), "l/sub/mod.txt", "1\n");
        write(d.path(), "r/sub/mod.txt", "2\n");
        write(d.path(), "l/del.txt", "d\n");
        write(d.path(), "r/sub/add.txt", "a\n");
        let set = plain(d.path(), "l", "r").unwrap();
        assert_eq!(
            listing(&set),
            [("del.txt", Status::Deleted), ("sub/add.txt", Status::Added), ("sub/mod.txt", Status::Modified)]
        );
        assert_eq!(texts(&set, 0), pair("d\n", ""));
        assert_eq!(texts(&set, 1), pair("", "a\n"));
        assert_eq!(texts(&set, 2), pair("1\n", "2\n"));
    }

    #[test]
    fn an_unborn_repository_against_the_empty_tree_shows_every_file_as_new() {
        let r = repo();
        write(r.path(), "a.txt", "a\n");
        let set = ChangeSet::load(&git_spec(false, &[crate::selection_title::EMPTY_TREE], &[], true), r.path()).unwrap();
        assert_eq!(listing(&set), [("a.txt", Status::Untracked)]);
    }

    #[test]
    fn folder_compare_lists_git_files_inside_a_work_tree() {
        let r = repo();
        write(r.path(), ".gitignore", "target/\n*.log\n");
        write(r.path(), "src/a.rs", "a\n");
        write(r.path(), "gone.txt", "g\n");
        commit_all(r.path(), "1");
        let other = r.path().join("wt");
        git(r.path(), &["worktree", "add", "-q", other.to_str().unwrap()]);
        write(r.path(), ".gitignore", "target/\n*.log\nwt/\n");
        for dir in [r.path(), other.as_path()] {
            write(dir, "target/debug/out", "built\n");
            write(dir, "run.log", "log\n");
        }
        write(&other, "src/a.rs", "b\n");
        write(&other, "new.txt", "n\n");
        std::fs::remove_file(other.join("gone.txt")).unwrap();
        git(&other, &["init", "-q", "nested"]);
        write(&other, "nested/z.txt", "z\n");
        let set = plain(r.path(), ".", "wt").unwrap();
        assert_eq!(
            listing(&set),
            [(".gitignore", Status::Modified), ("gone.txt", Status::Deleted), ("new.txt", Status::Added), ("src/a.rs", Status::Modified)]
        );
    }

    #[test]
    fn folder_compare_of_two_subfolders_of_one_repo() {
        let r = repo();
        write(r.path(), ".gitignore", "*.log\n");
        write(r.path(), "l/a.txt", "1\n");
        write(r.path(), "r/a.txt", "2\n");
        write(r.path(), "l/x.log", "x\n");
        commit_all(r.path(), "1");
        write(r.path(), "r/sub/new.txt", "n\n");
        let set = plain(&r.path().join("l"), ".", "../r").unwrap();
        assert_eq!(listing(&set), [("a.txt", Status::Modified), ("sub/new.txt", Status::Added)]);
    }

    #[test]
    fn an_ignored_folder_is_walked() {
        let r = repo();
        write(r.path(), ".gitignore", "target/\n");
        commit_all(r.path(), "1");
        write(r.path(), "target/a/out", "1\n");
        write(r.path(), "plain/a/out", "2\n");
        let set = plain(r.path(), "target", "plain").unwrap();
        assert_eq!(listing(&set), [("a/out", Status::Modified)]);
    }

    #[test]
    fn plain_folders_never_compare_dot_git() {
        let d = tempfile::tempdir().unwrap();
        write(d.path(), "l/.git", "gitdir: /somewhere\n");
        write(d.path(), "r/.git/config", "[core]\n");
        write(d.path(), "l/a.txt", "1\n");
        write(d.path(), "r/a.txt", "1\n");
        assert!(plain(d.path(), "l", "r").unwrap().comparisons.is_empty());
    }

    #[test]
    fn identical_files_at_different_paths_are_renames_but_edited_ones_are_not() {
        let d = tempfile::tempdir().unwrap();
        write(d.path(), "l/old/moved.txt", "same bytes\n");
        write(d.path(), "r/new/moved.txt", "same bytes\n");
        write(d.path(), "l/edited.txt", "before\n");
        write(d.path(), "r/edited2.txt", "after\n");
        write(d.path(), "l/empty.txt", "");
        write(d.path(), "r/empty2.txt", "");
        let set = plain(d.path(), "l", "r").unwrap();
        assert_eq!(
            listing(&set),
            [
                ("edited.txt", Status::Deleted),
                ("edited2.txt", Status::Added),
                ("empty.txt", Status::Deleted),
                ("empty2.txt", Status::Added),
                ("new/moved.txt", Status::Renamed { similarity: 100 }),
            ]
        );
        assert_eq!(set.comparisons[4].old_path, "old/moved.txt");
        assert_eq!(texts(&set, 4), pair("same bytes\n", "same bytes\n"));
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_files_are_followed_as_difftool_dir_diff_makes_them() {
        let d = tempfile::tempdir().unwrap();
        write(d.path(), "l/a.txt", "old\n");
        write(d.path(), "worktree/a.txt", "new\n");
        std::fs::create_dir(d.path().join("r")).unwrap();
        std::os::unix::fs::symlink(d.path().join("worktree/a.txt"), d.path().join("r/a.txt")).unwrap();
        let set = plain(d.path(), "l", "r").unwrap();
        assert_eq!(texts(&set, 0), pair("old\n", "new\n"));
    }

    #[cfg(unix)]
    #[test]
    fn directory_symlinks_are_skipped_without_losing_real_or_symlinked_files() {
        use std::os::unix::fs::symlink;
        let d = tempfile::tempdir().unwrap();
        write(d.path(), "l/changed.txt", "old\n");
        write(d.path(), "r/changed.txt", "new\n");
        write(d.path(), "l/linked.txt", "before\n");
        write(d.path(), "worktree/linked.txt", "after\n");
        write(d.path(), "outside/hidden.txt", "do not traverse\n");
        symlink(d.path().join("worktree/linked.txt"), d.path().join("r/linked.txt")).unwrap();
        for side in ["l", "r"] {
            symlink(d.path().join("outside"), d.path().join(side).join("directory-link")).unwrap();
            symlink(".", d.path().join(side).join("loop")).unwrap();
            let set = plain(d.path(), "l", "r").unwrap();
            assert_eq!(listing(&set), [("changed.txt", Status::Modified), ("linked.txt", Status::Modified)]);
            assert_eq!(texts(&set, 0), pair("old\n", "new\n"));
            assert_eq!(texts(&set, 1), pair("before\n", "after\n"));
            let reversed = plain(d.path(), "r", "l").unwrap();
            assert_eq!(listing(&reversed), listing(&set));
            assert_eq!(texts(&reversed, 1), pair("after\n", "before\n"));
        }
    }

    #[test]
    fn plain_files_are_binary_by_a_nul_alone() {
        let d = tempfile::tempdir().unwrap();
        write(d.path(), "l/.gitattributes", "* -diff\n");
        write(d.path(), "l/a.bin", [1u8, 0]);
        write(d.path(), "r/a.bin", [1u8, 0, 2]);
        write(d.path(), "l/b.txt", "b\n");
        write(d.path(), "r/b.txt", "bb\n");
        let set = plain(d.path(), "l", "r").unwrap();
        let got = verdicts(&set);
        assert_eq!(got[1..], [("a.bin", true, true, false, false), ("b.txt", false, false, false, false)]);
        assert_eq!((set.sizes(1).unwrap(), set.identical(1).unwrap()), ([2, 3], false));
    }

    #[test]
    fn missing_paths_and_a_file_against_a_directory_are_errors() {
        let d = tempfile::tempdir().unwrap();
        write(d.path(), "f.txt", "x");
        write(d.path(), "dir/g.txt", "y");
        assert!(plain(d.path(), "f.txt", "nope.txt").is_err());
        assert!(plain(d.path(), "f.txt", "dir").is_err());
    }

    // --- archives ---

    use crate::archive::{Compression, testing as archive_testing};

    fn paths(set: &ChangeSet) -> Vec<(&str, &str, Status)> {
        set.comparisons.iter().map(|c| (c.old_path.as_str(), c.path.as_str(), c.status.clone())).collect()
    }

    #[test]
    fn a_committed_zip_is_a_folder_of_its_changed_files() {
        let r = repo();
        let v1 = archive_testing::zip(&[("doc/a.txt", b"one\ntwo\n"), ("same.txt", b"same\n"), ("gone.txt", b"bye\n"), ("old.bin", b"moved\0")]);
        let v2 = archive_testing::zip(&[("doc/a.txt", b"one\n2\n"), ("same.txt", b"same\n"), ("new.txt", b"hi\n"), ("new.bin", b"moved\0")]);
        write(r.path(), "pkg/app.zip", &v1);
        write(r.path(), "z.txt", "z\n");
        commit_all(r.path(), "1");
        write(r.path(), "pkg/app.zip", &v2);
        write(r.path(), "z.txt", "zz\n");
        commit_all(r.path(), "2");
        let set = load(r.path(), git_spec(false, &["HEAD~", "HEAD"], &[], false));
        assert_eq!(
            paths(&set),
            [
                ("z.txt", "z.txt", Status::Modified),
                ("pkg/app.zip/gone.txt", "pkg/app.zip/gone.txt", Status::Deleted),
                ("pkg/app.zip/old.bin", "pkg/app.zip/new.bin", Status::Renamed { similarity: 100 }),
                ("pkg/app.zip/new.txt", "pkg/app.zip/new.txt", Status::Added),
                ("pkg/app.zip/doc/a.txt", "pkg/app.zip/doc/a.txt", Status::Modified),
            ]
        );
        let modified = set.comparisons.iter().position(|c| c.path.ends_with("doc/a.txt")).unwrap();
        assert_eq!(set.texts(modified).unwrap(), ("one\ntwo\n".into(), "one\n2\n".into()));
        assert_eq!(set.sizes(modified).unwrap(), [8, 6]);
        let [l, r] = set.sides(2).unwrap();
        assert!(l.binary && r.binary && l.bytes == b"moved\0");
        let c = &set.comparisons[modified];
        assert!(c.in_archive() && !set.comparisons[0].in_archive());
        assert_eq!(c.repository_path(), "pkg/app.zip");
        assert_eq!(set.comparisons[1].repository_path(), "pkg/app.zip", "a deleted member names the archive too");
        assert_eq!(crate::moves::editable(&c.right, &std::env::temp_dir()), Err("inside an archive"));
    }

    #[test]
    fn an_added_tarball_lists_every_file_and_an_untracked_one_stays_untracked() {
        let r = repo();
        write(r.path(), "keep.txt", "k\n");
        commit_all(r.path(), "1");
        let tgz = archive_testing::tar(&[("b.txt", b"b\n"), ("a/c.txt", b"c\n")], Compression::Gzip);
        write(r.path(), "rel.tgz", &tgz);
        let set = load(r.path(), git_spec(false, &[], &[], true));
        assert_eq!(
            paths(&set),
            [("rel.tgz/b.txt", "rel.tgz/b.txt", Status::Untracked), ("rel.tgz/a/c.txt", "rel.tgz/a/c.txt", Status::Untracked)]
        );
        assert_eq!(set.texts(1).unwrap(), (String::new(), "c\n".into()));
        let spec = git_spec(false, &[], &[], true);
        assert_eq!(crate::file_actions::discard_disabled(&set, 0, &spec), Some("Untracked file has no indexed version"));
        commit_all(r.path(), "2");
        let set = load(r.path(), git_spec(false, &["HEAD~", "HEAD"], &[], false));
        assert_eq!(set.comparisons.iter().map(|c| c.status.clone()).collect::<Vec<_>>(), [Status::Added, Status::Added]);
        git(r.path(), &["rm", "-q", "rel.tgz"]);
        let set = load(r.path(), git_spec(true, &[], &[], false));
        assert_eq!(paths(&set)[1], ("rel.tgz/a/c.txt", "rel.tgz/a/c.txt", Status::Deleted));
        assert_eq!(set.texts(1).unwrap(), ("c\n".into(), String::new()));
    }

    #[test]
    fn working_tree_archive_members_cannot_be_discarded_one_by_one() {
        let r = repo();
        write(r.path(), "a.tar", archive_testing::tar(&[("x.txt", b"1\n")], Compression::None));
        commit_all(r.path(), "1");
        write(r.path(), "a.tar", archive_testing::tar(&[("x.txt", b"2\n")], Compression::None));
        let spec = git_spec(false, &[], &[], true);
        let set = load(r.path(), spec.clone());
        assert_eq!(paths(&set), [("a.tar/x.txt", "a.tar/x.txt", Status::Modified)]);
        assert!(matches!(&set.comparisons[0].right, Content::Archived { archive, .. } if matches!(**archive, Content::File(_))));
        assert_eq!(set.texts(0).unwrap(), ("1\n".into(), "2\n".into()));
        assert_eq!(crate::file_actions::discard_disabled(&set, 0, &spec), Some("A file inside an archive cannot be discarded on its own"));
    }

    #[test]
    fn archives_inside_archives_open_as_folders_too() {
        let r = repo();
        let inner = |text: &[u8]| archive_testing::zip(&[("deep.txt", text)]);
        write(r.path(), "outer.tar.xz", archive_testing::tar(&[("lib/inner.zip", &inner(b"old\n"))], Compression::Xz));
        commit_all(r.path(), "1");
        write(r.path(), "outer.tar.xz", archive_testing::tar(&[("lib/inner.zip", &inner(b"new\n"))], Compression::Xz));
        commit_all(r.path(), "2");
        let set = load(r.path(), git_spec(false, &["HEAD~", "HEAD"], &[], false));
        assert_eq!(paths(&set), [("outer.tar.xz/lib/inner.zip/deep.txt", "outer.tar.xz/lib/inner.zip/deep.txt", Status::Modified)]);
        assert_eq!(set.texts(0).unwrap(), ("old\n".into(), "new\n".into()));
        assert_eq!(set.comparisons[0].repository_path(), "outer.tar.xz");
    }

    #[test]
    fn an_unreadable_or_unchanged_archive_stays_one_binary_file_with_a_note() {
        let r = repo();
        write(r.path(), "bad.zip", "not a zip\n");
        // same member content, different packing: stored, then deflated
        let stored = {
            use std::io::Write;
            let mut out = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
            out.start_file("a.txt", zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored)).unwrap();
            out.write_all(b"same\n").unwrap();
            out.finish().unwrap().into_inner()
        };
        write(r.path(), "repacked.zip", &stored);
        commit_all(r.path(), "1");
        write(r.path(), "bad.zip", "still not a zip\n");
        write(r.path(), "repacked.zip", archive_testing::zip(&[("a.txt", b"same\n")]));
        let set = load(r.path(), git_spec(false, &[], &[], true));
        assert_eq!(paths(&set), [("bad.zip", "bad.zip", Status::Modified), ("repacked.zip", "repacked.zip", Status::Modified)]);
        assert!(set.note(0).unwrap().starts_with("not shown as a folder: invalid zip archive"), "{:?}", set.note(0));
        assert_eq!(set.note(1), Some("no file inside the archive changed: only its packing differs"));
        assert!(!set.comparisons[0].in_archive());
        for index in 0..2 {
            let c = &set.comparisons[index];
            let doc = crate::document::PreparedDocument::load(&set, index, Path::new(&c.old_path), Path::new(&c.path), false).unwrap();
            assert!(doc.text.notes().iter().any(|note| Some(note.as_str()) == set.note(index)));
        }
    }

    #[test]
    fn staged_and_unstaged_archive_members_stay_in_their_groups() {
        let r = repo();
        write(r.path(), "a.zip", archive_testing::zip(&[("f.txt", b"1\n")]));
        write(r.path(), "b.txt", "b\n");
        commit_all(r.path(), "1");
        write(r.path(), "a.zip", archive_testing::zip(&[("f.txt", b"2\n")]));
        git(r.path(), &["add", "a.zip"]);
        write(r.path(), "a.zip", archive_testing::zip(&[("f.txt", b"3\n"), ("g.txt", b"g\n")]));
        write(r.path(), "b.txt", "bb\n");
        let set = crate::repo::local_changes(r.path(), false).unwrap();
        let names: Vec<_> = set.groups.iter().map(|(name, range)| (name.as_str(), range.clone())).collect();
        assert_eq!(names, [("Staged", 0..1), ("Unstaged", 1..4)]);
        assert_eq!(set.comparisons.iter().map(|c| c.path.as_str()).collect::<Vec<_>>(), ["a.zip/f.txt", "b.txt", "a.zip/f.txt", "a.zip/g.txt"]);
        assert_eq!(set.texts(0).unwrap(), ("1\n".into(), "2\n".into()));
        assert_eq!(set.texts(2).unwrap(), ("2\n".into(), "3\n".into()));
    }

    #[test]
    fn compressed_tar_streams_are_decompressed_once_and_at_most_two_are_kept() {
        let r = repo();
        let tgz = |files: &[(&str, &[u8])]| archive_testing::tar(files, Compression::Gzip);
        write(r.path(), "a.tgz", tgz(&[("1.txt", b"a\n"), ("2.txt", b"b\n")]));
        write(r.path(), "b.tgz", tgz(&[("1.txt", b"a\n")]));
        commit_all(r.path(), "1");
        write(r.path(), "a.tgz", tgz(&[("1.txt", b"A\n"), ("2.txt", b"B\n")]));
        write(r.path(), "b.tgz", tgz(&[("1.txt", b"A\n")]));
        commit_all(r.path(), "2");
        let set = load(r.path(), git_spec(false, &["HEAD~", "HEAD"], &[], false));
        assert_eq!(set.comparisons.iter().map(|c| c.path.as_str()).collect::<Vec<_>>(), ["a.tgz/1.txt", "a.tgz/2.txt", "b.tgz/1.txt"]);
        let kept = |set: &ChangeSet| set.unpacked.lock().unwrap().streams.iter().map(|(c, tar)| (c.clone(), tar.clone())).collect::<Vec<_>>();
        assert!(kept(&set).is_empty(), "listing reads archives whole, without keeping them");
        assert_eq!(set.texts(0).unwrap(), ("a\n".into(), "A\n".into()));
        let first = kept(&set);
        assert_eq!(first.len(), 2, "both Sides of a.tgz");
        // the next member of the same archives reads the kept streams
        assert_eq!(set.texts(1).unwrap(), ("b\n".into(), "B\n".into()));
        let again = kept(&set);
        assert!(first.iter().zip(&again).all(|((c1, t1), (c2, t2))| c1 == c2 && std::sync::Arc::ptr_eq(t1, t2)));
        // another archive replaces the oldest: never more than two
        assert_eq!(set.texts(2).unwrap(), ("a\n".into(), "A\n".into()));
        let after = kept(&set);
        assert_eq!(after.len(), 2);
        assert!(after.iter().all(|(c, _)| !first.iter().any(|(old, _)| old == c)), "b.tgz's Sides replaced a.tgz's");
    }

    #[test]
    fn plain_directories_open_their_archives_as_folders() {
        let d = tempfile::tempdir().unwrap();
        write(d.path(), "l/x.tar.bz2", archive_testing::tar(&[("in.txt", b"l\n")], Compression::Bzip2));
        write(d.path(), "r/x.tar.bz2", archive_testing::tar(&[("in.txt", b"r\n")], Compression::Bzip2));
        let set = plain(d.path(), "l", "r").unwrap();
        assert_eq!(paths(&set), [("x.tar.bz2/in.txt", "x.tar.bz2/in.txt", Status::Modified)]);
        assert_eq!(set.texts(0).unwrap(), ("l\n".into(), "r\n".into()));
        let set = plain(d.path(), "l/x.tar.bz2", "r/x.tar.bz2").unwrap();
        assert_eq!(paths(&set), [("l/x.tar.bz2/in.txt", "r/x.tar.bz2/in.txt", Status::Modified)]);
    }
    #[cfg(unix)]
    #[test]
    fn archive_reload_does_not_cache_a_read_started_before_reload() {
        use std::io::Write;
        let d = tempfile::tempdir().unwrap();
        let pipe = d.path().join("input.tgz");
        assert!(std::process::Command::new("mkfifo").arg(&pipe).status().unwrap().success());
        let set = std::sync::Arc::new(ChangeSet::load(&Spec::Empty, d.path()).unwrap());
        let member = Content::Archived { archive: Box::new(Content::File(pipe.clone())), kind: archive::Kind::Tar(Compression::Gzip), member: "f.txt".into() };
        let reader_set = set.clone(); let reader_member = member.clone();
        let reader = std::thread::spawn(move || reader_set.read(&reader_member).unwrap());
        // Opening the writer waits until the reader has checked the cache and started the file read.
        let mut writer = std::fs::OpenOptions::new().write(true).open(&pipe).unwrap();
        set.invalidate_archive_cache();
        writer.write_all(&archive_testing::tar(&[("f.txt", b"old\n")], Compression::Gzip)).unwrap();
        drop(writer);
        assert_eq!(reader.join().unwrap(), b"old\n");
        std::fs::remove_file(&pipe).unwrap();
        std::fs::write(&pipe, archive_testing::tar(&[("f.txt", b"updated\n")], Compression::Gzip)).unwrap();
        assert_eq!(set.read(&member).unwrap(), b"updated\n");
    }

}
