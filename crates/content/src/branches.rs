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

pub fn worktrees(cwd: &Path) -> Result<Vec<Worktree>, String> {
    let out = git::run(cwd, &["worktree", "list", "--porcelain"])?;
    let here = toplevel(cwd).ok().and_then(|p| p.canonicalize().ok());
    let mut list = Vec::new();
    // ponytail: `-z` needs git 2.36; a path with a newline would be C-quoted here.
    for block in String::from_utf8_lossy(&out).split("\n\n").filter(|b| !b.trim().is_empty()) {
        let mut w = Worktree { path: PathBuf::new(), head: None, branch: None, current: false, dirty: None };
        let mut readable = true;
        for line in block.lines() {
            let (key, value) = line.split_once(' ').unwrap_or((line, ""));
            match key {
                "worktree" => w.path = PathBuf::from(value),
                "HEAD" => w.head = Some(value.to_string()),
                "branch" => w.branch = Some(value.strip_prefix("refs/heads/").unwrap_or(value).to_string()),
                "bare" | "prunable" => readable = false,
                _ => {}
            }
        }
        w.current = here.is_some() && w.path.canonicalize().ok() == here;
        w.dirty = if readable { dirty(&w.path).ok() } else { None };
        list.push(w);
    }
    Ok(list)
}

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

/// The local branch a remote-tracking branch checks out as (`origin/feature/x` → `feature/x`).
pub fn local_name(remote: &str) -> &str {
    remote.split_once('/').map_or(remote, |(_, b)| b)
}

/// Checks `target` out as the user would in a terminal (hooks and filters run). A refusal over local changes comes
/// back as [`CheckoutError::Blocked`]; Smart Checkout of untracked files is refused the same way, untouched.
pub fn checkout(root: &Path, target: &Target, mode: Mode) -> Result<Outcome, CheckoutError> {
    let args = switch_args(root, target)?;
    if mode == Mode::Smart {
        return smart(root, &args);
    }
    let mut args: Vec<&str> = args.iter().map(String::as_str).collect();
    if mode == Mode::Force {
        args.insert(1, "--discard-changes");
    }
    switch(root, &args).map(|warning| Outcome { warning, ..Outcome::default() })
}

/// Creates branch `name` at `from`, and checks it out when `checkout` (local changes go along when git allows).
pub fn create_branch(root: &Path, name: &str, from: &str, checkout: bool) -> Result<Outcome, CheckoutError> {
    check_branch_name(root, name).map_err(CheckoutError::Git)?;
    let from = verify_commit(root, from).map_err(CheckoutError::Git)?;
    if checkout {
        switch(root, &["switch", "-c", name, &from]).map(|warning| Outcome { warning, ..Outcome::default() })
    } else {
        git::run_as_user(root, &["branch", name, &from]).map(|_| Outcome::default()).map_err(CheckoutError::Git)
    }
}

fn switch_args(root: &Path, target: &Target) -> Result<Vec<String>, CheckoutError> {
    let own = |s: &str| s.to_string();
    Ok(match target {
        Target::Branch(b) => vec![own("switch"), own(b)],
        Target::Remote(r) => {
            let local = local_name(r);
            match refs(root).map_err(CheckoutError::Git)?.local.iter().any(|b| b == local) {
                true => vec![own("switch"), own(local)],
                false => vec![own("switch"), own("-c"), own(local), own("--track"), own(r)],
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
        [_, "-c", b, ..] | [_, "--discard-changes", "-c", b, ..] | [_, "--discard-changes", b] | [_, b] => name == format!("refs/heads/{b}"),
        _ => false,
    }
}

fn smart(root: &Path, args: &[String]) -> Result<Outcome, CheckoutError> {
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    // A dry try: carried changes need no stash, and untracked files in the way stay a refusal.
    match switch(root, &args) {
        Err(CheckoutError::Blocked { untracked, .. }) if untracked.is_empty() => {}
        other => return other.map(|warning| Outcome { warning, ..Outcome::default() }),
    }
    git::run_as_user(root, &["stash", "push", "-m", "Lenz Smart Checkout"]).map_err(CheckoutError::Git)?;
    let warning = match switch(root, &args) {
        Ok(warning) => warning,
        Err(e) => {
            // Back as it was before the stash.
            let _ = git::run_as_user(root, &["stash", "pop"]);
            return Err(e);
        }
    };
    if git::run_as_user(root, &["stash", "pop"]).is_ok() {
        return Ok(Outcome { warning, ..Outcome::default() });
    }
    let out = git::run(root, &["diff", "--name-only", "--diff-filter=U", "-z"]).unwrap_or_default();
    let files = out.split(|&b| b == 0).filter(|f| !f.is_empty()).map(|f| String::from_utf8_lossy(f).into_owned()).collect();
    Ok(Outcome { stash_kept: Some(files), warning })
}

/// Git's refusal over files in the way (its messages are read in the C locale: see [`git::command_as_user`]).
fn blocked(stderr: &str) -> Option<CheckoutError> {
    let (mut tracked, mut untracked) = (Vec::new(), Vec::new());
    let mut list: Option<&mut Vec<String>> = None;
    for line in stderr.lines() {
        if line.contains("local changes to the following files would be overwritten") {
            list = Some(&mut tracked);
        } else if line.contains("untracked working tree files would be overwritten") {
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
        assert_eq!(local_name("upstream/feature/x"), "feature/x");
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
        create_branch(r.path(), "kept", "other", false).unwrap();
        assert_eq!(branch(r.path()).as_deref(), Some("main"));
        create_branch(r.path(), "feature/new", "other", true).unwrap();
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
        let out = create_branch(r.path(), "made", "other", true).unwrap();
        assert_eq!(branch(r.path()).as_deref(), Some("made"));
        assert!(out.warning.is_some());
    }
}
