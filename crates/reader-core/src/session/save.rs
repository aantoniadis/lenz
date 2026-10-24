//! Explicit Save/recheck transitions. Adapters only dispatch these commands and present their state.
use super::*;
use crate::save::{self, DiskMatch, Outcome, Status};
use lenz_content::write::WriteOutcome;

impl Session {
    pub(super) fn save_disabled(&self, tab: TabId) -> Option<&'static str> {
        let Some(view) = self.state.view(tab) else { return Some("No loaded file is open") };
        if let Some(why) = view.edit_disabled() { return Some(why); }
        self.save_buffer_disabled(view.snapshot().key().buffer)
    }
    pub(super) fn save_buffer_disabled(&self, buffer: crate::text::BufferId) -> Option<&'static str> {
        let Some(entry) = self.state.sources.buffers.get(&buffer) else { return Some("No file is open") };
        let Some(source) = entry.snapshot.as_ref() else { return Some("Current source is unavailable") };
        if source.image().is_some() || source.source_meta().is_binary() || source.source_meta().is_lossy()
            || self.state.tabs.iter().any(|t| t.buffer == Some(buffer) && t.retrieved.is_some()) { return Some("This source is read-only"); }
        if entry.save.status.busy() { return Some("A save operation is running"); }
        if entry.save.status.blocked() { return Some("Recheck and confirm the disk outcome before saving again"); }
        let Some(read) = &entry.save.read else { return Some("This source has no confirmed write target; reopen the file before saving") };
        if read.snapshot.path != source.source() { return Some("The saved target path changed; recheck and confirm the current file before saving"); }
        if !entry.draft.as_ref().is_some_and(crate::draft::Draft::dirty) { return Some("No unsaved changes"); }
        None
    }
    pub(super) fn save(&mut self, tab: TabId) -> Effect {
        if let Some(why) = self.save_disabled(tab) { self.state.messages.push(why.into()); return Effect { repaint: true, copy: None }; }
        let Some(buffer) = self.state.tabs.iter().find(|t| t.id == tab).and_then(|t| t.buffer) else { return Effect::default() };
        self.save_buffer(buffer)
    }
    pub(super) fn save_buffer(&mut self, buffer: crate::text::BufferId) -> Effect {
        if let Some(why) = self.save_buffer_disabled(buffer) { self.state.messages.push(why.into()); return Effect { repaint: true, copy: None }; }
        let project = self.project_generation.load(Ordering::Acquire);
        self.save_seq += 1; let request = self.save_seq;
        let Some(entry) = self.state.sources.buffers.get_mut(&buffer) else { return Effect::default() };
        let (Some(snapshot), Some(read)) = (&entry.snapshot, &entry.save.read) else { return Effect::default() };
        if !self.analysis.request_write(buffer, request, project, || {
            let captured = snapshot.clone();
            let read = read.clone();
            (Some(crate::analysis::Input::new(captured, entry.document.clone(), project, false)), move |doc: Option<Arc<Document>>| {
                let Some(doc) = doc else { return Outcome::Rejected("Save source is unavailable; nothing was written".into()) };
                Outcome::Written(lenz_content::write::write_exact(&read.snapshot, &read.target, doc.text()))
            })
        }) { self.state.messages.push("Save admission is full; the draft is retained. Try Save explicitly later.".into()); return Effect { repaint: true, copy: None }; }
        if let Some(draft) = &mut entry.draft { draft.end_group(); }
        entry.save.status = Status::Saving; entry.save.checked = None; entry.save.revalidated = None;
        entry.save.captured = Some(save::Captured { request, text: snapshot.clone(), compacting: false });
        Effect { repaint: true, copy: None }
    }
    pub(super) fn recheck_save(&mut self, tab: TabId) -> Effect {
        let Some(view) = self.state.view(tab) else { return Effect::default() };
        if view.save.status.busy() || !view.write_path_changed() && !matches!(view.save.status, Status::Rejected(_) | Status::Uncertain(_) | Status::Checked(_)) { return Effect::default(); }
        let buffer = view.snapshot().key().buffer;
        let path = view.snapshot().source().to_owned();
        let project = self.project_generation.load(Ordering::Acquire);
        let versions = self.versions.clone();
        self.save_seq += 1; let request = self.save_seq;
        let Some(entry) = self.state.sources.buffers.get_mut(&buffer) else { return Effect::default() };
        let Some(text) = &entry.snapshot else { return Effect::default() };
        if !self.analysis.request_write(buffer, request, project, || (None, move |_: Option<Arc<Document>>| {
            let result = read(&path).map_err(|why| match why { ReadError::Failed(why) => why, ReadError::Unstable => "the file kept changing while rechecked".into() })
                .and_then(|(_, bytes)| {
                    let doc = decode_read(path, versions.fetch_add(1, Ordering::Relaxed) + 1, &bytes);
                    let baseline = doc.write_baseline.as_deref().ok_or_else(|| "The rechecked file has no writable exact baseline".to_owned())?.clone();
                    Ok((baseline, Arc::new(crate::text::TextSnapshot::from_document(buffer, &doc))))
                });
            Outcome::Rechecked(result)
        })) { self.state.messages.push("Recheck admission is full; try explicitly later".into()); return Effect { repaint: true, copy: None }; }
        entry.save.captured = Some(save::Captured { request, text: text.clone(), compacting: false });
        entry.save.checked = None; entry.save.revalidated = None; entry.save.status = Status::Rechecking;
        Effect { repaint: true, copy: None }
    }
    pub(super) fn confirm_rechecked_save(&mut self, tab: TabId) -> Effect {
        let Some(buffer) = self.state.tabs.iter().find(|t| t.id == tab).and_then(|t| t.buffer) else { return Effect::default() };
        let Some(entry) = self.state.sources.buffers.get_mut(&buffer) else { return Effect::default() };
        if !matches!(entry.save.status, Status::Checked(DiskMatch::Baseline | DiskMatch::Attempted)) { return Effect::default(); }
        let Some(checked) = entry.save.checked.take() else { return Effect::default() };
        self.saved_baseline(buffer, checked.text, checked.read);
        Effect { repaint: true, copy: None }
    }
    fn saved_baseline(&mut self, buffer: crate::text::BufferId, captured: Arc<crate::text::TextSnapshot>, read: save::Baseline) {
        let stamp = read.stamp();
        let Some(entry) = self.state.sources.buffers.get_mut(&buffer) else { return };
        if let (Some(draft), Some(current)) = (&mut entry.draft, &entry.snapshot) { draft.confirm_baseline(captured.clone(), current); }
        entry.saved = Some(SemanticSource::new(&captured));
        entry.stamp = Some(stamp); entry.save.read = Some(read); entry.save.status = Status::Ready;
        entry.save.attempted = None; entry.save.checked = None; entry.save.revalidated = None;
        for tab in self.state.tabs.iter_mut().filter(|tab| tab.buffer == Some(buffer)) {
            tab.stamp = Some(stamp); tab.stale_on_disk = false; tab.unavailable = None;
        }
        self.compact_saved_backing(buffer);
        self.synced = 0;
        self.refresh_blame();
        self.analysis.pump_requested(&self.jobs, &self.tx, &self.project_generation);
    }
    fn compact_saved_backing(&mut self, buffer: crate::text::BufferId) {
        let Some(entry) = self.state.sources.buffers.get_mut(&buffer) else { return };
        let (Some(current), Some(draft)) = (&entry.snapshot, &entry.draft) else { return };
        if !current.needs_compaction() && !draft.baseline.needs_compaction() { return; }
        let (current, baseline) = (current.clone(), draft.baseline.clone());
        self.save_seq += 1; let request = self.save_seq;
        let captured = current.clone();
        if self.analysis.request_compaction(buffer, request, || (None, move |_| {
            let same = Arc::ptr_eq(&current, &baseline);
            let current = Arc::new(current.compact_backing());
            let baseline = if same { current.clone() } else { Arc::new(baseline.compact_backing()) };
            Outcome::Compacted { current, baseline }
        })) {
            entry.save.status = Status::Saving;
            entry.save.captured = Some(save::Captured { request, text: captured, compacting: true });
        }
    }
    pub(super) fn account_saves(&mut self) -> bool {
        let outcomes = self.analysis.take_write_outcomes();
        let changed = !outcomes.is_empty();
        for (buffer, request, outcome) in outcomes {
            let Some(entry) = self.state.sources.buffers.get_mut(&buffer) else { continue };
            let Some(captured) = entry.save.captured.take_if(|captured| captured.request == request) else { continue };
            let captured = captured.text;
            match outcome {
                Outcome::Compacted { current, baseline } => {
                    if let Some(draft) = &mut entry.draft
                        && draft.baseline.key() == baseline.key() && Arc::ptr_eq(draft.baseline.source_meta(), baseline.source_meta())
                        && let Some(snapshot) = &entry.snapshot { draft.confirm_baseline(baseline, snapshot); }
                    if entry.snapshot.as_ref().is_some_and(|snapshot| snapshot.key() == current.key() && Arc::ptr_eq(snapshot.source_meta(), current.source_meta())) {
                        entry.snapshot = Some(current.clone());
                        self.analysis.rebind_backing(current, self.project_generation.load(Ordering::Acquire));
                    }
                    entry.save.status = Status::Ready;
                    self.compact_saved_backing(buffer);
                }
                Outcome::Written(WriteOutcome::Saved { snapshot, target }) => {
                    let read = save::Baseline { snapshot, target };
                    if entry.snapshot.as_ref().is_some_and(|current| Arc::ptr_eq(current.source_meta(), captured.source_meta())) {
                        entry.save.written = Some((request, SemanticSource::new(&captured)));
                        self.saved_baseline(buffer, captured, read);
                    } else {
                        entry.save.status = Status::Uncertain(format!("Captured Save completed at {}, but the current source identity changed. Recheck the current file before confirming its baseline.", read.snapshot.path.display()));
                        entry.save.read = Some(read); entry.save.attempted = Some(captured);
                    }
                },
                Outcome::Written(WriteOutcome::Rejected(why)) => entry.save.status = Status::Rejected(format!("Save rejected: {why}. The draft and history are retained.")),
                Outcome::Written(WriteOutcome::Uncertain(why)) => {
                    entry.save.attempted = Some(captured);
                    entry.save.status = Status::Uncertain(format!("Save outcome uncertain: {why}. Recheck disk before another explicit decision; the draft and history are retained."));
                }
                Outcome::Rejected(why) => entry.save.status = Status::Rejected(format!("Save rejected before writing: {why}")),
                Outcome::Uncertain(why) => { entry.save.attempted = Some(captured); entry.save.status = Status::Uncertain(why); }
                Outcome::Rechecked(Err(why)) => entry.save.status = Status::Uncertain(format!("Disk recheck failed: {why}. The confirmed baseline and draft are retained.")),
                Outcome::Rechecked(Ok((read, observed))) => {
                    entry.save.revalidated = (read.snapshot.path == captured.source()).then(|| save::Revalidated { request, source: SemanticSource::new(&captured) });
                    let baseline = entry.draft.as_ref().map(|d| d.baseline.clone()).or_else(|| entry.snapshot.clone());
                    let matched = if let Some(text) = entry.save.attempted.as_ref().filter(|text| text.content_eq(&observed)) {
                        Some((DiskMatch::Attempted, text.clone()))
                    } else { baseline.filter(|text| text.content_eq(&observed)).map(|text| (DiskMatch::Baseline, text)) };
                    match matched {
                        Some((kind, text)) => { entry.save.checked = Some(save::Checked { read, text }); entry.save.status = Status::Checked(kind); }
                        None => entry.save.status = Status::Checked(DiskMatch::Changed),
                    }
                }
            }
        }
        changed
    }
}
