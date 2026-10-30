//! Reconciling a dirty draft with a file that changed on disk: the three sides the user compares, the decisions
//! they can take and the guards that keep every one of them explicit and lossless.
use std::path::PathBuf;
use std::sync::Arc;

use crate::text::TextSnapshot;

/// Why Resolve External Change… cannot run: the file still holds the draft's confirmed baseline.
pub const NO_CONFLICT: &str = "The file has not changed on disk since its last confirmed save";

/// One side of the reconciliation, as identified in the dialog: its size, never its text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Side {
    pub lines: usize,
    pub bytes: usize,
}
impl Side {
    /// The sides as a reader counts them: a file ending in a line break has that many lines, not one more.
    pub fn of(text: &TextSnapshot) -> Self {
        let (logical, bytes) = (text.line_count(), text.len_bytes());
        let trailing_empty = logical > 1 && text.line_range(logical - 1).is_some_and(|last| last.is_empty());
        Self { lines: if bytes == 0 { 0 } else { logical - usize::from(trailing_empty) }, bytes }
    }
    fn describe(&self) -> String {
        format!("{} line{}, {} byte{}", self.lines, if self.lines == 1 { "" } else { "s" }, self.bytes, if self.bytes == 1 { "" } else { "s" })
    }
}

/// How the file on disk relates to the confirmed baseline the draft was edited from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiskChange {
    /// The file still holds the confirmed baseline.
    Baseline,
    /// The file holds the version a rejected or uncertain Save tried to write.
    Attempted,
    /// The file holds neither: someone else wrote it.
    Other,
}

/// A snapshot of the file on disk, read once and verified as stable during that read.
#[derive(Clone)]
pub struct Disk {
    /// The exact write target the read confirmed: its bytes, identity and stamp.
    pub baseline: crate::save::Baseline,
    /// The same content as a session snapshot.
    pub text: Arc<TextSnapshot>,
}

/// What a decision's own read of the file found.
pub enum Check {
    /// The same content and file identity the dialog showed: the decision applies.
    Same(Disk),
    /// Another version is on disk now: nothing is decided from the version the user did not see.
    Changed(Box<Disk>),
    /// The file could not be read.
    Failed(String),
}

/// The dialog's answer. Every choice except Keep Draft is a decision about the draft's lifecycle; none writes
/// without a fresh disk check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    /// Open the comparison of the file on disk, the current draft and the confirmed baseline.
    Compare,
    /// Replace the draft's text with the file on disk, as one undoable edit, and confirm that version.
    TakeDisk,
    /// Keep the draft's text and write it over the file on disk after one more identity and content check.
    SaveDraft,
    /// Keep the draft as it is: no new disk version becomes its baseline.
    KeepDraft,
}
impl Choice {
    pub const ALL: [Self; 4] = [Self::Compare, Self::TakeDisk, Self::SaveDraft, Self::KeepDraft];
    pub fn label(self) -> &'static str {
        match self {
            Self::Compare => "Compare Disk, Draft and Baseline…",
            Self::TakeDisk => "Take Disk Version",
            Self::SaveDraft => "Save Draft over Disk",
            Self::KeepDraft => "Keep Draft",
        }
    }
    /// The key that runs it while the dialog is open (`Esc` closes it, keeping the draft).
    pub fn key(self) -> &'static str {
        match self {
            Self::Compare => "c",
            Self::TakeDisk => "d",
            Self::SaveDraft => "s",
            Self::KeepDraft => "Esc",
        }
    }
}

/// What the adapters show and dispatch for one external change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prompt {
    pub ticket: u64,
    pub path: PathBuf,
    /// The file on disk as read for this dialog (`None`: still being read).
    pub disk: Option<Side>,
    /// Why the file could not be read, when it could not: no decision may then be taken from this dialog.
    pub read_failure: Option<String>,
    /// How the read disk content relates to the confirmed baseline.
    pub disk_change: Option<DiskChange>,
    pub draft: Side,
    pub baseline: Side,
}
impl Prompt {
    pub fn disabled(&self, choice: Choice) -> Option<&'static str> {
        match choice {
            Choice::KeepDraft => None,
            _ if self.read_failure.is_some() => Some("The file on disk could not be read, so no decision can be taken"),
            _ if self.disk.is_none() => Some("Reading the file on disk"),
            _ => None,
        }
    }
    /// The dialog's text: the three sides, what each decision does, and never the source itself.
    pub fn text(&self) -> String {
        let mut text = format!("{}\nchanged on disk while you were editing it.\n\nThe three sides:\n", self.path.display());
        match (&self.disk, &self.read_failure) {
            (_, Some(why)) => text.push_str(&format!("  Disk: cannot be read ({why})\n")),
            (Some(disk), None) => {
                let relation = match self.disk_change {
                    Some(DiskChange::Baseline) => "still the confirmed baseline",
                    Some(DiskChange::Attempted) => "the version your rejected or uncertain Save tried to write",
                    Some(DiskChange::Other) => "differs from the confirmed baseline",
                    None => "differs from the confirmed baseline",
                };
                text.push_str(&format!("  Disk: {} ({relation})\n", disk.describe()));
            }
            (None, None) => text.push_str("  Disk: reading…\n"),
        }
        text.push_str(&format!("  Current draft: {} (unsaved changes against the confirmed baseline)\n", self.draft.describe()));
        text.push_str(&format!("  Confirmed baseline (last confirmed save): {}\n", self.baseline.describe()));
        text.push_str("\nCompare Disk, Draft and Baseline… opens all three read-only sides around the draft you can edit, without deciding anything.\n");
        text.push_str("Take Disk Version replaces your text with the file on disk as one undoable edit and confirms that version.\n");
        text.push_str("Save Draft over Disk writes your text over the file after one more identity and content check; Undo still takes the write back.\n");
        text.push_str("Keep Draft keeps your text and the confirmed baseline it was edited from; the new disk version does not become its baseline.\n");
        for choice in Choice::ALL {
            text.push_str(&format!("\n{} ({})\n", choice.label(), choice.key()));
        }
        if let Some(why) = self.read_failure.as_ref() {
            text.push_str(&format!("\nThe file could not be read: {why}\n"));
        }
        text
    }
}
