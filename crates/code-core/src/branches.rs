//! The repository Lenz Code's folder (or file) is in: its branch for the status bar, and the Git Branches popup's
//! rows and actions. The git work itself is `lenz_content::branches`.

use std::path::PathBuf;

use lenz_content::branches::{self as git_branches, FileStatus, Head, Refs, Worktree};

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

impl Repo {
    /// The git folder every worktree of the repository shares: what Lenz keeps per repository (starred branches)
    /// is the same in each of them.
    pub fn common_dir(&self) -> PathBuf {
        let shared = std::fs::read_to_string(self.git_dir.join("commondir")).ok().map(|d| self.git_dir.join(d.trim()));
        let dir = shared.unwrap_or_else(|| self.git_dir.clone());
        std::fs::canonicalize(&dir).unwrap_or(dir)
    }
}

/// File statuses read at most: beyond it the explorer and tabs show the first ones and the rest have no badge.
pub const STATUS_LIMIT: usize = 20_000;

/// The repository's file statuses against HEAD, as last read: what the explorer's and tabs' badges show.
#[derive(Debug, Clone)]
pub struct Statuses {
    root: PathBuf,
    /// The folder the repository was looked up from, and where it is inside the repository.
    anchor: (PathBuf, PathBuf),
    files: std::collections::HashMap<String, FileStatus>,
    untracked_folders: bool,
    /// More paths changed than [`STATUS_LIMIT`].
    pub limited: bool,
}

impl Statuses {
    pub(crate) fn new(root: PathBuf, anchor: (PathBuf, PathBuf), read: git_branches::Statuses) -> Self {
        let untracked_folders = read.files.iter().any(|(p, s)| *s == FileStatus::Untracked && p.ends_with('/'));
        let files = read.files.into_iter().map(|(mut p, s)| { if p.ends_with('/') { p.pop(); } (p, s) }).collect();
        Self { root, anchor, files, untracked_folders, limited: read.limited }
    }
    /// Whether `path` is inside the repository's folder: its files' baselines are HEAD's (changed or not).
    pub fn contains(&self, path: &std::path::Path) -> bool {
        path.starts_with(&self.anchor.0) || path.starts_with(&self.root)
    }
    /// The status of the file or folder at `path`: a file inside an untracked folder is untracked.
    pub fn of(&self, path: &std::path::Path) -> Option<FileStatus> {
        let inside = match path.strip_prefix(&self.anchor.0) {
            Ok(rest) => self.anchor.1.join(rest),
            Err(_) => path.strip_prefix(&self.root).ok()?.to_path_buf(),
        };
        // ponytail: `/`-separated repository paths; Windows separators would need converting here.
        let inside = inside.to_str()?;
        if let Some(s) = self.files.get(inside) {
            return Some(*s);
        }
        if !self.untracked_folders { return None; }
        std::path::Path::new(inside).ancestors().skip(1).filter_map(|a| a.to_str()).filter(|a| !a.is_empty())
            .find_map(|a| self.files.get(a).copied().filter(|s| *s == FileStatus::Untracked))
    }
}

/// The Git Branches popup's lists, while it is open.
#[derive(Debug, Clone, Default)]
pub struct BranchesView {
    pub loading: bool,
    /// Cached rows remain usable while a fresh inventory is read.
    pub has_snapshot: bool,
    pub refs: Refs,
    pub worktrees: Vec<Worktree>,
    pub error: Option<String>,
    /// Compare with Worktree…: only the other worktrees, and Enter compares.
    pub worktrees_only: bool,
    /// Compare with Branch, Tag or Revision…: branches and tags (and what is typed), and Enter compares.
    pub revisions_only: bool,
    /// The sections opened past their first rows by their Show more row; kept while the lists refresh.
    pub expanded: Vec<&'static str>,
}

impl BranchesView {
    /// The popup's name.
    pub fn heading(&self) -> &'static str {
        match () {
            _ if self.worktrees_only => "Compare with Worktree",
            _ if self.revisions_only => "Compare with Branch, Tag or Revision",
            _ => "Git Branches",
        }
    }
    /// Why it shows no rows yet: loading, or what went wrong.
    pub fn state_text(&self) -> Option<&str> {
        if self.loading && !self.has_snapshot { Some("loading…") } else { self.error.as_deref() }
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
    /// What was typed in the pick list: any revision, checked when used.
    Revision(String),
    /// A section's Show more row: Enter lists the rest of it.
    More(&'static str),
    /// A folder of branches in a section (`feature` for `feature/login`…): Enter opens or closes it.
    Folder(&'static str, String),
}

impl BranchTarget {
    /// What checking it out means.
    pub fn checkout(&self) -> Option<git_branches::Target> {
        match self {
            Self::Branch(b) => Some(git_branches::Target::Branch(b.clone())),
            Self::Remote(r) => Some(git_branches::Target::Remote(r.clone())),
            // Fully named: a branch, remote, `$GIT_DIR` file or object id sharing a tag's short name must not win.
            Self::Tag(t) => Some(git_branches::Target::Detached(format!("refs/tags/{t}"))),
            Self::Revision(t) => Some(git_branches::Target::Detached(t.clone())),
            Self::Action(_) | Self::Worktree(_) | Self::More(_) | Self::Folder(..) => None,
        }
    }
    /// The revision a new branch from here starts at, fully named (a tag and a branch may share a name).
    pub fn rev(&self) -> Option<String> {
        match self {
            Self::Branch(b) => Some(format!("refs/heads/{b}")),
            Self::Remote(r) => Some(format!("refs/remotes/{r}")),
            Self::Tag(t) => Some(format!("refs/tags/{t}")),
            Self::Revision(r) => Some(r.clone()),
            Self::Action(_) | Self::Worktree(_) | Self::More(_) | Self::Folder(..) => None,
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
    /// Star or unstar the branch for this repository; the popup stays open.
    ToggleFavorite,
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
    /// Against the base branch's tip.
    BaseBranch,
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

/// A row of the popup: its section ("" for the popup's own commands), name, the dim detail on its right (a
/// branch's upstream, a remote branch's remote, a worktree's folder and changes), whether it is the current branch
/// or worktree, and what it stands for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchRow {
    pub section: &'static str,
    pub label: String,
    pub detail: String,
    pub current: bool,
    pub target: BranchTarget,
    /// Starred for this repository: listed first in its section.
    pub favorite: bool,
    /// Nesting under a folder row (0 or 1).
    pub depth: u8,
    /// A folder row that is open (its branches follow it).
    pub open: bool,
}

impl BranchRow {
    /// What tells the row's kind without colour: the current branch or worktree `●`, a branch `⎇`, a tag `◆`, a
    /// worktree `⧉` (as Lenz Diff's sidebar marks them); blank for the popup's commands and Show more rows.
    pub fn mark(&self) -> &'static str {
        match &self.target {
            _ if self.current => "●",
            _ if self.favorite => "★",
            BranchTarget::Folder(..) => "▸",
            BranchTarget::Branch(_) | BranchTarget::Remote(_) => "⎇",
            BranchTarget::Tag(_) => "◆",
            BranchTarget::Worktree(_) => "⧉",
            BranchTarget::Action(_) | BranchTarget::Revision(_) | BranchTarget::More(_) => " ",
        }
    }
    /// The text a filter looks in: the name, the detail, and the full name of what it stands for.
    fn haystack(&self) -> String {
        let full = match &self.target {
            BranchTarget::Remote(r) => r.clone(),
            BranchTarget::Worktree(p) => p.display().to_string(),
            _ => String::new(),
        };
        format!("{} {} {full}", self.label, self.detail).to_lowercase()
    }
}

/// Every mark a row can start with ([`BranchRow::mark`], an open folder's `▾`): what an adapter colours apart.
pub const MARKS: [&str; 7] = ["●", "★", "▸", "▾", "⎇", "◆", "⧉"];

/// How many rows each section shows before its Show more row, while nothing is typed.
pub fn section_cap(section: &str) -> usize {
    match section {
        "Local" => 8,
        "Remote" | "Worktrees" => 6,
        "Tags" => 5,
        _ => usize::MAX,
    }
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
    /// The explorer entries marked for Compare Selected, oldest first.
    pub fn tree_marks(&self) -> &[std::sync::Arc<str>] {
        &self.tree_marks
    }
    /// The Git Branches popup's lists, while it is open.
    pub fn branches(&self) -> Option<&BranchesView> {
        self.branches.as_ref()
    }
    /// The popup's rows whose text holds `query` (any case): its commands, then Recent, Local, Remote, Tags and (with
    /// more than one) Worktrees. While nothing is typed a long section shows its first rows ([`section_cap`]; the
    /// current branch first) and then a Show more row, until that row opens it.
    pub fn branch_rows(&self, query: &str) -> Vec<BranchRow> {
        let Some(v) = &self.branches else { return Vec::new() };
        let rows = self.matching_branch_rows(query);
        if !query.trim().is_empty() { return rows; }
        let rows = self.folded(rows);
        // Each long section: its first rows, then one row that lists the rest.
        let mut shown: Vec<BranchRow> = Vec::with_capacity(rows.len());
        let mut at = 0;
        while at < rows.len() {
            let section = rows[at].section;
            let end = rows[at..].iter().position(|r| r.section != section).map_or(rows.len(), |n| at + n);
            let cap = section_cap(section);
            if v.expanded.contains(&section) || end - at <= cap.saturating_add(1) {
                shown.extend(rows[at..end].iter().cloned());
            } else {
                shown.extend(rows[at..at + cap].iter().cloned());
                let more = end - at - cap;
                shown.push(BranchRow { section, label: format!("Show {more} more…"), detail: String::new(), current: false, target: BranchTarget::More(section), favorite: false, depth: 0, open: false });
            }
            at = end;
        }
        shown
    }
    /// Local and remote branches named under a folder (`feature/login`, `feature/search`) gathered into one folder row
    /// each where two or more share it, closed until opened ([`State::branch_folders`]); the current and starred
    /// branches stay on top, outside folders, so they never hide.
    fn folded(&self, rows: Vec<BranchRow>) -> Vec<BranchRow> {
        let folder = |r: &BranchRow| (matches!(r.section, "Local" | "Remote") && !r.current && !r.favorite && r.depth == 0).then(|| r.label.split_once('/').map(|(f, _)| f.to_string())).flatten();
        let mut out: Vec<BranchRow> = Vec::with_capacity(rows.len());
        let mut at = 0;
        while at < rows.len() {
            let Some(name) = folder(&rows[at]) else { out.push(rows[at].clone()); at += 1; continue };
            let section = rows[at].section;
            let end = rows[at..].iter().position(|r| r.section != section || folder(r).as_deref() != Some(name.as_str())).map_or(rows.len(), |n| at + n);
            if end - at < 2 { out.push(rows[at].clone()); at += 1; continue; }
            let open = self.branch_folders.iter().any(|(s, n)| *s == section && *n == name);
            out.push(BranchRow { section, label: format!("{name}/"), detail: (end - at).to_string(), current: false, target: BranchTarget::Folder(section, name.clone()), favorite: false, depth: 0, open });
            if open {
                let prefix = format!("{name}/");
                out.extend(rows[at..end].iter().map(|r| BranchRow { label: r.label.strip_prefix(&prefix).unwrap_or(&r.label).to_string(), depth: 1, ..r.clone() }));
            }
            at = end;
        }
        out
    }
    /// Every row whose text holds `query`, no section cut short.
    fn matching_branch_rows(&self, query: &str) -> Vec<BranchRow> {
        let Some(v) = &self.branches else { return Vec::new() };
        let mut rows = Vec::new();
        if v.revisions_only {
            rows.extend(self.ref_rows(v).into_iter().filter(|r| matches!(r.section, "Local" | "Remote" | "Tags")));
        } else {
            if !v.worktrees_only {
                rows.extend(self.ref_rows(v));
            }
            if v.worktrees.len() > 1 {
                // Compare with Worktree… lists the others that have a folder of files (a bare repository has none).
                let comparable = |w: &Worktree| !w.current && w.head.is_some() && w.path.is_dir();
                rows.extend(v.worktrees.iter().filter(|w| !v.worktrees_only || comparable(w)).map(|w| {
                    let name = w.branch.clone().or_else(|| w.head.as_ref().map(|h| h.get(..7).unwrap_or(h).to_string())).unwrap_or_else(|| "(bare)".into());
                    let changed = w.dirty.filter(|&n| n > 0).map_or(String::new(), |n| format!(" · {n} changed"));
                    let missing = if w.path.is_dir() { "" } else { " · missing" };
                    let detail = format!("{}{changed}{missing}", home_relative(&w.path));
                    BranchRow { section: "Worktrees", label: name, detail, current: w.current, target: BranchTarget::Worktree(w.path.clone()), favorite: false, depth: 0, open: false }
                }));
            }
        }
        let typed = query.trim();
        let query = typed.to_lowercase();
        rows.retain(|r| r.haystack().contains(&query));
        if v.revisions_only {
            // A name already listed is picked from the list (a branch is not mistaken for a tag of that name).
            if !typed.is_empty() && !rows.iter().any(|r| r.label == typed || matches!(&r.target, BranchTarget::Remote(full) if full == typed)) {
                rows.insert(0, BranchRow { section: "", label: format!("Compare with “{typed}”"), detail: String::new(), current: false, target: BranchTarget::Revision(typed.to_string()), favorite: false, depth: 0, open: false });
            }
        }
        rows
    }
    /// How many entries a section has in all (its heading's count): `None` for the popup's own commands, and while a
    /// filter is typed (the rows show what matched).
    pub fn branch_section_len(&self, section: &str, query: &str) -> Option<usize> {
        if section.is_empty() || !query.trim().is_empty() { return None; }
        Some(self.matching_branch_rows("").iter().filter(|r| r.section == section).count())
    }
    /// The popup's own commands, then Recent, Local (the current branch, then the starred ones, first), Remote (the
    /// starred ones first) and Tags.
    fn ref_rows(&self, v: &BranchesView) -> Vec<BranchRow> {
        let favorites = self.favorite_branches();
        let row = |section, label: &str, detail: String, current, target: BranchTarget| {
            let favorite = target.rev().is_some_and(|r| favorites.contains(&r)) && section != "Recent";
            BranchRow { section, label: label.to_string(), detail, current, target, favorite, depth: 0, open: false }
        };
        let mut rows = vec![
            row("", CommandId::NewBranch.label(), String::new(), false, BranchTarget::Action(CommandId::NewBranch)),
            row("", CommandId::CheckoutRevision.label(), String::new(), false, BranchTarget::Action(CommandId::CheckoutRevision)),
        ];
        let current = v.refs.current.as_deref();
        // `↑2 ↓1 origin/main`: how far it is from its upstream (once read), then the upstream
        let upstream = |b: &str| {
            let track = v.refs.track.get(b).map(|t| t.label()).unwrap_or_default();
            [track, v.refs.upstream.get(b).cloned().unwrap_or_default()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join("  ")
        };
        rows.extend(v.refs.recent.iter().map(|b| row("Recent", b, upstream(b), false, BranchTarget::Branch(b.clone()))));
        let starred = |full: String| favorites.contains(&full);
        let rank = |b: &&String| (Some(b.as_str()) != current, !starred(format!("refs/heads/{b}")));
        let mut local: Vec<_> = v.refs.local.iter().collect();
        local.sort_by_key(rank); // stable: by name within each rank
        rows.extend(local.into_iter().map(|b| row("Local", b, upstream(b), Some(b.as_str()) == current, BranchTarget::Branch(b.clone()))));
        // A remote branch reads by its name; its remote stands on the right.
        let mut remote: Vec<_> = v.refs.remote.iter().collect();
        remote.sort_by_key(|b| !starred(format!("refs/remotes/{b}")));
        rows.extend(remote.into_iter().map(|b| {
            let (remote, name) = b.split_once('/').unwrap_or(("", b));
            row("Remote", name, remote.to_string(), false, BranchTarget::Remote(b.clone()))
        }));
        rows.extend(v.refs.tags.iter().map(|t| row("Tags", t, String::new(), false, BranchTarget::Tag(t.clone()))));
        rows
    }
}

impl crate::session::Session {
    /// What Enter on a row does: check it out, or open the worktree (nothing for the current branch or worktree;
    /// the row's other actions are in its menu).
    pub fn branch_enter(&self, target: &BranchTarget) -> Option<BranchAction> {
        let compare = self.state().branches().is_some_and(|v| v.worktrees_only || v.revisions_only);
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
        let mut actions = self.row_actions(target);
        // The pick lists only compare.
        if self.state().branches().is_some_and(|v| v.revisions_only || v.worktrees_only) {
            actions.retain(|(_, a)| matches!(a, BranchAction::CompareWithWorkingTree | BranchAction::ChangesSinceBranching));
        }
        actions
    }
    /// A branch row's star: added, or taken away when it has one.
    fn favorite_action(&self, target: &BranchTarget) -> (&'static str, BranchAction) {
        let starred = target.rev().is_some_and(|r| self.state().favorite_branches().contains(&r));
        (if starred { "Remove from Favorites" } else { "Add to Favorites" }, BranchAction::ToggleFavorite)
    }
    fn row_actions(&self, target: &BranchTarget) -> Vec<(&'static str, BranchAction)> {
        // What is current comes from the repository itself, not the popup's lists (which close before an action runs).
        let repo = self.state().repo();
        let current = repo.and_then(|r| r.head.branch.as_deref());
        match target {
            BranchTarget::Action(_) | BranchTarget::More(_) | BranchTarget::Folder(..) => Vec::new(),
            BranchTarget::Branch(b) if Some(b.as_str()) == current => vec![("New Branch from Here…", BranchAction::NewBranchFrom), COMPARE, self.favorite_action(target)],
            BranchTarget::Branch(_) | BranchTarget::Remote(_) => vec![("Checkout", BranchAction::Checkout), ("New Branch from Here…", BranchAction::NewBranchFrom), COMPARE, ("Show Changes Since Branching from Here", BranchAction::ChangesSinceBranching), self.favorite_action(target)],
            BranchTarget::Tag(_) => vec![("Checkout", BranchAction::Checkout), ("New Branch from Here…", BranchAction::NewBranchFrom), COMPARE],
            BranchTarget::Revision(_) => vec![COMPARE],
            // A worktree whose folder is gone (`git worktree prune` tidies it), or a bare repository (no `.git` in
            // it, no files): nothing to open or compare.
            BranchTarget::Worktree(p) if !p.join(".git").exists() => Vec::new(),
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

/// git's tree with nothing in it.
const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

/// `rev` as Lenz Diff gets it: its name, or its commit `sha` when Lenz Diff would take the name for a subcommand or
/// git for a file in `root` ("both revision and filename").
fn by_name(root: &std::path::Path, rev: &str, sha: String) -> String {
    let file = root.join(rev).symlink_metadata().is_ok();
    if file || matches!(rev, "log" | "pr" | "merge" | "report") { sha } else { rev.to_string() }
}

const COMPARE: (&str, BranchAction) = ("Compare with Working Tree", BranchAction::CompareWithWorkingTree);

pub use lenz_content::branches::{target_label, CheckoutChoice, CheckoutProblem, Retry};

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
        self.state().checkout_problem().map_or_else(Vec::new, CheckoutProblem::choices)
    }
    /// The checkout-problem dialog's text, the same in both frontends.
    pub fn checkout_text(&self) -> Option<String> {
        self.state().checkout_problem().map(CheckoutProblem::text)
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
            GitDiff::BranchChanges(_) => "Show Changes Since Branch Start",
            GitDiff::BaseBranch => "Compare with Base Branch",
            GitDiff::Worktree(_) => "Compare with Worktree",
        };
        let repo = self.state().repo().ok_or(format!("{what}: Not in a Git repository"))?;
        let root = &repo.root;
        // ponytail: a git call or two on the UI thread, as Show Diff does
        let fail = |e: String| format!("{what}: {e}");
        let rev = match diff {
            GitDiff::Worktree(other) => {
                let launch = self.compare_launch(root, other, exe, terminal).map_err(|e| e.replacen("Compare With:", "Compare with Worktree:", 1))?;
                return Ok(lenz_content::open::Launch { cwd: Some(root.clone()), ..launch });
            }
            // No commit yet: every file is new.
            GitDiff::LocalChanges => match git_branches::verify_commit(root, "HEAD") {
                Ok(sha) => by_name(root, "HEAD", sha),
                Err(_) => EMPTY_TREE.to_string(),
            },
            GitDiff::Revision(rev) => {
                let sha = git_branches::verify_commit(root, rev).map_err(fail)?;
                by_name(root, rev.trim(), sha)
            }
            GitDiff::BranchChanges(from) => git_branches::changes_since(root, &repo.git_dir, from.as_deref()).map_err(fail)?,
            GitDiff::BaseBranch => {
                git_branches::verify_commit(root, "HEAD").map_err(|_| fail("there is no commit yet".into()))?;
                let base = git_branches::default_base(root).ok_or_else(|| fail("no main or master branch to compare with".into()))?.name;
                let sha = git_branches::verify_commit(root, &base).map_err(fail)?;
                by_name(root, &base, sha)
            }
        };
        let program = crate::session::lenz_diff(exe, terminal, what)?;
        Ok(lenz_content::open::Launch { program, args: vec![rev], cwd: Some(root.clone()) })
    }
}

/// Section headings shared by both branch pickers; keys remain stable for filtering.
pub fn branch_section_label(section: &str) -> &str {
    match section {
        "Local" => "Local branches",
        "Remote" => "Remote branches",
        "Recent" => "Recent branches",
        other => other,
    }
}

/// A worktree's folder as people read it: under the home folder as `~/…`.
fn home_relative(path: &std::path::Path) -> String {
    let home = std::env::var_os("HOME").map(PathBuf::from).filter(|h| !h.as_os_str().is_empty());
    match home.as_deref().and_then(|h| path.strip_prefix(h).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".into(),
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

/// Section headings shared by both branch pickers, with the section's size when it is known.
pub fn section_heading(section: &str, count: Option<usize>) -> String {
    match count {
        Some(n) => format!("{}  {n}", branch_section_label(section)),
        None => branch_section_label(section).to_string(),
    }
}

/// A row in `cols` cells on one line, never wrapped, in both frontends: its mark and name on the left, its detail on
/// the right. A detail takes at most about two fifths of the line; a name that does not fit keeps its start and its
/// end around a `…` (the end of `feature/…-settings-dialog` tells branches apart).
pub fn fit_row(r: &BranchRow, cols: usize) -> (String, String) {
    fit_text(&row_text(r), &r.detail, matches!(r.target, BranchTarget::Worktree(_)), cols)
}

/// [`fit_row`] for a row's text (its mark and name) and detail; `path`: the detail is a folder, which gives up its
/// start rather than its middle (its last folders tell worktrees apart).
pub fn fit_text(name: &str, detail: &str, path: bool, cols: usize) -> (String, String) {
    let room = (cols * 2 / 5).max(8);
    let detail = match () {
        _ if detail.is_empty() || cols < 24 => String::new(),
        // the folder gives up its start; what follows it (" · 3 changed") stays whole
        _ if path => {
            let (folder, rest) = detail.find(" · ").map_or((detail, ""), |at| detail.split_at(at));
            let keep = room.saturating_sub(lenz_content::wrap::width(rest)).max(8);
            format!("{}{rest}", start_ellipsis(folder, keep))
        }
        _ => middle_ellipsis(detail, room),
    };
    let room = cols.saturating_sub(if detail.is_empty() { 0 } else { lenz_content::wrap::width(&detail) + 2 });
    (middle_ellipsis(name, room.max(1)), detail)
}

/// `text` in at most `cols` cells: whole, or its start and end around a `…`.
pub fn middle_ellipsis(text: &str, cols: usize) -> String {
    use lenz_content::wrap::width;
    if width(text) <= cols { return text.to_string(); }
    if cols <= 1 { return "…".chars().take(cols).collect(); }
    let keep = cols - 1;
    let (head_cols, tail_cols) = (keep - keep * 2 / 5, keep * 2 / 5);
    let (mut head, mut used) = (String::new(), 0);
    for c in text.chars() {
        let w = width(c.encode_utf8(&mut [0; 4]));
        if used + w > head_cols { break; }
        head.push(c);
        used += w;
    }
    let (mut tail, mut used) = (Vec::new(), 0);
    for c in text.chars().rev() {
        let w = width(c.encode_utf8(&mut [0; 4]));
        if used + w > tail_cols { break; }
        tail.push(c);
        used += w;
    }
    format!("{head}…{}", tail.into_iter().rev().collect::<String>())
}

/// `text` in at most `cols` cells: whole, or `…` and as much of its end as fits.
pub fn start_ellipsis(text: &str, cols: usize) -> String {
    use lenz_content::wrap::width;
    if width(text) <= cols { return text.to_string(); }
    let (mut tail, mut used) = (Vec::new(), 0);
    for c in text.chars().rev() {
        let w = width(c.encode_utf8(&mut [0; 4]));
        if used + w + 1 > cols { break; }
        tail.push(c);
        used += w;
    }
    format!("…{}", tail.into_iter().rev().collect::<String>())
}

/// Row text under a separate section heading in both frontends: the row's mark, then its name.
pub fn row_text(r: &BranchRow) -> String {
    let mark = match &r.target { BranchTarget::Folder(..) if r.open => "▾", _ => r.mark() };
    format!("{}{mark} {}", "  ".repeat(usize::from(r.depth)), r.label)
}

/// A fully named ref as people say it (`refs/heads/main` → `main`, `refs/tags/v1` → `v1`).
pub fn short_ref(r: &str) -> &str {
    ["refs/heads/", "refs/remotes/", "refs/tags/"].iter().find_map(|p| r.strip_prefix(p)).unwrap_or(r)
}
