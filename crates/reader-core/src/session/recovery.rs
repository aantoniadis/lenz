//! Recovering unsaved work after an interruption: the private records a draft keeps, the offer made at start and
//! the explicit Discard that keeps discarded work from returning. Restoring never writes the original file.
use super::*;
use crate::recovery::{Choice, Key, Offer, Prompt, Record, Version};

/// Why a recovered draft cannot be restored now.
pub(crate) const RESTORE_DIRTY: &str = "The file already has unsaved changes; save or discard them before restoring";
/// Why a file is not edited while its recovered draft waits for a decision: editing would replace that draft.
pub(crate) const UNDECIDED: &str = "A recovered draft of this file is undecided: choose Restore Draft or Discard Draft (Recover Unsaved Draft…) before editing";
/// Why recovered drafts wait: they restore into the project's last tabs once those are back.
pub(crate) const RESTORING: &str = "The project's last tabs are still being restored";
/// Why Save stays closed for a draft whose baseline no longer matches the file.
pub(crate) const DISK_CHANGED: &str = "Recovered draft: the file on disk differs from its confirmed baseline. Recheck the current file before saving";

impl Session {
    /// The recovered drafts read at start, newest first.
    pub fn recovered_drafts(&self) -> &[Offer] { self.recovery.offers() }

    /// Waits until the pending recoverable version is on disk: a replay of a restart needs no sleep.
    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn flush_recovery_for_test(&self) { self.recovery.flush(); }

    /// Writes what is pending and gives up this session's claim on its records, as an interruption would, while the
    /// window stays open: a restart replayed beside it offers this session's drafts.
    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn interrupt_recovery_for_test(&mut self) { self.recovery.end_ownership(); }

    /// The recovery dialog's offer: the newest undecided one, unless the user chose Later. It waits while the
    /// project's last tabs are restored, so a restored draft joins them instead of being replaced by them.
    pub fn recovery_prompt(&self) -> Option<Prompt> {
        if self.restore_workspace { return None; }
        let offer = self.recovery.prompt_offer()?;
        Some(offer.prompt(self.recovery.offers().len(), self.recovery_restore_disabled(offer)))
    }

    /// Reopen the dialog for a postponed recovery.
    pub(super) fn show_recovered_drafts(&mut self) -> Effect {
        let repaint = Effect { repaint: true, copy: None, ..Effect::default() };
        if self.recovery.offers().is_empty() { self.state.messages.push("No recovered drafts are offered".into()); return repaint; }
        self.recovery.resume();
        repaint
    }

    pub(super) fn choose_recovered_draft(&mut self, ticket: u64, choice: Choice) -> Effect {
        let repaint = Effect { repaint: true, copy: None, ..Effect::default() };
        if self.restore_workspace { return Effect::default(); }
        let Some(offer) = self.recovery.offer(ticket) else { return Effect::default() };
        let label = offer.record().title().to_owned();
        match choice {
            Choice::Later => self.recovery.postpone(),
            Choice::Discard => {
                let Some(offer) = self.recovery.take(ticket) else { return repaint };
                self.recovery.discard(offer.record().key().clone());
                self.state.undecided_recovery = self.recovery.undecided_paths();
                self.state.messages.push(format!("Discarded the recovered draft for {label}. It will not be offered again."));
            }
            Choice::Restore => {
                if let Some(why) = self.recovery_restore_disabled(offer) { self.state.messages.push(why.into()); return repaint; }
                let Some(offer) = self.recovery.take(ticket) else { return repaint };
                self.state.undecided_recovery = self.recovery.undecided_paths();
                return self.restore_recovered(offer);
            }
        }
        repaint
    }

    pub(super) fn recovery_restore_disabled(&self, offer: &Offer) -> Option<&'static str> {
        let path = offer.record().key().path()?;
        self.state.tabs.iter().any(|t| t.is_file() && (t.source == path || t.canonical.as_deref() == Some(path)) && self.state.tab_dirty(t.id))
            .then_some(RESTORE_DIRTY)
    }

    /// A buffer's recovery identity: its path, or a New File document's own token.
    pub(super) fn recovery_key(&self, buffer: crate::text::BufferId) -> Option<Key> {
        let entry = self.state.sources.buffers.get(&buffer)?;
        if entry.untitled { return Some(Key::Untitled { token: entry.recovery.clone()? }); }
        let source = entry.snapshot.as_ref().filter(|source| source.image().is_none() && !source.source_meta().is_binary() && !source.source_meta().is_lossy())?;
        (!is_virtual(source.source()) && source.source().is_absolute()).then(|| Key::File { path: normalized(source.source()) })
    }

    /// Keep the current draft recoverable, or forget it when it holds no unsaved work.
    pub(super) fn recovery_changed(&mut self, buffer: crate::text::BufferId) {
        let Some(key) = self.recovery_key(buffer) else { return };
        let Some(entry) = self.state.sources.buffers.get(&buffer) else { return };
        let (Some(draft), Some(current)) = (entry.draft.as_ref(), entry.snapshot.clone()) else {
            self.recovery.discard(key);
            return;
        };
        if !draft.dirty() {
            self.recovery.discard(key);
            return;
        }
        let baseline = draft.baseline.clone();
        let title = self.state.tabs.iter().find(|tab| tab.buffer == Some(buffer)).map_or_else(|| current.source().display().to_string(), Tab::title);
        let (encoding, bom) = (current.source_meta().original_encoding(), current.source_meta().had_bom());
        self.recovery.save(Version { key, title, draft: current, baseline, encoding, bom });
    }

    /// The buffer's work is saved, discarded or replaced from disk: it is not offered again, and no longer labelled
    /// as recovered.
    pub(super) fn forget_recovery(&mut self, buffer: crate::text::BufferId) {
        if let Some(key) = self.recovery_key(buffer) { self.recovery.discard(key); }
        for tab in self.state.tabs.iter_mut().filter(|tab| tab.buffer == Some(buffer)) { tab.recovered = false; }
    }

    fn restore_recovered(&mut self, offer: Offer) -> Effect {
        let record = offer.record().clone();
        let version = self.versions.fetch_add(1, Ordering::Relaxed) + 1;
        match record.key().clone() {
            Key::File { path } => self.restore_file_draft(&record, &path, version),
            Key::Untitled { token } => self.restore_untitled_draft(&record, &token, version),
        }
    }

    /// A file draft: its own baseline comes from the file only when the file still holds it; a changed or missing
    /// file leaves Save closed until the user rechecks and compares.
    fn restore_file_draft(&mut self, record: &Record, path: &Path, version: u64) -> Effect {
        let path = normalized(path);
        self.close_saved_comparison();
        let disk = read(&path).ok().map(|(stamp, exact)| (stamp, Arc::new(decode_read(path.clone(), version, &exact))));
        let matches = disk.as_ref().is_some_and(|(_, doc)| doc.image().is_none() && !doc.source_meta().is_lossy() && &**doc.text() == record.baseline());
        let baseline = match (&disk, matches) {
            (Some((_, doc)), true) => doc.clone(),
            _ => Arc::new(record_document(path.clone(), version, record.baseline(), record.encoding(), record.bom())),
        };
        let draft = Arc::new(Document::from_snapshot_parts(baseline.source_meta().clone(), version, record.draft().into()));
        // Tabs the project restored for this file keep their pane and place; the shown one (else the first) takes
        // the draft, and the others share its buffer.
        let same: Vec<TabId> = self.state.tabs.iter().filter(|t| t.is_file() && (t.source == path || t.canonical.as_deref() == Some(path.as_path()))).map(|t| t.id).collect();
        let existing = same.iter().copied().find(|id| self.is_shown(*id)).or_else(|| same.first().copied());
        let buffer = existing.and_then(|id| self.state.tabs.iter().find(|t| t.id == id).and_then(|t| t.buffer))
            .or_else(|| self.state.sources.aliases.get(&path).copied())
            .unwrap_or_else(|| { self.state.sources.next += 1; crate::text::BufferId(self.state.sources.next) });
        self.state.sources.aliases.insert(path.clone(), buffer);
        if let Some(canonical) = disk.as_ref().and_then(|(_, doc)| doc.source_meta().canonical()) { self.state.sources.aliases.insert(canonical.to_path_buf(), buffer); }
        let baseline_snapshot = Arc::new(crate::text::TextSnapshot::from_document(buffer, &baseline));
        let draft_snapshot = Arc::new(crate::text::TextSnapshot::from_document(buffer, &draft));
        let stamp = disk.as_ref().map(|(stamp, _)| *stamp);
        let entry = self.state.sources.buffers.entry(buffer).or_insert_with(|| CurrentSource {
            read_only: false, untitled: false, recovery: None, document: Default::default(), snapshot: None, stamp, saved: None, draft: None, save: Default::default(),
        });
        entry.read_only = false;
        entry.untitled = false;
        entry.recovery = None;
        entry.document = Arc::downgrade(&draft);
        entry.snapshot = Some(draft_snapshot.clone());
        entry.saved = Some(SemanticSource::new(&baseline_snapshot));
        entry.draft = Some(crate::draft::Draft::restored(baseline_snapshot));
        entry.save = crate::save::State::default();
        entry.stamp = stamp;
        if matches {
            if let Some((_, doc)) = &disk { entry.save.read = doc.write_baseline.as_deref().cloned(); }
        } else if disk.is_some() {
            entry.save.status = crate::save::Status::Rejected(DISK_CHANGED.into());
        }
        let language = draft.language();
        let canonical = disk.as_ref().and_then(|(_, doc)| doc.source_meta().canonical().map(Path::to_path_buf));
        let tab = match existing {
            Some(id) => id,
            None => self.insert_document_tab(path.clone(), path.clone(), Some(language), canonical),
        };
        let (len, last) = (draft_snapshot.len_bytes(), draft_snapshot.line_count() - 1);
        let clamp = |at: usize| draft_snapshot.floor_char_boundary(at.min(len));
        for id in std::iter::once(tab).chain(same.iter().copied().filter(|id| *id != tab)) {
            let token = self.next_refresh();
            let Some(t) = self.tab_mut(id) else { continue };
            t.recovered = true;
            if id != tab && !t.loaded { continue; }
            t.loaded = true;
            t.buffer = Some(buffer);
            t.language = Some(language);
            t.refresh = token;
            t.stamp = stamp;
            t.stale_on_disk = false;
            t.unavailable = None;
            t.new_content();
            // The place it had, within the draft: a restart resumes where the work was.
            let selection = t.selection().map(|s| Selection { anchor: clamp(s.anchor), head: clamp(s.head) });
            t.place_selection(selection);
            let top = t.top_line.min(last);
            t.set_top(top, t.top_row());
        }
        let from = self.here();
        self.activate(tab);
        if let Some(to) = self.here() { self.visit(from, to); }
        self.analysis.invalidate(&draft_snapshot, self.project_generation.load(Ordering::Relaxed));
        for id in std::iter::once(tab).chain(same.iter().copied().filter(|id| *id != tab)) {
            if self.is_shown(id) { self.mark_shown(id, draft_snapshot.key().version); self.decorate(id); }
        }
        self.synced = 0;
        self.release_current();
        let state = match (matches, disk.is_some()) {
            (true, _) => "The file on disk still matches its confirmed baseline.",
            (_, true) => "The file on disk has changed since its confirmed baseline; Save stays closed until you recheck and compare it.",
            _ => "The file is not on disk; Save As writes the draft elsewhere.",
        };
        self.state.messages.push(format!("Restored the recovered draft for {}. {state}", record.title()));
        Effect { repaint: true, copy: None, ..Effect::default() }
    }

    /// A New File draft: it keeps the record's identity, so further work updates the same record.
    fn restore_untitled_draft(&mut self, record: &Record, token: &str, version: u64) -> Effect {
        self.close_saved_comparison();
        self.untitled_seq += 1;
        let n = self.untitled_seq;
        let source = PathBuf::from(format!("{}{n}", crate::tabs::UNTITLED));
        let draft = Arc::new(Document::retrieved(source.clone(), Path::new(""), version, record.draft()));
        let empty = Arc::new(Document::retrieved(source.clone(), Path::new(""), version, ""));
        let id = self.insert_document_tab(source, PathBuf::from(format!("Untitled-{n}")), Some(None), None);
        let from = self.here();
        self.admit_current(id, &draft, None);
        let Some(buffer) = self.state.tabs.iter().find(|t| t.id == id).and_then(|t| t.buffer) else {
            return Effect { repaint: true, copy: None, ..Effect::default() };
        };
        let baseline = Arc::new(crate::text::TextSnapshot::from_document(buffer, &empty));
        if let Some(entry) = self.state.sources.buffers.get_mut(&buffer) {
            entry.untitled = true;
            entry.recovery = Some(token.to_owned());
            entry.saved = Some(SemanticSource::new(&baseline));
            entry.draft = Some(crate::draft::Draft::restored(baseline));
        }
        let token = self.next_refresh();
        if let Some(t) = self.tab_mut(id) {
            t.loaded = true;
            t.refresh = token;
            t.new_content();
            t.place_selection(Some(Selection { anchor: 0, head: 0 }));
            t.top_line = 0;
            t.recovered = true;
        }
        self.activate(id);
        if let Some(to) = self.here() { self.visit(from, to); }
        self.synced = 0;
        self.release_current();
        self.state.messages.push(format!("Restored the recovered New File draft {}. Save As chooses where it goes.", record.title()));
        Effect { repaint: true, copy: None, ..Effect::default() }
    }

    /// A retained tab for a restored document, after the active one.
    fn insert_document_tab(&mut self, source: PathBuf, display: PathBuf, language: Option<Option<Language>>, canonical: Option<PathBuf>) -> TabId {
        self.next_tab += 1;
        let id = TabId(self.next_tab);
        let mut tab = Tab::new(id, source, true);
        tab.pane = self.state.focused;
        tab.display = display;
        tab.canonical = canonical;
        tab.language = language;
        tab.loaded = true;
        let at = self.state.active.and_then(|a| self.state.tabs.iter().position(|t| t.id == a)).map_or(self.state.tabs.len(), |i| i + 1);
        self.state.tabs.insert(at, tab);
        id
    }
}

/// A document from a record's text and the encoding the document had: a legacy encoding survives recovery.
fn record_document(path: PathBuf, version: u64, text: &str, encoding: &'static str, bom: bool) -> Document {
    Document::from_decoded(path, version, lenz_content::encoding::Decoded { text: text.to_owned(), encoding, had_bom: bom, lossy: false })
}
