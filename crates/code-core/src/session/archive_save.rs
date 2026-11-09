//! Archive members use ordinary drafts and capture receipts, with one serialized writer per outer
//! container. No member text is ever sent to the disk-text writer.
use super::*;
use crate::archive::{Baseline, ContainerSnapshot, Location, ReadEntry};
use std::collections::{HashMap, VecDeque};
use lenz_content::write::WriteOutcome;

#[derive(Default)]
pub(super) struct Saves { lanes: HashMap<PathBuf, Lane> }
#[derive(Default)]
struct Lane {
    running: Option<(crate::text::BufferId, u64)>,
    pending: VecDeque<(crate::text::BufferId, u64)>,
    uncertain: bool,
}
impl Saves {
    pub(super) fn clear_uncertain(&mut self, path: &Path) {
        if let Some(lane) = self.lanes.get_mut(path) { lane.uncertain = false; }
    }
}
pub struct Loaded {
    request: u64,
    tab: TabId,
    path: PathBuf,
    result: Result<ReadEntry, String>,
}
pub struct Saved {
    path: PathBuf,
    buffer: crate::text::BufferId,
    request: u64,
    previous: Arc<Baseline>,
    outcome: Result<Arc<ContainerSnapshot>, Failure>,
}
enum Failure { Rejected(String), Uncertain(String) }
pub struct Rechecked {
    buffer: crate::text::BufferId,
    request: u64,
    result: Result<ReadEntry, String>,
}
pub(crate) struct Checked { pub text: Arc<crate::text::TextSnapshot>, pub baseline: Arc<Baseline> }

impl Session {
    pub(super) fn archive_requires_recheck(&self, buffer: crate::text::BufferId) -> bool {
        self.state.sources.buffers.get(&buffer).and_then(|entry| entry.save.archive.as_ref())
            .is_some_and(|baseline| self.archive_saves.lanes.get(&baseline.location.container).is_some_and(|lane| lane.uncertain))
    }
    pub(super) fn retarget_archive_saved(&mut self, tab: TabId, path: &Path) {
        let Some(location) = Location::from_uri(path) else { return };
        let Some(buffer) = self.state.tabs.iter().find(|item| item.id == tab).and_then(|tab| tab.buffer) else { return };
        let Some(entry) = self.state.sources.buffers.get_mut(&buffer) else { return };
        let Some(previous) = &entry.save.archive else { return };
        let Some(stamp) = Stamp::of(&location.container) else { return };
        if stamp.file != Some((previous.container.target.device, previous.container.target.inode)) { return; }
        let Ok(canonical) = std::fs::canonicalize(&location.container) else { return };
        let mut snapshot = previous.container.snapshot.clone();
        snapshot.path = location.container;
        let mut target = previous.container.target.clone();
        target.canonical = canonical.clone();
        entry.save.archive = Some(Arc::new(Baseline {
            location: Location { container: canonical, members: location.members },
            container: Arc::new(ContainerSnapshot { snapshot, target, fingerprint: previous.container.fingerprint }),
            crlf: previous.crlf, encoding: previous.encoding, bom: previous.bom,
        }));
    }
    pub(super) fn load_archive_entry(&mut self, id: TabId, reload: bool, location: Location) {
        let request = self.next_request();
        let path = self.state.tabs.iter().find(|tab| tab.id == id).map_or_else(|| location.to_uri(), |tab| tab.source.clone());
        self.pending = Some((id, reload));
        self.reload_source = reload.then(|| self.state.source(id).map(|source| SemanticSource::new(source))).flatten();
        self.state.loading = Some(path.clone());
        let (latest, versions, tx) = (self.latest.clone(), self.versions.clone(), self.tx.clone());
        self.jobs.submit(Priority::Visible, move || {
            let cancel = || latest.load(Ordering::Relaxed) != request;
            if cancel() { return; }
            let result = crate::archive::read_entry(&location, versions.fetch_add(1, Ordering::Relaxed) + 1, &cancel);
            let _ = tx.send_blocking(Completion::ArchiveLoaded(Box::new(Loaded { request, tab: id, path, result })));
        });
    }

    pub(super) fn archive_loaded(&mut self, loaded: Loaded) -> Effect {
        if loaded.request != self.request { return Effect::default(); }
        let result = loaded.result.map(|entry| {
            let stamp = entry.baseline.container.stamp();
            let canonical = Some(entry.baseline.location.to_uri());
            (Some(stamp), Arc::new(entry.document), canonical)
        });
        self.complete(Completion::Loaded { request: loaded.request, tab: loaded.tab, path: loaded.path, result })
    }

    pub(super) fn archive_save_disabled(&self, buffer: crate::text::BufferId) -> Option<&'static str> {
        let entry = self.state.sources.buffers.get(&buffer)?;
        let Some(baseline) = &entry.save.archive else { return Some("This archive member has no confirmed container; reload it before saving") };
        if self.archive_saves.lanes.get(&baseline.location.container).is_some_and(|lane| lane.uncertain) {
            return Some("The archive has an unconfirmed save outcome; recheck it before saving another member");
        }
        if !entry.draft.as_ref().is_some_and(crate::draft::Draft::dirty) { return Some("No unsaved changes"); }
        None
    }

    pub(super) fn save_archive_buffer(&mut self, buffer: crate::text::BufferId) -> Effect {
        let repaint = Effect { repaint: true, ..Effect::default() };
        let Some(entry) = self.state.sources.buffers.get_mut(&buffer) else { return repaint };
        let (Some(snapshot), Some(baseline)) = (entry.snapshot.clone(), entry.save.archive.clone()) else { return repaint };
        self.save_seq += 1;
        let request = self.save_seq;
        if let Some(draft) = &mut entry.draft { draft.end_group(); }
        entry.save.status = crate::save::Status::Saving;
        entry.save.checked = None;
        entry.save.archive_checked = None;
        entry.save.revalidated = None;
        entry.save.captured = Some(crate::save::Captured { request, text: snapshot, compacting: false, save_as: None });
        let path = baseline.location.container.clone();
        self.archive_saves.lanes.entry(path.clone()).or_default().pending.push_back((buffer, request));
        self.start_archive_save(&path);
        repaint
    }

    fn start_archive_save(&mut self, path: &Path) {
        loop {
            let Some(lane) = self.archive_saves.lanes.get_mut(path) else { return };
            if lane.running.is_some() || lane.uncertain { return; }
            let Some((buffer, request)) = lane.pending.pop_front() else { self.archive_saves.lanes.remove(path); return };
            let Some(entry) = self.state.sources.buffers.get(&buffer) else { continue };
            let Some(captured) = entry.save.captured.as_ref().filter(|captured| captured.request == request) else { continue };
            let Some(baseline) = entry.save.archive.clone() else { continue };
            let snapshot = captured.text.clone();
            lane.running = Some((buffer, request));
            let (path, tx) = (path.to_path_buf(), self.tx.clone());
            self.jobs.submit(Priority::Visible, move || {
                let outcome = write_member(&snapshot, &baseline);
                let _ = tx.send_blocking(Completion::ArchiveSaved(Box::new(Saved { path, buffer, request, previous: baseline, outcome })));
            });
            return;
        }
    }

    pub(super) fn archive_saved(&mut self, saved: Saved) -> Effect {
        let repaint = Effect { repaint: true, ..Effect::default() };
        let Some(lane) = self.archive_saves.lanes.get_mut(&saved.path) else { return repaint };
        if lane.running != Some((saved.buffer, saved.request)) { return repaint; }
        lane.running = None;
        let Some(entry) = self.state.sources.buffers.get_mut(&saved.buffer) else { self.start_archive_save(&saved.path); return repaint };
        let Some(captured) = entry.save.captured.take_if(|captured| captured.request == saved.request) else { self.start_archive_save(&saved.path); return repaint };
        match saved.outcome {
            Ok(container) if entry.snapshot.as_ref().is_some_and(|current| Arc::ptr_eq(current.source_meta(), captured.text.source_meta())) => {
                // Only our own successful rewrite rebases sibling baselines. A separately loaded external
                // revision is not silently adopted, and queued saves use this new baseline when they start.
                let mut rebased = Vec::new();
                for (buffer, entry) in &mut self.state.sources.buffers {
                    if let Some(baseline) = &entry.save.archive
                        && baseline.container.same(&saved.previous.container) {
                        entry.save.archive = Some(Arc::new(Baseline { location: baseline.location.clone(), crlf: baseline.crlf, encoding: baseline.encoding, bom: baseline.bom, container: container.clone() }));
                        rebased.push(*buffer);
                    }
                }
                let refresh = self.next_refresh();
                for tab in self.state.tabs.iter_mut().filter(|tab| tab.buffer.is_some_and(|buffer| rebased.contains(&buffer))) {
                    tab.refresh = refresh; tab.stamp = Some(container.stamp()); tab.stale_on_disk = false;
                }
                let baseline = Arc::new(Baseline { location: saved.previous.location.clone(), crlf: saved.previous.crlf, encoding: saved.previous.encoding, bom: saved.previous.bom, container });
                self.archive_confirm_baseline(saved.buffer, captured.text.clone(), baseline);
                if let Some(entry) = self.state.sources.buffers.get_mut(&saved.buffer) {
                    entry.save.written = Some((saved.request, SemanticSource::new(&captured.text)));
                }
                self.archive_container_changed(&saved.path);
            }
            Ok(_) => self.archive_uncertain(&saved.path, saved.buffer, captured.text, "The archive was saved, but the document identity changed. Recheck its container before saving again.".into()),
            Err(Failure::Rejected(why)) => {
                if let Some(entry) = self.state.sources.buffers.get_mut(&saved.buffer) {
                    entry.save.status = crate::save::Status::Rejected(format!("Archive Save rejected: {why}. The draft and history are retained."));
                }
            }
            Err(Failure::Uncertain(why)) => self.archive_uncertain(&saved.path, saved.buffer, captured.text, format!("Archive Save outcome uncertain: {why}. Recheck the container; all drafts and history are retained.")),
        }
        self.start_archive_save(&saved.path);
        repaint
    }

    fn archive_uncertain(&mut self, path: &Path, buffer: crate::text::BufferId, attempted: Arc<crate::text::TextSnapshot>, why: String) {
        let Some(lane) = self.archive_saves.lanes.get_mut(path) else { return };
        lane.uncertain = true;
        if let Some(entry) = self.state.sources.buffers.get_mut(&buffer) {
            entry.save.status = crate::save::Status::Uncertain(why.clone());
            entry.save.attempted = Some(attempted);
        }
        for (buffer, request) in lane.pending.drain(..) {
            if let Some(entry) = self.state.sources.buffers.get_mut(&buffer)
                && entry.save.captured.as_ref().is_some_and(|captured| captured.request == request) {
                entry.save.captured = None;
                entry.save.status = crate::save::Status::Rejected("Another member's archive Save is unconfirmed; recheck the container before retrying. This draft was not written.".into());
            }
        }
    }

    pub(super) fn archive_confirm_baseline(&mut self, buffer: crate::text::BufferId, captured: Arc<crate::text::TextSnapshot>, baseline: Arc<Baseline>) {
        let stamp = baseline.container.stamp();
        let Some(entry) = self.state.sources.buffers.get_mut(&buffer) else { return };
        if let (Some(draft), Some(current)) = (&mut entry.draft, &entry.snapshot) { draft.confirm_baseline(captured.clone(), current); }
        entry.saved = Some(SemanticSource::new(&captured));
        entry.stamp = Some(stamp);
        entry.save.archive = Some(baseline.clone());
        entry.save.read = None;
        entry.save.status = crate::save::Status::Ready;
        entry.save.attempted = None;
        entry.save.checked = None;
        entry.save.archive_checked = None;
        entry.save.revalidated = None;
        if let Some(lane) = self.archive_saves.lanes.get_mut(&baseline.location.container) { lane.uncertain = false; }
        for tab in self.state.tabs.iter_mut().filter(|tab| tab.buffer == Some(buffer)) {
            tab.stamp = Some(stamp); tab.stale_on_disk = false; tab.unavailable = None;
        }
        self.compact_saved_backing(buffer);
        self.synced = 0;
        if self.state.sources.buffers.get(&buffer).and_then(|entry| entry.draft.as_ref()).is_some_and(crate::draft::Draft::dirty) {
            self.recovery_changed(buffer);
        } else { self.forget_recovery(buffer); }
    }

    pub(super) fn check_archive(&mut self, id: TabId, location: Location) {
        let refresh = self.next_refresh();
        let Some(tab) = self.tab_mut(id) else { return };
        tab.refresh = refresh; tab.moving = false;
        let stamp = tab.stamp;
        let (tx, versions) = (self.tx.clone(), self.versions.clone());
        self.jobs.submit(Priority::Visible, move || {
            let result = match crate::cache::Stamp::of(&location.container) {
                Some(now) if Some(now) == stamp => Refresh::Same(now),
                Some(_) => match crate::archive::read_entry(&location, versions.fetch_add(1, Ordering::Relaxed) + 1, &|| false) {
                    Ok(entry) => Refresh::Changed(entry.baseline.container.stamp(), Arc::new(entry.document)),
                    Err(why) => Refresh::Gone(why),
                },
                None => Refresh::Gone("archive deleted or moved".into()),
            };
            let _ = tx.send_blocking(Completion::Refreshed { tab: id, token: refresh, result });
        });
    }

    pub(super) fn recheck_archive_save(&mut self, tab: TabId) -> Effect {
        let repaint = Effect { repaint: true, ..Effect::default() };
        let Some(view) = self.state.view(tab) else { return repaint };
        if view.edit_disabled().is_some() || view.save.status.busy() { return repaint; }
        let buffer = view.snapshot().key().buffer;
        let Some(location) = view.snapshot().source_meta().archive().cloned() else { return repaint };
        let captured = view.snapshot().clone();
        self.save_seq += 1;
        let request = self.save_seq;
        let Some(entry) = self.state.sources.buffers.get_mut(&buffer) else { return repaint };
        entry.save.captured = Some(crate::save::Captured { request, text: captured, compacting: false, save_as: None });
        entry.save.status = crate::save::Status::Rechecking;
        entry.save.archive_checked = None;
        let (tx, versions) = (self.tx.clone(), self.versions.clone());
        self.jobs.submit(Priority::Visible, move || {
            let result = crate::archive::read_entry(&location, versions.fetch_add(1, Ordering::Relaxed) + 1, &|| false);
            let _ = tx.send_blocking(Completion::ArchiveRechecked(Box::new(Rechecked { buffer, request, result })));
        });
        repaint
    }

    pub(super) fn archive_rechecked(&mut self, reply: Rechecked) -> Effect {
        let repaint = Effect { repaint: true, ..Effect::default() };
        let Some(entry) = self.state.sources.buffers.get_mut(&reply.buffer) else { return repaint };
        let Some(captured) = entry.save.captured.take_if(|captured| captured.request == reply.request) else { return repaint };
        let result = match reply.result {
            Ok(result) => result,
            Err(why) => { entry.save.status = crate::save::Status::Uncertain(format!("Archive recheck failed: {why}. The draft is retained.")); return repaint; }
        };
        let observed = crate::text::TextSnapshot::from_document(reply.buffer, &result.document);
        entry.save.revalidated = Some(crate::save::Revalidated { request: reply.request, source: SemanticSource::new(&captured.text) });
        let baseline = entry.draft.as_ref().map(|draft| draft.baseline.clone()).or_else(|| entry.snapshot.clone());
        let matched = if let Some(attempted) = entry.save.attempted.as_ref().filter(|attempted| attempted.content_eq(&observed)) {
            Some((crate::save::DiskMatch::Attempted, attempted.clone()))
        } else { baseline.filter(|baseline| baseline.content_eq(&observed)).map(|baseline| (crate::save::DiskMatch::Baseline, baseline)) };
        if let Some((kind, text)) = matched {
            entry.save.archive_checked = Some(Checked { text, baseline: result.baseline });
            entry.save.status = crate::save::Status::Checked(kind);
        } else { entry.save.status = crate::save::Status::Checked(crate::save::DiskMatch::Changed); }
        repaint
    }
    pub(super) fn confirm_archive_recheck(&mut self, buffer: crate::text::BufferId) -> Effect {
        let Some(entry) = self.state.sources.buffers.get_mut(&buffer) else { return Effect::default() };
        if !matches!(entry.save.status, crate::save::Status::Checked(crate::save::DiskMatch::Baseline | crate::save::DiskMatch::Attempted)) { return Effect::default(); }
        let Some(checked) = entry.save.archive_checked.take() else { return Effect::default() };
        self.archive_confirm_baseline(buffer, checked.text, checked.baseline);
        Effect { repaint: true, ..Effect::default() }
    }
}

fn write_member(snapshot: &crate::text::TextSnapshot, baseline: &Baseline) -> Result<Arc<ContainerSnapshot>, Failure> {
    let cancel = || false;
    let read = crate::archive::read_container(&baseline.container.snapshot.path, &cancel).map_err(Failure::Rejected)?;
    if !read.baseline.same(&baseline.container) { return Err(Failure::Rejected("the archive changed on disk".into())); }
    let text = snapshot.copy_range(0..snapshot.len_bytes()).ok_or_else(|| Failure::Rejected("the member source is unavailable".into()))?;
    let encoded = lenz_content::write::encode_exact(&text, baseline.encoding, baseline.bom).map_err(|e| Failure::Rejected(e.to_string()))?;
    let updates = [(baseline.location.members.clone(), encoded)];
    let bytes = crate::archive::replace_locations(&baseline.location, &read.bytes, &updates, &cancel).map_err(Failure::Rejected)?;
    match lenz_content::write::write_bytes_exact(&read.baseline.snapshot, &read.baseline.target, &bytes) {
        WriteOutcome::Saved { snapshot, target } => {
            use sha2::Digest;
            Ok(Arc::new(ContainerSnapshot { snapshot, target, fingerprint: sha2::Sha256::digest(&bytes).into() }))
        }
        WriteOutcome::Rejected(why) => Err(Failure::Rejected(why.to_string())),
        WriteOutcome::Uncertain(why) => Err(Failure::Uncertain(why.to_string())),
    }
}
