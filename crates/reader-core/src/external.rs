//! Explicit handoff of a disk file while Reader retains an independent draft.
use std::path::PathBuf;
use crate::{session::SemanticSource, tabs::TabId};
use lenz_content::open::Plan;

pub const COMPARISON_PROMPT: &str = "Compare saved disk inputs · enter a file or folder path";
pub const COMPARISON_NOTICE: &str = "External comparison uses saved disk inputs; drafts remain in Lenz.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice { SaveAndOpen, OpenSavedVersion, Cancel }
impl Choice {
    pub const ALL: [Self; 3] = [Self::SaveAndOpen, Self::OpenSavedVersion, Self::Cancel];
    pub fn label(self) -> &'static str {
        match self {
            Self::SaveAndOpen => "Save and Open",
            Self::OpenSavedVersion => "Open Saved Version",
            Self::Cancel => "Cancel",
        }
    }
}

/// Constructed from current state by Session; dispatch revalidates that identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub(crate) tab: TabId,
    pub(crate) source: Option<SemanticSource>,
    pub(crate) permission: Option<&'static str>,
    pub(crate) project: u64,
    pub(crate) path: PathBuf,
    pub(crate) current: Plan,
    pub(crate) saved: Plan,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prompt {
    pub ticket: u64,
    pub path: PathBuf,
    pub waiting: bool,
    pub save_disabled: Option<&'static str>,
    pub saved_disabled: Option<&'static str>,
}
impl Prompt {
    pub fn disabled(&self, choice: Choice) -> Option<&'static str> {
        match choice {
            Choice::SaveAndOpen => self.save_disabled,
            Choice::OpenSavedVersion => self.saved_disabled,
            Choice::Cancel => None,
        }
    }
    pub fn text(&self) -> String {
        let mut text = format!("Open {} outside Lenz.\n\nExternal tools open the file on disk. Open Saved Version keeps your draft and history in Lenz.\n", self.path.display());
        text.push_str(&format!("\n{} (S)\n{} (O)\n{} (Esc)\n",Choice::SaveAndOpen.label(),Choice::OpenSavedVersion.label(),Choice::Cancel.label()));
        if self.waiting { text.push_str("\nWaiting for the submitted Save to finish.\n"); }
        if let Some(why) = self.save_disabled { text.push_str(&format!("\nSave and Open: {why}\n")); }
        if let Some(why) = self.saved_disabled { text.push_str(&format!("\nOpen Saved Version: {why}\n")); }
        text
    }
}
