//! Bounded current-buffer analysis; the generic job pool remains the executor.
use std::{collections::{HashMap, VecDeque}, sync::{Arc, OnceLock, Weak, atomic::{AtomicBool, Ordering}}};
use crate::{document::Document, jobs::{Jobs, Priority, MAX_JOBS}, session::Completion, tabs::TabId, text::{BufferId, SnapshotKey, TextSnapshot}};

const BUFFERS: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Consumer { Find(TabId), Syntax, Markdown }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Scope { key: SnapshotKey, source: usize, project: u64 }

#[derive(Clone, Copy, Debug)]
pub struct Ticket { scope: Scope, consumer: Consumer, request: u64 }
impl Ticket {
    pub(crate) fn matches(&self, snapshot: &TextSnapshot, project: u64) -> bool { self.scope == scope(snapshot, project) }
    pub(crate) fn buffer(&self) -> BufferId { self.scope.key.buffer }
}
fn scope(snapshot: &TextSnapshot, project: u64) -> Scope {
    Scope { key: snapshot.key(), source: Arc::as_ptr(snapshot.source_meta()) as usize, project }
}

pub(crate) struct Input { snapshot: Arc<TextSnapshot>, loaded: Weak<Document>, scope: Scope }
impl Input {
    pub(crate) fn new(snapshot: Arc<TextSnapshot>, loaded: Weak<Document>, project: u64) -> Self {
        let scope = scope(&snapshot, project);
        Self { snapshot, loaded, scope }
    }
}

type Work = Box<dyn FnOnce(Arc<Document>, Lease) + Send>;
struct Intent { request: u64, priority: Priority, work: Work }
struct Task { done: Arc<AtomicBool>, cancel: Arc<AtomicBool> }
struct Group {
    scope: Scope,
    snapshot: Weak<TextSnapshot>,
    document: Arc<OnceLock<Arc<Document>>>,
    cancel: Arc<AtomicBool>,
    materializing: Option<Task>,
    inflight: HashMap<Consumer, Task>,
    waiting: HashMap<Consumer, Intent>,
    latest: HashMap<Consumer, u64>,
}
impl Group {
    fn new(input: Input) -> Self {
        let document = OnceLock::new();
        if let Some(loaded) = input.loaded.upgrade().filter(|doc| doc.version() == input.scope.key.version && Arc::ptr_eq(doc.source_meta(), input.snapshot.source_meta())) {
            let _ = document.set(loaded);
        }
        Self { scope: input.scope, snapshot: Arc::downgrade(&input.snapshot), document: Arc::new(document), cancel: Arc::new(AtomicBool::new(true)), materializing: None, inflight: HashMap::new(), waiting: HashMap::new(), latest: HashMap::new() }
    }
    fn running(&self) -> bool { self.materializing.is_some() || !self.inflight.is_empty() }
    fn cancel(&mut self) {
        self.cancel.store(false, Ordering::Release);
        self.waiting.clear();
        self.latest.clear();
    }
}
struct Pending { input: Input, intents: HashMap<Consumer, Intent> }
struct Entry { group: Group, pending: Option<Pending>, queued: bool, live: bool }

/// A consumer's result and completion lease. Dropping it always releases the CPU slot,
/// including cancellation, an unwinding parser, and a closed/full result channel.
pub(crate) struct Lease {
    ticket: Ticket,
    group: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
    done: Arc<AtomicBool>,
    tx: async_channel::Sender<Completion>,
}
impl Lease {
    pub(crate) fn wanted(&self) -> bool { self.group.load(Ordering::Acquire) && self.cancel.load(Ordering::Acquire) }
    pub(crate) fn send(&self, result: Completion) -> bool {
        self.wanted() && self.tx.send_blocking(Completion::Analyzed { ticket: self.ticket, result: Box::new(result) }).is_ok()
    }
    pub(crate) fn cancellation(&self) -> impl Fn() -> bool + Send + Clone + 'static {
        let (group, cancel) = (self.group.clone(), self.cancel.clone());
        move || group.load(Ordering::Acquire) && cancel.load(Ordering::Acquire)
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        self.done.store(true, Ordering::Release);
        // A full inbox already guarantees a dispatch; dispatch also reaps completed leases.
        let _ = self.tx.try_send(Completion::AnalysisWake);
    }
}

#[derive(Default)]
pub(crate) struct Analysis {
    entries: HashMap<BufferId, Entry>, queue: VecDeque<BufferId>, running: usize,
    #[cfg(test)]
    pub(crate) materializations: Arc<std::sync::atomic::AtomicUsize>,
}
impl Analysis {
    pub(crate) fn request(&mut self, input: Input, consumer: Consumer, request: u64, priority: Priority, work: impl FnOnce(Arc<Document>, Lease) + Send + 'static) -> bool {
        let id = input.scope.key.buffer;
        if !self.entries.contains_key(&id) && self.entries.len() == BUFFERS {
            let discard = self.entries.iter().find(|(_, entry)| !entry.live && !entry.group.running()).map(|(id, _)| *id);
            let Some(discard) = discard else { return false };
            self.entries.remove(&discard);
            self.queue.retain(|id| *id != discard);
        }
        let intent = Intent { request, priority, work: Box::new(work) };
        if let Some(entry) = self.entries.get_mut(&id) {
            entry.live = true;
            if entry.group.scope != input.scope || !entry.group.cancel.load(Ordering::Acquire) {
                entry.group.cancel();
                if entry.pending.as_ref().is_none_or(|pending| pending.input.scope != input.scope) { entry.pending = Some(Pending { input, intents: HashMap::new() }); }
                if let Some(pending) = &mut entry.pending { pending.intents.insert(consumer, intent); }
            } else {
                if entry.group.latest.get(&consumer) == Some(&request) { return true; }
                if let Some(old) = entry.group.inflight.get(&consumer) { old.cancel.store(false, Ordering::Release); }
                entry.group.latest.insert(consumer, request);
                entry.group.waiting.insert(consumer, intent);
            }
        } else {
            let mut group = Group::new(input);
            group.latest.insert(consumer, request);
            group.waiting.insert(consumer, intent);
            self.entries.insert(id, Entry { group, pending: None, queued: false, live: true });
        }
        self.enqueue(id);
        true
    }

    fn enqueue(&mut self, id: BufferId) {
        if let Some(entry) = self.entries.get_mut(&id) && !entry.queued {
            entry.queued = true;
            self.queue.push_back(id);
        }
    }

    pub(crate) fn accepts(&self, ticket: Ticket) -> bool {
        self.entries.get(&ticket.scope.key.buffer).is_some_and(|entry| entry.pending.is_none() && entry.group.scope == ticket.scope
            && entry.group.cancel.load(Ordering::Acquire) && entry.group.latest.get(&ticket.consumer) == Some(&ticket.request))
    }

    pub(crate) fn forget(&mut self, id: BufferId) {
        if let Some(entry) = self.entries.get_mut(&id) { entry.live = false; entry.pending = None; entry.queued = false; entry.group.cancel(); }
        self.queue.retain(|queued| *queued != id);
        if self.entries.get(&id).is_some_and(|entry| !entry.group.running()) { self.entries.remove(&id); }
    }

    pub(crate) fn invalidate(&mut self, snapshot: &TextSnapshot, project: u64) {
        let current = scope(snapshot, project);
        let id = current.key.buffer;
        if let Some(entry) = self.entries.get_mut(&id)
            && entry.pending.as_ref().map_or(entry.group.scope, |pending| pending.input.scope) != current
        {
            entry.group.cancel();
            entry.pending = None;
            entry.queued = false;
            self.queue.retain(|queued| *queued != id);
        }
    }

    pub(crate) fn close(&mut self) {
        for entry in self.entries.values_mut() { entry.group.cancel(); entry.pending = None; }
        self.queue.clear();
    }

    pub(crate) fn pump(&mut self, jobs: &Jobs, tx: &async_channel::Sender<Completion>) {
        let mut retry = Vec::new();
        self.entries.retain(|id, entry| {
            if entry.group.materializing.as_ref().is_some_and(|task| task.done.load(Ordering::Acquire)) {
                entry.group.materializing = None;
                self.running -= 1;
            }
            entry.group.inflight.retain(|_, task| {
                let keep = !task.done.load(Ordering::Acquire);
                if !keep { self.running -= 1; }
                keep
            });
            if !entry.group.running() && let Some(pending) = entry.pending.take() {
                entry.group = Group::new(pending.input);
                for (consumer, intent) in pending.intents {
                    entry.group.latest.insert(consumer, intent.request);
                    entry.group.waiting.insert(consumer, intent);
                }
            }
            if !entry.group.waiting.is_empty() { retry.push(*id); }
            (entry.live && entry.group.cancel.load(Ordering::Acquire)) || entry.group.running()
        });
        for id in retry { self.enqueue(id); }
        let mut turns = self.queue.len().saturating_add(MAX_JOBS);
        while self.running < MAX_JOBS && turns > 0 {
            turns -= 1;
            let Some(id) = self.queue.pop_front() else { break };
            let Some(entry) = self.entries.get_mut(&id) else { continue };
            entry.queued = false;
            let group = &mut entry.group;
            if !group.cancel.load(Ordering::Acquire) || entry.pending.is_some() { continue; }
            if group.document.get().is_none() && group.materializing.is_some() { continue; }
            let intent = group.waiting.iter().filter(|(consumer, _)| !group.inflight.contains_key(consumer))
                .min_by_key(|(_, intent)| match intent.priority { Priority::Visible => 0, Priority::Background => 1 }).map(|(key, _)| *key);
            let Some(consumer) = intent else { continue };
            let Some(intent) = group.waiting.remove(&consumer) else { continue };
            let (done, cancel) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(true)));
            let lease = Lease { ticket: Ticket { scope: group.scope, consumer, request: intent.request }, group: group.cancel.clone(), cancel: cancel.clone(), done: done.clone(), tx: tx.clone() };
            if let Some(doc) = group.document.get().cloned() {
                group.inflight.insert(consumer, Task { done, cancel });
                self.running += 1;
                jobs.submit(intent.priority, move || { if lease.wanted() { (intent.work)(doc, lease); } });
            } else {
                group.waiting.insert(consumer, intent);
                if group.materializing.is_some() { continue; }
                let Some(snapshot) = group.snapshot.upgrade() else { continue };
                group.materializing = Some(Task { done, cancel });
                self.running += 1;
                let document = group.document.clone();
                #[cfg(test)]
                let materializations = self.materializations.clone();
                let priority = group.waiting.get(&consumer).map_or(Priority::Background, |intent| intent.priority);
                jobs.submit(priority, move || {
                    if lease.wanted() {
                        #[cfg(test)]
                        materializations.fetch_add(1, Ordering::Relaxed);
                        let doc = Arc::new(snapshot.materialize());
                        if lease.wanted() { let _ = document.set(doc); }
                    }
                });
            }
            if !group.waiting.is_empty() { entry.queued = true; self.queue.push_back(id); }
        }
    }
}
