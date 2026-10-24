//! The external-change dialog: its identity, the file read behind it and the decisions it takes. Adapters only
//! dispatch its choices and present the sides; nothing here writes before a fresh disk check.
use super::*;
use crate::reconcile::{Check, Choice, Disk, DiskChange, Prompt, Side, NO_CONFLICT};

/// Whether a fresh read holds the same file and content the dialog showed; its own stamp is checked by the write.
fn same_file(fresh: &Disk, shown: &Disk) -> bool {
    fresh.text.content_eq(&shown.text)
        && fresh.baseline.target.device == shown.baseline.target.device
        && fresh.baseline.target.inode == shown.baseline.target.inode
}

/// The open dialog, as captured when it was opened.
pub(super) struct Pending {
    pub(super) ticket: u64,
    pub(super) tab: TabId,
    pub(super) buffer: crate::text::BufferId,
    pub(super) path: PathBuf,
    /// The project the dialog was opened in: another project's file is not this draft.
    pub(super) project: u64,
    /// The file on disk as read for this dialog, or why not; `None` while it is being read.
    pub(super) read: Option<Result<Disk, String>>,
    /// The comparison opened from this dialog is showing; the dialog returns when it closes.
    pub(super) comparing: bool,
}

/// Read a regular file as the exact writable text this session would write, for `buffer`.
fn read_disk(path: &Path, versions: &AtomicU64, buffer: crate::text::BufferId) -> Result<Disk, String> {
    let (_, bytes) = read(path).map_err(|why| match why {
        ReadError::Failed(why) => why,
        ReadError::Unstable => "the file kept changing while it was read".into(),
    })?;
    let doc = decode_read(path.to_path_buf(), versions.fetch_add(1, Ordering::Relaxed) + 1, &bytes);
    if doc.image().is_some() || doc.source_meta().is_binary() || doc.source_meta().is_lossy() {
        return Err("it is not writable text".into());
    }
    let baseline = doc.write_baseline.as_deref().ok_or_else(|| "it has no writable exact baseline".to_owned())?.clone();
    Ok(Disk { baseline, text: Arc::new(crate::text::TextSnapshot::from_document(buffer, &doc)) })
}

impl Session {
    /// Why Resolve External Change… cannot run now.
    pub(super) fn resolve_external_disabled(&self) -> Option<&'static str> {
        let Some(tab) = self.state.active() else { return Some("No file is open") };
        let Some(view) = self.state.view(tab.id) else { return Some("No file is open") };
        if tab.untitled() { return Some(UNTITLED_NO_FILE); }
        if let Some(why) = view.edit_disabled() { return Some(why); }
        if self.state.comparison.is_some() { return Some("Close the open comparison before resolving an external change"); }
        let entry = self.state.sources.buffers.get(&view.snapshot().key().buffer);
        let status = entry.map(|entry| &entry.save.status);
        let conflict = tab.stale_on_disk()
            || matches!(status, Some(crate::save::Status::Rejected(_) | crate::save::Status::Checked(crate::save::DiskMatch::Changed)))
            || view.write_path_changed();
        (!conflict).then_some(NO_CONFLICT)
    }

    /// Read the file on disk and show the three sides without deciding anything.
    pub(super) fn resolve_external_change(&mut self) -> Effect {
        let repaint = Effect { repaint: true, copy: None, ..Effect::default() };
        if let Some(why) = self.resolve_external_disabled() { self.state.messages.push(why.into()); return repaint; }
        let Some(tab) = self.state.active() else { return Effect::default() };
        let Some(current) = self.state.source(tab.id).cloned() else { return Effect::default() };
        let buffer = current.key().buffer;
        self.reconcile_seq += 1;
        let ticket = self.reconcile_seq;
        self.reconcile = Some(Pending {
            ticket, tab: tab.id, buffer, path: current.source().to_path_buf(),
            project: self.project_generation.load(Ordering::Acquire), read: None, comparing: false,
        });
        self.read_reconcile_disk(ticket, buffer, current.source().to_path_buf());
        repaint
    }

    /// The file is read off the UI thread; the dialog shows the sides once it lands.
    fn read_reconcile_disk(&mut self, ticket: u64, buffer: crate::text::BufferId, path: PathBuf) {
        let (tx, versions) = (self.tx.clone(), self.versions.clone());
        self.jobs.submit(Priority::Visible, move || {
            let result = read_disk(&path, &versions, buffer);
            let _ = tx.send_blocking(Completion::ReconcileRead { ticket, result });
        });
    }

    /// Whether the dialog still describes this tab, project and draft version.
    pub(super) fn reconcile_current(&self, pending: &Pending) -> bool {
        !self.closed && !self.state.tabs.is_empty()
            && pending.project == self.project_generation.load(Ordering::Acquire)
            && self.state.tabs.iter().any(|tab| tab.id == pending.tab && tab.buffer == Some(pending.buffer) && tab.source == pending.path)
            && self.state.source(pending.tab).is_some()
    }

    /// What both adapters present for the open dialog.
    pub fn reconcile_prompt(&self) -> Option<Prompt> {
        let pending = self.reconcile.as_ref().filter(|pending| !pending.comparing)?;
        if !self.reconcile_current(pending) { return None; }
        let current = self.state.source(pending.tab)?;
        let baseline = self.state.confirmed_baseline(current.key().buffer)?;
        let entry = self.state.sources.buffers.get(&pending.buffer)?;
        let (disk, read_failure, disk_change) = match &pending.read {
            None => (None, None, None),
            Some(Ok(disk)) => {
                let change = if disk.text.content_eq(baseline) { DiskChange::Baseline }
                    else if entry.save.attempted.as_ref().is_some_and(|attempted| attempted.content_eq(&disk.text)) { DiskChange::Attempted }
                    else { DiskChange::Other };
                (Some(Side::of(&disk.text)), None, Some(change))
            }
            Some(Err(why)) => (None, Some(why.clone()), None),
        };
        Some(Prompt {
            ticket: pending.ticket, path: pending.path.clone(), disk, read_failure, disk_change,
            draft: Side::of(current), baseline: Side::of(baseline),
        })
    }

    /// Apply one dialog's answer. A ticket from an older dialog, or one whose file, tab or project is gone, decides
    /// nothing, and every decision reads the file once more first: the draft, the history and the bytes are kept
    /// unless the version the dialog showed is still there.
    pub(super) fn choose_reconcile(&mut self, ticket: u64, choice: Choice) -> Effect {
        let repaint = Effect { repaint: true, copy: None, ..Effect::default() };
        let Some(pending) = self.reconcile.as_ref() else { return Effect::default() };
        if pending.ticket != ticket { return Effect::default(); }
        if !self.reconcile_current(pending) {
            self.reconcile = None;
            self.state.messages.push("The file or draft changed since the dialog opened; the draft is retained. Resolve the external change again to review it.".into());
            return repaint;
        }
        let Some(Ok(disk)) = pending.read.clone() else { return repaint };
        match choice {
            Choice::KeepDraft => {
                self.reconcile = None;
                self.state.messages.push("Kept the draft. The file on disk is unchanged and the new disk version is not its baseline.".into());
                repaint
            }
            Choice::TakeDisk | Choice::SaveDraft => {
                // The decision is taken against one more read of the file, off the UI thread.
                let (tx, versions) = (self.tx.clone(), self.versions.clone());
                let (path, buffer) = (pending.path.clone(), pending.buffer);
                self.jobs.submit(Priority::Visible, move || {
                    let result = match read_disk(&path, &versions, buffer) {
                        Ok(fresh) if same_file(&fresh, &disk) => Check::Same(fresh),
                        Ok(fresh) => Check::Changed(Box::new(fresh)),
                        Err(why) => Check::Failed(why),
                    };
                    let _ = tx.send_blocking(Completion::ReconcileChecked { ticket, choice, result });
                });
                repaint
            }
            Choice::Compare => {
                if self.state.comparison.is_some() { return repaint; }
                if let Some(pending) = self.reconcile.as_mut() { pending.comparing = true; }
                self.compare_conflict(&disk);
                repaint
            }
        }
    }

    /// The re-read for one decision: the same version applies it; another one decides nothing.
    pub(super) fn reconcile_checked(&mut self, ticket: u64, choice: Choice, result: Check) -> Effect {
        let repaint = Effect { repaint: true, copy: None, ..Effect::default() };
        let Some(pending) = self.reconcile.as_ref().filter(|pending| pending.ticket == ticket) else { return Effect::default() };
        if !self.reconcile_current(pending) {
            self.reconcile = None;
            self.state.messages.push("The file or draft changed since the dialog opened; the draft is retained and nothing was written.".into());
            return repaint;
        }
        match result {
            Check::Same(disk) => match choice {
                Choice::TakeDisk => self.take_disk(disk),
                Choice::SaveDraft => self.save_draft_over_disk(disk),
                Choice::Compare | Choice::KeepDraft => Effect::default(),
            },
            Check::Changed(fresh) => {
                if let Some(pending) = self.reconcile.as_mut() { pending.read = Some(Ok(*fresh)); }
                self.state.messages.push("The file changed again on disk while you decided; nothing was written. The sides were read again.".into());
                repaint
            }
            Check::Failed(why) => {
                if let Some(pending) = self.reconcile.as_mut() { pending.read = Some(Err(why.clone())); }
                self.state.messages.push(format!("The file could not be read: {why}. The draft is retained and nothing was written."));
                repaint
            }
        }
    }

    /// The file on disk becomes the draft's text and its confirmed baseline, as one undoable edit.
    fn take_disk(&mut self, disk: Disk) -> Effect {
        let repaint = Effect { repaint: true, copy: None, ..Effect::default() };
        let Some(pending) = self.reconcile.take() else { return Effect::default() };
        let Some(current) = self.state.source(pending.tab).cloned() else { return repaint };
        let mut effect = repaint;
        if !current.content_eq(&disk.text) {
            let Some(text) = disk.text.copy_range(0..disk.text.len_bytes()) else { return effect };
            let kind = crate::text::EditKind::Replace;
            effect = self.edit(pending.tab, current.key(), vec![crate::text::TextEdit { range: 0..current.len_bytes(), text: text.into() }], kind);
        }
        // Only the version now in the draft is confirmed: a take that could not be applied changes no baseline.
        if !self.state.source(pending.tab).is_some_and(|text| text.content_eq(&disk.text)) {
            self.state.messages.push("The disk version could not be put into the draft; the draft, its history and its baseline are unchanged.".into());
            return effect;
        }
        let taken = Arc::new(disk.text.with_buffer(pending.buffer));
        self.saved_baseline(pending.buffer, taken, disk.baseline);
        self.state.messages.push("Took the file on disk as the draft's text and confirmed baseline; Undo restores the previous draft.".into());
        effect
    }

    /// The draft is written over the version the dialog showed, after this decision's own check.
    fn save_draft_over_disk(&mut self, disk: Disk) -> Effect {
        let repaint = Effect { repaint: true, copy: None, ..Effect::default() };
        let Some(pending) = self.reconcile.take() else { return Effect::default() };
        if self.state.source(pending.tab).is_none() { return repaint; }
        let Some(entry) = self.state.sources.buffers.get_mut(&pending.buffer) else { return repaint };
        // The observed version becomes the write target only because the user chose it; the confirmed baseline
        // stays the last confirmed save until a write is confirmed.
        entry.save.read = Some(disk.baseline);
        entry.save.status = crate::save::Status::Ready;
        entry.save.checked = None;
        entry.save.revalidated = None;
        self.state.messages.push("Writing the draft over the file on disk; that version was checked again first.".into());
        let effect = self.save_buffer(pending.buffer);
        Effect { repaint: true, ..effect }
    }

    /// A read the dialog asked for: the sides, or why the file cannot be read.
    pub(super) fn reconcile_read(&mut self, ticket: u64, result: Result<Disk, String>) -> Effect {
        let Some(pending) = self.reconcile.as_mut().filter(|pending| pending.ticket == ticket) else { return Effect::default() };
        pending.read = Some(result);
        Effect { repaint: true, copy: None, ..Effect::default() }
    }
}
