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

pub(crate) struct Captured { pub request: u64, pub text: Arc<TextSnapshot>, pub compacting: bool, pub save_as: Option<SaveAs> }
/// A Save As write: its destination, and the status to restore if it does not succeed (the original file was not written).
pub(crate) struct SaveAs { pub path: std::path::PathBuf, pub prior: Status }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplaceChoice { Replace, Cancel }
impl ReplaceChoice {
    pub const ALL: [Self; 2] = [Self::Replace, Self::Cancel];
    pub fn label(self) -> &'static str { match self { Self::Replace => "Replace", Self::Cancel => "Cancel" } }
}
/// Save As found an existing file: nothing is written until Replace, and only while that same file is still there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplacePrompt { pub ticket: u64, pub path: std::path::PathBuf }
impl ReplacePrompt {
    pub fn text(&self) -> String {
        format!("{} already exists.\n\nReplace overwrites it with this document. Cancel keeps both files unchanged.\n\n{} (R)\n{} (Esc)\n",
            self.path.display(), ReplaceChoice::Replace.label(), ReplaceChoice::Cancel.label())
    }
}
/// The existing target as observed when it was offered for replacement.
pub(crate) struct Replacing {
    pub ticket: u64,
    pub tab: crate::tabs::TabId,
    pub source: crate::session::SemanticSource,
    pub path: std::path::PathBuf,
    pub file: (u64, u64, u64, Option<std::time::SystemTime>),
}
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
