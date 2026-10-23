//! Bounded semantic result intents share Analysis ownership, queue, credits and leases.
use super::*;
use crate::{semantics::Lane, session::SemanticSource};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ResultSlot { pub server: u64, pub lane: Lane }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ResultStage { Targets, Retrieved, Hierarchy, SyntaxHierarchy }
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
type ResultWork = Box<dyn FnOnce(Lease) + Send>;
struct Intent { tag: ResultTicket, priority: Priority, work: ResultWork }
struct Running { tag: ResultTicket, task: Task, published: Arc<AtomicBool> }
pub(super) struct ResultEntry {
    pub(super) latest: ResultTicket,
    running: Option<Running>,
    pending: Option<Intent>,
    completed: Option<ResultTicket>,
    pub(super) ack: u64,
    queued: bool,
}
impl Analysis {
    /// Reserve before constructing the payload: saturated/duplicate replies never clone sent maps.
    pub(crate) fn request_result<W: FnOnce(Lease) + Send + 'static>(&mut self, fence: ResultFence, priority: Priority, build: impl FnOnce() -> W) -> Result<bool, ()> {
        let slot = fence.slot;
        if self.results.get(&slot).is_some_and(|entry| entry.latest.fence == fence) { return Ok(false); }
        if !self.results.contains_key(&slot) && self.entries.len() + self.results.len() == BUFFERS {
            let discard = self.entries.iter().find(|(_, entry)| !entry.live && !entry.group.running()).map(|(id, _)| *id);
            let Some(discard) = discard else { return Err(()); };
            self.entries.remove(&discard);
            self.queue.retain(|id| *id != Owner::Buffer(discard));
        }
        self.next += 1;
        let tag = ResultTicket { slot, fence, serial: self.next };
        let entry = self.results.entry(slot).or_insert_with(|| ResultEntry { latest: tag.clone(), running: None, pending: None, completed: None, ack: 0, queued: false });
        if let Some(run) = &entry.running { run.task.cancel.store(false, Ordering::Release); }
        entry.latest = tag.clone();
        entry.completed = None;
        // Replacing this box releases its old sent map immediately.
        entry.pending = Some(Intent { tag, priority, work: Box::new(build()) });
        self.dirty = true;
        self.enqueue_result(slot);
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
            self.failures.insert(*slot, (entry.latest.fence.clone(), "source or semantic request context changed".into()));
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
                    if run.published.load(Ordering::Acquire) { entry.completed = Some(run.tag); }
                    else {
                        entry.ack = run.tag.serial;
                        self.failures.insert(*slot, (run.tag.fence, "semantic work was cancelled or could not complete".into()));
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
        if intent.priority == Priority::Background && *background == MAX_JOBS {
            entry.pending = Some(intent); self.enqueue_result(slot); return;
        }
        let done = Arc::new(AtomicBool::new(false)); let cancel = Arc::new(AtomicBool::new(true)); let published = Arc::new(AtomicBool::new(false));
        let lease = Lease { ticket: Ticket::Result(intent.tag.clone()), group: cancel.clone(), cancel: cancel.clone(), done: done.clone(), project: project.clone(), published: Some(published.clone()), tx: tx.clone() };
        entry.running = Some(Running { tag: intent.tag, task: Task { done, cancel, job: None, priority: intent.priority }, published });
        self.running += 1; *background += usize::from(intent.priority == Priority::Background);
        jobs.submit(intent.priority, move || { if lease.wanted() { (intent.work)(lease); } });
    }
    pub(super) fn result_background(&self) -> usize { self.results.values().filter(|entry| entry.running.as_ref().is_some_and(|run| run.task.priority == Priority::Background)).count() }
    pub(super) fn close_results(&mut self) { for entry in self.results.values_mut() { if let Some(run) = &entry.running { run.task.cancel.store(false, Ordering::Release); } entry.pending = None; } }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{sync::mpsc, time::{Duration, Instant}};

    fn fence(server: u64, request: u64) -> ResultFence {
        ResultFence { slot: ResultSlot { server, lane: Lane::Foreground }, project: 0, request, stage: ResultStage::Targets, result_set: None, node: None, source: None }
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
}
