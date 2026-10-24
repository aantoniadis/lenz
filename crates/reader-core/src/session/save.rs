//! Explicit Save/recheck transitions. Adapters only dispatch these commands and present their state.
use super::*;
use crate::save::{self, DiskMatch, Outcome, Status};
use lenz_content::write::WriteOutcome;

impl Session {
    pub(super) fn save_disabled(&self, tab: TabId) -> Option<&'static str> {
        let Some(view) = self.state.view(tab) else { return Some("No loaded file is open") };
        if let Some(why) = view.edit_disabled() { return Some(why); }
        if view.snapshot().source_meta().is_binary() { return Some("Binary files are read-only"); }
        if view.snapshot().source_meta().is_lossy() { return Some("Lossy text is read-only"); }
        if view.save_status().busy() { return Some("A save operation is running"); }
        if view.save_status().blocked() { return Some("Recheck and confirm the disk outcome before saving again"); }
        if view.save.read.is_none() { return Some("This source has no confirmed write target; reopen the file before saving"); }
        if !view.dirty() { return Some("No unsaved changes"); }
        None
    }
    pub(super) fn save(&mut self, tab: TabId) -> Effect {
        if let Some(why) = self.save_disabled(tab) { self.state.messages.push(why.into()); return Effect { repaint: true, copy: None }; }
        let Some(view) = self.state.view(tab) else { return Effect::default() };
        let buffer = view.snapshot().key().buffer;
        let read = view.save.read.clone().expect("eligible save baseline");
        let captured = view.snapshot().clone();
        let project = self.project_generation.load(Ordering::Acquire);
        let loaded = self.state.sources.buffers[&buffer].document.clone();
        self.save_seq += 1; let request = self.save_seq;
        if !self.analysis.request_write(buffer, request, project, || {
            (Some(crate::analysis::Input::new(captured.clone(), loaded, project, false)), move |doc: Option<Arc<Document>>| {
                let doc = doc.expect("write source is ready");
                Outcome::Written(lenz_content::write::write_exact(&read.snapshot, &read.target, doc.text()))
            })
        }) { self.state.messages.push("Save admission is full; the draft is retained. Try Save explicitly later.".into()); return Effect { repaint: true, copy: None }; }
        let entry = self.state.sources.buffers.get_mut(&buffer).expect("save buffer");
        if let Some(draft) = &mut entry.draft { draft.end_group(); }
        entry.save.status = Status::Saving; entry.save.checked = None;
        entry.save.captured = Some(save::Captured { request, text: captured });
        Effect { repaint: true, copy: None }
    }
    pub(super) fn recheck_save(&mut self, tab: TabId) -> Effect {
        let Some(view) = self.state.view(tab) else { return Effect::default() };
        if view.save.status.busy() || !matches!(view.save.status, Status::Rejected(_) | Status::Uncertain(_) | Status::Checked(_)) { return Effect::default(); }
        let buffer = view.snapshot().key().buffer;
        let path = view.snapshot().source().to_owned();
        let project = self.project_generation.load(Ordering::Acquire);
        let versions = self.versions.clone();
        self.save_seq += 1; let request = self.save_seq;
        if !self.analysis.request_write(buffer, request, project, || (None, move |_: Option<Arc<Document>>| {
            let result = read(&path).map_err(|why| match why { ReadError::Failed(why) => why, ReadError::Unstable => "the file kept changing while rechecked".into() })
                .map(|(_, bytes)| {
                    let doc = decode_read(path, versions.fetch_add(1, Ordering::Relaxed) + 1, &bytes);
                    let baseline = doc.write_baseline.as_deref().expect("exact recheck").clone();
                    (baseline, Arc::new(crate::text::TextSnapshot::from_document(buffer, &doc)))
                });
            Outcome::Rechecked(result)
        })) { self.state.messages.push("Recheck admission is full; try explicitly later".into()); return Effect { repaint: true, copy: None }; }
        let entry = self.state.sources.buffers.get_mut(&buffer).expect("recheck buffer");
        let text = entry.snapshot.as_ref().expect("current recheck source").clone();
        entry.save.captured = Some(save::Captured { request, text });
        entry.save.checked = None; entry.save.status = Status::Rechecking;
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
        entry.save.attempted = None; entry.save.checked = None;
        for tab in self.state.tabs.iter_mut().filter(|tab| tab.buffer == Some(buffer)) {
            tab.stamp = Some(stamp); tab.stale_on_disk = false; tab.unavailable = None;
        }
        self.synced = 0;
    }
    pub(super) fn account_saves(&mut self) -> bool {
        let outcomes = self.analysis.take_write_outcomes();
        let changed = !outcomes.is_empty();
        for (buffer, request, outcome) in outcomes {
            let Some(entry) = self.state.sources.buffers.get_mut(&buffer) else { continue };
            if !entry.save.captured.as_ref().is_some_and(|c| c.request == request) { continue; }
            let captured = entry.save.captured.take().expect("matching save request").text;
            match outcome {
                Outcome::Written(WriteOutcome::Saved { snapshot, target }) => self.saved_baseline(buffer, captured, save::Baseline { snapshot, target }),
                Outcome::Written(WriteOutcome::Rejected(why)) => entry.save.status = Status::Rejected(format!("Save rejected: {why}. The draft and history are retained.")),
                Outcome::Written(WriteOutcome::Uncertain(why)) => {
                    entry.save.attempted = Some(captured);
                    entry.save.status = Status::Uncertain(format!("Save outcome uncertain: {why}. Recheck disk before another explicit decision; the draft and history are retained."));
                }
                Outcome::Rejected(why) => entry.save.status = Status::Rejected(format!("Save rejected before writing: {why}")),
                Outcome::Uncertain(why) => { entry.save.attempted = Some(captured); entry.save.status = Status::Uncertain(why); }
                Outcome::Rechecked(Err(why)) => entry.save.status = Status::Uncertain(format!("Disk recheck failed: {why}. The confirmed baseline and draft are retained.")),
                Outcome::Rechecked(Ok((read, observed))) => {
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
