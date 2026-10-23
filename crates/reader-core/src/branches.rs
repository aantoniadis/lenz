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
    /// What changes when HEAD may have (see `lenz_content::branches::head_signal`).
    pub signal: Option<std::time::SystemTime>,
}

/// The Git Branches popup's lists, while it is open.
#[derive(Debug, Clone, Default)]
pub struct BranchesView {
    pub loading: bool,
    pub refs: Refs,
    pub worktrees: Vec<Worktree>,
    pub error: Option<String>,
    /// Compare with Worktree…: only the other worktrees, and Enter compares.
    pub worktrees_only: bool,
}

impl BranchesView {
    /// The popup's name.
    pub fn heading(&self) -> &'static str {
        if self.worktrees_only { "Compare with Worktree" } else { "Git Branches" }
    }
    /// Why it shows no rows yet: loading, or what went wrong.
    pub fn state_text(&self) -> Option<&str> {
        if self.loading { Some("loading…") } else { self.error.as_deref() }
    }
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
    /// The revision a new branch from here starts at, fully named (a tag and a branch may share a name).
    pub fn rev(&self) -> Option<String> {
        match self {
            Self::Branch(b) => Some(format!("refs/heads/{b}")),
            Self::Remote(r) => Some(format!("refs/remotes/{r}")),
            Self::Tag(t) => Some(format!("refs/tags/{t}")),
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
    /// Lenz Diff: the working tree against the row (`GitDiff::of`).
    CompareWithWorkingTree,
    /// Lenz Diff: everything since HEAD split from the row's branch.
    ChangesSinceBranching,
}

/// A comparison Lenz Code opens Lenz Diff on, the working tree as it is on the right.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitDiff {
    /// Against HEAD (what the next commit would hold, untracked files as new).
    LocalChanges,
    /// Against a branch, tag or commit.
    Revision(String),
    /// Against where HEAD split from this branch (the base branch when `None`).
    BranchChanges(Option<String>),
    /// This worktree's folder against another's.
    Worktree(PathBuf),
}

impl GitDiff {
    /// What a row's comparison action compares with.
    pub fn of(target: &BranchTarget, action: BranchAction) -> Option<Self> {
        match (action, target) {
            (BranchAction::CompareWithWorkingTree, BranchTarget::Worktree(p)) => Some(Self::Worktree(p.clone())),
            (BranchAction::CompareWithWorkingTree, t) => t.rev().map(Self::Revision),
            (BranchAction::ChangesSinceBranching, BranchTarget::Branch(_) | BranchTarget::Remote(_)) => target.rev().map(|r| Self::BranchChanges(Some(r))),
            _ => None,
        }
    }
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
        if v.worktrees_only {
            rows.clear();
        }
        if v.worktrees.len() > 1 {
            rows.extend(v.worktrees.iter().filter(|w| !v.worktrees_only || !w.current && w.path.is_dir()).map(|w| {
                let name = w.branch.clone().or_else(|| w.head.as_ref().map(|h| h.get(..7).unwrap_or(h).to_string())).unwrap_or_else(|| "(bare)".into());
                let changed = w.dirty.filter(|&n| n > 0).map_or(String::new(), |n| format!(" · {n} changed"));
                let missing = if w.path.is_dir() { "" } else { " · missing" };
                let label = format!("{name}  {}{changed}{missing}", w.path.display());
                row("Worktrees", &label, w.current, BranchTarget::Worktree(w.path.clone()))
            }));
        }
        let query = query.to_lowercase();
        rows.retain(|r| r.label.to_lowercase().contains(&query));
        rows
    }
}

impl crate::session::Session {
    /// What Enter on a row does: check it out, or open the worktree (nothing for the current branch or worktree;
    /// the row's other actions are in its menu).
    pub fn branch_enter(&self, target: &BranchTarget) -> Option<BranchAction> {
        let compare = self.state().branches().is_some_and(|v| v.worktrees_only);
        if compare {
            return self.branch_actions(target).into_iter().map(|(_, a)| a).find(|a| *a == BranchAction::CompareWithWorkingTree);
        }
        self.branch_actions(target).into_iter().map(|(_, a)| a).find(|a| matches!(a, BranchAction::Checkout | BranchAction::Open))
    }
    /// The status bar's branch: `⎇ main`, or the detached commit.
    pub fn branch_label(&self) -> Option<String> {
        self.state().repo().map(|r| format!("⎇ {}", r.head.label()))
    }
    /// The actions a row's submenu offers (none for the popup's own commands, which run on Enter).
    pub fn branch_actions(&self, target: &BranchTarget) -> Vec<(&'static str, BranchAction)> {
        // What is current comes from the repository itself, not the popup's lists (which close before an action runs).
        let repo = self.state().repo();
        let current = repo.and_then(|r| r.head.branch.as_deref());
        match target {
            BranchTarget::Action(_) => Vec::new(),
            BranchTarget::Branch(b) if Some(b.as_str()) == current => vec![("New Branch from Here…", BranchAction::NewBranchFrom), COMPARE],
            BranchTarget::Branch(_) | BranchTarget::Remote(_) => vec![("Checkout", BranchAction::Checkout), ("New Branch from Here…", BranchAction::NewBranchFrom), COMPARE, ("Show Changes Since Branching from Here", BranchAction::ChangesSinceBranching)],
            BranchTarget::Tag(_) => vec![("Checkout", BranchAction::Checkout), ("New Branch from Here…", BranchAction::NewBranchFrom), COMPARE],
            // A worktree whose folder is gone (`git worktree prune` tidies it): nothing to open.
            BranchTarget::Worktree(p) if !p.is_dir() => Vec::new(),
            BranchTarget::Worktree(p) => {
                let canonical = |p: &std::path::Path| std::fs::canonicalize(p).ok();
                let here = repo.is_some_and(|r| canonical(p).is_some() && canonical(p) == canonical(&r.root));
                match here {
                    true => vec![("Open in New Window", BranchAction::OpenInNewWindow)],
                    false => vec![("Open", BranchAction::Open), ("Open in New Window", BranchAction::OpenInNewWindow), COMPARE],
                }
            }
        }
    }
}

const COMPARE: (&str, BranchAction) = ("Compare with Working Tree", BranchAction::CompareWithWorkingTree);

/// What the checkout-problem dialog offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckoutChoice {
    Smart,
    /// The first step of Force Checkout: list everything it would discard.
    Force,
    /// The second step: discard it and check out.
    ConfirmForce,
    Cancel,
}

/// What a refused checkout runs again once the dialog is answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Retry {
    Checkout(git_branches::Target),
    /// A new branch `name` at `from` (HEAD when `None`), checked out.
    NewBranch { name: String, from: Option<String> },
}

impl Retry {
    /// How it is named in messages.
    pub fn label(&self) -> &str {
        match self {
            Self::Checkout(t) => target_label(t),
            Self::NewBranch { name, .. } => name,
        }
    }
}

/// A checkout git refused over files in the way, while the dialog is open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckoutProblem {
    pub retry: Retry,
    pub tracked: Vec<String>,
    pub untracked: Vec<String>,
    /// Once Force Checkout is chosen: every uncommitted change it would discard (the confirmation step).
    pub discards: Option<Vec<String>>,
}

/// How a checkout target is named in messages: the branch it ends on, or the revision.
pub fn target_label(t: &git_branches::Target) -> &str {
    match t {
        git_branches::Target::Branch(b) | git_branches::Target::Detached(b) | git_branches::Target::Remote(b) => b,
    }
}

impl State {
    pub fn checkout_problem(&self) -> Option<&CheckoutProblem> {
        self.checkout_problem.as_ref()
    }
    /// A checkout or new branch is running.
    pub fn checking_out(&self) -> bool {
        self.checkout_running.is_some()
    }
}

impl crate::session::Session {
    /// The checkout-problem dialog's buttons, in order.
    pub fn checkout_choices(&self) -> Vec<(&'static str, CheckoutChoice)> {
        match self.state().checkout_problem() {
            None => Vec::new(),
            Some(p) if !p.untracked.is_empty() => vec![("Cancel", CheckoutChoice::Cancel)],
            Some(p) if p.discards.is_some() => vec![("Force Checkout", CheckoutChoice::ConfirmForce), ("Cancel", CheckoutChoice::Cancel)],
            Some(_) => vec![("Smart Checkout", CheckoutChoice::Smart), ("Force Checkout", CheckoutChoice::Force), ("Cancel", CheckoutChoice::Cancel)],
        }
    }
    /// The checkout-problem dialog's text, the same in both frontends.
    pub fn checkout_text(&self) -> Option<String> {
        let p = self.state().checkout_problem()?;
        let name = p.retry.label();
        let list = |files: &[String]| files.iter().map(|f| format!("  {f}")).collect::<Vec<_>>().join("\n");
        Some(match (&p.discards, p.untracked.is_empty()) {
            (_, false) => format!("Checking out {name} would overwrite these untracked files:\n\n{}\n\nMove or delete them first, then check out again.", list(&p.untracked)),
            (Some(d), true) => format!("Force Checkout discards your uncommitted changes to these files:\n\n{}\n\nThey cannot be recovered.", list(d)),
            (None, true) => format!("Checking out {name} would overwrite your local changes to:\n\n{}\n\nSmart Checkout stashes your changes, checks out {name} and puts them back. Force Checkout discards them.", list(&p.tracked)),
        })
    }
    /// Whether `name` can be a new branch here.
    pub fn validate_branch_name(&self, name: &str) -> Result<(), String> {
        let root = self.state().repo().ok_or("Not in a Git repository")?.root.clone();
        git_branches::check_branch_name(&root, name.trim())
    }
    /// `rev` as a commit id, or why not.
    pub fn validate_revision(&self, rev: &str) -> Result<String, String> {
        let root = self.state().repo().ok_or("Not in a Git repository")?.root.clone();
        git_branches::verify_commit(&root, rev)
    }
    /// Lenz Diff on the repository's conflicts (after a Smart Checkout kept its stash), from `exe`'s folder.
    pub fn merge_launch(&self, exe: &std::path::Path, terminal: bool) -> Result<lenz_content::open::Launch, String> {
        let root = self.state().repo().ok_or("Not in a Git repository")?.root.clone();
        let program = crate::session::lenz_diff(exe, terminal, "Resolve Conflicts")?;
        Ok(lenz_content::open::Launch { program, args: vec!["merge".into()], cwd: Some(root) })
    }

    /// Lenz Diff on `diff`, in the repository's folder, from `exe`'s folder (the terminal app when `terminal`).
    pub fn git_diff_launch(&self, diff: &GitDiff, exe: &std::path::Path, terminal: bool) -> Result<lenz_content::open::Launch, String> {
        let what = match diff {
            GitDiff::LocalChanges => "Show Local Changes",
            GitDiff::Revision(_) => "Compare",
            GitDiff::BranchChanges(_) => "Show Branch Changes",
            GitDiff::Worktree(_) => "Compare with Worktree",
        };
        let repo = self.state().repo().ok_or(format!("{what}: Not in a Git repository"))?;
        let root = &repo.root;
        // ponytail: a git call or two on the UI thread, as Show Diff does
        let fail = |e: String| format!("{what}: {e}");
        let rev = match diff {
            GitDiff::Worktree(other) => return self.compare_launch(root, other, exe, terminal),
            GitDiff::LocalChanges => {
                git_branches::verify_commit(root, "HEAD").map_err(|_| fail("there is no commit yet".into()))?;
                "HEAD".to_string()
            }
            GitDiff::Revision(rev) => {
                let sha = git_branches::verify_commit(root, rev).map_err(fail)?;
                // A name Lenz Diff takes as a subcommand goes by its commit.
                match rev.trim() {
                    "log" | "pr" | "merge" | "report" => sha,
                    rev => rev.to_string(),
                }
            }
            GitDiff::BranchChanges(from) => {
                let base = match from {
                    Some(from) => from.clone(),
                    None => {
                        let base = git_branches::default_base(root).ok_or_else(|| fail("no main or master branch to compare with".into()))?;
                        // Read now: HEAD may have moved since the last tick.
                        let branch = git_branches::head(root, &repo.git_dir).and_then(|h| h.branch);
                        if let Some(b) = branch.filter(|b| base.short == *b || base.short == format!("origin/{b}")) {
                            return Err(fail(format!("{b} is the base branch; Show Local Changes shows its changes")));
                        }
                        base.name
                    }
                };
                git_branches::merge_base(root, &base, "HEAD").map_err(fail)?
            }
        };
        let program = crate::session::lenz_diff(exe, terminal, what)?;
        Ok(lenz_content::open::Launch { program, args: vec![rev], cwd: Some(root.clone()) })
    }
}

/// How both frontends show a popup row: its section, the current marker, its text.
pub fn row_text(r: &BranchRow) -> String {
    format!("{:<10}{}{}", r.section, if r.current { "● " } else { "  " }, r.label)
}

/// A fully named ref as people say it (`refs/heads/main` → `main`, `refs/tags/v1` → `v1`).
pub fn short_ref(r: &str) -> &str {
    ["refs/heads/", "refs/remotes/", "refs/tags/"].iter().find_map(|p| r.strip_prefix(p)).unwrap_or(r)
}
