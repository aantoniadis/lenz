//! Libraries reached through navigation. Only expanded directories are read; none enter the project index.
use crate::{project::TreeRow, semantics::VirtualSource};
use std::{collections::{BTreeMap, HashMap, HashSet}, path::{Path, PathBuf}, sync::{Arc, atomic::{AtomicBool, Ordering}}};

const HEADING: &str = "lenz://libraries";
const MAX_ENTRIES: usize = 20_000;
const MAX_DIRECTORY: usize = 2_000;
const MAX_LIBRARIES: usize = 128;

#[derive(Default)]
pub(crate) struct Libraries {
    roots: BTreeMap<PathBuf, String>,
    folders: BTreeMap<PathBuf, Vec<(PathBuf, bool)>>,
    virtuals: BTreeMap<Arc<str>, VirtualSource>,
    expanded: HashSet<Arc<str>>,
    pending: HashMap<PathBuf, (u64, Arc<AtomicBool>)>,
    sequence: u64,
    rows: Vec<TreeRow>,
}

/// Worker input. A reveal discovers a library root; expansion reads just one directory.
pub struct Request {
    id: u64,
    live: Arc<AtomicBool>,
    path: PathBuf,
    reveal: bool,
}
pub struct Read {
    request: Request,
    result: Result<Contents, String>,
}
enum Contents {
    Root(PathBuf, String),
    Directory(Vec<(PathBuf, bool)>, bool),
}

impl Request {
    pub(crate) fn read(self, current: impl Fn() -> bool) -> Read {
        let current = || self.live.load(Ordering::Relaxed) && current();
        let result = if !current() {
            Err("cancelled".into())
        } else if self.reveal {
            library_root(&self.path).map(|(root, name)| Contents::Root(root, name))
        } else {
            read_directory(&self.path, current).map(|(entries, limited)| Contents::Directory(entries, limited))
        };
        Read { request: self, result }
    }
}

impl Read {
    pub(crate) fn current(&self) -> bool { self.request.live.load(Ordering::Relaxed) }
}

impl Libraries {
    pub(crate) fn rows(&self) -> &[TreeRow] { &self.rows }

    pub(crate) fn contains(&self, path: &Path) -> bool {
        self.roots.keys().any(|root| path.starts_with(root)) || self.virtuals.contains_key(path.to_string_lossy().as_ref())
    }

    pub(crate) fn virtual_source(&self, path: &str) -> Option<&VirtualSource> { self.virtuals.get(path) }

    fn request(&mut self, path: PathBuf, reveal: bool) -> Option<Request> {
        if self.pending.contains_key(&path) || self.pending.len() >= 64 { return None; }
        self.sequence += 1;
        let live = Arc::new(AtomicBool::new(true));
        self.pending.insert(path.clone(), (self.sequence, live.clone()));
        Some(Request { id: self.sequence, live, path, reveal })
    }

    #[cold]
    pub(crate) fn reveal(&mut self, path: &Path, folders_first: bool) -> Vec<Request> {
        if let Some(source) = self.virtuals.get(path.to_string_lossy().as_ref()) {
            self.expanded.insert(virtual_group(source.provider).into());
            self.expanded.insert(HEADING.into());
            self.rebuild(folders_first);
            return Vec::new();
        }
        let root = self.roots.keys().filter(|root| path.starts_with(root)).max_by_key(|p| p.components().count()).cloned();
        let Some(root) = root else { return self.request(path.to_owned(), true).into_iter().collect() };
        self.expanded.insert(HEADING.into());
        if self.folders.values().map(Vec::len).sum::<usize>() + 32 > MAX_ENTRIES {
            self.folders.clear();
            for (_, live) in self.pending.values() { live.store(false, Ordering::Relaxed); }
            self.pending.clear();
            self.expanded.retain(|p| !Path::new(p.as_ref()).is_absolute());
        }
        let mut requests = Vec::new();
        // Seed the known path immediately; sibling listings arrive independently.
        let mut child = path.to_owned();
        while child != root {
            let Some(parent) = child.parent().filter(|p| p.starts_with(&root)).map(Path::to_owned) else { break };
            let entries = self.folders.entry(parent.clone()).or_default();
            if !entries.iter().any(|(p, _)| p == &child) { entries.push((child.clone(), child != path)); }
            let newly_open = self.expanded.insert(parent.to_string_lossy().as_ref().into());
            if newly_open && let Some(request) = self.request(parent.clone(), false) { requests.push(request); }
            child = parent;
        }
        self.rebuild(folders_first);
        requests
    }

    #[cold]
    pub(crate) fn add_virtual(&mut self, source: VirtualSource, folders_first: bool) {
        if self.virtuals.len() >= MAX_LIBRARIES && !self.virtuals.contains_key(&source.uri) { return; }
        self.expanded.insert(HEADING.into());
        self.expanded.insert(virtual_group(source.provider).into());
        self.virtuals.insert(source.uri.clone(), source);
        self.rebuild(folders_first);
    }

    #[cold]
    pub(crate) fn toggle(&mut self, key: &str, folders_first: bool) -> Vec<Request> {
        if !self.rows.iter().any(|r| r.path.as_ref() == key && r.dir.is_some()) { return Vec::new(); }
        let mut requests = Vec::new();
        if self.expanded.remove(key) {
            let path = Path::new(key);
            if path.is_absolute() {
                // Collapsing releases listings and invalidates their outstanding reads; re-opening refreshes them.
                self.folders.retain(|p, _| !p.starts_with(path));
                self.pending.retain(|p, (_, live)| {
                    let keep = !p.starts_with(path);
                    if !keep { live.store(false, Ordering::Relaxed); }
                    keep
                });
                self.expanded.retain(|p| !Path::new(p.as_ref()).starts_with(path));
            }
        } else {
            self.expanded.insert(key.into());
            if Path::new(key).is_absolute() && let Some(request) = self.request(key.into(), false) { requests.push(request); }
        }
        self.rebuild(folders_first);
        requests
    }

    #[cold]
    pub(crate) fn apply(&mut self, read: Read, folders_first: bool) -> (Vec<Request>, Option<String>, Option<PathBuf>) {
        let Read { request, result } = read;
        if self.pending.get(&request.path).map(|(id, _)| *id) != Some(request.id) { return (Vec::new(), None, None); }
        self.pending.remove(&request.path);
        let mut requests = Vec::new();
        let mut revealed = None;
        let message = match result {
            Ok(Contents::Root(root, name)) => {
                if self.roots.len() >= MAX_LIBRARIES && !self.roots.contains_key(&root) {
                    Some(format!("Libraries: showing the first {MAX_LIBRARIES} libraries"))
                } else {
                    let (root, name) = if request.path.strip_prefix(&root).is_ok_and(|p| p.components().count() > 32) {
                        let parent = request.path.parent().unwrap_or(&root).to_owned();
                        let name = parent.file_name().unwrap_or_default().to_string_lossy().into_owned();
                        (parent, name)
                    } else { (root, name) };
                    self.roots.insert(root, name);
                    requests = self.reveal(&request.path, folders_first);
                    revealed = Some(request.path);
                    None
                }
            }
            Ok(Contents::Directory(mut entries, limited)) => {
                let used = self.folders.iter().filter(|(p, _)| **p != request.path).map(|(_, entries)| entries.len()).sum::<usize>();
                let room = MAX_ENTRIES.saturating_sub(used);
                let limited = limited || entries.len() > room;
                // Keep the seeded definition path ahead of a capped directory listing.
                let known = self.folders.remove(&request.path).unwrap_or_default();
                let mut seen: HashSet<PathBuf> = known.iter().map(|(p, _)| p.clone()).collect();
                entries.retain(|(p, _)| seen.insert(p.clone()));
                let entries = known.into_iter().chain(entries).take(room).collect();
                self.folders.insert(request.path.clone(), entries);
                limited.then(|| format!("Libraries: {} is only partly listed (limits: {MAX_DIRECTORY} entries per folder, {MAX_ENTRIES} overall; non-Unicode names omitted); collapse unused folders to release space", request.path.display()))
            }
            Err(why) => Some(format!("Libraries: {}: {why}; collapse and expand to retry", request.path.display())),
        };
        self.rebuild(folders_first);
        (requests, message, revealed)
    }

    #[cold]
    pub(crate) fn rebuild(&mut self, folders_first: bool) {
        let mut rows = Vec::new();
        if self.roots.is_empty() && self.virtuals.is_empty() { self.rows = rows; return; }
        let open = self.expanded.contains(HEADING);
        rows.push(TreeRow { depth: 0, name: "Libraries".into(), path: HEADING.into(), dir: Some(open) });
        if open {
            for (root, name) in &self.roots { self.walk(root, name, true, 1, folders_first, &mut rows); }
            let providers: std::collections::BTreeSet<_> = self.virtuals.values().map(|s| s.provider).collect();
            for provider in providers {
                let key = virtual_group(provider);
                let open = self.expanded.contains(key.as_str());
                rows.push(TreeRow { depth: 1, name: format!("{provider} sources").into(), path: key.into(), dir: Some(open) });
                if open {
                    for source in self.virtuals.values().filter(|s| s.provider == provider) {
                        rows.push(TreeRow { depth: 2, name: source.name().into(), path: source.uri.clone(), dir: None });
                    }
                }
            }
        }
        self.rows = rows;
    }

    fn walk(&self, path: &Path, name: &str, dir: bool, depth: usize, folders_first: bool, rows: &mut Vec<TreeRow>) {
        let key: Arc<str> = path.to_string_lossy().as_ref().into();
        let open = self.expanded.contains(&key);
        let name = if self.pending.contains_key(path) { format!("{name} (loading…)").into() } else { name.into() };
        rows.push(TreeRow { depth, name, path: key, dir: dir.then_some(open) });
        if dir && open && let Some(entries) = self.folders.get(path) {
            let mut entries: Vec<_> = entries.iter().collect();
            entries.sort_unstable_by(|(a, ad), (b, bd)| (folders_first && !ad, a).cmp(&(folders_first && !bd, b)));
            for (path, dir) in entries { self.walk(path, &path.file_name().unwrap_or_default().to_string_lossy(), *dir, depth + 1, folders_first, rows); }
        }
    }
}

fn virtual_group(provider: &str) -> String { format!("{HEADING}/{provider}") }

fn read_directory(path: &Path, current: impl Fn() -> bool) -> Result<(Vec<(PathBuf, bool)>, bool), String> {
    let entries = std::fs::read_dir(path).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    let mut partial = false;
    for (i, entry) in entries.enumerate() {
        if !current() { return Err("cancelled".into()); }
        if i == MAX_DIRECTORY { return Ok((out, true)); }
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if path.to_str().is_none() { partial = true; continue; }
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        let dir = if kind.is_symlink() { path.is_dir() } else { kind.is_dir() };
        if dir || kind.is_file() || kind.is_symlink() { out.push((path, dir)); }
    }
    Ok((out, partial))
}

/// Runs off the UI thread. Prefer package/cache boundaries, then the nearest manifest, then the containing folder.
fn library_root(file: &Path) -> Result<(PathBuf, String), String> {
    let text = file.to_str().ok_or("non-Unicode library paths are not supported")?;
    #[cfg(windows)]
    let text = text.replace('\\', "/");
    let package = |marker: &str, segments: usize| -> Option<PathBuf> {
        let at = text.rfind(marker)? + marker.len();
        let mut end = at;
        for part in text.get(at..)?.split('/').take(segments) { end += part.len() + 1; }
        let root = PathBuf::from(text.get(..end.checked_sub(1)?)?);
        (root != file).then_some(root)
    };
    let named = |root: PathBuf| { let name = root.file_name().unwrap_or_default().to_string_lossy().into_owned(); (root, name) };
    if let Some(root) = package("/typeshed-fallback/", 1) { return Ok((root, "Python type stubs".into())); }
    for marker in ["/site-packages/", "/dist-packages/", "/gems/"] {
        if let Some(root) = package(marker, 1) { return Ok(named(root)); }
    }
    if let Some(at) = text.rfind("/node_modules/") {
        let scoped = text.get(at + "/node_modules/".len()..).is_some_and(|s| s.starts_with('@'));
        if let Some(root) = package("/node_modules/", if scoped { 2 } else { 1 }) {
            let (root, name) = named(root);
            let name = if scoped { format!("{}/{name}", root.parent().and_then(Path::file_name).unwrap_or_default().to_string_lossy()) } else { name };
            return Ok((root, name));
        }
    }
    if let Some(root) = package("/.cargo/registry/src/", 2) { return Ok(named(root)); }
    if let Some(root) = package("/rustlib/src/rust/", 1) { return Ok((root, "Rust standard library".into())); }
    for parent in file.ancestors().skip(1).take(32) {
        let name = parent.file_name().unwrap_or_default().to_string_lossy();
        if name.starts_with("python3.") { return Ok((parent.to_owned(), format!("Python {name} standard library"))); }
        if parent.parent().is_some_and(|p| p.ends_with("lib/ruby")) && name.starts_with("3.") { return Ok((parent.to_owned(), format!("Ruby {name} standard library"))); }
        if ["Cargo.toml", "package.json", "pyproject.toml", "go.mod", "Gemfile", "pom.xml", "build.gradle", "build.gradle.kts", "CMakeLists.txt"].iter().any(|manifest| parent.join(manifest).is_file()) {
            return Ok(named(parent.to_owned()));
        }
    }
    // shortcut: unfamiliar layouts group by containing folder; add a package boundary when a provider needs one.
    file.parent().filter(|p| p.parent().is_some()).map(|p| named(p.to_owned())).ok_or_else(|| "no library folder".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_roots_follow_package_boundaries_and_standard_libraries() {
        for (file, root) in [
            ("/home/me/.cargo/registry/src/index/serde-1.0/src/lib.rs", "/home/me/.cargo/registry/src/index/serde-1.0"),
            ("/app/vendor/bundle/ruby/3.3.0/gems/rails-8.0/lib/rails.rb", "/app/vendor/bundle/ruby/3.3.0/gems/rails-8.0"),
            ("/usr/lib/ruby/3.3.0/json/common.rb", "/usr/lib/ruby/3.3.0"),
            ("/app/node_modules/@types/react/index.d.ts", "/app/node_modules/@types/react"),
            ("/app/node_modules/.pnpm/foo/node_modules/foo/index.js", "/app/node_modules/.pnpm/foo/node_modules/foo"),
            ("/venv/lib/python3.13/site-packages/numpy/core/__init__.py", "/venv/lib/python3.13/site-packages/numpy"),
            ("/usr/lib/python3.13/collections/__init__.py", "/usr/lib/python3.13"),
            ("/rust/rustlib/src/rust/library/std/src/lib.rs", "/rust/rustlib/src/rust/library"),
        ] {
            assert_eq!(library_root(Path::new(file)).unwrap().0, Path::new(root), "{file}");
        }
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "").unwrap();
        assert_eq!(library_root(&dir.path().join("src/inner/lib.rs")).unwrap().0, dir.path());
    }

    #[cfg(unix)]
    #[test]
    fn non_unicode_entries_cannot_alias_a_displayable_file() {
        use std::os::unix::ffi::OsStringExt;
        let dir = tempfile::tempdir().unwrap();
        let bad = dir.path().join(std::ffi::OsString::from_vec(vec![0xff]));
        let good = dir.path().join("�");
        for path in [&bad, &good] { std::fs::write(path, "").unwrap(); }
        let (entries, partial) = read_directory(dir.path(), || true).unwrap();
        assert!(partial);
        assert_eq!(entries, vec![(good, false)]);
        assert!(library_root(&bad).is_err());
    }

    fn drain(tree: &mut Libraries, requests: Vec<Request>) {
        for request in requests {
            let (next, message, _) = tree.apply(request.read(|| true), true);
            assert!(message.is_none(), "{message:?}");
            drain(tree, next);
        }
    }

    #[test]
    fn library_tree_reads_only_expanded_folders_rejects_stale_reads_and_refreshes() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("node_modules/pkg");
        std::fs::create_dir_all(root.join("src/deep")).unwrap();
        std::fs::create_dir_all(root.join("unopened")).unwrap();
        let file = root.join("src/target.ts");
        std::fs::write(&file, "target").unwrap();
        std::fs::write(root.join("unopened/secret.ts"), "unvisited").unwrap();
        let mut tree = Libraries::default();
        assert!(tree.rows().is_empty());
        let requests = tree.reveal(&file, true);
        drain(&mut tree, requests);
        assert!(tree.rows().iter().any(|r| r.path.as_ref() == file.to_str().unwrap()));
        assert!(!tree.folders.contains_key(&root.join("unopened")));
        let key = root.join("unopened").to_string_lossy().into_owned();
        let request = tree.toggle(&key, true).pop().unwrap();
        tree.toggle(&key, true);
        let cancelled = request.read(|| true);
        assert!(matches!(&cancelled.result, Err(why) if why == "cancelled"), "a collapsed folder's queued work does no I/O");
        tree.apply(cancelled, true);
        assert!(!tree.folders.contains_key(Path::new(&key)), "late results cannot reopen a collapsed folder");
        let finished = tree.toggle(&key, true).pop().unwrap().read(|| true);
        tree.toggle(&key, true);
        let requests = tree.toggle(&key, true);
        tree.apply(finished, true);
        assert!(!tree.folders.contains_key(Path::new(&key)), "a previous expansion's result cannot replace the current request");
        drain(&mut tree, requests);
        assert!(tree.rows().iter().any(|r| r.name.as_ref() == "secret.ts"));
        tree.toggle(&key, true);
        std::fs::rename(root.join("unopened/secret.ts"), root.join("unopened/renamed.ts")).unwrap();
        let requests = tree.toggle(&key, true);
        drain(&mut tree, requests);
        assert!(tree.rows().iter().any(|r| r.name.as_ref() == "renamed.ts"));
        assert!(!tree.rows().iter().any(|r| r.name.as_ref() == "secret.ts"));
        tree.toggle(root.to_str().unwrap(), true);
        assert!(tree.folders.is_empty(), "collapsing a library releases its listings");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "target");
    }

    #[test]
    fn library_listing_limits_and_errors_do_not_hide_the_definition() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("node_modules/pkg");
        std::fs::create_dir_all(&root).unwrap();
        for i in 0..MAX_DIRECTORY + 10 { std::fs::write(root.join(format!("{i:05}.ts")), "").unwrap(); }
        let file = root.join("zz-target.ts");
        std::fs::write(&file, "").unwrap();
        let mut tree = Libraries::default();
        let discover = tree.reveal(&file, true).pop().unwrap();
        let (mut requests, _, _) = tree.apply(discover.read(|| true), true);
        let (_, message, _) = tree.apply(requests.pop().unwrap().read(|| true), true);
        assert!(message.unwrap().contains("partly listed"));
        assert!(tree.rows().iter().any(|r| r.path.as_ref() == file.to_str().unwrap()));
        assert!(tree.folders.values().map(Vec::len).sum::<usize>() <= MAX_ENTRIES);
        tree.toggle(root.to_str().unwrap(), true);
        std::fs::remove_dir_all(&root).unwrap();
        let read = tree.toggle(root.to_str().unwrap(), true).pop().unwrap().read(|| true);
        let (_, message, _) = tree.apply(read, true);
        assert!(message.unwrap().contains("retry"));
    }
}

#[cfg(test)]
mod performance {
    use super::*;
    #[test]
    #[ignore = "release workload, run alone"]
    fn library_browsing_perf() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("node_modules/package");
        std::fs::create_dir_all(root.join("unopened/deep")).unwrap();
        for i in 0..1000 { std::fs::write(root.join(format!("file{i:04}.ts")), "export const item = 1;\n").unwrap(); }
        for i in 0..10_000 { std::fs::write(root.join(format!("unopened/deep/file{i:05}.ts")), "").unwrap(); }
        let target = root.join("file0999.ts");
        let mut tree = Libraries::default();
        let started = std::time::Instant::now();
        let mut requests = tree.reveal(&target, true);
        while let Some(request) = requests.pop() {
            let (next, message, _) = tree.apply(request.read(|| true), true);
            assert!(message.is_none()); requests.extend(next);
        }
        let reveal_ms = started.elapsed().as_secs_f64() * 1000.;
        assert_eq!(tree.rows().len(), 1003);
        assert_eq!(tree.folders.len(), 1, "the 10,000-file subtree remains unread");
        let memory = || -> Option<f64> {
            let status = std::fs::read_to_string("/proc/self/status").ok()?;
            status.lines().find_map(|s| s.strip_prefix("VmRSS:"))?.split_whitespace().next()?.parse::<f64>().ok().map(|kib| kib / 1024.)
        };
        let baseline = memory();
        let started = std::time::Instant::now();
        let mut samples = Vec::new();
        let mut peak = baseline.unwrap_or_default();
        while started.elapsed().as_secs() < 10 {
            let tick = std::time::Instant::now();
            tree.toggle(root.to_str().unwrap(), true);
            assert!(tree.folders.is_empty());
            let request = tree.toggle(root.to_str().unwrap(), true).pop().unwrap();
            let (_, message, _) = tree.apply(request.read(|| true), true);
            assert!(message.is_none());
            assert_eq!(tree.rows().len(), 1003);
            samples.push(tick.elapsed().as_secs_f64() * 1000.);
            if samples.len().is_multiple_of(100) { peak = peak.max(memory().unwrap_or_default()); }
        }
        let duration = started.elapsed().as_secs_f64();
        samples.sort_by(f64::total_cmp);
        println!("{}", serde_json::json!({"reveal_ms":reveal_ms,"cycles":samples.len(),"duration_seconds":duration,"median_ms":samples[samples.len()/2],"p95_ms":samples[samples.len()*95/100],"baseline_mib":baseline,"settled_mib":memory(),"sampled_peak_mib":peak,"samples_ms":samples}));
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "export const item = 1;\n");
    }
}
