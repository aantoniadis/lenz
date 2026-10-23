//! Bounded result intents share Analysis ownership, source conversions, credits and leases.
use super::*;
use crate::{semantics::Lane, session::SemanticSource};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum ResultSlot { Semantic { server: u64, lane: Lane }, Search, Preview, Validate }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ResultStage { Targets, Retrieved, Hierarchy, SyntaxHierarchy, Search, Outline, Preview, Validate }
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ResultFence {
    pub slot: ResultSlot,
    pub project: u64,
    pub request: u64,
    pub stage: ResultStage,
    pub result_set: Option<u64>,
    pub node: Option<usize>,
    pub source: Option<SemanticSource>,
}
#[derive(Clone, Debug)]
pub struct ResultTicket { pub(super) slot: ResultSlot, pub(crate) fence: ResultFence, pub(super) serial: u64 }
enum ResultWork {
    Ready(Box<dyn FnOnce(Lease) + Send>),
    Document { source: Arc<SourceDependency>, work: Box<dyn FnOnce(Arc<Document>, Lease) + Send> },
}
struct Intent { tag: ResultTicket, priority: Priority, work: ResultWork }
struct Running { tag: ResultTicket, task: Task, published: Arc<AtomicBool>, source: Option<Arc<SourceDependency>> }
pub(super) struct ResultEntry {
    pub(super) latest: ResultTicket,
    running: Option<Running>,
    pending: Option<Intent>,
    // Preserve source reuse after CPU release and before queued publication is acknowledged.
    completed: Option<(ResultTicket, Option<Arc<SourceDependency>>)>,
    pub(super) ack: u64,
    queued: bool,
}
impl Analysis {
    fn reserve_result(&mut self, fence: ResultFence, priority: Priority) -> Result<Option<ResultTicket>, ()> {
        let slot = fence.slot;
        if let Some(entry) = self.results.get_mut(&slot) && entry.latest.fence == fence {
            if priority == Priority::Visible && let Some(pending) = &mut entry.pending { pending.priority = Priority::Visible; self.dirty = true; }
            return Ok(None);
        }
        if !self.results.contains_key(&slot) && self.entries.len() + self.results.len() == BUFFERS {
            let discard = self.entries.iter().find(|(_, entry)| !entry.live && !entry.group.running(&self.conversions)).map(|(id, _)| *id);
            let Some(discard) = discard else { return Err(()); };
            self.entries.remove(&discard);
            self.queue.retain(|id| *id != Owner::Buffer(discard));
        }
        self.next += 1;
        let tag = ResultTicket { slot, fence, serial: self.next };
        let entry = self.results.entry(slot).or_insert_with(|| ResultEntry { latest: tag.clone(), running: None, pending: None, completed: None, ack: 0, queued: false });
        if let Some(run) = &entry.running { run.task.cancel.store(false, Ordering::Release); }
        entry.latest = tag.clone();
        // Keep the old dependency through the factory so a same-scope replacement reuses its cell.
        Ok(Some(tag))
    }
    fn install_result(&mut self, tag: ResultTicket, priority: Priority, work: ResultWork) {
        let slot = tag.slot;
        if let Some(entry) = self.results.get_mut(&slot) {
            entry.completed = None;
            entry.pending = Some(Intent { tag, priority, work });
        }
        self.dirty = true;
        self.enqueue_result(slot);
    }
    /// Reserve before constructing the payload: saturated/duplicate replies never clone sent maps.
    pub(crate) fn request_result<W: FnOnce(Lease) + Send + 'static>(&mut self, fence: ResultFence, priority: Priority, build: impl FnOnce() -> W) -> Result<bool, ()> {
        let Some(tag) = self.reserve_result(fence, priority)? else { return Ok(false); };
        self.install_result(tag, priority, ResultWork::Ready(Box::new(build())));
        Ok(true)
    }
    /// Capture source handles only after owner admission. Waiting for conversion occupies no consumer slot.
    pub(crate) fn request_result_document<W: FnOnce(Arc<Document>, Lease) + Send + 'static>(&mut self, fence: ResultFence, priority: Priority, build: impl FnOnce() -> (Input, W)) -> Result<bool, ()> {
        let Some(tag) = self.reserve_result(fence, priority)? else { return Ok(false); };
        let (input, work) = build();
        let source = self.source_dependency(input);
        self.install_result(tag, priority, ResultWork::Document { source, work: Box::new(work) });
        Ok(true)
    }
    fn enqueue_result(&mut self, slot: ResultSlot) {
        if let Some(entry) = self.results.get_mut(&slot) && !entry.queued {
            entry.queued = true;
            self.queue.push_back(Owner::Result(slot));
        }
    }
    pub(crate) fn acknowledge_result(&mut self, tag: &ResultTicket) -> bool {
        let Some(entry) = self.results.get_mut(&tag.slot) else { return false };
        if entry.latest.serial != tag.serial || entry.ack == tag.serial { return false; }
        entry.ack = tag.serial;
        entry.completed = None;
        self.dirty = true;
        true
    }
    pub(crate) fn invalidate_results(&mut self, current: impl Fn(&ResultFence) -> bool) {
        for (slot, entry) in &mut self.results {
            if entry.ack == entry.latest.serial || current(&entry.latest.fence) { continue; }
            if let Some(run) = &entry.running { run.task.cancel.store(false, Ordering::Release); }
            entry.pending = None;
            entry.completed = None;
            entry.ack = entry.latest.serial;
            self.failures.insert(*slot, (entry.latest.fence.clone(), "source or result request context changed".into()));
            self.dirty = true;
        }
    }
    pub(crate) fn take_result_failures(&mut self) -> Vec<(ResultFence, String)> { self.failures.drain().map(|(_, failure)| failure).collect() }
    pub(super) fn reap_results(&mut self) {
        let mut waiting = Vec::new();
        self.results.retain(|slot, entry| {
            if entry.running.as_ref().is_some_and(|run| run.task.done.load(Ordering::Acquire))
                && let Some(run) = entry.running.take()
            {
                self.running -= 1;
                if run.tag.serial == entry.latest.serial && entry.ack != run.tag.serial {
                    if run.published.load(Ordering::Acquire) { entry.completed = Some((run.tag, run.source)); }
                    else {
                        entry.ack = run.tag.serial;
                        self.failures.insert(*slot, (run.tag.fence, "result work was cancelled or could not complete".into()));
                    }
                }
            }
            if entry.pending.is_some() { waiting.push(*slot); }
            entry.running.is_some() || entry.pending.is_some() || entry.completed.is_some()
        });
        for slot in waiting { self.enqueue_result(slot); }
    }
    pub(super) fn submit_result(&mut self, slot: ResultSlot, jobs: &Jobs, tx: &async_channel::Sender<Completion>, project: &Arc<AtomicU64>, background: &mut usize) {
        let Some(entry) = self.results.get_mut(&slot) else { return };
        entry.queued = false;
        if entry.running.is_some() { return; }
        let Some(intent) = entry.pending.take() else { return };
        if intent.priority == Priority::Background && *background >= MAX_JOBS {
            entry.pending = Some(intent); self.enqueue_result(slot); return;
        }
        let Intent { tag, priority, work } = intent;
        let (source, work): (_, Box<dyn FnOnce(Lease) + Send>) = match work {
            ResultWork::Ready(work) => (None, work),
            ResultWork::Document { source, work } => match self.ensure_source(&source, priority, jobs, tx, project, background) {
                SourceReady::Ready(doc) => (Some(source), Box::new(move |lease| work(doc, lease))),
                SourceReady::Waiting => {
                    if let Some(entry) = self.results.get_mut(&slot) { entry.pending = Some(Intent { tag, priority, work: ResultWork::Document { source, work } }); }
                    self.enqueue_result(slot); return;
                }
                SourceReady::Failed(why) => {
                    if let Some(entry) = self.results.get_mut(&slot) && entry.latest.serial == tag.serial && entry.ack != tag.serial {
                        entry.ack = tag.serial;
                        self.failures.insert(slot, (tag.fence, why.into()));
                        self.dirty = true;
                    }
                    return;
                }
            },
        };
        let done = Arc::new(AtomicBool::new(false)); let cancel = Arc::new(AtomicBool::new(true)); let published = Arc::new(AtomicBool::new(false));
        let lease = Lease { ticket: Ticket::Result(tag.clone()), group: cancel.clone(), cancel: cancel.clone(), done: done.clone(), project: project.clone(), published: Some(published.clone()), tx: tx.clone() };
        if let Some(entry) = self.results.get_mut(&slot) {
            entry.running = Some(Running { tag, task: Task { done, cancel, job: None, priority }, published, source });
        }
        self.running += 1; *background += usize::from(priority == Priority::Background);
        jobs.submit(priority, move || { if lease.wanted() { work(lease); } });
    }
    pub(super) fn result_background(&self) -> usize { self.results.values().filter(|entry| entry.running.as_ref().is_some_and(|run| run.task.priority == Priority::Background)).count() }
    pub(super) fn close_results(&mut self) {
        for entry in self.results.values_mut() {
            if let Some(run) = &mut entry.running { run.task.cancel.store(false, Ordering::Release); run.source = None; }
            entry.pending = None; entry.completed = None; entry.ack = entry.latest.serial;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{sync::mpsc, time::{Duration, Instant}};

    fn fence(server: u64, request: u64) -> ResultFence {
        ResultFence { slot: ResultSlot::Semantic { server, lane: Lane::Foreground }, project: 0, request, stage: ResultStage::Targets, result_set: None, node: None, source: None }
    }
    fn receive(rx: &async_channel::Receiver<Completion>) -> Ticket {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            assert!(Instant::now() < deadline, "result was not published");
            if let Ok(Completion::Analyzed { ticket, .. }) = rx.try_recv() { return ticket; }
            std::thread::yield_now();
        }
    }
    fn drain(analysis: &mut Analysis, jobs: &Jobs, tx: &async_channel::Sender<Completion>, project: &Arc<AtomicU64>) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while analysis.running != 0 {
            assert!(Instant::now() < deadline, "CPU lease did not finish");
            analysis.pump(jobs, tx, project);
            std::thread::yield_now();
        }
    }

    #[test]
    fn originless_servers_keep_independent_results_valid_after_cpu_lease_release() {
        let mut analysis = Analysis::default(); let jobs = Jobs::new();
        let (tx, rx) = async_channel::bounded(64); let project = Arc::new(AtomicU64::new(0));
        for server in [1, 2] { assert_eq!(analysis.request_result(fence(server, 7), Priority::Visible, || |lease| { lease.send(Completion::AnalysisWake); }), Ok(true)); }
        analysis.pump(&jobs, &tx, &project);
        let first = receive(&rx); let second = receive(&rx);
        drain(&mut analysis, &jobs, &tx, &project);
        assert!(analysis.entries.is_empty(), "originless symbols must not need a source carrier");
        assert_eq!(analysis.results.len(), 2);
        for ticket in [first, second] {
            assert!(analysis.accepts(&ticket), "successful queued completion outlives its CPU lease");
            assert!(analysis.acknowledge_result(ticket.result().unwrap()));
            assert!(!analysis.acknowledge_result(ticket.result().unwrap()), "answer finalized exactly once");
        }
        analysis.pump(&jobs, &tx, &project); assert!(analysis.results.is_empty());
    }

    #[test]
    fn replacing_result_intents_releases_old_maps_and_bounds_jobs() {
        let mut analysis = Analysis::default(); let jobs = Jobs::new();
        let (tx, rx) = async_channel::bounded(64); let project = Arc::new(AtomicU64::new(0));
        let (started, entered) = mpsc::channel(); let (release, wait) = mpsc::channel();
        let retained = Arc::new("first sent map".to_string()); let first = Arc::downgrade(&retained);
        analysis.request_result(fence(1, 0), Priority::Visible, || move |lease| { let _held = retained; started.send(()).unwrap(); wait.recv().unwrap(); assert!(!lease.wanted()); }).unwrap();
        analysis.pump(&jobs, &tx, &project); entered.recv_timeout(Duration::from_secs(5)).unwrap();
        let mut previous = Weak::new();
        for request in 1..130 {
            let map = Arc::new(format!("sent map {request}")); let current = Arc::downgrade(&map);
            analysis.request_result(fence(1, request), Priority::Visible, || move |lease| { let _held = map; lease.send(Completion::AnalysisWake); }).unwrap();
            assert!(previous.upgrade().is_none(), "replaced pending map stayed owned"); previous = current;
            analysis.pump(&jobs, &tx, &project);
            assert!(jobs.queued() <= 2 * MAX_JOBS);
        }
        assert!(first.upgrade().is_some(), "running work retains only its original map until unwind");
        release.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while analysis.results.values().any(|entry| entry.pending.is_some()) { assert!(Instant::now() < deadline); analysis.pump(&jobs, &tx, &project); std::thread::yield_now(); }
        let ticket = receive(&rx); assert_eq!(ticket.result().unwrap().fence.request, 129);
        drain(&mut analysis, &jobs, &tx, &project);
        assert!(first.upgrade().is_none()); assert!(previous.upgrade().is_none());
        assert!(analysis.accepts(&ticket));
    }

    #[test]
    fn saturated_result_admission_does_not_build_payloads() {
        let mut analysis = Analysis::default();
        for server in 0..BUFFERS as u64 { analysis.request_result(fence(server, 1), Priority::Visible, || |_| {}).unwrap(); }
        assert_eq!(analysis.request_result(fence(1000, 1), Priority::Visible, || -> fn(Lease) { panic!("a saturated request must not clone its sent map") }), Err(()));
        assert_eq!(analysis.results.len(), BUFFERS); assert_eq!(analysis.queue.len(), BUFFERS);
        analysis.close();
        assert!(analysis.results.values().all(|entry| entry.pending.is_none()));
    }

    #[test]
    fn panic_cancel_and_closed_channel_finalize_each_current_result_once() {
        for mode in 0..3 {
            let mut analysis = Analysis::default(); let jobs = Jobs::new();
            let (tx, rx) = async_channel::bounded(64); let project = Arc::new(AtomicU64::new(0));
            if mode == 2 { rx.close(); }
            analysis.request_result(fence(1, 1), Priority::Visible, || move |lease| {
                match mode { 0 => panic!("gated worker failure"), 1 => {}, _ => { assert!(!lease.send(Completion::AnalysisWake)); } }
            }).unwrap();
            analysis.pump(&jobs, &tx, &project); drain(&mut analysis, &jobs, &tx, &project);
            assert_eq!(analysis.take_result_failures().len(), 1);
            analysis.pump(&jobs, &tx, &project); assert!(analysis.take_result_failures().is_empty()); assert!(analysis.results.is_empty());
            analysis.request_result(fence(1, 2), Priority::Visible, || |_| {}).unwrap();
            analysis.invalidate_results(|_| false); assert_eq!(analysis.take_result_failures().len(), 1);
            analysis.pump(&jobs, &tx, &project); assert!(analysis.results.is_empty());
        }
    }

    fn snapshot(buffer: u64, version: u64) -> Arc<TextSnapshot> {
        Arc::new(TextSnapshot::from_document(BufferId(buffer), &Document::decode("result.txt".into(), version, b"captured source\n")))
    }
    fn document_fence(slot: ResultSlot, stage: ResultStage, request: u64) -> ResultFence {
        ResultFence { slot, project: 0, request, stage, result_set: Some(41), node: None, source: None }
    }
    fn pending_source(analysis: &Analysis, slot: ResultSlot) -> Weak<SourceDependency> {
        match &analysis.results[&slot].pending.as_ref().unwrap().work {
            ResultWork::Document { source, .. } => Arc::downgrade(source),
            ResultWork::Ready(_) => panic!("expected document intent"),
        }
    }
    fn hold_workers(jobs: &Jobs) -> Vec<mpsc::Sender<()>> {
        let (entered, starts) = mpsc::channel();
        let releases: Vec<_> = (0..MAX_JOBS).map(|_| {
            let (release, wait) = mpsc::channel(); let entered = entered.clone();
            jobs.submit(Priority::Visible, move || { entered.send(()).unwrap(); let _ = wait.recv_timeout(Duration::from_secs(5)); }); release
        }).collect();
        for _ in &releases { starts.recv_timeout(Duration::from_secs(5)).unwrap(); }
        releases
    }
    fn pump_until(analysis: &mut Analysis, jobs: &Jobs, tx: &async_channel::Sender<Completion>, project: &Arc<AtomicU64>, ready: impl Fn(&Analysis) -> bool) {
        let until = Instant::now() + Duration::from_secs(5);
        while !ready(analysis) { assert!(Instant::now() < until, "result scheduler did not progress"); analysis.pump(jobs, tx, project); std::thread::yield_now(); }
    }

    #[test]
    fn document_intents_wait_without_consumer_slots_and_share_one_conversion() {
        let mut analysis = Analysis::default(); let jobs = Jobs::new();
        let (tx, rx) = async_channel::bounded(64); let project = Arc::new(AtomicU64::new(0));
        let source = snapshot(7, 1); let mut gates = hold_workers(&jobs);
        let (documents, ready) = mpsc::channel();
        for (slot, stage) in [(ResultSlot::Search, ResultStage::Search), (ResultSlot::Preview, ResultStage::Preview)] {
            let source = source.clone(); let documents = documents.clone();
            analysis.request_result_document(document_fence(slot, stage, 1), Priority::Visible, ||
                (Input::new(source, Weak::new(), 0, false), move |doc: Arc<Document>, lease: Lease| { documents.send(doc).unwrap(); lease.send(Completion::AnalysisWake); })).unwrap();
        }
        let dependency = pending_source(&analysis, ResultSlot::Search);
        assert!(dependency.ptr_eq(&pending_source(&analysis, ResultSlot::Preview)));
        analysis.pump(&jobs, &tx, &project);
        assert_eq!(analysis.running, 1, "only the shared conversion consumes a CPU slot");
        assert!(analysis.results.values().all(|entry| entry.running.is_none())); assert_eq!(jobs.queued(), 1);
        analysis.request_result(fence(9, 1), Priority::Visible, || |lease| { lease.send(Completion::AnalysisWake); }).unwrap();
        analysis.pump(&jobs, &tx, &project); assert_eq!(analysis.running, 2);
        gates.remove(0).send(()).unwrap(); let semantic = receive(&rx);
        assert!(matches!(semantic.result().unwrap().fence.slot, ResultSlot::Semantic { .. }));
        assert!(analysis.results[&ResultSlot::Search].running.is_none(), "waiting consumers do not block a worker");
        for gate in gates { gate.send(()).unwrap(); }
        pump_until(&mut analysis, &jobs, &tx, &project, |a| a.results[&ResultSlot::Search].completed.is_some() && a.results[&ResultSlot::Preview].completed.is_some());
        let first = receive(&rx); let second = receive(&rx); let doc = ready.recv().unwrap(); let other = ready.recv().unwrap();
        assert!(Arc::ptr_eq(&doc, &other)); assert_eq!(analysis.materializations.load(Ordering::Relaxed), 1);
        assert!(analysis.accepts(&first) && analysis.accepts(&second), "queued completions survive CPU release");
        let input = source.clone(); let documents = documents.clone();
        analysis.request_result_document(document_fence(ResultSlot::Validate, ResultStage::Validate, 2), Priority::Visible, ||
            (Input::new(input, Weak::new(), 0, false), move |doc: Arc<Document>, lease: Lease| { documents.send(doc).unwrap(); lease.send(Completion::AnalysisWake); })).unwrap();
        pump_until(&mut analysis, &jobs, &tx, &project, |a| a.results[&ResultSlot::Validate].completed.is_some());
        let third = receive(&rx); assert!(Arc::ptr_eq(&doc, &ready.recv().unwrap()));
        assert_eq!(analysis.materializations.load(Ordering::Relaxed), 1, "unacknowledged completions retain the shared conversion");
        for ticket in [semantic, first, second, third] { assert!(analysis.acknowledge_result(ticket.result().unwrap())); }
        analysis.pump(&jobs, &tx, &project); assert!(dependency.upgrade().is_none());
    }

    #[test]
    fn captured_document_result_survives_releasing_its_source_group() {
        let mut analysis = Analysis::default(); let jobs = Jobs::new();
        let (tx, rx) = async_channel::bounded(64); let project = Arc::new(AtomicU64::new(0));
        let source = snapshot(3, 1); let gates = hold_workers(&jobs);
        analysis.ensure(Input::new(source.clone(), Weak::new(), 0, true), Priority::Background);
        analysis.pump(&jobs, &tx, &project);
        analysis.request_result_document(document_fence(ResultSlot::Search, ResultStage::Search, 1), Priority::Visible, ||
            (Input::new(source, Weak::new(), 0, false), |doc: Arc<Document>, lease: Lease| { assert_eq!(&**doc.text(), "captured source\n"); lease.send(Completion::AnalysisWake); })).unwrap();
        analysis.release_view(BufferId(3), TabId(1), true); analysis.retain_sources(|_| false);
        for gate in gates { gate.send(()).unwrap(); }
        pump_until(&mut analysis, &jobs, &tx, &project, |a| a.results[&ResultSlot::Search].completed.is_some());
        let ticket = receive(&rx); assert!(analysis.entries.is_empty()); assert!(analysis.accepts(&ticket));
        assert_eq!(analysis.materializations.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn replacing_waiting_document_intents_releases_dependencies_before_work() {
        let mut analysis = Analysis::default(); let mut previous = Weak::new();
        for request in 1..130 {
            let source = snapshot(7, request);
            analysis.request_result_document(document_fence(ResultSlot::Search, ResultStage::Search, request), Priority::Visible, ||
                (Input::new(source, Weak::new(), 0, false), |_: Arc<Document>, _: Lease| {})).unwrap();
            assert!(previous.upgrade().is_none()); previous = pending_source(&analysis, ResultSlot::Search);
        }
        assert_eq!(analysis.results.len(), 1); assert_eq!(analysis.queue.len(), 1); assert_eq!(analysis.running, 0);
        assert_eq!(analysis.request_result_document(document_fence(ResultSlot::Search, ResultStage::Search, 129), Priority::Visible,
            || -> (Input, fn(Arc<Document>, Lease)) { panic!("duplicate must not capture input") }), Ok(false));
        analysis.invalidate_results(|_| false); assert!(previous.upgrade().is_none()); assert_eq!(analysis.take_result_failures().len(), 1);
    }

    #[test]
    fn search_and_outline_replace_stage_without_recopying_completed_source() {
        let mut analysis = Analysis::default(); let jobs = Jobs::new();
        let (tx, rx) = async_channel::bounded(64); let project = Arc::new(AtomicU64::new(0));
        let source = snapshot(4, 1); let (documents, ready) = mpsc::channel();
        for stage in [ResultStage::Search, ResultStage::Outline] {
            let input = source.clone(); let documents = documents.clone();
            analysis.request_result_document(document_fence(ResultSlot::Search, stage, 1), Priority::Visible, ||
                (Input::new(input, Weak::new(), 0, false), move |doc: Arc<Document>, lease: Lease| { documents.send(doc).unwrap(); lease.send(Completion::AnalysisWake); })).unwrap();
            pump_until(&mut analysis, &jobs, &tx, &project, |a| a.results[&ResultSlot::Search].completed.is_some());
        }
        let obsolete = receive(&rx); let current = receive(&rx);
        assert!(!analysis.accepts(&obsolete)); assert!(analysis.accepts(&current));
        assert!(Arc::ptr_eq(&ready.recv().unwrap(), &ready.recv().unwrap()));
        assert_eq!(analysis.materializations.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn document_admission_reserves_capacity_before_capturing_source() {
        let mut analysis = Analysis::default();
        for server in 0..BUFFERS as u64 { analysis.request_result(fence(server, 1), Priority::Visible, || |_| {}).unwrap(); }
        assert_eq!(analysis.request_result_document(document_fence(ResultSlot::Preview, ResultStage::Preview, 1), Priority::Visible,
            || -> (Input, fn(Arc<Document>, Lease)) { panic!("saturation must not capture source") }), Err(()));
    }

    #[test]
    fn failed_document_readiness_finishes_once_without_a_consumer_slot() {
        let mut analysis = Analysis::default(); let jobs = Jobs::new();
        let (tx, _rx) = async_channel::bounded(64); let project = Arc::new(AtomicU64::new(0)); let source = snapshot(8, 1);
        let mut tag = document_fence(ResultSlot::Validate, ResultStage::Validate, 1); tag.project = 1;
        analysis.request_result_document(tag.clone(), Priority::Visible, ||
            (Input::new(source, Weak::new(), 1, false), |_: Arc<Document>, _: Lease| panic!("stale context must not run"))).unwrap();
        let dependency = pending_source(&analysis, ResultSlot::Validate);
        analysis.pump(&jobs, &tx, &project); assert_eq!(analysis.running, 0); assert!(dependency.upgrade().is_none());
        let failures = analysis.take_result_failures(); assert_eq!(failures.len(), 1); assert_eq!(failures[0].0, tag);
        analysis.pump(&jobs, &tx, &project); assert!(analysis.take_result_failures().is_empty());
    }

    #[test]
    fn closing_running_document_result_drops_dependency_without_capturing_it_in_worker() {
        let mut analysis = Analysis::default(); let jobs = Jobs::new();
        let (tx, _rx) = async_channel::bounded(64); let project = Arc::new(AtomicU64::new(0)); let source = snapshot(5, 1);
        let (started, entered) = mpsc::channel(); let (release, wait) = mpsc::channel();
        analysis.request_result_document(document_fence(ResultSlot::Search, ResultStage::Search, 1), Priority::Visible, ||
            (Input::new(source, Weak::new(), 0, false), move |doc: Arc<Document>, lease: Lease| {
                started.send(Arc::downgrade(&doc)).unwrap(); wait.recv().unwrap(); assert!(!lease.send(Completion::AnalysisWake));
            })).unwrap();
        let dependency = pending_source(&analysis, ResultSlot::Search);
        pump_until(&mut analysis, &jobs, &tx, &project, |a| a.results[&ResultSlot::Search].running.is_some());
        let document = entered.recv_timeout(Duration::from_secs(5)).unwrap();
        analysis.close(); assert!(dependency.upgrade().is_none()); assert!(document.upgrade().is_some());
        release.send(()).unwrap(); drain(&mut analysis, &jobs, &tx, &project); assert!(document.upgrade().is_none());
    }

}
