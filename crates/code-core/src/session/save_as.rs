//! New untitled documents and Save As. Identity moves to the destination only after its write is confirmed.
use super::*;
use crate::save::{self, Outcome, ReplaceChoice, ReplacePrompt, Status};
use lenz_content::write::{WriteError, WriteOutcome};
use std::os::unix::fs::MetadataExt;

type FileIdentity = (u64, u64, u64, Option<std::time::SystemTime>);

impl Session {
    pub(super) fn new_document(&mut self) -> Effect {
        self.close_saved_comparison();
        self.untitled_seq += 1;
        let n = self.untitled_seq;
        let source = PathBuf::from(format!("{}{n}", crate::tabs::UNTITLED));
        let version = self.versions.fetch_add(1, Ordering::Relaxed) + 1;
        let doc = Arc::new(Document::retrieved(source.clone(), Path::new(""), version, ""));
        self.next_tab += 1;
        let id = TabId(self.next_tab);
        let mut tab = Tab::new(id, source, true);
        tab.pane = self.state.focused;
        tab.display = PathBuf::from(format!("Untitled-{n}"));
        tab.language = Some(None);
        tab.loaded = true;
        let at = self.state.active.and_then(|a| self.state.tabs.iter().position(|t| t.id == a)).map_or(self.state.tabs.len(), |i| i + 1);
        self.state.tabs.insert(at, tab);
        let from = self.here();
        self.admit_current(id, &doc, None);
        if let Some(entry) = self.state.tabs.iter().find(|t| t.id == id).and_then(|t| t.buffer).and_then(|b| self.state.sources.buffers.get_mut(&b)) {
            entry.untitled = true;
            entry.recovery = Some(format!("{}-{n}", self.recovery_token));
        }
        self.activate(id);
        if let Some(to) = self.here() { self.visit(from, to); }
        Effect { repaint: true, copy: None, ..Effect::default() }
    }

    pub(super) fn save_as_disabled(&self, tab: TabId) -> Option<&'static str> {
        let Some(view) = self.state.view(tab) else { return Some("No loaded file is open") };
        if let Some(why) = view.intrinsic_edit_disabled() { return Some(why); }
        if view.save.status.busy() { return Some("A save operation is running"); }
        if self.save_as_prompt().is_some() { return Some("Choose whether to replace the existing file first"); }
        None
    }

    pub(super) fn save_as(&mut self, tab: TabId, path: PathBuf) -> Effect {
        let effect = Effect { repaint: true, copy: None, ..Effect::default() };
        if let Some(why) = self.save_as_disabled(tab) { self.state.messages.push(why.into()); return effect; }
        match self.save_as_target(tab, &path) {
            Err(why) => self.state.messages.push(format!("Save As: {why}. Nothing was written.")),
            Ok((path, None)) => { self.close_saved_comparison(); self.submit_save_as(tab, path, None) }
            Ok((path, Some(file))) => {
                let Some(source) = self.state.source(tab).map(|s| SemanticSource::new(s)) else { return effect };
                self.save_seq += 1;
                self.replacing = Some(save::Replacing { ticket: self.save_seq, tab, source, path, file });
            }
        }
        effect
    }

    /// Validated before any write: an absolute path in an existing folder that no other open view holds.
    fn save_as_target(&self, tab: TabId, path: &Path) -> Result<(PathBuf, Option<FileIdentity>), String> {
        if !path.is_absolute() || is_virtual(path) { return Err("choose an absolute file path".into()); }
        let path = normalized(path);
        let Some(t) = self.state.tabs.iter().find(|t| t.id == tab) else { return Err("the document is closed".into()) };
        let canonical = std::fs::canonicalize(&path).ok();
        let names = |o: &Tab| o.source == path || o.canonical.as_ref() == Some(&path)
            || canonical.as_ref().is_some_and(|c| o.source == *c || o.canonical.as_ref() == Some(c));
        if names(t) { return Err("this is the document's current file; use Save".into()); }
        if let Some(other) = self.state.tabs.iter().find(|o| o.buffer != t.buffer && o.read_only_side().is_none() && names(o)) {
            return Err(format!("{} is open in another tab; close it first", other.display.display()));
        }
        if !path.parent().is_some_and(Path::is_dir) { return Err("its folder does not exist".into()); }
        match std::fs::symlink_metadata(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok((path, None)),
            Err(e) => Err(e.to_string()),
            Ok(_) => match std::fs::metadata(&path) {
                Ok(m) if m.is_file() => Ok((path, Some((m.dev(), m.ino(), m.len(), m.modified().ok())))),
                Ok(m) if m.is_dir() => Err("it is a folder".into()),
                Ok(_) => Err("it is not a regular file".into()),
                Err(_) => Err("it is a broken symbolic link".into()),
            },
        }
    }

    /// The pending Replace decision, while its document is unchanged.
    pub fn save_as_prompt(&self) -> Option<ReplacePrompt> {
        let pending = self.replacing.as_ref()?;
        self.state.source(pending.tab).filter(|s| pending.source.matches(s))?;
        Some(ReplacePrompt { ticket: pending.ticket, path: pending.path.clone() })
    }

    pub(super) fn choose_replace(&mut self, ticket: u64, choice: ReplaceChoice) -> Effect {
        if self.replacing.as_ref().is_none_or(|p| p.ticket != ticket) { return Effect::default(); }
        let current = self.save_as_prompt().is_some();
        let Some(pending) = self.replacing.take() else { return Effect::default() };
        match choice {
            ReplaceChoice::Cancel => {}
            ReplaceChoice::Replace if !current => self.state.messages.push("Save As cancelled: the document changed. Nothing was written.".into()),
            ReplaceChoice::Replace => match self.save_as_disabled(pending.tab) {
                Some(why) => self.state.messages.push(why.into()),
                None => { self.close_saved_comparison(); self.submit_save_as(pending.tab, pending.path, Some(pending.file)) }
            },
        }
        Effect { repaint: true, copy: None, ..Effect::default() }
    }

    fn submit_save_as(&mut self, tab: TabId, target: PathBuf, replace: Option<FileIdentity>) {
        let Some(buffer) = self.state.tabs.iter().find(|t| t.id == tab).and_then(|t| t.buffer) else { return };
        let project = self.project_generation.load(Ordering::Acquire);
        self.save_seq += 1;
        let request = self.save_seq;
        let Some(entry) = self.state.sources.buffers.get_mut(&buffer) else { return };
        let Some(snapshot) = entry.snapshot.clone() else { return };
        // The document's own encoding choice; an untitled document is UTF-8 without a BOM.
        let (encoding, bom) = entry.save.read.as_ref().map_or_else(
            || (snapshot.source_meta().original_encoding(), snapshot.source_meta().had_bom()),
            |read| (read.snapshot.encoding.name(), read.snapshot.bom));
        let path = target.clone();
        let document = entry.document.clone();
        if !self.analysis.request_write(buffer, request, project, || {
            (Some(crate::analysis::Input::new(snapshot.clone(), document, project, false)), move |doc: Option<Arc<Document>>| {
                let Some(doc) = doc else { return Outcome::Rejected("Save As source is unavailable".into()) };
                Outcome::Written(match replace {
                    None => lenz_content::write::create_exact(&path, encoding, bom, doc.text()),
                    Some(file) => replace_existing(&path, file, encoding, bom, doc.text()),
                })
            })
        }) {
            self.state.messages.push("Save As admission is full; nothing was written. Try again later.".into());
            return;
        }
        if let Some(draft) = &mut entry.draft { draft.end_group(); }
        let prior = std::mem::replace(&mut entry.save.status, Status::Saving);
        entry.save.captured = Some(save::Captured { request, text: snapshot, compacting: false, save_as: Some(save::SaveAs { path: target, prior }) });
    }

    /// A Save As outcome. Only a confirmed write moves the document; otherwise it keeps its original identity and state.
    pub(super) fn saved_as(&mut self, buffer: crate::text::BufferId, request: u64, captured: Arc<crate::text::TextSnapshot>, save_as: save::SaveAs, outcome: Outcome) {
        let Some(entry) = self.state.sources.buffers.get_mut(&buffer) else { return };
        let path = save_as.path.display().to_string();
        let message = match outcome {
            Outcome::Written(WriteOutcome::Saved { snapshot, target }) => {
                if !entry.snapshot.as_ref().is_some_and(|current| Arc::ptr_eq(current.source_meta(), captured.source_meta())) {
                    entry.save.status = save_as.prior;
                    self.state.messages.push(format!("Saved a copy as {path}, but the document changed identity meanwhile; it stays at its original location."));
                    return;
                }
                let canonical = target.canonical.clone();
                // The New File document's record is superseded by the file it now is.
                let untitled = self.recovery_key(buffer);
                self.retarget(buffer, &snapshot.path, &canonical);
                if let Some(key) = untitled { self.recovery.discard(key); }
                let Some(entry) = self.state.sources.buffers.get_mut(&buffer) else { return };
                let Some(meta) = entry.snapshot.as_ref().map(|s| s.source_meta().clone()) else { return };
                let captured = Arc::new(captured.with_source_meta(meta));
                entry.save.written = Some((request, SemanticSource::new(&captured)));
                self.saved_baseline(buffer, captured, save::Baseline { snapshot, target });
                return;
            }
            Outcome::Written(WriteOutcome::Rejected(why)) => format!("Save As rejected: {why}. Nothing was written; the document is unchanged."),
            Outcome::Rejected(why) => format!("Save As rejected: {why}. Nothing was written; the document is unchanged."),
            Outcome::Written(WriteOutcome::Uncertain(why)) => format!("Save As outcome uncertain: {why}. The document stays at its original location with its draft; inspect {path} before choosing it again."),
            Outcome::Uncertain(why) => format!("Save As outcome uncertain: {why}. The document stays at its original location with its draft; inspect {path} before choosing it again."),
            Outcome::Rechecked(_) | Outcome::Compacted { .. } => String::new(),
        };
        entry.save.status = save_as.prior;
        if !message.is_empty() { self.state.messages.push(message); }
    }

    /// Move every view of `buffer` to its confirmed new file. The original file and its places are left alone.
    fn retarget(&mut self, buffer: crate::text::BufferId, to: &Path, canonical: &Path) {
        let to = normalized(to);
        let language = self.state.sources.buffers.get(&buffer).and_then(|e| e.snapshot.as_ref()).and_then(|s| {
            let first = s.reader_line_range(0);
            highlight::detect(&to, &s.copy_range(first.start..first.end.min(first.start + 4096)).unwrap_or_default())
        });
        let mut from = Vec::new();
        for t in self.state.tabs.iter_mut().filter(|t| t.buffer == Some(buffer)) {
            from.push(std::mem::replace(&mut t.source, to.clone()));
            t.display = to.clone();
            t.canonical = Some(canonical.to_path_buf());
            t.language = Some(language);
            t.unavailable = None;
            t.new_content();
        }
        for path in &from {
            self.cache.take_any(path);
            if path.to_str().is_some_and(|p| p.starts_with(crate::tabs::UNTITLED)) { self.state.history.rename(path, &to); }
        }
        self.state.sources.aliases.retain(|_, b| *b != buffer);
        self.state.sources.aliases.insert(canonical.to_path_buf(), buffer);
        let Some(entry) = self.state.sources.buffers.get_mut(&buffer) else { return };
        entry.untitled = false;
        entry.recovery = None;
        entry.document = Default::default();
        if let Some(snapshot) = &entry.snapshot {
            let meta = snapshot.source_meta().renamed(to.clone()).with_canonical(Some(canonical.to_path_buf())).with_language(language);
            entry.snapshot = Some(Arc::new(snapshot.with_source_meta(Arc::new(meta))));
        }
        if let Some(snapshot) = entry.snapshot.clone() { self.analysis.invalidate(&snapshot, self.project_generation.load(Ordering::Relaxed)); }
        self.rewatch();
    }
}

/// Replace only the file that was offered for replacement, through the exact identity-checked writer.
fn replace_existing(path: &Path, file: FileIdentity, encoding: &'static str, bom: bool, text: &str) -> WriteOutcome {
    let raw = match lenz_content::write::read_exact_bytes(path) { Ok(raw) => raw, Err(e) => return WriteOutcome::Rejected(e) };
    let meta = raw.metadata();
    if (raw.target.device, raw.target.inode, meta.len(), meta.modified().ok()) != file { return WriteOutcome::Rejected(WriteError::ChangedOnDisk); }
    let snapshot = raw.snapshot(text, encoding, bom, false);
    lenz_content::write::write_exact(&snapshot, &raw.target, text)
}

impl Session {
    /// Where Save As starts and the name it suggests: the active file's folder and name, else the project root,
    /// else the working directory.
    pub fn save_as_start(&self) -> (PathBuf, Option<String>) {
        let file = self.state.active().filter(|t| t.is_file());
        let dir = file.and_then(|t| t.source.parent().map(Path::to_path_buf))
            .or_else(|| self.state.project.as_ref().map(|p| p.root().to_owned()))
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        (dir, file.and_then(|t| t.source.file_name()).map(|n| n.to_string_lossy().into_owned()))
    }
}
