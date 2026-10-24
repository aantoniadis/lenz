//! Session-local close fences. The coordinator never owns source text.
use super::*;
use crate::{
    close::{Choice, Intent, Permit, Prompt, SessionId},
    text::BufferId,
};

#[derive(PartialEq, Eq)]
struct Owner {
    tab: TabId,
    buffer: Option<BufferId>,
    pane: u8,
    path: PathBuf,
}
struct Fence {
    buffer: BufferId,
    source: SemanticSource,
    consent: Option<Choice>,
    save_request: Option<u64>,
}
pub(super) struct Pending {
    ticket: u64,
    batch: Option<u64>,
    intent: Intent,
    owners: Vec<Owner>,
    focused: u8,
    split: Option<Split>,
    project: u64,
    buffers: Vec<Fence>,
}
impl Session {
    pub fn id(&self) -> SessionId {
        self.id
    }
    pub(crate) fn has_close_intent(&self) -> bool {
        self.closing.is_some()
    }
    pub(crate) fn prepare_close_batch(&mut self, batch: u64) {
        self.command(Command::PrepareClose {
            batch,
            intent: Intent::Session,
        });
    }
    pub(crate) fn cancel_close_batch(&mut self, ticket: u64) {
        self.command(Command::CancelClose { ticket });
    }
    pub(crate) fn commit_close_batch(&mut self, permit: Permit) {
        self.command(Command::CommitClose(permit));
    }
    pub fn last_close(&self) -> Option<&crate::close::Committed> {
        self.last_close.as_ref()
    }
    pub fn closed(&self) -> bool {
        self.closed
    }
    pub(crate) fn close_ticket(&self) -> u64 {
        self.closing.as_ref().map_or(0, |p| p.ticket)
    }
    fn close_owners(&self) -> Vec<Owner> {
        self.state
            .tabs
            .iter()
            .map(|t| Owner {
                tab: t.id,
                buffer: t.buffer,
                pane: t.pane,
                path: t.source.clone(),
            })
            .collect()
    }
    pub fn close_prompt(&self) -> Option<Prompt> {
        let pending = self.closing.as_ref()?;
        for fence in &pending.buffers {
            let entry = self.state.sources.buffers.get(&fence.buffer)?;
            let dirty = entry.draft.as_ref().is_some_and(crate::draft::Draft::dirty);
            let waiting = entry.save.status.busy() || fence.save_request.is_some();
            if fence.consent == Some(Choice::Discard) {
                continue;
            }
            if !dirty && !waiting && !entry.save.status.blocked() {
                continue;
            }
            let source = entry.snapshot.as_ref()?;
            return Some(Prompt {
                ticket: pending.ticket,
                buffer: fence.buffer,
                path: source.source().to_owned(),
                waiting,
                save_disabled: self.save_buffer_disabled(fence.buffer),
                discard_disabled: (waiting || entry.save.status.blocked())
                    .then_some("Wait for a confirmed disk outcome before another close decision"),
            });
        }
        None
    }
    pub(crate) fn prepare_close(&mut self, intent: Intent, batch: Option<u64>) -> Effect {
        if self.closed || self.closing.is_some() {
            return Effect {
                repaint: true,
                copy: None,
            };
        }
        let removed: Vec<_> = match intent {
            Intent::Tab(id) => self
                .state
                .tabs
                .iter()
                .filter(|t| t.id == id)
                .map(|t| t.id)
                .collect(),
            Intent::Session => self.state.tabs.iter().map(|t| t.id).collect(),
            Intent::Unsplit => self
                .state
                .tabs
                .iter()
                .filter(|t| {
                    t.pane != self.state.focused
                        && self
                            .state
                            .tabs
                            .iter()
                            .any(|u| u.pane == self.state.focused && u.source == t.source)
                })
                .map(|t| t.id)
                .collect(),
        };
        let mut buffers = Vec::new();
        for tab in self.state.tabs.iter().filter(|t| removed.contains(&t.id)) {
            let Some(buffer) = tab.buffer else { continue };
            if buffers.iter().any(|f: &Fence| f.buffer == buffer)
                || self
                    .state
                    .tabs
                    .iter()
                    .any(|t| t.buffer == Some(buffer) && !removed.contains(&t.id))
            {
                continue;
            }
            if let Some(source) = self
                .state
                .sources
                .buffers
                .get(&buffer)
                .and_then(|e| e.snapshot.as_ref())
            {
                buffers.push(Fence {
                    buffer,
                    source: SemanticSource::new(source),
                    consent: None,
                    save_request: self
                        .state
                        .sources
                        .buffers
                        .get(&buffer)
                        .filter(|e| e.save.status.busy())
                        .and_then(|e| e.save.captured.as_ref())
                        .map(|c| c.request),
                });
            }
        }
        self.close_seq += 1;
        self.closing = Some(Pending {
            ticket: self.close_seq,
            batch,
            intent,
            owners: self.close_owners(),
            focused: self.state.focused,
            split: self.state.settings.split,
            project: self.project_generation.load(Ordering::Acquire),
            buffers,
        });
        self.advance_close();
        Effect {
            repaint: true,
            copy: None,
        }
    }
    pub(crate) fn cancel_close(&mut self) {
        self.closing = None;
    }
    pub(crate) fn close_valid(&self, ticket: u64) -> bool {
        self.closing.as_ref().is_some_and(|p| {
            p.ticket == ticket
                && p.owners == self.close_owners()
                && p.project == self.project_generation.load(Ordering::Acquire)
                && (!matches!(p.intent, Intent::Unsplit)
                    || p.focused == self.state.focused && p.split == self.state.settings.split)
                && p.buffers.iter().all(|f| {
                    self.state
                        .sources
                        .buffers
                        .get(&f.buffer)
                        .and_then(|e| e.snapshot.as_ref())
                        .is_some_and(|s| f.source.matches(s))
                })
        })
    }
    pub(crate) fn close_ready(&self, ticket: u64) -> bool {
        self.close_valid(ticket)
            && self.close_prompt().is_none()
            && !self
                .state
                .sources
                .buffers
                .values()
                .any(|e| e.save.status.busy() || e.save.captured.is_some())
    }
    pub(crate) fn account_close_writes(&mut self) {
        self.account_saves();
        self.advance_close();
    }
    pub(super) fn advance_close(&mut self) -> bool {
        let Some(pending) = self.closing.as_ref() else {
            return false;
        };
        let ticket = pending.ticket;
        if !self.close_valid(ticket) {
            self.cancel_close();
            return true;
        }
        for fence in &pending.buffers {
            if let Some(request) = fence.save_request {
                let Some(entry) = self.state.sources.buffers.get(&fence.buffer) else {
                    self.cancel_close();
                    return true;
                };
                if entry
                    .save
                    .written
                    .as_ref()
                    .is_some_and(|(r, source)| *r == request && source == &fence.source)
                {
                    continue;
                }
                if entry.save.status.busy() {
                    return false;
                }
                self.cancel_close();
                self.state.messages.push(
                    "Close cancelled: the requested Save was not confirmed. The draft is retained."
                        .into(),
                );
                return true;
            }
        }
        if let Some(pending) = self.closing.as_mut() {
            for fence in &mut pending.buffers {
                if fence.save_request.is_some_and(|request| {
                    self.state
                        .sources
                        .buffers
                        .get(&fence.buffer)
                        .and_then(|e| e.save.written.as_ref())
                        .is_some_and(|(r, _)| *r == request)
                }) {
                    fence.save_request = None;
                    fence.consent = Some(Choice::Save);
                }
            }
        }
        if self.closing.as_ref().is_some_and(|p| p.batch.is_none()) && self.close_ready(ticket) {
            self.commit_close(Permit {
                session: self.id,
                ticket,
            });
            return true;
        }
        false
    }
    pub(super) fn choose_close(&mut self, ticket: u64, buffer: BufferId, choice: Choice) -> Effect {
        if self.close_ticket() != ticket {
            return Effect::default();
        }
        if !self.close_valid(ticket) {
            self.cancel_close();
            return Effect {
                repaint: true,
                copy: None,
            };
        }
        let Some(prompt) = self
            .close_prompt()
            .filter(|p| p.ticket == ticket && p.buffer == buffer)
        else {
            return Effect::default();
        };
        match choice {
            Choice::Cancel => self.cancel_close(),
            Choice::Discard if prompt.discard_disabled.is_none() => {
                if let Some(p) = self.closing.as_mut()
                    && let Some(f) = p.buffers.iter_mut().find(|f| f.buffer == buffer)
                {
                    f.consent = Some(choice);
                }
            }
            Choice::Save if prompt.save_disabled.is_none() => {
                self.save_buffer(buffer);
                let request = self
                    .state
                    .sources
                    .buffers
                    .get(&buffer)
                    .and_then(|e| e.save.captured.as_ref())
                    .map(|c| c.request);
                if let Some(request) = request {
                    if let Some(p) = self.closing.as_mut()
                        && let Some(f) = p.buffers.iter_mut().find(|f| f.buffer == buffer)
                    {
                        f.save_request = Some(request);
                    }
                } else {
                    self.cancel_close();
                }
            }
            Choice::Save | Choice::Discard => {}
        }
        self.advance_close();
        Effect {
            repaint: true,
            copy: None,
        }
    }
    pub(super) fn commit_close(&mut self, permit: Permit) -> Effect {
        if permit.session != self.id || !self.close_ready(permit.ticket) {
            return Effect::default();
        }
        let Some(pending) = self.closing.take() else {
            return Effect::default();
        };
        self.last_close = Some(crate::close::Committed {
            ticket: pending.ticket,
            intent: pending.intent,
            discarded: pending
                .buffers
                .iter()
                .filter(|f| f.consent == Some(Choice::Discard))
                .map(|f| f.buffer)
                .collect(),
        });
        match pending.intent {
            Intent::Tab(id) => {
                self.close_tab(id);
            }
            Intent::Unsplit => {
                self.unsplit();
            }
            Intent::Session => {
                self.closed = true;
            }
        }
        // Authorization is consumed before releasing orphan draft/history ownership.
        for fence in pending.buffers {
            if !self
                .state
                .tabs
                .iter()
                .any(|t| t.buffer == Some(fence.buffer))
            {
                if fence.consent == Some(Choice::Discard) {
                    if let Some(entry) = self.state.sources.buffers.remove(&fence.buffer)
                        && let Some(source) = entry.snapshot
                    {
                        self.cache.take_any(source.source());
                    }
                    self.state
                        .sources
                        .aliases
                        .retain(|_, id| *id != fence.buffer);
                } else if let Some(entry) = self.state.sources.buffers.get_mut(&fence.buffer) {
                    // Clean cached snapshots still depend on their registry identity when reopened.
                    entry.draft = None;
                }
            }
        }
        self.release_current();
        Effect {
            repaint: true,
            copy: None,
        }
    }
}
