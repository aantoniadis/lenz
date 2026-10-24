//! Bounded current-buffer analysis; the generic job pool remains the executor.
use std::{collections::{HashMap, VecDeque}, sync::{Arc, OnceLock, Weak, atomic::{AtomicBool, AtomicU64, Ordering}}};
use crate::{document::{Document, SourceMeta}, jobs::{Jobs, Priority, MAX_JOBS}, session::Completion, tabs::TabId, text::{BufferId, SnapshotKey, TextSnapshot}};

const BUFFERS: usize = 64;
#[path = "analysis_result.rs"]
mod result;
#[path = "analysis_write.rs"]
mod write;
pub(crate) use result::{ResultFence, ResultSlot, ResultStage};
use result::{ResultEntry, ResultTicket};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Owner { Buffer(BufferId), Result(ResultSlot), Write(BufferId) }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Consumer { Blame(TabId), Find(TabId), Selection(TabId), RootKind, Syntax, Markdown, Index, Ready, Documentation, Decode(crate::decor::Decor) }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Scope { key: SnapshotKey, source: usize, project: u64 }

#[derive(Clone, Debug)]
pub enum Ticket { Source(SourceTicket), Result(ResultTicket) }
#[derive(Clone, Debug)]
pub struct SourceTicket { scope: Scope, consumer: Consumer, request: u64, run: u64, metadata: Weak<SourceMeta> }
impl Ticket {
    pub(crate) fn matches(&self, snapshot: &TextSnapshot, project: u64) -> bool {
        match self { Self::Source(t) => t.scope == scope(snapshot, project) && t.metadata.ptr_eq(&Arc::downgrade(snapshot.source_meta())), Self::Result(_) => false }
    }
    pub(crate) fn buffer(&self) -> Option<BufferId> { match self { Self::Source(t) => Some(t.scope.key.buffer), Self::Result(_) => None } }
    pub(crate) fn result(&self) -> Option<&ResultTicket> { match self { Self::Result(t) => Some(t), Self::Source(_) => None } }
    fn project(&self) -> u64 { match self { Self::Source(t) => t.scope.project, Self::Result(t) => t.fence.project } }
}
fn scope(snapshot: &TextSnapshot, project: u64) -> Scope {
    Scope { key: snapshot.key(), source: Arc::as_ptr(snapshot.source_meta()) as usize, project }
}

pub(crate) struct Input { snapshot: Arc<TextSnapshot>, loaded: Weak<Document>, scope: Scope, live: bool }
impl Input {
    pub(crate) fn new(snapshot: Arc<TextSnapshot>, loaded: Weak<Document>, project: u64, live: bool) -> Self {
        let scope = scope(&snapshot, project);
        Self { snapshot, loaded, scope, live }
    }
}

type Work = Box<dyn FnOnce(Arc<Document>, Lease) + Send>;
struct Intent { request: u64, priority: Priority, work: Work }
struct Task { done: Arc<AtomicBool>, cancel: Arc<AtomicBool>, job: Option<u64>, priority: Priority }
/// One admitted immutable version, shared by current consumers and captured result intents.
/// Workers capture its snapshot/cell/cancellation, never this owner handle.
struct SourceDependency {
    scope: Scope,
    snapshot: Arc<TextSnapshot>,
    document: Arc<OnceLock<Arc<Document>>>,
}
enum SourceReady { Ready(Arc<Document>), Waiting, Failed(&'static str) }
struct Conversion { source: Weak<SourceDependency>, retired: Weak<SourceDependency>, materializing: Option<Task>, failure: Option<&'static str> }
struct ConversionDone { done: Arc<AtomicBool>, tx: async_channel::Sender<Completion> }
impl Drop for ConversionDone {
    fn drop(&mut self) { self.done.store(true, Ordering::Release); let _ = self.tx.try_send(Completion::AnalysisWake); }
}
struct Group {
    run: u64,
    metadata: Weak<SourceMeta>,
    scope: Scope,
    source: Option<Arc<SourceDependency>>,
    cancel: Arc<AtomicBool>,
    inflight: HashMap<Consumer, Task>,
    waiting: HashMap<Consumer, Intent>,
    latest: HashMap<Consumer, u64>,
}
impl Group {
    fn new(source: Arc<SourceDependency>, run: u64) -> Self {
        Self { run, metadata: Arc::downgrade(source.snapshot.source_meta()), scope: source.scope, source: Some(source),
            cancel: Arc::new(AtomicBool::new(true)), inflight: HashMap::new(), waiting: HashMap::new(), latest: HashMap::new() }
    }
    fn running(&self, conversions: &HashMap<Scope, Conversion>) -> bool {
        !self.inflight.is_empty() || conversions.get(&self.scope).is_some_and(|c| c.materializing.is_some())
    }
    fn cancel(&mut self) {
        self.cancel.store(false, Ordering::Release);
        self.source = None;
        self.waiting.clear();
        self.latest.clear();
    }
}
struct Pending { source: Arc<SourceDependency>, live: bool, intents: HashMap<Consumer, Intent> }
struct Entry { group: Group, pending: Option<Pending>, queued: bool, live: bool }

/// A consumer's result and completion lease. Dropping it always releases the CPU slot,
/// including cancellation, an unwinding parser, and a closed/full result channel.
pub(crate) struct Lease {
    ticket: Ticket,
    group: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
    done: Arc<AtomicBool>,
    project: Arc<AtomicU64>,
    published: Option<Arc<AtomicBool>>,
    tx: async_channel::Sender<Completion>,
}
impl Lease {
    pub(crate) fn wanted(&self) -> bool { self.group.load(Ordering::Acquire) && self.cancel.load(Ordering::Acquire) && self.project.load(Ordering::Acquire) == self.ticket.project() }
    pub(crate) fn ticket(&self) -> Ticket { self.ticket.clone() }
    pub(crate) fn send(&self, result: Completion) -> bool {
        if !self.wanted() { return false; }
        if self.published.as_ref().is_some_and(|sent| sent.swap(true, Ordering::AcqRel)) { return false; }
        let sent = self.tx.send_blocking(Completion::Analyzed { ticket: self.ticket.clone(), result: Box::new(result) }).is_ok();
        if !sent && let Some(published) = &self.published { published.store(false, Ordering::Release); }
        sent
    }
    pub(crate) fn cancellation(&self) -> impl Fn() -> bool + Send + Clone + 'static {
        let (group, cancel, project, expected) = (self.group.clone(), self.cancel.clone(), self.project.clone(), self.ticket.project());
        move || group.load(Ordering::Acquire) && cancel.load(Ordering::Acquire) && project.load(Ordering::Acquire) == expected
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
    entries: HashMap<BufferId, Entry>, conversions: HashMap<Scope, Conversion>, results: HashMap<ResultSlot, ResultEntry>, writes: HashMap<BufferId, write::WriteEntry>, queue: VecDeque<Owner>, failures: HashMap<ResultSlot, (ResultFence, String)>, running: usize, next: u64,
    pub(crate) revision: u64,
    dirty: bool, project: u64,
    visible_reservation: Option<crate::jobs::VisibleReservation>,
    #[cfg(test)]
    pub(crate) materializations: Arc<std::sync::atomic::AtomicUsize>,
}
impl Analysis {
    /// Called only after a buffer/result owner has been admitted to the combined owner bound.
    fn source_dependency(&mut self, input: Input) -> Arc<SourceDependency> {
        if let Some(conversion) = self.conversions.get_mut(&input.scope)
            && let Some(previous) = conversion.source.upgrade().or_else(|| conversion.retired.upgrade()) {
            // The fallback preserves captured-result demand, never old backing for a reopened current group.
            if previous.snapshot.needs_compaction() && !input.snapshot.needs_compaction() {
                let source = Arc::new(SourceDependency { scope: input.scope, snapshot: input.snapshot, document: previous.document.clone() });
                conversion.retired = Arc::downgrade(&previous); conversion.source = Arc::downgrade(&source);
                return source;
            }
            return previous;
        }
        let document = OnceLock::new();
        if let Some(loaded) = input.loaded.upgrade().filter(|doc| doc.version() == input.scope.key.version && Arc::ptr_eq(doc.source_meta(), input.snapshot.source_meta())) {
            let _ = document.set(loaded);
        }
        let source = Arc::new(SourceDependency { scope: input.scope, snapshot: input.snapshot, document: Arc::new(document) });
        let conversion = self.conversions.entry(input.scope).or_insert_with(|| Conversion { source: Weak::new(), retired: Weak::new(), materializing: None, failure: None });
        // A previous owner expired while its cancelled worker unwinds. Reuse the slot, not its cell.
        if let Some(task) = &conversion.materializing { task.cancel.store(false, Ordering::Release); }
        conversion.source = Arc::downgrade(&source);
        conversion.retired = Weak::new();
        conversion.failure = None;
        self.dirty = true;
        source
    }

    pub(crate) fn request(&mut self, mut input: Input, consumer: Consumer, request: u64, priority: Priority, work: impl FnOnce(Arc<Document>, Lease) + Send + 'static) -> bool {
        let id = input.scope.key.buffer;
        self.dirty = true;
        let live = input.live;
        if !self.entries.contains_key(&id) && self.entries.len() + self.results.len() + self.writes.len() >= BUFFERS {
            let discard = self.entries.iter().find(|(_, entry)| !entry.live && !entry.group.running(&self.conversions)).map(|(id, _)| *id);
            let Some(discard) = discard else { return false };
            self.entries.remove(&discard);
            self.queue.retain(|id| *id != Owner::Buffer(discard));
        }
        if let Some(group) = self.entries.get(&id).map(|entry| &entry.group)
            && group.scope.key == input.scope.key && group.metadata.ptr_eq(&Arc::downgrade(input.snapshot.source_meta()))
            && let Some(doc) = group.source.as_ref().and_then(|source| source.document.get()) { input.loaded = Arc::downgrade(doc); }
        let source = self.source_dependency(input);
        let intent = Intent { request, priority, work: Box::new(work) };
        if let Some(entry) = self.entries.get_mut(&id) {
            entry.live = live;
            if entry.group.scope != source.scope || !entry.group.cancel.load(Ordering::Acquire) {
                entry.group.cancel();
                if entry.pending.as_ref().is_none_or(|pending| pending.source.scope != source.scope) { entry.pending = Some(Pending { source, live, intents: HashMap::new() }); }
                if let Some(pending) = &mut entry.pending { pending.intents.insert(consumer, intent); }
            } else {
                if entry.group.latest.get(&consumer) == Some(&request) {
                    if priority == Priority::Visible && let Some(waiting) = entry.group.waiting.get_mut(&consumer) { waiting.priority = Priority::Visible; }
                    return true;
                }
                if let Some(old) = entry.group.inflight.get(&consumer) { old.cancel.store(false, Ordering::Release); }
                entry.group.latest.insert(consumer, request);
                entry.group.waiting.insert(consumer, intent);
            }
        } else {
            self.next += 1;
            let mut group = Group::new(source, self.next);
            group.latest.insert(consumer, request);
            group.waiting.insert(consumer, intent);
            self.entries.insert(id, Entry { group, pending: None, queued: false, live });
        }
        self.enqueue(id);
        true
    }

    /// Register demand without scheduling a consumer: the matching loaded document is available in
    /// this turn; a differing draft is materialized once by the existing workers.
    pub(crate) fn ensure(&mut self, input: Input, priority: Priority) -> bool {
        self.request(input, Consumer::Ready, 0, priority, |_, _| {})
    }

    pub(crate) fn ready(&self, snapshot: &TextSnapshot, project: u64) -> Option<Arc<Document>> {
        let entry = self.entries.get(&snapshot.key().buffer)?;
        (entry.pending.is_none() && entry.group.scope == scope(snapshot, project) && entry.group.cancel.load(Ordering::Acquire))
            .then(|| entry.group.source.as_ref().and_then(|source| source.document.get().cloned())).flatten()
    }

    /// Identical immutable bytes with compact backing reuse the same admitted conversion cell.
    /// Captured result owners keep their original dependency until their own acknowledgement.
    pub(crate) fn rebind_backing(&mut self, snapshot: Arc<TextSnapshot>, project: u64) {
        let scope = scope(&snapshot, project);
        let Some(entry) = self.entries.get_mut(&scope.key.buffer) else { return };
        let owner = if entry.group.scope == scope && entry.group.source.is_some() { &mut entry.group.source }
            else if let Some(pending) = entry.pending.as_mut().filter(|p| p.source.scope == scope) {
                let previous = Arc::downgrade(&pending.source);
                let source = Arc::new(SourceDependency { scope, snapshot, document: pending.source.document.clone() });
                pending.source = source.clone();
                if let Some(conversion) = self.conversions.get_mut(&scope) { conversion.retired = previous; conversion.source = Arc::downgrade(&source); }
                return;
            } else { return };
        let Some(previous) = owner else { return };
        let retired = Arc::downgrade(previous);
        let source = Arc::new(SourceDependency { scope, snapshot, document: previous.document.clone() });
        *owner = Some(source.clone());
        if let Some(conversion) = self.conversions.get_mut(&scope) { conversion.retired = retired; conversion.source = Arc::downgrade(&source); }
    }

    pub(crate) fn source_failure(&self, snapshot: &TextSnapshot, project: u64) -> Option<&'static str> {
        self.conversions.get(&scope(snapshot, project)).and_then(|conversion| conversion.failure)
    }

    fn enqueue(&mut self, id: BufferId) {
        if let Some(entry) = self.entries.get_mut(&id) && !entry.queued {
            entry.queued = true;
            self.queue.push_back(Owner::Buffer(id));
        }
        if self.entries.get(&id).is_some_and(|entry| entry.group.waiting.values().any(|intent| intent.priority == Priority::Visible)
            || entry.pending.as_ref().is_some_and(|pending| pending.intents.values().any(|intent| intent.priority == Priority::Visible)))
            && let Some(at) = self.queue.iter().position(|queued| *queued == Owner::Buffer(id))
        { self.queue.remove(at); self.queue.push_front(Owner::Buffer(id)); }
    }

    pub(crate) fn accepts(&self, ticket: &Ticket) -> bool {
        match ticket {
            Ticket::Source(t) => self.entries.get(&t.scope.key.buffer).is_some_and(|entry| entry.pending.is_none() && entry.group.scope == t.scope
                && entry.group.run == t.run && entry.group.cancel.load(Ordering::Acquire) && entry.group.latest.get(&t.consumer) == Some(&t.request)),
            Ticket::Result(t) => self.results.get(&t.slot).is_some_and(|entry| entry.latest.serial == t.serial && entry.ack != t.serial),
        }
    }

    pub(crate) fn cancel_consumer(&mut self, id: BufferId, consumer: Consumer) {
        self.dirty = true;
        if let Some(entry) = self.entries.get_mut(&id) {
            entry.group.latest.remove(&consumer);
            entry.group.waiting.remove(&consumer);
            if let Some(task) = entry.group.inflight.get(&consumer) { task.cancel.store(false, Ordering::Release); }
            if let Some(pending) = &mut entry.pending { pending.intents.remove(&consumer); }
        }
    }

    pub(crate) fn release_view(&mut self, id: BufferId, tab: TabId, last: bool) {
        self.cancel_consumer(id, Consumer::Find(tab));
        self.cancel_consumer(id, Consumer::Selection(tab));
        self.cancel_consumer(id, Consumer::Blame(tab));
        if !last { return; }
        for consumer in [Consumer::Syntax, Consumer::RootKind, Consumer::Markdown, Consumer::Ready, Consumer::Documentation, Consumer::Decode(crate::decor::Decor::Hints), Consumer::Decode(crate::decor::Decor::Tokens)] { self.cancel_consumer(id, consumer); }
        if let Some(entry) = self.entries.get_mut(&id) {
            entry.live = false;
            if let Some(pending) = &mut entry.pending { pending.live = false; }
            let indexing = entry.group.latest.contains_key(&Consumer::Index) || entry.pending.as_ref().is_some_and(|p| p.intents.contains_key(&Consumer::Index));
            if !indexing { entry.pending = None; entry.group.cancel(); }
        }
    }

    pub(crate) fn forget(&mut self, id: BufferId) {
        self.dirty = true;
        if let Some(entry) = self.entries.get_mut(&id) { entry.live = false; entry.pending = None; entry.queued = false; entry.group.cancel(); }
        self.queue.retain(|queued| *queued != Owner::Buffer(id));
        if self.entries.get(&id).is_some_and(|entry| !entry.group.running(&self.conversions)) { self.entries.remove(&id); }
    }

    pub(crate) fn invalidate(&mut self, snapshot: &TextSnapshot, project: u64) {
        self.dirty = true;
        let current = scope(snapshot, project);
        let id = current.key.buffer;
        if let Some(entry) = self.entries.get_mut(&id)
            && entry.pending.as_ref().map_or(entry.group.scope, |pending| pending.source.scope) != current
        {
            entry.group.cancel();
            entry.pending = None;
            entry.queued = false;
            self.queue.retain(|queued| *queued != Owner::Buffer(id));
        }
    }

    pub(crate) fn retain_sources(&mut self, current: impl Fn(BufferId) -> bool) {
        let gone: Vec<_> = self.entries.keys().copied().filter(|id| !current(*id)).collect();
        for id in gone { self.forget(id); }
    }

    pub(crate) fn close(&mut self) {
        self.visible_reservation = None;
        for entry in self.entries.values_mut() { entry.group.cancel(); entry.pending = None; }
        self.close_results();
        self.close_writes();
        for conversion in self.conversions.values_mut() { if let Some(task) = &conversion.materializing { task.cancel.store(false, Ordering::Release); } }
        self.queue.clear();
    }

    fn reap_conversions(&mut self, project: u64) {
        self.conversions.retain(|scope, conversion| {
            let source = conversion.source.upgrade().or_else(|| conversion.retired.upgrade());
            if (source.is_none() || scope.project != project) && let Some(task) = &conversion.materializing { task.cancel.store(false, Ordering::Release); }
            if let Some(task) = conversion.materializing.take_if(|task| task.done.load(Ordering::Acquire)) {
                self.running -= 1;
                self.revision = self.revision.wrapping_add(1);
                if task.cancel.load(Ordering::Acquire) && scope.project == project && source.as_ref().is_some_and(|source| source.document.get().is_none()) {
                    conversion.failure = Some("source conversion failed or was cancelled");
                }
            }
            source.is_some() || conversion.materializing.is_some()
        });
    }

    fn ensure_source(&mut self, source: &Arc<SourceDependency>, priority: Priority, jobs: &Jobs, tx: &async_channel::Sender<Completion>, project: &Arc<AtomicU64>, background: &mut usize) -> SourceReady {
        if source.scope.project != project.load(Ordering::Acquire) { return SourceReady::Failed("source analysis context changed"); }
        if let Some(document) = source.document.get() { return SourceReady::Ready(document.clone()); }
        let Some(conversion) = self.conversions.get_mut(&source.scope) else { return SourceReady::Failed("source conversion owner was released"); };
        if let Some(why) = conversion.failure { return SourceReady::Failed(why); }
        if priority == Priority::Visible && jobs.has_workers() && self.visible_reservation.is_none() {
            self.visible_reservation = Some(jobs.reserve_visible());
        }
        if let Some(task) = &mut conversion.materializing {
            if priority == Priority::Visible && task.priority == Priority::Background {
                if let Some(job) = task.job { jobs.promote(job); }
                task.priority = Priority::Visible;
                *background = background.saturating_sub(1);
            }
            return SourceReady::Waiting;
        }
        if self.running == 2 * MAX_JOBS || priority == Priority::Background && *background == MAX_JOBS { return SourceReady::Waiting; }
        if !jobs.has_workers() { conversion.failure = Some("source analysis workers are unavailable"); return SourceReady::Failed("source analysis workers are unavailable"); }
        let done = Arc::new(AtomicBool::new(false)); let cancel = Arc::new(AtomicBool::new(true));
        conversion.materializing = Some(Task { done: done.clone(), cancel: cancel.clone(), job: None, priority });
        self.running += 1; *background += usize::from(priority == Priority::Background);
        let (snapshot, document) = (source.snapshot.clone(), source.document.clone());
        let (current_project, expected) = (project.clone(), source.scope.project);
        let finished = ConversionDone { done, tx: tx.clone() };
        #[cfg(test)]
        let materializations = self.materializations.clone();
        let job = jobs.submit_promotable(priority, move || {
            let _finished = finished;
            let current = || cancel.load(Ordering::Acquire) && current_project.load(Ordering::Acquire) == expected;
            if !current() { return; }
            #[cfg(test)]
            materializations.fetch_add(1, Ordering::Relaxed);
            if let Some(doc) = snapshot.materialize_while(current) && current() { let _ = document.set(Arc::new(doc)); }
        });
        if let Some(task) = &mut conversion.materializing { task.job = job; }
        SourceReady::Waiting
    }

    pub(crate) fn pump_requested(&mut self, jobs: &Jobs, tx: &async_channel::Sender<Completion>, project: &Arc<AtomicU64>) {
        if self.dirty { self.pump(jobs, tx, project); }
    }

    pub(crate) fn pump(&mut self, jobs: &Jobs, tx: &async_channel::Sender<Completion>, project: &Arc<AtomicU64>) {
        let current_project = project.load(Ordering::Acquire);
        if !self.dirty && self.running == 0 && self.queue.is_empty() && self.project == current_project { return; }
        self.dirty = false;
        self.project = current_project;
        self.reap_conversions(current_project);
        let mut retry = Vec::new();
        self.entries.retain(|id, entry| {
            if entry.group.scope.project != current_project { entry.group.cancel(); }
            if entry.pending.as_ref().is_some_and(|pending| pending.source.scope.project != current_project) { entry.pending = None; }
            entry.group.inflight.retain(|_, task| {
                let keep = !task.done.load(Ordering::Acquire);
                if !keep { self.running -= 1; }
                keep
            });
            if !entry.group.running(&self.conversions) && let Some(pending) = entry.pending.take() {
                self.next += 1;
                entry.group = Group::new(pending.source, self.next);
                entry.live = pending.live;
                self.revision = self.revision.wrapping_add(1);
                for (consumer, intent) in pending.intents {
                    entry.group.latest.insert(consumer, intent.request);
                    entry.group.waiting.insert(consumer, intent);
                }
            }
            if entry.group.source.as_ref().is_some_and(|source| source.document.get().is_some()) { entry.group.waiting.remove(&Consumer::Ready); }
            if !entry.group.waiting.is_empty() { retry.push(*id); }
            (entry.group.cancel.load(Ordering::Acquire) && (entry.live || !entry.group.waiting.is_empty())) || entry.group.running(&self.conversions)
        });
        self.reap_results();
        self.reap_writes();
        self.queue.retain(|id| match id { Owner::Buffer(id) => self.entries.contains_key(id), Owner::Result(slot) => self.results.contains_key(slot), Owner::Write(id) => self.writes.contains_key(id) });
        for id in retry { self.enqueue(id); }
        // Background work cannot occupy the visible queue's entire admission allowance. The same
        // three workers execute both classes; at most six task closures can be submitted here.
        let mut background: usize = self.entries.values().map(|e| e.group.inflight.values().filter(|t| t.priority == Priority::Background).count()).sum();
        background += self.conversions.values().filter(|c| c.materializing.as_ref().is_some_and(|t| t.priority == Priority::Background)).count();
        background += self.result_background();
        let mut turns = self.queue.len().saturating_add(2 * MAX_JOBS);
        while self.running < 2 * MAX_JOBS && turns > 0 {
            turns -= 1;
            let Some(owner) = self.queue.pop_front() else { break };
            let id = match owner {
                Owner::Buffer(id) => id,
                Owner::Write(id) => { self.submit_write(id, jobs, tx, project, &mut background); continue; }
                Owner::Result(slot) => { self.submit_result(slot, jobs, tx, project, &mut background); continue; }
            };
            let Some(entry) = self.entries.get_mut(&id) else { continue };
            entry.queued = false;
            if !entry.group.cancel.load(Ordering::Acquire) || entry.pending.is_some() { continue; }
            let intent = entry.group.waiting.iter().filter(|(consumer, _)| !entry.group.inflight.contains_key(consumer))
                .min_by_key(|(_, intent)| match intent.priority { Priority::Visible => 0, Priority::Background => 1 }).map(|(key, value)| (*key, value.priority));
            let (Some((consumer, priority)), Some(source)) = (intent, entry.group.source.clone()) else { continue };
            match self.ensure_source(&source, priority, jobs, tx, project, &mut background) {
                SourceReady::Waiting => { self.enqueue(id); continue; }
                SourceReady::Failed(_) => { if let Some(entry) = self.entries.get_mut(&id) { entry.group.cancel(); } continue; }
                SourceReady::Ready(doc) => {
                    let Some(entry) = self.entries.get_mut(&id) else { continue };
                    let group = &mut entry.group;
                    let Some(intent) = group.waiting.remove(&consumer) else { continue };
                    if intent.priority == Priority::Background && background == MAX_JOBS { group.waiting.insert(consumer, intent); self.enqueue(id); continue; }
                    let done = Arc::new(AtomicBool::new(false)); let cancel = Arc::new(AtomicBool::new(true));
                    let lease = Lease { ticket: Ticket::Source(SourceTicket { scope: group.scope, consumer, request: intent.request, run: group.run, metadata: group.metadata.clone() }), group: group.cancel.clone(), cancel: cancel.clone(), done: done.clone(), project: project.clone(), published: None, tx: tx.clone() };
                    group.inflight.insert(consumer, Task { done, cancel, job: None, priority });
                    self.running += 1; background += usize::from(priority == Priority::Background);
                    jobs.submit(priority, move || { if lease.wanted() { (intent.work)(doc, lease); } });
                }
            }
            if self.entries.get(&id).is_some_and(|entry| !entry.group.waiting.is_empty()) { self.enqueue(id); }
        }
        // A ready document still needs its Session-thread consumer submission before capacity is released.
        if self.visible_reservation.is_some() && !self.visible_dependency_waiting(current_project) {
            self.visible_reservation = None;
        }
    }

    fn visible_dependency_waiting(&self, project: u64) -> bool {
        self.entries.values().any(|entry| entry.pending.is_none() && entry.group.scope.project == project
            && entry.group.cancel.load(Ordering::Acquire) && entry.group.source.is_some()
            && entry.group.waiting.iter().any(|(consumer, intent)| intent.priority == Priority::Visible && !entry.group.inflight.contains_key(consumer)))
            || self.visible_result_dependency_waiting(project) || self.visible_write_dependency_waiting(project)
    }
}

#[cfg(test)]
mod priority_tests {
    use super::*;
    use std::{sync::mpsc, time::Duration};

    #[test]
    fn visible_result_and_write_dependencies_keep_capacity_until_their_consumers_are_submitted() {
        for writing in [false, true] {
            let jobs = Jobs::new(); let (tx, rx) = async_channel::bounded(64); let project = Arc::new(AtomicU64::new(0));
            let (entered, started) = mpsc::channel();
            let gates: Vec<_> = (0..MAX_JOBS).map(|_| {
                let (release, wait) = mpsc::channel(); let entered = entered.clone();
                jobs.submit(Priority::Visible, move || { entered.send(()).unwrap(); let _ = wait.recv_timeout(Duration::from_secs(5)); }); release
            }).collect();
            for _ in &gates { started.recv_timeout(Duration::from_secs(2)).unwrap(); }
            let background: Vec<_> = (0..MAX_JOBS).map(|_| { let (release, wait) = mpsc::channel(); jobs.submit(Priority::Background, move || { let _ = wait.recv_timeout(Duration::from_secs(5)); }); release }).collect();
            let mut analysis = Analysis::default();
            let snapshot = Arc::new(TextSnapshot::from_document(BufferId(1), &Document::decode("captured.txt".into(), 1, b"captured\n")));
            let input = Input::new(snapshot, Weak::new(), 0, false);
            if writing {
                assert!(analysis.request_write(BufferId(1), 1, 0, || (Some(input), |doc: Option<Arc<Document>>| {
                    assert_eq!(&**doc.unwrap().text(), "captured\n"); crate::save::Outcome::Rejected("observed captured source".into())
                })));
            } else {
                analysis.request_result_document(ResultFence { slot: ResultSlot::Search, project: 0, request: 1, stage: ResultStage::Search, result_set: Some(1), node: None, source: None }, Priority::Visible, || (input, |doc, lease| {
                    assert_eq!(&**doc.text(), "captured\n"); lease.send(Completion::AnalysisWake);
                })).unwrap();
            }
            analysis.pump(&jobs, &tx, &project); gates[0].send(()).unwrap();
            let until = std::time::Instant::now() + Duration::from_secs(2);
            loop {
                analysis.pump(&jobs, &tx, &project);
                if writing {
                    if let Some((buffer, request, crate::save::Outcome::Rejected(why))) = analysis.take_write_outcomes().pop() {
                        assert_eq!((buffer, request, why.as_str()), (BufferId(1), 1, "observed captured source")); break;
                    }
                } else if let Ok(Completion::Analyzed { ticket, .. }) = rx.try_recv() {
                    assert!(analysis.acknowledge_result(ticket.result().unwrap())); break;
                }
                assert!(std::time::Instant::now() < until, "background work occupied the conversion-to-consumer gap; writing={writing}");
                std::thread::yield_now();
            }
            for gate in gates.into_iter().skip(1).chain(background) { gate.send(()).unwrap(); }
        }
    }

    #[test]
    fn cancelled_replaced_closed_and_dropped_dependency_demand_restores_background_capacity() {
        for mode in 0..7 {
            let jobs = Jobs::new(); let (tx, _rx) = async_channel::bounded(64);
            let project = Arc::new(AtomicU64::new(0));
            let (entered, started) = mpsc::channel();
            let gates: Vec<_> = (0..MAX_JOBS).map(|_| {
                let (release, wait) = mpsc::channel(); let entered = entered.clone();
                jobs.submit(Priority::Visible, move || { entered.send(()).unwrap(); let _ = wait.recv_timeout(Duration::from_secs(5)); }); release
            }).collect();
            for _ in &gates { started.recv_timeout(Duration::from_secs(2)).unwrap(); }
            let snapshot = Arc::new(TextSnapshot::from_document(BufferId(1), &Document::decode("draft.txt".into(), 1, b"draft\n")));
            let mut analysis = Some(Analysis::default()); let a = analysis.as_mut().unwrap();
            let input = || Input::new(snapshot.clone(), Weak::new(), 0, false);
            if mode == 5 {
                a.request_result_document(ResultFence { slot: ResultSlot::Search, project: 0, request: 1, stage: ResultStage::Search, result_set: Some(1), node: None, source: None }, Priority::Visible, || (input(), |_, _| {})).unwrap();
            } else if mode == 6 {
                a.request_write(BufferId(1), 1, 0, || (Some(input()), |_| crate::save::Outcome::Rejected("not written".into())));
            } else { a.request(input(), Consumer::Find(TabId(1)), 1, Priority::Visible, |_, _| {}); }
            a.pump(&jobs, &tx, &project);
            match mode {
                0 => a.cancel_consumer(BufferId(1), Consumer::Find(TabId(1))),
                1 => {
                    let newer = Arc::new(snapshot.apply_edits(2, &[crate::text::TextEdit { range: 0..0, text: "new ".into() }]).unwrap());
                    a.request(Input::new(newer, Weak::new(), 0, false), Consumer::Syntax, 2, Priority::Background, |_, _| {});
                }
                2 => { project.store(1, Ordering::Release); }
                3 | 6 => a.close(),
                4 => { analysis = None; }
                5 => a.invalidate_results(|_| false),
                _ => unreachable!(),
            }
            if let Some(a) = &mut analysis { a.pump(&jobs, &tx, &project); }
            let (entered, started) = mpsc::channel();
            let background: Vec<_> = (0..MAX_JOBS).map(|_| {
                let (release, wait) = mpsc::channel(); let entered = entered.clone();
                jobs.submit(Priority::Background, move || { entered.send(()).unwrap(); let _ = wait.recv_timeout(Duration::from_secs(5)); }); release
            }).collect();
            for gate in gates { gate.send(()).unwrap(); }
            for _ in &background { started.recv_timeout(Duration::from_secs(2)).unwrap_or_else(|e| panic!("mode {mode} retained obsolete capacity: {e}")); }
            for gate in background { gate.send(()).unwrap(); }
        }
    }
}
