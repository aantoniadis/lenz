//! Durable buffer write intents share conversion and CPU admission, not read-result cancellation.
use super::*;
use crate::save::Outcome;
use std::sync::Mutex;
type WriteWork = Box<dyn FnOnce(Option<Arc<Document>>) -> Outcome + Send>;
pub(super) struct WriteEntry {
    request: u64,
    source: Option<Arc<SourceDependency>>,
    project: u64,
    pending: Option<WriteWork>,
    submitted: bool,
    done: Arc<AtomicBool>,
    outcome: Arc<Mutex<Option<Outcome>>>,
}
impl Analysis {
    /// Reserve capacity before capturing a snapshot or read-back context. One intent per buffer.
    pub(crate) fn request_write<W: FnOnce(Option<Arc<Document>>) -> Outcome + Send + 'static>(&mut self,
        buffer: BufferId, request: u64, project: u64, build: impl FnOnce() -> (Option<Input>, W)) -> bool {
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
            else if entry.project != project.load(Ordering::Acquire) { Err(Some("save context changed before writing")) }
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
