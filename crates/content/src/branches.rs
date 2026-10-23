//! Branches, tags and worktrees of a repository, and switching between them: what Lenz Code's Git Branches popup and
//! Lenz Diff's repository window show and do. Reads go through [`git::run`] (the repository's own programs stay off);
//! the checkouts and branches the user asks for go through [`git::run_as_user`], as `git` in their terminal would.

use std::path::{Path, PathBuf};

use crate::git;

/// What `HEAD` points at, read from its file (no git process).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Head {
    /// The checked-out branch; `None` when detached.
    pub branch: Option<String>,
    /// The commit when detached.
    pub sha: Option<String>,
}

impl Head {
    /// `main`, or the detached commit's short id.
    pub fn label(&self) -> String {
        match (&self.branch, &self.sha) {
            (Some(b), _) => b.clone(),
            (None, Some(sha)) => sha.get(..7).unwrap_or(sha).to_string(),
            (None, None) => "HEAD".into(),
        }
    }
}

/// The repository's git directory (a worktree's own, for a linked worktree).
pub fn git_dir(root: &Path) -> Result<PathBuf, String> {
    let out = git::run(root, &["rev-parse", "--absolute-git-dir"]).map_err(not_git)?;
    Ok(PathBuf::from(git::path_text(out.strip_suffix(b"\n").unwrap_or(&out))?))
}

/// The work tree's top-level folder.
pub fn toplevel(dir: &Path) -> Result<PathBuf, String> {
    let out = git::run(dir, &["rev-parse", "--show-toplevel"]).map_err(not_git)?;
    Ok(PathBuf::from(git::path_text(out.strip_suffix(b"\n").unwrap_or(&out))?))
}

/// The work tree's top-level folder and its git directory, in one git run.
pub fn locate(dir: &Path) -> Result<(PathBuf, PathBuf), String> {
    let out = git::run(dir, &["rev-parse", "--show-toplevel", "--absolute-git-dir"]).map_err(not_git)?;
    let text = git::path_text(&out)?;
    match text.lines().collect::<Vec<_>>()[..] {
        [top, dir] => Ok((PathBuf::from(top), PathBuf::from(dir))),
        _ => Err("Not in a Git repository".into()),
    }
}

fn not_git(e: String) -> String {
    if git::version().is_some() { "Not in a Git repository".into() } else { e }
}

/// `HEAD` of the repository at `root`: from its file, or through git for a reftable repository (whose `HEAD` file
/// is a placeholder).
pub fn head(root: &Path, git_dir: &Path) -> Option<Head> {
    if !git_dir.join("reftable").is_dir() {
        return head_of(git_dir);
    }
    let text = |out: Vec<u8>| String::from_utf8_lossy(&out).trim().to_string();
    match git::run(root, &["symbolic-ref", "-q", "HEAD"]).map(text) {
        Ok(r) => Some(Head { branch: Some(r.strip_prefix("refs/heads/").unwrap_or(&r).to_string()), sha: None }),
        Err(_) => git::run(root, &["rev-parse", "HEAD"]).ok().map(text).map(|sha| Head { branch: None, sha: Some(sha) }),
    }
}

/// What changes when HEAD may have: the `HEAD` file's time, or a reftable repository's table list's.
pub fn head_signal(git_dir: &Path) -> Option<std::time::SystemTime> {
    let file = if git_dir.join("reftable").is_dir() { git_dir.join("reftable/tables.list") } else { git_dir.join("HEAD") };
    std::fs::metadata(file).and_then(|m| m.modified()).ok()
}

/// `HEAD` of the git directory `git_dir`, from its file.
pub fn head_of(git_dir: &Path) -> Option<Head> {
    let text = std::fs::read_to_string(git_dir.join("HEAD")).ok()?;
    let text = text.trim_end();
    Some(match text.strip_prefix("ref: ") {
        Some(r) => Head { branch: Some(r.strip_prefix("refs/heads/").unwrap_or(r).to_string()), sha: None },
        None => Head { branch: None, sha: Some(text.to_string()) },
    })
}

/// The branches and tags to choose from, each list sorted by name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Refs {
    /// Branches checked out lately, newest first (not the current one).
    pub recent: Vec<String>,
    pub local: Vec<String>,
    /// `origin/main` and the like (not `origin/HEAD`).
    pub remote: Vec<String>,
    pub tags: Vec<String>,
    pub current: Option<String>,
}

/// How many recently checked-out branches [`refs`] lists.
pub const RECENT: usize = 5;

pub fn refs(root: &Path) -> Result<Refs, String> {
    let out = git::run(root, &["for-each-ref", "--sort=refname", "--format=%(refname)%00%(HEAD)", "refs/heads", "refs/remotes", "refs/tags"])?;
    let mut refs = Refs::default();
    for line in String::from_utf8_lossy(&out).lines() {
        let (name, head) = line.split_once('\0').unwrap_or((line, ""));
        if name.rsplit('/').next().is_some_and(|n| n.starts_with('-')) || name.contains("/-") {
            continue; // named like an option: only `update-ref` makes those, and git is never handed them
        }
        if let Some(b) = name.strip_prefix("refs/heads/") {
            if head == "*" {
                refs.current = Some(b.to_string());
            }
            refs.local.push(b.to_string());
        } else if let Some(r) = name.strip_prefix("refs/remotes/").filter(|r| !r.ends_with("/HEAD")) {
            refs.remote.push(r.to_string());
        } else if let Some(t) = name.strip_prefix("refs/tags/") {
            refs.tags.push(t.to_string());
        }
    }
    // The reflog's "checkout: moving from A to B", newest first; a repository without a reflog has none.
    let log = git::run(root, &["reflog", "-n", "200", "--format=%gs"]).unwrap_or_default();
    for line in String::from_utf8_lossy(&log).lines() {
        let Some((_, to)) = line.strip_prefix("checkout: moving from ").and_then(|l| l.rsplit_once(" to ")) else { continue };
        if refs.recent.len() < RECENT && refs.local.iter().any(|b| b == to) && refs.current.as_deref() != Some(to) && !refs.recent.iter().any(|b| b == to) {
            refs.recent.push(to.to_string());
        }
    }
    Ok(refs)
}

#[derive(Debug, Clone, PartialEq)]
pub struct Worktree {
    pub path: PathBuf,
    /// `None` for a bare repository.
    pub head: Option<String>,
    /// `None` when detached (or bare).
    pub branch: Option<String>,
    /// The worktree Lenz runs in.
    pub current: bool,
    /// Paths with local changes; `None` when they can't be read (bare, or its directory is gone).
    pub dirty: Option<usize>,
}

/// The repository's worktrees, main first, each with its count of changed paths.
pub fn worktrees(cwd: &Path) -> Result<Vec<Worktree>, String> {
    let mut list = worktree_list(cwd)?;
    count_changes(&mut list);
    Ok(list)
}

/// [`worktrees`] without the changed-path counts (`dirty` is `None`): no `git status` run.
pub fn worktree_list(cwd: &Path) -> Result<Vec<Worktree>, String> {
    let out = git::run(cwd, &["worktree", "list", "--porcelain"])?;
    let here = toplevel(cwd).ok().and_then(|p| p.canonicalize().ok());
    let mut list = Vec::new();
    // ponytail: `-z` needs git 2.36; a path with a newline would be C-quoted here.
    for block in String::from_utf8_lossy(&out).split("\n\n").filter(|b| !b.trim().is_empty()) {
        let mut w = Worktree { path: PathBuf::new(), head: None, branch: None, current: false, dirty: None };
        for line in block.lines() {
            let (key, value) = line.split_once(' ').unwrap_or((line, ""));
            match key {
                "worktree" => w.path = PathBuf::from(value),
                "HEAD" => w.head = Some(value.to_string()),
                "branch" => w.branch = Some(value.strip_prefix("refs/heads/").unwrap_or(value).to_string()),
                _ => {}
            }
        }
        w.current = here.is_some() && w.path.canonicalize().ok() == here;
        list.push(w);
    }
    Ok(list)
}

/// Counts each worktree's changed paths (one `git status` each); `None` where they can't be read (bare, or its
/// folder is gone).
pub fn count_changes(list: &mut [Worktree]) {
    for w in list.iter_mut() {
        w.dirty = w.path.is_dir().then(|| dirty(&w.path).ok()).flatten();
    }
}

/// How many paths `git status` lists in `dir`, untracked files included.
fn dirty(dir: &Path) -> Result<usize, String> {
    let args = ["status", "--porcelain=v2", "-z", "--untracked-files=all"];
    let out = git::run(dir, &args)?;
    let mut entries = out.split(|&b| b == 0).filter(|e| !e.is_empty());
    let mut count = 0;
    while let Some(entry) = entries.next() {
        if entry.starts_with(b"2 ") {
            entries.next(); // a rename's original path
        }
        count += 1;
    }
    Ok(count)
}

/// What to check out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Branch(String),
    /// A remote-tracking branch (`origin/foo`): its local branch, made to track it when missing.
    Remote(String),
    /// A tag or any revision, on a detached `HEAD`.
    Detached(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Local changes go along when git allows.
    Normal,
    /// Tracked local changes are stashed, the checkout made, and the stash restored.
    Smart,
    /// Every uncommitted change to tracked files is discarded.
    Force,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckoutError {
    /// Git refused: these local changes (tracked) or untracked files would be overwritten.
    Blocked { tracked: Vec<String>, untracked: Vec<String> },
    /// Any other failure, in git's words.
    Git(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Outcome {
    /// After Smart Checkout: the stash could not be restored cleanly; it is kept, and these files conflict.
    pub stash_kept: Option<Vec<String>>,
    /// The checkout happened but git (a failing hook, say) still reported an error: its words.
    pub warning: Option<String>,
}

/// The local branch remote-tracking branch `remote` (`origin/feature/x`) checks out as (`feature/x`), by the
/// repository's remotes, so a remote named `team/fork` splits right.
pub fn local_branch(root: &Path, remote: &str) -> String {
    let remotes = git::run(root, &["remote"]).unwrap_or_default();
    String::from_utf8_lossy(&remotes)
        .lines()
        .filter_map(|r| remote.strip_prefix(r).and_then(|b| b.strip_prefix('/')))
        .min_by_key(|b| b.len())
        .unwrap_or_else(|| remote.split_once('/').map_or(remote, |(_, b)| b))
        .to_string()
}

/// Checks `target` out as the user would in a terminal (hooks and filters run). A refusal over local changes comes
/// back as [`CheckoutError::Blocked`]; Smart Checkout of untracked files is refused the same way, untouched.
pub fn checkout(root: &Path, target: &Target, mode: Mode) -> Result<Outcome, CheckoutError> {
    run_switch(root, switch_args(root, target)?, mode)
}

/// Creates branch `name` at `from` (HEAD when `None`), and checks it out when `checkout`, carrying local changes as
/// [`checkout`] does in `mode`. On an unborn branch (no commits yet) checking out names the branch to be.
pub fn create_branch(root: &Path, name: &str, from: Option<&str>, checkout: bool, mode: Mode) -> Result<Outcome, CheckoutError> {
    check_branch_name(root, name).map_err(CheckoutError::Git)?;
    let start = match from {
        Some(f) => Some(verify_commit(root, f).map_err(CheckoutError::Git)?),
        None => verify_commit(root, "HEAD").ok(),
    };
    let own = |s: &str| s.to_string();
    match (checkout, start) {
        (true, Some(start)) => run_switch(root, vec![own("switch"), own("-c"), own(name), start], mode),
        (true, None) => run_switch(root, vec![own("switch"), own("-c"), own(name)], mode),
        (false, Some(start)) => git::run_as_user(root, &["branch", name, &start]).map(|_| Outcome::default()).map_err(CheckoutError::Git),
        (false, None) => Err(CheckoutError::Git("There are no commits yet: a branch needs one to start at".into())),
    }
}

fn run_switch(root: &Path, mut args: Vec<String>, mode: Mode) -> Result<Outcome, CheckoutError> {
    if mode == Mode::Force {
        args.insert(1, "--discard-changes".into());
    }
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    if mode == Mode::Smart {
        return smart(root, &args);
    }
    switch(root, &args).map(|warning| Outcome { warning, ..Outcome::default() })
}

fn switch_args(root: &Path, target: &Target) -> Result<Vec<String>, CheckoutError> {
    let own = |s: &str| s.to_string();
    let name = match target {
        Target::Branch(b) | Target::Remote(b) | Target::Detached(b) => b,
    };
    // A ref named like an option (only `update-ref` can make one) is never handed to git.
    if name.starts_with('-') {
        return Err(CheckoutError::Git(format!("\"{name}\" cannot be checked out")));
    }
    Ok(match target {
        Target::Branch(b) => vec![own("switch"), own(b)],
        Target::Remote(r) => {
            let local = local_branch(root, r);
            match refs(root).map_err(CheckoutError::Git)?.local.contains(&local) {
                true => vec![own("switch"), local],
                // git names the local branch from the remote's refspec.
                false => vec![own("switch"), own("--track"), own(r)],
            }
        }
        Target::Detached(rev) => vec![own("switch"), own("--detach"), verify_commit(root, rev).map_err(CheckoutError::Git)?],
    })
}

/// Runs `git switch …`; a failure after HEAD already moved where asked (a post-checkout hook's) is a success
/// with a warning.
fn switch(root: &Path, args: &[&str]) -> Result<Option<String>, CheckoutError> {
    if args.iter().skip(1).any(|a| a.is_empty()) {
        return Err(CheckoutError::Git("Nothing to check out".into()));
    }
    let before = here(root);
    match git::run_as_user(root, args) {
        Ok(_) => Ok(None),
        Err(e) => match blocked(&e) {
            Some(b) => Err(b),
            None if moved(root, args, before) => Ok(Some(e)),
            None => Err(CheckoutError::Git(e)),
        },
    }
}

/// HEAD's full name (`refs/heads/main`, or `HEAD` when detached) and commit.
fn here(root: &Path) -> Option<(String, String)> {
    let text = |out: Vec<u8>| String::from_utf8_lossy(&out).trim().to_string();
    git::run(root, &["rev-parse", "--symbolic-full-name", "HEAD"]).ok().map(text).zip(git::run(root, &["rev-parse", "HEAD"]).ok().map(text))
}

/// Whether `git switch args` took HEAD where it was asked to, away from `before`.
fn moved(root: &Path, args: &[&str], before: Option<(String, String)>) -> bool {
    let Some((name, sha)) = here(root).filter(|now| Some(now) != before.as_ref()) else { return false };
    match args {
        [.., "--detach", rev] => name == "HEAD" && sha == *rev,
        [.., "--track", _] => name.starts_with("refs/heads/"),
        [_, "-c", b, ..] | [_, "--discard-changes", "-c", b, ..] | [_, "--discard-changes", b] | [_, b] => name == format!("refs/heads/{b}"),
        _ => false,
    }
}

/// The name the stash of a Smart Checkout has (`git stash list` shows it).
pub const SMART_STASH: &str = "Lenz Smart Checkout";

fn smart(root: &Path, args: &[&str]) -> Result<Outcome, CheckoutError> {
    // A dry try: carried changes need no stash, and untracked files in the way stay a refusal.
    match switch(root, args) {
        Err(CheckoutError::Blocked { untracked, .. }) if untracked.is_empty() => {}
        other => return other.map(|warning| Outcome { warning, ..Outcome::default() }),
    }
    git::run_as_user(root, &["stash", "push", "-m", SMART_STASH]).map_err(CheckoutError::Git)?;
    let warning = match switch(root, args) {
        Ok(warning) => warning,
        // Back as it was before the stash, or said where the changes are.
        Err(e) => {
            return Err(match restore(root) {
                Ok(()) => e,
                Err(_) => CheckoutError::Git(format!("The checkout failed and your changes could not be put back: they are kept in the stash \"{SMART_STASH}\" ({})", describe(&e))),
            });
        }
    };
    match restore(root) {
        Ok(()) => Ok(Outcome { warning, ..Outcome::default() }),
        Err(conflicted) => Ok(Outcome { stash_kept: Some(conflicted), warning }),
    }
}

/// Puts the Smart Checkout stash back, staged changes staged again when they still can be. On failure the stash
/// stays and the conflicted files (possibly none) are returned.
fn restore(root: &Path) -> Result<(), Vec<String>> {
    let conflicted = || {
        let out = git::run(root, &["diff", "--name-only", "--diff-filter=U", "-z"]).unwrap_or_default();
        out.split(|&b| b == 0).filter(|f| !f.is_empty()).map(|f| String::from_utf8_lossy(f).into_owned()).collect::<Vec<_>>()
    };
    if git::run_as_user(root, &["stash", "pop", "--index"]).is_ok() {
        return Ok(());
    }
    // `--index` refuses without touching anything when the staged part cannot be restored: try without it, unless
    // the attempt already left conflicts.
    let left = conflicted();
    if left.is_empty() && git::run_as_user(root, &["stash", "pop"]).is_ok() {
        return Ok(());
    }
    Err(if left.is_empty() { conflicted() } else { left })
}

fn describe(e: &CheckoutError) -> String {
    match e {
        CheckoutError::Blocked { .. } => "local changes would be overwritten".into(),
        CheckoutError::Git(e) => e.clone(),
    }
}

/// Git's refusal over files in the way (its messages are read in the C locale: see [`git::command_as_user`]).
fn blocked(stderr: &str) -> Option<CheckoutError> {
    let (mut tracked, mut untracked) = (Vec::new(), Vec::new());
    let mut list: Option<&mut Vec<String>> = None;
    for line in stderr.lines() {
        if line.contains("local changes to the following files would be overwritten") {
            list = Some(&mut tracked);
        } else if line.contains("untracked working tree files would be overwritten") || line.contains("would lose untracked files in them") {
            list = Some(&mut untracked);
        } else if let (Some(l), Some(path)) = (list.as_mut(), line.strip_prefix('\t')) {
            l.push(path.to_string());
        } else {
            list = None;
        }
    }
    (!tracked.is_empty() || !untracked.is_empty()).then_some(CheckoutError::Blocked { tracked, untracked })
}

/// Every tracked file with uncommitted changes, staged or not: what Force Checkout discards.
pub fn discarded_by_force(root: &Path) -> Result<Vec<String>, String> {
    let out = git::run(root, &["status", "--porcelain=v1", "-z", "--untracked-files=no"])?;
    let mut entries = out.split(|&b| b == 0).filter(|e| !e.is_empty());
    let mut files = Vec::new();
    while let Some(entry) = entries.next() {
        if entry.len() > 3 {
            files.push(String::from_utf8_lossy(&entry[3..]).into_owned());
        }
        if entry.first().is_some_and(|&c| c == b'R' || c == b'C') {
            entries.next(); // the original path of a rename or copy
        }
    }
    Ok(files)
}

/// `rev` as a full commit id, or why not.
pub fn verify_commit(root: &Path, rev: &str) -> Result<String, String> {
    let rev = rev.trim();
    if rev.is_empty() || rev.starts_with('-') {
        return Err(format!("No commit named \"{rev}\""));
    }
    let out = git::run(root, &["rev-parse", "--verify", "--quiet", "--end-of-options", &format!("{rev}^{{commit}}")]).map_err(|_| format!("No commit named \"{rev}\""))?;
    Ok(String::from_utf8_lossy(&out).trim().to_string())
}

/// Whether `name` can be a new branch: a valid name not already used.
pub fn check_branch_name(root: &Path, name: &str) -> Result<(), String> {
    if name.is_empty() || name.starts_with('-') || git::run(root, &["check-ref-format", "--branch", name]).is_err() {
        return Err(format!("\"{name}\" is not a valid branch name"));
    }
    if git::run(root, &["rev-parse", "--verify", "--quiet", &format!("refs/heads/{name}")]).is_ok() {
        return Err(format!("A branch named \"{name}\" already exists"));
    }
    Ok(())
}

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
    Checkout(Target),
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
pub fn target_label(t: &Target) -> &str {
    match t {
        Target::Branch(b) | Target::Detached(b) | Target::Remote(b) => b,
    }
}

impl CheckoutProblem {
    /// The dialog's buttons, in order: only Cancel while untracked files are in the way; Force's second step;
    /// else Smart, Force, Cancel.
    pub fn choices(&self) -> Vec<(&'static str, CheckoutChoice)> {
        match () {
            _ if !self.untracked.is_empty() => vec![("Cancel", CheckoutChoice::Cancel)],
            _ if self.discards.is_some() => vec![("Force Checkout", CheckoutChoice::ConfirmForce), ("Cancel", CheckoutChoice::Cancel)],
            _ => vec![("Smart Checkout", CheckoutChoice::Smart), ("Force Checkout", CheckoutChoice::Force), ("Cancel", CheckoutChoice::Cancel)],
        }
    }
    /// The dialog's text, the same in every frontend.
    pub fn text(&self) -> String {
        let name = self.retry.label();
        let list = |files: &[String]| files.iter().map(|f| format!("  {f}")).collect::<Vec<_>>().join("\n");
        match (&self.discards, self.untracked.is_empty()) {
            (_, false) => format!("Checking out {name} would overwrite these untracked files:\n\n{}\n\nMove or delete them first, then check out again.", list(&self.untracked)),
            (Some(d), true) => format!("Force Checkout discards your uncommitted changes to these files:\n\n{}\n\nThey cannot be recovered.", list(d)),
            (None, true) => format!("Checking out {name} would overwrite your local changes to:\n\n{}\n\nSmart Checkout stashes your changes, checks out {name} and puts them back. Force Checkout discards them.", list(&self.tracked)),
        }
    }
}

/// Runs `retry` in `mode`: the checkout, or the new branch made and checked out.
pub fn attempt(root: &Path, retry: &Retry, mode: Mode) -> Result<Outcome, CheckoutError> {
    match retry {
        Retry::Checkout(target) => checkout(root, target, mode),
        Retry::NewBranch { name, from } => create_branch(root, name, from.as_deref(), true, mode),
    }
}

/// The commit HEAD's changes count from (Show Branch Changes): where HEAD split from `from`, else from the base
/// branch; refused with no commit yet, with no base, or on the base itself. `git_dir` is `root`'s.
pub fn changes_since(root: &Path, git_dir: &Path, from: Option<&str>) -> Result<String, String> {
    verify_commit(root, "HEAD").map_err(|_| "there is no commit yet".to_string())?;
    let base = match from {
        Some(from) => from.to_string(),
        None => {
            let base = default_base(root).ok_or("no main or master branch to compare with")?;
            // Read now: HEAD may have moved since anything here last looked.
            if let Some(b) = head(root, git_dir).and_then(|h| h.branch).filter(|b| base.short == *b || base.short == format!("origin/{b}")) {
                return Err(format!("{b} is the base branch; Show Local Changes shows its changes"));
            }
            base.name
        }
    };
    merge_base(root, &base, "HEAD")
}

/// The branch a branch's changes are counted from (Show Branch Changes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Base {
    /// Fully named: `refs/remotes/origin/main`.
    pub name: String,
    /// As people say it: `origin/main`.
    pub short: String,
}

const BASES: [&str; 4] = ["refs/heads/main", "refs/heads/master", "refs/remotes/origin/main", "refs/remotes/origin/master"];

/// The remote's default branch (`origin/HEAD`), else the first of `main`, `master`, `origin/main`, `origin/master`.
pub fn default_base(root: &Path) -> Option<Base> {
    let mut args = vec!["for-each-ref", "--format=%(refname)%00%(symref)", "refs/remotes/origin/HEAD"];
    args.extend(BASES);
    let out = git::run(root, &args).ok()?;
    let found: Vec<(&[u8], &[u8])> = out.split(|&b| b == b'\n').filter_map(|l| {
        let at = l.iter().position(|&b| b == 0)?;
        Some((&l[..at], &l[at + 1..]))
    }).collect();
    // A broken origin/HEAD (its branch deleted) is not listed.
    let name = found.iter().find(|(r, s)| *r == b"refs/remotes/origin/HEAD" && !s.is_empty()).map(|(_, s)| *s)
        .or_else(|| BASES.iter().map(|b| b.as_bytes()).find(|b| found.iter().any(|(r, _)| r == b)))?;
    let name = String::from_utf8_lossy(name).into_owned();
    let short = ["refs/heads/", "refs/remotes/"].iter().find_map(|p| name.strip_prefix(p)).unwrap_or(&name).to_string();
    Some(Base { name, short })
}

/// The commit `a` and `b` last shared.
pub fn merge_base(root: &Path, a: &str, b: &str) -> Result<String, String> {
    let none = || format!("No common ancestor of {a} and {b}");
    if a.starts_with('-') || b.starts_with('-') {
        return Err(none());
    }
    let out = git::run(root, &["merge-base", a, b]).map_err(|_| none())?;
    Ok(String::from_utf8_lossy(&out).trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::testing::{commit_all, git, repo, write};

    /// A repository on `main` with `a.txt` and branch `other` changing it; the checkout commands (run as the user)
    /// get a committer from the repository's own config.
    fn two_branches() -> tempfile::TempDir {
        let r = repo();
        git(r.path(), &["config", "user.name", "t"]);
        git(r.path(), &["config", "user.email", "t@t"]);
        write(r.path(), "a.txt", "main\n");
        commit_all(r.path(), "main");
        git(r.path(), &["branch", "other"]);
        git(r.path(), &["switch", "-q", "other"]);
        write(r.path(), "a.txt", "other\n");
        write(r.path(), "only-other.txt", "o\n");
        commit_all(r.path(), "other");
        git(r.path(), &["switch", "-q", "main"]);
        r
    }
    fn branch(r: &Path) -> Option<String> {
        head_of(&git_dir(r).unwrap()).unwrap().branch
    }

    #[test]
    fn the_base_is_origin_head_else_main_master_origin_main_origin_master() {
        let r = two_branches();
        let base = |r: &Path| default_base(r).map(|b| (b.name, b.short));
        let pair = |n: &str, s: &str| Some((n.to_string(), s.to_string()));
        assert_eq!(base(r.path()), pair("refs/heads/main", "main"));
        git(r.path(), &["update-ref", "refs/remotes/origin/trunk", "other"]);
        git(r.path(), &["symbolic-ref", "refs/remotes/origin/HEAD", "refs/remotes/origin/trunk"]);
        assert_eq!(base(r.path()), pair("refs/remotes/origin/trunk", "origin/trunk"));
        git(r.path(), &["update-ref", "-d", "refs/remotes/origin/trunk"]);
        assert_eq!(base(r.path()), pair("refs/heads/main", "main"), "origin/HEAD pointing nowhere");
        git(r.path(), &["branch", "-q", "-m", "main", "master"]);
        assert_eq!(base(r.path()), pair("refs/heads/master", "master"));
        git(r.path(), &["update-ref", "refs/remotes/origin/main", "HEAD"]);
        assert_eq!(base(r.path()), pair("refs/heads/master", "master"), "local master before origin/main");
        git(r.path(), &["branch", "-q", "-m", "master", "dev"]);
        assert_eq!(base(r.path()), pair("refs/remotes/origin/main", "origin/main"));
        git(r.path(), &["update-ref", "-d", "refs/remotes/origin/main"]);
        assert_eq!(base(r.path()), None);
    }

    #[test]
    fn the_checkout_dialog_offers_what_is_safe() {
        let p = |untracked: &[&str], discards: Option<&[&str]>| CheckoutProblem {
            retry: Retry::Checkout(Target::Branch("other".into())),
            tracked: vec!["a.txt".into()],
            untracked: untracked.iter().map(|s| s.to_string()).collect(),
            discards: discards.map(|d| d.iter().map(|s| s.to_string()).collect()),
        };
        let labels = |p: &CheckoutProblem| p.choices().into_iter().map(|c| c.0).collect::<Vec<_>>();
        assert_eq!(labels(&p(&[], None)), ["Smart Checkout", "Force Checkout", "Cancel"]);
        assert_eq!(labels(&p(&["new.txt"], None)), ["Cancel"], "untracked files are never lost");
        assert_eq!(labels(&p(&[], Some(&["a.txt", "b.txt"]))), ["Force Checkout", "Cancel"]);
        assert!(p(&[], None).text().contains("would overwrite your local changes to:\n\n  a.txt"));
        assert!(p(&["new.txt"], None).text().contains("untracked files:\n\n  new.txt"));
        assert!(p(&[], Some(&["b.txt"])).text().contains("discards your uncommitted changes to these files:\n\n  b.txt"));
        assert_eq!(Retry::NewBranch { name: "x".into(), from: None }.label(), "x");
    }

    #[test]
    fn changes_count_from_where_head_split_from_its_base() {
        let r = two_branches();
        let gd = git_dir(r.path()).unwrap();
        let main = git(r.path(), &["rev-parse", "main"]).trim().to_string();
        assert_eq!(changes_since(r.path(), &gd, None), Err("main is the base branch; Show Local Changes shows its changes".into()));
        git(r.path(), &["switch", "-q", "other"]);
        assert_eq!(changes_since(r.path(), &gd, None), Ok(main.clone()));
        assert_eq!(changes_since(r.path(), &gd, Some("refs/heads/main")), Ok(main));
        let fresh = repo();
        assert_eq!(changes_since(fresh.path(), &git_dir(fresh.path()).unwrap(), None), Err("there is no commit yet".into()));
    }

    #[test]
    fn merge_base_is_where_a_branch_split() {
        let r = two_branches();
        let main = git(r.path(), &["rev-parse", "main"]).trim().to_string();
        assert_eq!(merge_base(r.path(), "refs/heads/main", "refs/heads/other"), Ok(main.clone()));
        git(r.path(), &["switch", "-q", "--orphan", "alone"]);
        write(r.path(), "z.txt", "z\n");
        commit_all(r.path(), "alone");
        assert_eq!(merge_base(r.path(), "refs/heads/main", "HEAD"), Err("No common ancestor of refs/heads/main and HEAD".into()));
        assert!(merge_base(r.path(), "--all", "HEAD").is_err());
    }

    #[test]
    fn head_reads_a_branch_a_detached_commit_and_an_unborn_branch() {
        let r = two_branches();
        let h = head_of(&git_dir(r.path()).unwrap()).unwrap();
        assert_eq!((h.label(), h.branch.as_deref()), ("main".into(), Some("main")));
        let sha = git(r.path(), &["rev-parse", "HEAD"]).trim().to_string();
        git(r.path(), &["switch", "-q", "--detach", "HEAD"]);
        assert_eq!(head_of(&git_dir(r.path()).unwrap()).unwrap().label(), sha[..7]);
        let fresh = repo();
        assert_eq!(head_of(&git_dir(fresh.path()).unwrap()).unwrap().label(), "main", "unborn: the branch to be");
        let outside = tempfile::tempdir().unwrap();
        assert_eq!(git_dir(outside.path()), Err("Not in a Git repository".into()));
    }

    #[test]
    fn refs_list_local_remote_tags_and_recent_with_odd_names() {
        let r = two_branches();
        git(r.path(), &["branch", "feature/x"]);
        git(r.path(), &["branch", "é"]);
        git(r.path(), &["tag", "v1"]);
        let up = repo();
        git(r.path(), &["remote", "add", "upstream", &up.path().to_string_lossy()]);
        git(r.path(), &["update-ref", "refs/remotes/upstream/topic", "HEAD"]);
        git(r.path(), &["symbolic-ref", "refs/remotes/upstream/HEAD", "refs/remotes/upstream/topic"]);
        git(r.path(), &["switch", "-q", "other"]);
        git(r.path(), &["switch", "-q", "feature/x"]);
        git(r.path(), &["switch", "-q", "main"]);
        let refs = refs(r.path()).unwrap();
        assert_eq!(refs.local, ["feature/x", "main", "other", "é"]);
        assert_eq!(refs.remote, ["upstream/topic"], "not upstream/HEAD");
        assert_eq!(refs.tags, ["v1"]);
        assert_eq!(refs.current.as_deref(), Some("main"));
        assert_eq!(refs.recent, ["feature/x", "other"], "newest first, not the current one");
        assert_eq!(local_branch(r.path(), "upstream/feature/x"), "feature/x");
        git(r.path(), &["remote", "add", "team/fork", &up.path().to_string_lossy()]);
        assert_eq!(local_branch(r.path(), "team/fork/topic"), "topic", "a remote with a slash in its name");
        // A ref named like an option is not listed.
        git(r.path(), &["update-ref", "refs/heads/-x", "HEAD"]);
        assert!(!super::refs(r.path()).unwrap().local.iter().any(|b| b.starts_with('-')));
        assert_eq!(checkout(r.path(), &Target::Branch("-x".into()), Mode::Normal), Err(CheckoutError::Git("\"-x\" cannot be checked out".into())));
    }

    #[test]
    fn checkout_of_each_kind_carries_local_changes() {
        let r = two_branches();
        write(r.path(), "new.txt", "untracked, not in the way\n");
        checkout(r.path(), &Target::Branch("other".into()), Mode::Normal).unwrap();
        assert_eq!(branch(r.path()).as_deref(), Some("other"));
        assert!(r.path().join("new.txt").exists(), "carried");
        checkout(r.path(), &Target::Detached("main".into()), Mode::Normal).unwrap();
        assert_eq!(branch(r.path()), None);
        assert_eq!(std::fs::read_to_string(r.path().join("a.txt")).unwrap(), "main\n");
        // A remote branch: its local branch is made to track it, then reused.
        git(r.path(), &["remote", "add", "origin", &r.path().to_string_lossy()]);
        git(r.path(), &["update-ref", "refs/remotes/origin/feature/y", "other"]);
        checkout(r.path(), &Target::Remote("origin/feature/y".into()), Mode::Normal).unwrap();
        assert_eq!(branch(r.path()).as_deref(), Some("feature/y"));
        assert_eq!(git(r.path(), &["config", "branch.feature/y.merge"]).trim(), "refs/heads/feature/y");
        checkout(r.path(), &Target::Branch("main".into()), Mode::Normal).unwrap();
        checkout(r.path(), &Target::Remote("origin/feature/y".into()), Mode::Normal).unwrap();
        assert_eq!(branch(r.path()).as_deref(), Some("feature/y"));
        assert!(matches!(checkout(r.path(), &Target::Detached("nope".into()), Mode::Normal), Err(CheckoutError::Git(e)) if e == "No commit named \"nope\""));
    }

    #[test]
    fn local_changes_in_the_way_are_listed_tracked_and_untracked() {
        let r = two_branches();
        write(r.path(), "a.txt", "edited\n");
        write(r.path(), "only-other.txt", "mine\n");
        let Err(CheckoutError::Blocked { tracked, untracked }) = checkout(r.path(), &Target::Branch("other".into()), Mode::Normal) else { panic!() };
        assert_eq!((tracked, untracked), (vec!["a.txt".to_string()], vec!["only-other.txt".to_string()]));
        // Smart Checkout leaves untracked files in the way as a refusal, touching nothing.
        assert!(matches!(checkout(r.path(), &Target::Branch("other".into()), Mode::Smart), Err(CheckoutError::Blocked { .. })));
        assert_eq!(std::fs::read_to_string(r.path().join("a.txt")).unwrap(), "edited\n");
        assert_eq!(git(r.path(), &["stash", "list"]), "");
    }

    #[test]
    fn smart_checkout_restores_the_change_or_keeps_the_stash() {
        let r = two_branches();
        // c.txt: `other` changes its first line, the local edit its last; git refuses, the stash merges back.
        write(r.path(), "c.txt", "1\n2\n3\n");
        commit_all(r.path(), "c on main");
        git(r.path(), &["switch", "-q", "other"]);
        git(r.path(), &["merge", "-q", "main", "-m", "m"]);
        write(r.path(), "c.txt", "X\n2\n3\n");
        commit_all(r.path(), "c on other");
        git(r.path(), &["switch", "-q", "main"]);
        write(r.path(), "c.txt", "1\n2\nZ\n");
        assert!(matches!(checkout(r.path(), &Target::Branch("other".into()), Mode::Normal), Err(CheckoutError::Blocked { .. })));
        let out = checkout(r.path(), &Target::Branch("other".into()), Mode::Smart).unwrap();
        assert_eq!(out, Outcome::default());
        assert_eq!(branch(r.path()).as_deref(), Some("other"));
        assert_eq!(std::fs::read_to_string(r.path().join("c.txt")).unwrap(), "X\n2\nZ\n");
        assert_eq!(git(r.path(), &["stash", "list"]), "", "restored and dropped");
        // a.txt's whole line differs on the two branches: the restored edit conflicts; the stash is kept.
        git(r.path(), &["checkout", "-q", "--", "c.txt"]);
        write(r.path(), "a.txt", "edited\n");
        let out = checkout(r.path(), &Target::Branch("main".into()), Mode::Smart).unwrap();
        assert_eq!(out.stash_kept, Some(vec!["a.txt".to_string()]));
        assert_eq!(branch(r.path()).as_deref(), Some("main"));
        assert!(git(r.path(), &["stash", "list"]).contains("Lenz Smart Checkout"));
    }

    #[test]
    fn force_checkout_discards_what_it_listed() {
        let r = two_branches();
        write(r.path(), "a.txt", "edited\n");
        write(r.path(), "keep.txt", "untracked\n");
        write(r.path(), "c.txt", "c\n");
        git(r.path(), &["add", "c.txt"]);
        assert_eq!(discarded_by_force(r.path()).unwrap(), ["a.txt", "c.txt"]);
        checkout(r.path(), &Target::Branch("other".into()), Mode::Force).unwrap();
        assert_eq!(std::fs::read_to_string(r.path().join("a.txt")).unwrap(), "other\n");
        assert!(r.path().join("keep.txt").exists(), "untracked files stay");
    }

    #[test]
    fn new_branches_are_checked_and_made() {
        let r = two_branches();
        assert_eq!(check_branch_name(r.path(), "a..b"), Err("\"a..b\" is not a valid branch name".into()));
        assert_eq!(check_branch_name(r.path(), "-x"), Err("\"-x\" is not a valid branch name".into()));
        assert_eq!(check_branch_name(r.path(), "other"), Err("A branch named \"other\" already exists".into()));
        assert_eq!(check_branch_name(r.path(), "feature/new"), Ok(()));
        create_branch(r.path(), "kept", Some("other"), false, Mode::Normal).unwrap();
        assert_eq!(branch(r.path()).as_deref(), Some("main"));
        create_branch(r.path(), "feature/new", Some("other"), true, Mode::Normal).unwrap();
        assert_eq!(branch(r.path()).as_deref(), Some("feature/new"));
        assert_eq!(std::fs::read_to_string(r.path().join("a.txt")).unwrap(), "other\n");
    }

    #[test]
    fn checkout_runs_the_repository_hooks_but_reads_do_not() {
        let r = two_branches();
        let hook = r.path().join(".git/hooks/post-checkout");
        std::fs::write(&hook, "#!/bin/sh\ntouch hook-ran\n").unwrap();
        std::fs::set_permissions(&hook, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        refs(r.path()).unwrap();
        worktrees(r.path()).unwrap();
        assert!(!r.path().join("hook-ran").exists());
        checkout(r.path(), &Target::Branch("other".into()), Mode::Normal).unwrap();
        assert!(r.path().join("hook-ran").exists(), "as git in a terminal would");
        // A failing hook after the switch: checked out all the same, with git's words as a warning.
        std::fs::write(&hook, "#!/bin/sh\necho hook says no >&2\nexit 1\n").unwrap();
        let out = checkout(r.path(), &Target::Branch("main".into()), Mode::Normal).unwrap();
        assert_eq!(branch(r.path()).as_deref(), Some("main"));
        assert!(out.warning.is_some_and(|w| w.contains("hook says no")));
        let out = create_branch(r.path(), "made", Some("other"), true, Mode::Normal).unwrap();
        assert_eq!(branch(r.path()).as_deref(), Some("made"));
        assert!(out.warning.is_some());
    }

    #[test]
    fn a_new_branch_over_blocking_changes_has_smart_and_force_too() {
        let r = two_branches();
        write(r.path(), "a.txt", "edited\n");
        assert!(matches!(create_branch(r.path(), "fresh", Some("other"), true, Mode::Normal), Err(CheckoutError::Blocked { .. })));
        assert!(refs(r.path()).unwrap().local.iter().all(|b| b != "fresh"), "not made either");
        let out = create_branch(r.path(), "fresh", Some("other"), true, Mode::Smart).unwrap();
        assert_eq!(branch(r.path()).as_deref(), Some("fresh"));
        assert_eq!(out.stash_kept, Some(vec!["a.txt".to_string()]), "edited vs other's whole line: kept");
        git(r.path(), &["reset", "-q", "--hard"]);
        write(r.path(), "a.txt", "edited again\n");
        create_branch(r.path(), "forced", Some("main"), true, Mode::Force).unwrap();
        assert_eq!((branch(r.path()).as_deref(), std::fs::read_to_string(r.path().join("a.txt")).unwrap().as_str()), (Some("forced"), "main\n"));
    }

    #[test]
    fn an_unborn_branch_takes_a_new_name_but_cannot_be_a_start() {
        let r = repo();
        create_branch(r.path(), "first", None, true, Mode::Normal).unwrap();
        assert_eq!(head_of(&git_dir(r.path()).unwrap()).unwrap().label(), "first");
        assert_eq!(create_branch(r.path(), "second", None, false, Mode::Normal), Err(CheckoutError::Git("There are no commits yet: a branch needs one to start at".into())));
    }

    #[test]
    fn staged_changes_stay_staged_through_smart_checkout() {
        let r = two_branches();
        write(r.path(), "c.txt", "1\n2\n3\n");
        commit_all(r.path(), "c on main");
        git(r.path(), &["switch", "-q", "other"]);
        git(r.path(), &["merge", "-q", "main", "-m", "m"]);
        write(r.path(), "c.txt", "X\n2\n3\n");
        commit_all(r.path(), "c on other");
        git(r.path(), &["switch", "-q", "main"]);
        write(r.path(), "c.txt", "1\n2\nZ\n");
        write(r.path(), "s.txt", "staged\n");
        git(r.path(), &["add", "s.txt"]);
        checkout(r.path(), &Target::Branch("other".into()), Mode::Smart).unwrap();
        assert_eq!(git(r.path(), &["diff", "--cached", "--name-only"]).trim(), "s.txt", "still staged");
    }

    #[test]
    fn refusals_over_untracked_folders_are_recognised() {
        let e = "error: Updating the following directories would lose untracked files in them:\n\tdir\n\nAborting\n";
        assert_eq!(blocked(e), Some(CheckoutError::Blocked { tracked: vec![], untracked: vec!["dir".into()] }));
    }

    #[test]
    fn a_reftable_repository_reads_head_through_git() {
        let r = tempfile::tempdir().unwrap();
        if !crate::git::testing::try_git(r.path(), &["init", "-q", "-b", "main", "--ref-format=reftable"]) {
            eprintln!("skipped: this git has no reftable");
            return;
        }
        git(r.path(), &["config", "user.name", "t"]);
        git(r.path(), &["config", "user.email", "t@t"]);
        write(r.path(), "a.txt", "a\n");
        commit_all(r.path(), "a");
        let dir = git_dir(r.path()).unwrap();
        assert_eq!(head(r.path(), &dir).unwrap().label(), "main");
        let before = head_signal(&dir);
        std::thread::sleep(std::time::Duration::from_millis(20));
        git(r.path(), &["switch", "-q", "-c", "next"]);
        assert_ne!(head_signal(&dir), before);
        assert_eq!(head(r.path(), &dir).unwrap().label(), "next");
    }
}

