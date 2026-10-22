//! The repository Lenz Code's folder (or file) is in: its branch for the status bar, and the Git Branches popup's
//! rows and actions. The git work itself is `lenz_content::branches`.

use std::path::PathBuf;

use lenz_content::branches::{self as git_branches, Head, Refs, Worktree};

use crate::session::{CommandId, State};

/// The repository holding the folder, or the open file when no folder is open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repo {
    pub root: PathBuf,
    pub git_dir: PathBuf,
    pub head: Head,
}

/// The Git Branches popup's lists, while it is open.
#[derive(Debug, Clone, Default)]
pub struct BranchesView {
    pub loading: bool,
    pub refs: Refs,
    pub worktrees: Vec<Worktree>,
    pub error: Option<String>,
}

/// What a popup row stands for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BranchTarget {
    /// One of the popup's own commands (New Branch…, Checkout Tag or Revision…).
    Action(CommandId),
    Branch(String),
    Remote(String),
    Tag(String),
    Worktree(PathBuf),
}

impl BranchTarget {
    /// What checking it out means.
    pub fn checkout(&self) -> Option<git_branches::Target> {
        match self {
            Self::Branch(b) => Some(git_branches::Target::Branch(b.clone())),
            Self::Remote(r) => Some(git_branches::Target::Remote(r.clone())),
            Self::Tag(t) => Some(git_branches::Target::Detached(t.clone())),
            Self::Action(_) | Self::Worktree(_) => None,
        }
    }
    /// The revision a new branch from here starts at.
    pub fn rev(&self) -> Option<&str> {
        match self {
            Self::Branch(r) | Self::Remote(r) | Self::Tag(r) => Some(r),
            Self::Action(_) | Self::Worktree(_) => None,
        }
    }
}

/// What a row's submenu offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BranchAction {
    Checkout,
    /// The adapter asks for the name (the row's revision is where it starts).
    NewBranchFrom,
    Open,
    /// The adapter opens a window (or terminal view) on the worktree.
    OpenInNewWindow,
}

/// A row of the popup: its section ("" for the popup's own commands), text, whether it is the current branch or
/// worktree, and what it stands for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchRow {
    pub section: &'static str,
    pub label: String,
    pub current: bool,
    pub target: BranchTarget,
}

impl State {
    /// The repository found for the folder or file shown (`None` outside git, or not looked up yet).
    pub fn repo(&self) -> Option<&Repo> {
        self.repo.as_ref()
    }
    /// Whether the repository lookup for what is shown has answered.
    pub fn repo_checked(&self) -> bool {
        self.repo_anchor.as_ref().is_none_or(|a| self.repo_answered.as_ref() == Some(a))
    }
    /// The Git Branches popup's lists, while it is open.
    pub fn branches(&self) -> Option<&BranchesView> {
        self.branches.as_ref()
    }
    /// The popup's rows whose text holds `query` (any case): its commands, then Recent, Local, Remote, Tags and (with
    /// more than one) Worktrees.
    pub fn branch_rows(&self, query: &str) -> Vec<BranchRow> {
        let Some(v) = &self.branches else { return Vec::new() };
        let row = |section, label: &str, current, target| BranchRow { section, label: label.to_string(), current, target };
        let mut rows = vec![
            row("", CommandId::NewBranch.label(), false, BranchTarget::Action(CommandId::NewBranch)),
            row("", CommandId::CheckoutRevision.label(), false, BranchTarget::Action(CommandId::CheckoutRevision)),
        ];
        let current = v.refs.current.as_deref();
        rows.extend(v.refs.recent.iter().map(|b| row("Recent", b, false, BranchTarget::Branch(b.clone()))));
        rows.extend(v.refs.local.iter().map(|b| row("Local", b, Some(b.as_str()) == current, BranchTarget::Branch(b.clone()))));
        rows.extend(v.refs.remote.iter().map(|b| row("Remote", b, false, BranchTarget::Remote(b.clone()))));
        rows.extend(v.refs.tags.iter().map(|t| row("Tags", t, false, BranchTarget::Tag(t.clone()))));
        if v.worktrees.len() > 1 {
            rows.extend(v.worktrees.iter().map(|w| {
                let name = w.branch.clone().or_else(|| w.head.as_ref().map(|h| h.get(..7).unwrap_or(h).to_string())).unwrap_or_else(|| "(bare)".into());
                let changed = w.dirty.filter(|&n| n > 0).map_or(String::new(), |n| format!(" · {n} changed"));
                let label = format!("{name}  {}{changed}", w.path.display());
                row("Worktrees", &label, w.current, BranchTarget::Worktree(w.path.clone()))
            }));
        }
        let query = query.to_lowercase();
        rows.retain(|r| r.label.to_lowercase().contains(&query));
        rows
    }
}

impl crate::session::Session {
    /// The status bar's branch: `⎇ main`, or the detached commit.
    pub fn branch_label(&self) -> Option<String> {
        self.state().repo().map(|r| format!("⎇ {}", r.head.label()))
    }
    /// The actions a row's submenu offers (none for the popup's own commands, which run on Enter).
    pub fn branch_actions(&self, target: &BranchTarget) -> Vec<(&'static str, BranchAction)> {
        let current = self.state().branches().and_then(|v| v.refs.current.as_deref());
        match target {
            BranchTarget::Action(_) => Vec::new(),
            BranchTarget::Branch(b) if Some(b.as_str()) == current => vec![("New Branch from Here…", BranchAction::NewBranchFrom)],
            BranchTarget::Branch(_) | BranchTarget::Remote(_) | BranchTarget::Tag(_) => vec![("Checkout", BranchAction::Checkout), ("New Branch from Here…", BranchAction::NewBranchFrom)],
            BranchTarget::Worktree(p) => {
                let here = self.state().branches().is_some_and(|v| v.worktrees.iter().any(|w| w.current && &w.path == p));
                let mut actions = vec![("Open in New Window", BranchAction::OpenInNewWindow)];
                if !here {
                    actions.insert(0, ("Open", BranchAction::Open));
                }
                actions
            }
        }
    }
}
