//! Durable buffer write intents share conversion and CPU admission, not read-result cancellation.
use super::*;
use crate::save::Outcome;
use std::sync::Mutex;
type WriteWork = Box<dyn FnOnce(Option<Arc<Document>>) -> Outcome + Send>;
pub(super) struct WriteEntry {
    request: u64,
    source: Option<Arc<SourceDependency>>,
    project: Option<u64>,
    pending: Option<WriteWork>,
    submitted: bool,
    done: Arc<AtomicBool>,
    outcome: Arc<Mutex<Option<Outcome>>>,
}
impl Analysis {
    /// Reserve capacity before capturing a snapshot or read-back context. One intent per buffer.
    pub(crate) fn request_write<W: FnOnce(Option<Arc<Document>>) -> Outcome + Send + 'static>(&mut self,
        buffer: BufferId, request: u64, project: u64, build: impl FnOnce() -> (Option<Input>, W)) -> bool {
        self.reserve_write(buffer, request, Some(project), build)
    }
    pub(crate) fn request_compaction<W: FnOnce(Option<Arc<Document>>) -> Outcome + Send + 'static>(&mut self,
        buffer: BufferId, request: u64, build: impl FnOnce() -> (Option<Input>, W)) -> bool {
        self.reserve_write(buffer, request, None, build)
    }
    fn reserve_write<W: FnOnce(Option<Arc<Document>>) -> Outcome + Send + 'static>(&mut self,
        buffer: BufferId, request: u64, project: Option<u64>, build: impl FnOnce() -> (Option<Input>, W)) -> bool {
        if self.writes.contains_key(&buffer) || self.entries.len() + self.results.len() + self.writes.len() >= BUFFERS { return false; }
        let (input, work) = build();
        let source = input.map(|input| self.source_dependency(input));
        self.writes.insert(buffer, WriteEntry { request, source, project, pending: Some(Box::new(work)), submitted: false,
            done: Arc::new(AtomicBool::new(false)), outcome: Arc::new(Mutex::new(None)) });
        self.queue.push_front(Owner::Write(buffer)); self.dirty = true;
        true
    }
    pub(super) fn submit_write(&mut self, id: BufferId, jobs: &Jobs, tx: &async_channel::Sender<Completion>, project: &Arc<AtomicU64>, background: &mut usize) {
        let Some(entry) = self.writes.get(&id).filter(|entry| entry.pending.is_some()) else { return };
        let source = entry.source.clone();
        let ready = if !jobs.has_workers() { Err(Some("save workers are unavailable")) }
            else if entry.project.is_some_and(|expected| expected != project.load(Ordering::Acquire)) { Err(Some("save context changed before writing")) }
            else if let Some(source) = &source {
                match self.ensure_source(source, Priority::Visible, jobs, tx, project, background) {
                    SourceReady::Ready(doc) => Ok(Some(doc)), SourceReady::Waiting => Err(None), SourceReady::Failed(why) => Err(Some(why)),
                }
            } else { Ok(None) };
        match ready {
            Err(None) => { self.queue.push_back(Owner::Write(id)); }
            Err(Some(why)) => {
                let entry = self.writes.get_mut(&id).expect("reserved write"); entry.pending = None; entry.source = None;
                *entry.outcome.lock().unwrap_or_else(|e| e.into_inner()) = Some(Outcome::Rejected(why.into()));
                entry.done.store(true, Ordering::Release);
            }
            Ok(document) => {
                let entry = self.writes.get_mut(&id).expect("reserved write");
                let Some(work) = entry.pending.take() else { return };
                entry.submitted = true; entry.source = None;
                let (done, outcome, tx) = (entry.done.clone(), entry.outcome.clone(), tx.clone());
                drop(source);
                self.running += 1;
                jobs.submit(Priority::Visible, move || {
                    // Once submitted this is independent of all tab/project/read cancellation.
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(document)))
                        .unwrap_or_else(|_| Outcome::Uncertain("Save worker failed after submission; disk state is uncertain".into()));
                    *outcome.lock().unwrap_or_else(|e| e.into_inner()) = Some(result);
                    done.store(true, Ordering::Release);
                    let _ = tx.try_send(Completion::AnalysisWake);
                });
            }
        }
    }
    pub(super) fn reap_writes(&mut self) {
        for entry in self.writes.values_mut() {
            if entry.submitted && entry.done.load(Ordering::Acquire) { entry.submitted = false; self.running -= 1; }
        }
    }
    pub(crate) fn take_write_outcomes(&mut self) -> Vec<(BufferId, u64, Outcome)> {
        self.reap_writes();
        let ready: Vec<_> = self.writes.iter().filter_map(|(id, entry)| entry.done.load(Ordering::Acquire).then_some(*id)).collect();
        ready.into_iter().filter_map(|id| {
            let entry = self.writes.remove(&id)?;
            let outcome = entry.outcome.lock().unwrap_or_else(|e| e.into_inner()).take()?;
            Some((id, entry.request, outcome))
        }).collect()
    }
    pub(crate) fn finish_submitted_writes(&self) {
        for entry in self.writes.values().filter(|entry| entry.submitted) {
            while !entry.done.load(Ordering::Acquire) { std::thread::yield_now(); }
        }
    }
    pub(super) fn close_writes(&mut self) {
        for entry in self.writes.values_mut().filter(|entry| !entry.submitted) { entry.pending = None; entry.source = None; }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lenz_content::write::{read_exact, write_exact, WriteOutcome};
    use std::time::{Duration, Instant};
    #[test]
    fn compact_current_backing_reuses_an_inflight_conversion_cell_and_releases_original_allocations() {
        let mut analysis = Analysis::default(); let jobs = Jobs::new();
        let (tx, _rx) = async_channel::bounded(64); let project = Arc::new(AtomicU64::new(0));
        let original = Document::decode("compact.txt".into(), 1, &vec![b'a'; 256 << 10]);
        let (text, index) = (Arc::downgrade(original.text()), Arc::downgrade(original.line_starts()));
        let snapshot = TextSnapshot::from_document(BufferId(3), &original);
        let current = Arc::new(snapshot.apply_edits(3, &[crate::text::TextEdit { range: 0..(256 << 10) - 16384, text: "".into() }]).unwrap());
        drop(snapshot); drop(original);
        let compact = Arc::new(current.compact_backing());
        let (entered, starts) = std::sync::mpsc::channel();
        let gates: Vec<_> = (0..MAX_JOBS).map(|_| { let (release, wait) = std::sync::mpsc::channel(); let entered = entered.clone(); jobs.submit(Priority::Visible, move || { entered.send(()).unwrap(); let _ = wait.recv_timeout(Duration::from_secs(5)); }); release }).collect();
        for _ in &gates { starts.recv_timeout(Duration::from_secs(5)).unwrap(); }
        analysis.ensure(Input::new(current.clone(), Weak::new(), 0, true), Priority::Background);
        analysis.pump(&jobs, &tx, &project);
        let old = Arc::downgrade(analysis.entries[&BufferId(3)].group.source.as_ref().unwrap());
        let cell = analysis.entries[&BufferId(3)].group.source.as_ref().unwrap().document.clone();
        analysis.rebind_backing(compact.clone(), 0);
        assert!(old.upgrade().is_none());
        assert!(Arc::ptr_eq(&cell, &analysis.entries[&BufferId(3)].group.source.as_ref().unwrap().document));
        assert!(analysis.conversions[&scope(&compact, 0)].source.upgrade().is_some());
        drop(current); assert!(text.upgrade().is_some(), "the queued conversion still owns its original immutable input");
        for gate in gates { gate.send(()).unwrap(); }
        let deadline = Instant::now() + Duration::from_secs(5);
        while analysis.ready(&compact, 0).is_none() || text.upgrade().is_some() || index.upgrade().is_some() {
            assert!(Instant::now() < deadline); analysis.pump(&jobs, &tx, &project); std::thread::yield_now();
        }
        assert_eq!(analysis.materializations.load(Ordering::Relaxed), 1);
        assert_eq!(&**analysis.ready(&compact, 0).unwrap().text(), "a".repeat(16384));
    }
    #[test]
    fn captured_result_keeps_the_original_shared_conversion_alive_after_compact_group_release() {
        let mut analysis = Analysis::default(); let jobs = Jobs::new(); let (tx, rx) = async_channel::bounded(64); let project = Arc::new(AtomicU64::new(0));
        let original = Document::decode("capture.txt".into(), 1, &vec![b'a'; 256 << 10]);
        let (text, index) = (Arc::downgrade(original.text()), Arc::downgrade(original.line_starts()));
        let snapshot = TextSnapshot::from_document(BufferId(3), &original);
        let current = Arc::new(snapshot.apply_edits(3, &[crate::text::TextEdit { range: 0..(256 << 10) - 16384, text: "".into() }]).unwrap());
        drop(original); drop(snapshot); let compact = Arc::new(current.compact_backing());
        let (entered, starts) = std::sync::mpsc::channel();
        let gates: Vec<_> = (0..MAX_JOBS).map(|_| { let (release, wait) = std::sync::mpsc::channel(); let entered = entered.clone(); jobs.submit(Priority::Visible, move || { entered.send(()).unwrap(); let _ = wait.recv_timeout(Duration::from_secs(5)); }); release }).collect();
        for _ in &gates { starts.recv_timeout(Duration::from_secs(5)).unwrap(); }
        analysis.ensure(Input::new(current.clone(), Weak::new(), 0, true), Priority::Background); analysis.pump(&jobs, &tx, &project);
        let original_owner = Arc::downgrade(analysis.entries[&BufferId(3)].group.source.as_ref().unwrap());
        let cell = analysis.entries[&BufferId(3)].group.source.as_ref().unwrap().document.clone();
        let fence = ResultFence { slot: ResultSlot::Search, stage: ResultStage::Search, request: 1, project: 0, result_set: Some(1), node: None, source: None };
        analysis.request_result_document(fence, Priority::Visible, || (Input::new(current, Weak::new(), 0, false), |doc: Arc<Document>, lease: Lease| { assert_eq!(doc.text().len(), 16384); lease.send(Completion::AnalysisWake); })).unwrap();
        analysis.rebind_backing(compact.clone(), 0); analysis.release_view(BufferId(3), TabId(1), true); analysis.retain_sources(|_| false);
        for gate in gates { gate.send(()).unwrap(); }
        let deadline = Instant::now() + Duration::from_secs(5);
        let ticket = loop {
            analysis.pump(&jobs, &tx, &project); assert!(analysis.take_result_failures().is_empty(), "captured source demand was lost during backing rebind"); assert!(Instant::now() < deadline);
            if let Ok(Completion::Analyzed { ticket, .. }) = rx.try_recv() { break ticket; }
            std::thread::yield_now();
        };
        analysis.ensure(Input::new(compact.clone(), Weak::new(), 0, true), Priority::Visible); analysis.pump(&jobs, &tx, &project);
        assert!(Arc::ptr_eq(&cell, &analysis.entries[&BufferId(3)].group.source.as_ref().unwrap().document));
        assert!(!analysis.entries[&BufferId(3)].group.source.as_ref().unwrap().snapshot.needs_compaction());
        assert!(analysis.acknowledge_result(ticket.result().unwrap()));
        while original_owner.upgrade().is_some() || text.upgrade().is_some() || index.upgrade().is_some() { assert!(Instant::now() < deadline); analysis.pump(&jobs, &tx, &project); std::thread::yield_now(); }
        assert_eq!(analysis.materializations.load(Ordering::Relaxed), 1);
    }
    #[test]
    fn submitted_write_survives_project_close_and_a_full_notification_inbox() {
        let d = tempfile::tempdir().unwrap(); let path = d.path().join("write.txt"); std::fs::write(&path, "old").unwrap();
        let (baseline, target, _) = read_exact(&path, None).unwrap();
        let mut analysis = Analysis::default(); let jobs = Jobs::new();
        let (tx, rx) = async_channel::bounded(1); tx.send_blocking(Completion::AnalysisWake).unwrap();
        let project = Arc::new(AtomicU64::new(0));
        let (entered, started) = std::sync::mpsc::channel(); let (release, wait) = std::sync::mpsc::channel();
        assert!(analysis.request_write(BufferId(7), 11, 0, || (None, move |_| {
            entered.send(()).unwrap(); wait.recv_timeout(Duration::from_secs(5)).unwrap();
            Outcome::Written(write_exact(&baseline, &target, "new"))
        })));
        analysis.pump(&jobs, &tx, &project); started.recv_timeout(Duration::from_secs(5)).unwrap();
        project.store(1, Ordering::Release); analysis.close(); release.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !analysis.writes[&BufferId(7)].done.load(Ordering::Acquire) { assert!(Instant::now() < deadline); std::thread::yield_now(); }
        let outcomes = analysis.take_write_outcomes();
        assert!(matches!(outcomes.as_slice(), [(BufferId(7), 11, Outcome::Written(WriteOutcome::Saved { .. }))]));
        assert_eq!(std::fs::read(path).unwrap(), b"new"); assert_eq!(rx.len(), 1);
        assert!(analysis.take_write_outcomes().is_empty()); assert_eq!(analysis.running, 0);
    }
    #[test]
    fn prewrite_context_rejection_and_worker_panic_have_distinct_durable_outcomes() {
        let jobs = Jobs::new(); let (tx, _rx) = async_channel::bounded(1); let project = Arc::new(AtomicU64::new(1));
        let mut analysis = Analysis::default();
        assert!(analysis.request_write(BufferId(1), 1, 0, || (None, |_| panic!("rejected intent must never enter writer"))));
        analysis.pump(&jobs, &tx, &project);
        assert!(matches!(analysis.take_write_outcomes().as_slice(), [(BufferId(1), 1, Outcome::Rejected(_))]));
        assert!(analysis.request_write(BufferId(2), 2, 1, || (None, |_| panic!("submitted writer failed"))));
        analysis.pump(&jobs, &tx, &project); analysis.finish_submitted_writes();
        assert!(matches!(analysis.take_write_outcomes().as_slice(), [(BufferId(2), 2, Outcome::Uncertain(_))]));
    }
    #[test]
    fn write_admission_reserves_before_capture_and_shares_the_six_cpu_credits() {
        let jobs = Jobs::new(); let (tx, _rx) = async_channel::bounded(1); let project = Arc::new(AtomicU64::new(0));
        let mut analysis = Analysis::default();
        let (entered, starts) = std::sync::mpsc::channel();
        let gates: Vec<_> = (0..MAX_JOBS).map(|_| { let (release, wait) = std::sync::mpsc::channel(); let entered = entered.clone(); jobs.submit(Priority::Visible, move || { entered.send(()).unwrap(); let _ = wait.recv_timeout(Duration::from_secs(5)); }); release }).collect();
        for _ in &gates { starts.recv_timeout(Duration::from_secs(5)).unwrap(); }
        for id in 0..BUFFERS as u64 { assert!(analysis.request_write(BufferId(id), id, 0, || (None, |_| Outcome::Rejected("test".into())))); }
        type FactoryOutput = (Option<Input>, fn(Option<Arc<Document>>) -> Outcome);
        assert!(!analysis.request_write(BufferId(999), 999, 0, || -> FactoryOutput { panic!("saturated factory must not capture") }));
        assert!(!analysis.request_write(BufferId(1), 999, 0, || -> FactoryOutput { panic!("busy factory must not capture") }));
        analysis.pump(&jobs, &tx, &project); assert_eq!(analysis.running, 2 * MAX_JOBS);
        assert_eq!(analysis.writes.values().filter(|e| e.pending.is_some()).count(), BUFFERS - 2 * MAX_JOBS);
        for gate in gates { gate.send(()).unwrap(); }
        analysis.close(); analysis.finish_submitted_writes();
    }
}
