//! Confirmed disk state and explicit write outcomes, owned by the shared buffer.
use std::sync::Arc;
use crate::text::TextSnapshot;
use lenz_content::write::{Snapshot, TargetIdentity};

#[derive(Clone, Debug)]
pub struct Baseline { pub snapshot: Snapshot, pub target: TargetIdentity }
impl Baseline {
    pub(crate) fn stamp(&self) -> crate::cache::Stamp { crate::cache::Stamp { len: self.snapshot.len_bytes(), modified: self.snapshot.modified(), file: Some((self.target.device, self.target.inode)) } }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiskMatch { Baseline, Attempted, Changed }

#[derive(Default, Clone, Debug, PartialEq, Eq)]
pub enum Status {
    #[default]
    Ready,
    Saving,
    Rechecking,
    Rejected(String),
    Uncertain(String),
    Checked(DiskMatch),
}
impl Status {
    pub fn message(&self) -> Option<&str> {
        match self {
            Self::Ready => None,
            Self::Saving => Some("Saving…"),
            Self::Rechecking => Some("Rechecking saved file…"),
            Self::Rejected(why) | Self::Uncertain(why) => Some(why),
            Self::Checked(DiskMatch::Baseline) => Some("Disk matches the last confirmed baseline. Confirm recheck before another Save."),
            Self::Checked(DiskMatch::Attempted) => Some("Disk matches the attempted save. Confirm recheck to accept that saved version."),
            Self::Checked(DiskMatch::Changed) => Some("Disk differs from the baseline and attempted save. The draft is retained; resolve the external change before saving."),
        }
    }
    pub(crate) fn busy(&self) -> bool { matches!(self, Self::Saving | Self::Rechecking) }
    pub(crate) fn blocked(&self) -> bool { matches!(self, Self::Uncertain(_) | Self::Checked(_)) }
}

pub(crate) struct Captured { pub request: u64, pub text: Arc<TextSnapshot>, pub compacting: bool }
pub(crate) struct Checked { pub read: Baseline, pub text: Arc<TextSnapshot> }
/// Successful recheck identity, including Changed, without retaining another text snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Revalidated { pub request: u64, pub source: crate::session::SemanticSource }
#[derive(Default)]
pub(crate) struct State {
    pub read: Option<Baseline>,
    pub status: Status,
    pub captured: Option<Captured>,
    /// Actual accounted successful write; compaction and recheck never create this receipt.
    pub written: Option<(u64, crate::session::SemanticSource)>,
    pub attempted: Option<Arc<TextSnapshot>>,
    pub checked: Option<Checked>,
    pub revalidated: Option<Revalidated>,
}

/// Request-owned outcomes are polled exactly once; channel messages only wake dispatch.
pub(crate) enum Outcome {
    Written(lenz_content::write::WriteOutcome),
    Rechecked(Result<(Baseline, Arc<TextSnapshot>), String>),
    Compacted { current: Arc<TextSnapshot>, baseline: Arc<TextSnapshot> },
    Rejected(String),
    Uncertain(String),
}
