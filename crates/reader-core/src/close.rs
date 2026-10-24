//! Shared close decisions and all-session preparation. Consent never discards a draft by itself.
use crate::{session::Session, tabs::TabId, text::BufferId};
use std::{
    collections::HashSet,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SessionId(u64);
impl SessionId {
    pub(crate) fn allocate() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Intent {
    Tab(TabId),
    Unsplit,
    Session,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    Save,
    Discard,
    Cancel,
}
impl Choice {
    pub fn label(self) -> &'static str {
        match self {
            Self::Save => "Save",
            Self::Discard => "Discard",
            Self::Cancel => "Cancel",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prompt {
    pub ticket: u64,
    pub buffer: BufferId,
    pub path: PathBuf,
    pub waiting: bool,
    pub save_disabled: Option<&'static str>,
    pub discard_disabled: Option<&'static str>,
}
impl Prompt {
    pub fn text(&self) -> String {
        let mut text = format!(
            "Unsaved changes in {}\n\nChoose Save, Discard or Cancel.\nDiscard is applied only after every view approves.\n",
            self.path.display()
        );
        if self.waiting {
            text.push_str("\nWaiting for the submitted operation to finish.\n");
        }
        if let Some(why) = self.save_disabled {
            text.push_str(&format!("\nSave: {why}\n"));
        }
        if let Some(why) = self.discard_disabled {
            text.push_str(&format!("\nDiscard: {why}\n"));
        }
        text
    }
}
/// The latest authorized transition; identities only, for later recovery integration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Committed {
    pub ticket: u64,
    pub intent: Intent,
    pub discarded: Vec<BufferId>,
}
/// Only the coordinator can construct this single-use authorization.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Permit {
    pub(crate) session: SessionId,
    pub(crate) ticket: u64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Progress {
    Idle,
    Pending(SessionId),
    Cancelled,
    Committed(Vec<SessionId>),
}
struct Batch {
    generation: u64,
    participants: Vec<(SessionId, u64)>,
}
#[derive(Default)]
pub struct CloseCoordinator {
    registered: HashSet<SessionId>,
    generation: u64,
    next: u64,
    batch: Option<Batch>,
}
impl CloseCoordinator {
    pub fn register(&mut self, id: SessionId) {
        if self.registered.insert(id) {
            self.generation += 1;
        }
    }
    pub fn unregister(&mut self, id: SessionId) {
        if self.registered.remove(&id) {
            self.generation += 1;
        }
    }
    pub fn pending(&self) -> bool {
        self.batch.is_some()
    }
    /// The adapter supplies live participants; identity, consent and final validation stay in core.
    pub fn begin(&mut self, sessions: &mut [&mut Session]) {
        if self.batch.is_some() || sessions.iter().any(|s| s.has_close_intent()) {
            return;
        }
        self.next += 1;
        let participants = sessions
            .iter_mut()
            .map(|s| {
                let id = s.id();
                s.prepare_close_batch(self.next);
                (id, s.close_ticket())
            })
            .collect();
        self.batch = Some(Batch {
            generation: self.generation,
            participants,
        });
    }
    pub fn progress(&mut self, sessions: &mut [&mut Session]) -> Progress {
        let Some(batch) = &self.batch else {
            return Progress::Idle;
        };
        // Account request-owned disk outcomes before inspecting *any* participant's consent.
        for session in sessions.iter_mut() {
            session.account_close_writes();
        }
        let valid = batch.generation == self.generation
            && batch.participants.iter().all(|(id, ticket)| {
                self.registered.contains(id)
                    && sessions
                        .iter()
                        .any(|s| s.id() == *id && s.close_valid(*ticket))
            });
        if !valid {
            for (id, ticket) in &batch.participants {
                if let Some(s) = sessions.iter_mut().find(|s| s.id() == *id) {
                    s.cancel_close_batch(*ticket);
                }
            }
            self.batch = None;
            return Progress::Cancelled;
        }
        if let Some((id, _)) = batch.participants.iter().find(|(id, ticket)| {
            !sessions
                .iter()
                .any(|s| s.id() == *id && s.close_ready(*ticket))
        }) {
            return Progress::Pending(*id);
        }
        let participants = self
            .batch
            .take()
            .map(|batch| batch.participants)
            .unwrap_or_default();
        // No await, completion dispatch or platform removal between this validation and all commits.
        for (id, ticket) in &participants {
            if let Some(session) = sessions.iter_mut().find(|s| s.id() == *id) {
                session.commit_close_batch(Permit {
                    session: *id,
                    ticket: *ticket,
                });
            }
        }
        Progress::Committed(participants.into_iter().map(|(id, _)| id).collect())
    }
}
