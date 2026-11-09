//! Renaming and moving a file or folder from the explorer. Every affected document is checked before anything moves,
//! the move never replaces another entry, and once it is confirmed each open tab, draft, history entry, place and
//! watch follows the entry to its new path. Nothing else is rewritten: references in other files stay as they were.
use super::*;
use crate::relocate::{self, Kind};

/// The entry being removed or moved, captured before its path disappears. A symlink entry only
/// affects its own spelling; a real entry also affects tabs opened through canonical aliases.
pub(super) struct EntryScope {
    source: PathBuf,
    canonical: Option<PathBuf>,
}
impl EntryScope {
    pub(super) fn new(source: &Path) -> Self {
        let canonical = std::fs::symlink_metadata(source).ok()
            .filter(|metadata| !metadata.file_type().is_symlink())
            .and_then(|_| std::fs::canonicalize(source).ok());
        Self { source: source.to_path_buf(), canonical }
    }
    pub(super) fn contains(&self, tab: &Tab) -> bool {
        crate::archive::physical_path(&tab.source).starts_with(&self.source) || self.canonical.as_deref().is_some_and(|source|
            tab.canonical.as_deref().is_some_and(|path| crate::archive::physical_path(path).starts_with(source)))
    }
    fn contains_path(&self, path: &Path) -> bool {
        let path = crate::archive::physical_path(path);
        path.starts_with(&self.source) || self.canonical.as_deref().is_some_and(|source|
            path.starts_with(source) || std::fs::canonicalize(&path).is_ok_and(|path| path.starts_with(source)))
    }
    fn moved_path(&self, tab: &Tab, target: &Path) -> Option<PathBuf> {
        let named = crate::archive::physical_path(&tab.source);
        let canonical = tab.canonical.as_deref().map(crate::archive::physical_path);
        let relative = named.strip_prefix(&self.source).ok().or_else(||
            canonical.as_deref()?.strip_prefix(self.canonical.as_deref()?).ok())?;
        let path = if relative.as_os_str().is_empty() { target.to_path_buf() } else { target.join(relative) };
        Some(if let Some(location) = crate::archive::Location::from_uri(&tab.source) {
            crate::archive::Location::new(path, location.members).ok()?.to_uri()
        } else { path })
    }
}

impl Session {
    /// The explorer's highlighted entry, project-relative: what Rename and Move act on. `None` when no folder is
    /// open or nothing but the root is highlighted.
    pub fn relocate_target(&self) -> Option<String> {
        let project = self.state.project.as_ref()?;
        let selected = self.state.explorer.selected.as_deref().filter(|s| !s.is_empty() && crate::create::is_relative(s))?;
        (project.has_file(selected) || project.has_dir(selected)).then(|| selected.to_string())
    }

    /// What the prompt starts with for `kind` of `from`.
    pub fn relocate_start(&self, kind: Kind, from: &str) -> String { relocate::start(kind, from) }

    /// Why `from` cannot be renamed or moved for the typed `input`: what both adapters' prompts show, and what the
    /// session itself checks again before anything moves. A taken destination is refused here, and the move itself
    /// never replaces an entry that appears after this check.
    pub fn relocate_refusal(&self, kind: Kind, from: &str, input: &str) -> Option<String> {
        let Some(project) = self.state.project.as_ref() else { return Some("No folder is open".into()) };
        let to = match relocate::destination(kind, from, input) { Ok(to) => to, Err(why) => return Some(why) };
        let (source, target) = (project.root().join(from), project.root().join(&to));
        match std::fs::symlink_metadata(&source) {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Some(format!("{from} is no longer there")),
            Err(e) => return Some(format!("{from} cannot be checked: {e}")),
        }
        match std::fs::symlink_metadata(&target) {
            Ok(_) => return Some(format!("{to} already exists")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Some(format!("{to} cannot be checked: {e}")),
        }
        match target.parent().map(std::fs::metadata) {
            Some(Ok(m)) if m.is_dir() => {}
            Some(Ok(_)) => return Some(format!("{} is not a folder", relocate::split(&to).0)),
            _ => return Some(format!("The folder {} does not exist", relocate::split(&to).0)),
        }
        self.relocate_blocked(&source)
    }

    /// Why a document at or inside `source` must not move now: its save is running or its outcome is unknown, or
    /// its recovered draft is still undecided. A draft that is simply unsaved moves with its file.
    pub(super) fn relocate_blocked(&self, source: &Path) -> Option<String> {
        let scope = EntryScope::new(source);
        for tab in self.state.tabs.iter().filter(|t| t.is_file() && scope.contains(t)) {
            let name = tab.title();
            let save = tab.buffer.and_then(|b| self.state.sources.buffers.get(&b)).map(|entry| &entry.save.status);
            if save.is_some_and(crate::save::Status::busy) { return Some(format!("{name} is being saved; wait for the save to finish")); }
            if save.is_some_and(crate::save::Status::blocked) { return Some(format!("{name} has a save whose outcome is not confirmed; recheck it first")); }
        }
        if self.state.undecided_recovery.iter().any(|path| scope.contains_path(path)) {
            return Some("A recovered draft inside it is undecided: restore or discard it first (Recover Unsaved Draft…)".into());
        }
        None
    }

    /// Move `from` (project-relative) for the typed `input`, then let every affected view follow it.
    pub(super) fn relocate_entry(&mut self, kind: Kind, from: String, input: String) -> Effect {
        let repaint = Effect { repaint: true, copy: None, ..Effect::default() };
        if let Some(why) = self.relocate_refusal(kind, &from, &input) {
            self.state.messages.push(format!("{}: {why}. Nothing was moved.", kind.label()));
            return repaint;
        }
        let (Some(root), Ok(to)) = (self.state.project.as_ref().map(|p| p.root().to_path_buf()), relocate::destination(kind, &from, &input)) else {
            return Effect::default();
        };
        let (source, target) = (root.join(&from), root.join(&to));
        let scope = EntryScope::new(&source);
        // Each open document's confirmed file, as it was before the move: only that same file is followed as it.
        let affected: Vec<(TabId, PathBuf)> = self.state.tabs.iter().filter(|t| t.retrieved.is_none() && !t.untitled())
            .filter_map(|t| Some((t.id, scope.moved_path(t, &target)?)))
            .collect();
        if let Err(e) = lenz_content::write::rename_noclobber(&source, &target) {
            let why = if e.kind() == std::io::ErrorKind::AlreadyExists { format!("{to} already exists") } else { e.to_string() };
            self.state.messages.push(format!("{}: {why}. Nothing was moved.", kind.label()));
            return repaint;
        }
        for (tab, path) in affected {
            let path = normalized(&path);
            let previous = self.state.tabs.iter().find(|item| item.id == tab).map(|tab| tab.source.clone());
            let token = self.next_refresh();
            if let Some(t) = self.tab_mut(tab) { t.refresh = token; } // a check started before the move is stale
            let canonical = match crate::archive::Location::from_uri(&path) {
                Some(location) => location.canonical_source().ok(),
                None => std::fs::canonicalize(&path).ok(),
            };
            self.follow_move(tab, path.clone(), Stamp::of(&path), canonical.clone());
            self.retarget_saved(tab, &path, canonical.as_deref());
            self.retarget_archive_saved(tab, &path);
            self.redetect_language(tab, &path);
            if let Some(previous) = previous.filter(|source| crate::archive::Location::from_uri(source).is_some()) {
                self.state.history.rename(&previous, &path);
            }
        }
        self.state.history.rename_tree(&source, &target);
        if Arc::make_mut(&mut self.state.places).rename_tree(&source, &target) { self.places_changed(); }
        self.archive_container_moved(&source, &target);
        if let Some(canonical) = scope.canonical.as_deref().filter(|canonical| *canonical != source)
            && let Ok(destination) = std::fs::canonicalize(&target) { self.archive_container_moved(canonical, &destination); }
        if let Some(p) = &mut self.state.project { p.rename(&from, &to); }
        self.state.tree_revision += 1;
        self.show_created(to.clone().into());
        self.synced = 0;
        self.rewatch();
        let (verb, what) = match kind { Kind::Rename => ("Renamed", relocate::split(&to).1.to_string()), Kind::Move => ("Moved", to.clone()) };
        self.state.messages.push(format!("{verb} {from} to {what}. References in other files were not changed."));
        repaint
    }

    /// A new name can mean another language (`a.rs` to `a.txt`): the document, its tabs and its analysis follow the
    /// name, as after Save As.
    fn redetect_language(&mut self, tab: TabId, path: &Path) {
        let Some(buffer) = self.state.tabs.iter().find(|t| t.id == tab).and_then(|t| t.buffer) else { return };
        let Some(snapshot) = self.state.sources.buffers.get(&buffer).and_then(|e| e.snapshot.clone()) else { return };
        let first = snapshot.reader_line_range(0);
        let name = snapshot.source_meta().archive().and_then(|location| location.member_name()).map(Path::new).unwrap_or(path);
        let language = highlight::detect(name, &snapshot.copy_range(first.start..first.end.min(first.start + 4096)).unwrap_or_default());
        if snapshot.language() == language { return; }
        let renamed = Arc::new(snapshot.with_source_meta(Arc::new((**snapshot.source_meta()).clone().with_language(language))));
        if let Some(entry) = self.state.sources.buffers.get_mut(&buffer) {
            let was_saved = entry.saved.as_ref().is_some_and(|saved| saved.matches(&snapshot));
            entry.snapshot = Some(renamed.clone());
            if was_saved { entry.saved = Some(SemanticSource::new(&renamed)); }
            entry.document = Default::default();
        }
        for t in self.state.tabs.iter_mut().filter(|t| t.buffer == Some(buffer)) {
            t.language = Some(language);
            t.new_content();
        }
        self.analysis.invalidate(&renamed, self.project_generation.load(Ordering::Relaxed));
        for id in self.state.tabs.iter().filter(|t| t.buffer == Some(buffer)).map(|t| t.id).collect::<Vec<_>>() {
            if self.is_shown(id) { self.decorate(id); }
        }
    }

    /// The tab's confirmed save target follows a move only when the file now at `path` is the one it confirmed.
    /// Otherwise Save stays closed until the file is rechecked, as after a move made elsewhere.
    fn retarget_saved(&mut self, tab: TabId, path: &Path, canonical: Option<&Path>) {
        let Some(buffer) = self.state.tabs.iter().find(|t| t.id == tab).and_then(|t| t.buffer) else { return };
        let Some(entry) = self.state.sources.buffers.get_mut(&buffer) else { return };
        let (Some(read), Some(canonical), Ok(meta)) = (entry.save.read.as_mut(), canonical, std::fs::metadata(path)) else { return };
        use std::os::unix::fs::MetadataExt;
        if (meta.dev(), meta.ino()) != (read.target.device, read.target.inode) { return; }
        read.snapshot.path = path.to_path_buf();
        read.target.canonical = canonical.to_path_buf();
    }
}
