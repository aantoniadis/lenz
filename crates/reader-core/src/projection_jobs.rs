//! Two visible panes, each with one running projection and one replaceable pending request.

use std::sync::{Arc, atomic::{AtomicU64, Ordering}};
use crate::{jobs::{Jobs, Priority}, projection::{Batch, Finished, Request, WINDOW_WORK}, session::Completion, text::TextSnapshot};

pub(crate) const BATCH_WINDOWS: usize = 16;

struct Pending {
    request: Request,
    snapshot: Arc<TextSnapshot>,
    ticket: u64,
}

#[derive(Default)]
struct Slot {
    running: Option<u64>,
    pending: Option<Pending>,
    wanted: Option<Request>,
    latest: Arc<AtomicU64>,
    ready: Option<Arc<Batch>>,
}

#[derive(Default)]
pub(crate) struct Scheduler {
    slots: [Slot; 2],
    ticket: u64,
}

impl Scheduler {
    pub(crate) fn ready(&self, pane: u8) -> Option<&Arc<Batch>> {
        self.slots.get(pane as usize)?.ready.as_ref()
    }

    pub(crate) fn prepare(&mut self, request: Request, snapshot: Arc<TextSnapshot>) {
        let Some(slot) = self.slots.get_mut(request.pane as usize) else { return };
        if slot.wanted.as_ref().is_some_and(|old| old.request >= request.request) { return; }
        self.ticket = self.ticket.wrapping_add(1).max(1);
        slot.latest.store(self.ticket, Ordering::Relaxed);
        slot.wanted = Some(request.clone());
        slot.ready = None;
        slot.pending = Some(Pending { request, snapshot, ticket: self.ticket });
    }

    pub(crate) fn cancel(&mut self, pane: u8, request: u64) {
        if let Some(slot) = self.slots.get_mut(pane as usize).filter(|s| s.wanted.as_ref().is_some_and(|w| w.request == request)) {
            slot.latest.store(0, Ordering::Relaxed);
            slot.wanted = None;
            slot.pending = None;
            slot.ready = None;
        }
    }

    pub(crate) fn invalidate(&mut self, current: impl Fn(&Request) -> bool) {
        for slot in &mut self.slots {
            if slot.wanted.as_ref().is_some_and(|r| !current(r)) {
                slot.latest.store(0, Ordering::Relaxed);
                slot.wanted = None;
                slot.pending = None;
                slot.ready = None;
            }
        }
    }

    pub(crate) fn cancel_all(&mut self) {
        for slot in &mut self.slots {
            slot.latest.store(0, Ordering::Relaxed);
            slot.wanted = None;
            slot.pending = None;
            slot.ready = None;
        }
    }

    pub(crate) fn pump(&mut self, jobs: &Jobs, tx: &async_channel::Sender<Completion>) {
        for (pane, slot) in self.slots.iter_mut().enumerate() {
            if slot.running.is_some() { continue; }
            let Some(work) = slot.pending.take() else { continue };
            slot.running = Some(work.ticket);
            let (latest, tx) = (slot.latest.clone(), tx.clone());
            jobs.submit(Priority::Visible, move || {
                let mut cursor = work.request.cursor.clone();
                let mut windows = Vec::new();
                let mut complete = false;
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    for _ in 0..BATCH_WINDOWS {
                        if latest.load(Ordering::Relaxed) != work.ticket { return None; }
                        let window = cursor.window(&work.snapshot, WINDOW_WORK)?;
                        complete = window.complete;
                        windows.push(window);
                        if complete { break; }
                    }
                    Some(())
                }));
                let batch = match result {
                    Ok(Some(())) => Some(Batch { request: work.request, windows, next: cursor, complete, failed: false }),
                    Ok(None) if latest.load(Ordering::Relaxed) != work.ticket => None,
                    Ok(None) => Some(Batch { next: work.request.cursor.clone(), request: work.request, windows: Vec::new(), complete: false, failed: true }),
                    Err(_) => Some(Batch { next: work.request.cursor.clone(), request: work.request, windows: Vec::new(), complete: false, failed: true }),
                };
                let _ = tx.send_blocking(Completion::Projected(Finished { pane: pane as u8, ticket: work.ticket, batch }));
            });
        }
    }

    pub(crate) fn finish(&mut self, done: Finished, current: impl Fn(&Request) -> bool) -> bool {
        let Some(slot) = self.slots.get_mut(done.pane as usize) else { return false };
        if slot.running != Some(done.ticket) { return false; }
        slot.running = None;
        if slot.latest.load(Ordering::Relaxed) != done.ticket { return false; }
        if let Some(batch) = done.batch.filter(|b| current(&b.request)) {
            slot.ready = Some(Arc::new(batch));
            return true;
        }
        false
    }
}

impl Drop for Scheduler {
    fn drop(&mut self) {
        for slot in &self.slots { slot.latest.store(0, Ordering::Relaxed); }
    }
}
