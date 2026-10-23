//! One running and one replaceable pending logical-layout job per visible pane.
use std::sync::{Arc, atomic::{AtomicU64, Ordering}};
use crate::{jobs::{Jobs, Priority}, layout::{Delta, Finished, Request}, session::Completion, text::TextSnapshot};

struct Pending { request: Request, snapshot: Arc<TextSnapshot>, ticket: u64 }
#[derive(Default)]
struct Slot {
    running: Option<u64>, pending: Option<Pending>, wanted: Option<Request>, latest: Arc<AtomicU64>,
}
#[derive(Default)]
pub(crate) struct Scheduler { slots: [Slot; 2], ticket: u64 }

impl Scheduler {
    pub(crate) fn prepare(&mut self, request: Request, snapshot: Arc<TextSnapshot>) {
        let Some(slot) = self.slots.get_mut(request.pane as usize) else { return };
        if slot.wanted.as_ref() == Some(&request) { return; }
        self.ticket = self.ticket.wrapping_add(1).max(1);
        slot.latest.store(self.ticket, Ordering::Relaxed);
        slot.wanted = Some(request.clone());
        slot.pending = Some(Pending { request, snapshot, ticket: self.ticket });
    }
    pub(crate) fn invalidate(&mut self, current: impl Fn(&Request) -> bool) {
        for slot in &mut self.slots {
            if slot.wanted.as_ref().is_some_and(|request| !current(request)) { Self::cancel(slot); }
        }
    }
    fn cancel(slot: &mut Slot) {
        slot.latest.store(0, Ordering::Relaxed); slot.wanted = None; slot.pending = None;
    }
    pub(crate) fn cancel_all(&mut self) { for slot in &mut self.slots { Self::cancel(slot); } }
    pub(crate) fn pump(&mut self, jobs: &Jobs, tx: &async_channel::Sender<Completion>) {
        for (pane, slot) in self.slots.iter_mut().enumerate() {
            if slot.running.is_some() { continue; }
            let Some(pending) = slot.pending.take() else { continue };
            slot.running = Some(pending.ticket);
            let (tx, latest) = (tx.clone(), slot.latest.clone());
            jobs.submit(Priority::Visible, move || {
                let current = || latest.load(Ordering::Relaxed) == pending.ticket;
                let delta = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| pending.request.work.clone().run(&pending.snapshot, 16 * crate::projection::WINDOW_WORK, current))).ok().flatten();
                let _ = tx.send_blocking(Completion::LaidOut(Box::new(Finished { pane: pane as u8, ticket: pending.ticket, request: pending.request, delta })));
            });
        }
    }
    pub(crate) fn finish(&mut self, done: Finished, current: impl Fn(&Request) -> bool) -> Option<(Request, Delta)> {
        let slot = self.slots.get_mut(done.pane as usize)?;
        if slot.running != Some(done.ticket) { return None; }
        slot.running = None;
        if slot.latest.load(Ordering::Relaxed) != done.ticket || !current(&done.request) { return None; }
        slot.wanted = None;
        done.delta.map(|delta| (done.request, delta))
    }
}

impl Drop for Scheduler { fn drop(&mut self) { self.cancel_all(); } }
