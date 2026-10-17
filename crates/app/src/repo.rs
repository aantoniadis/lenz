//! Sidebar sources, read from git and never written: local changes, branches, remotes, tags, worktrees, stashes.


use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use crate::{
    changeset::{ChangeSet, Status, list_git, toplevel},
    git,
};

/// A local branch, or a remote-tracking one (`origin/main`) in a [`Remote`].
#[derive(Debug, Clone, PartialEq)]
pub struct Branch {
    pub name: String,
    /// The tip commit.
    pub sha: String,
    /// The tip's committer date, seconds since the epoch.
    pub date: i64,
    /// Checked out here.
    pub current: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Remote {
    pub name: String,
    /// Most recent first.
    pub branches: Vec<Branch>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tag {
    pub name: String,
    /// The commit it points at (annotated tags peeled).
    pub commit: String,
    /// The tagger date of an annotated tag, else the commit's date; seconds since the epoch.
    pub date: i64,
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

#[derive(Debug, Clone, PartialEq)]
pub struct Stash {
    /// `n` in `stash@{n}`.
    pub index: usize,
    /// The stash commit: load it by this, as indexes shift when a stash is pushed or dropped.
    pub sha: String,
    /// git's reflog subject, e.g. `On main: message` or `WIP on main: <sha> <subject>`.
    pub message: String,
    /// Seconds since the epoch.
    pub date: i64,
}

/// The local changes of the repository or worktree around `dir`, in groups Staged (HEAD ⟷ index), Unstaged
/// (index ⟷ working tree) and Untracked; a file staged and changed again is in both of the first two. Conflicted
/// Files only with `conflicted` (the Conflicts entry lists the window's own).
pub fn local_changes(dir: &Path, conflicted: bool) -> Result<ChangeSet, String> {
    let root = toplevel(dir)?;
    let keep = |c: &crate::changeset::Comparison| conflicted || c.status != Status::Conflicted;
    // On an unborn branch `--cached` compares with the empty tree.
    let mut staged = list_git(&root, &root, true, &[], &[], false)?;
    staged.retain(keep);
    let (untracked, unstaged) =
        list_git(&root, &root, false, &[], &[], true)?.into_iter().filter(keep).partition(|c| c.status == Status::Untracked);
    ChangeSet::grouped(root, vec![("Staged".into(), staged), ("Unstaged".into(), unstaged), ("Untracked".into(), untracked)])
}

/// A stash (`stash@{n}` or its sha) in groups Changes (base ⟷ stashed working tree), Staged (base ⟷ stashed index,
/// when that differs) and Untracked (stashed with `-u`).
pub fn stash_changes(cwd: &Path, stash: &str) -> Result<ChangeSet, String> {
    let root = toplevel(cwd)?;
    let list = |left: &str, right: &str| list_git(&root, &root, false, &[left.into(), right.into()], &[], false);
    let base = format!("{stash}^1");
    let mut groups = vec![("Changes".into(), list(&base, stash)?), ("Staged".into(), list(&base, &format!("{stash}^2"))?)];
    let untracked = format!("{stash}^3");
    if git::is_commit(&root, &untracked) {
        let empty_tree = git::run_with_input(&root, &["hash-object", "-t", "tree", "--stdin"], Vec::new())?;
        let mut files = list(String::from_utf8_lossy(&empty_tree).trim(), &untracked)?;
        files.iter_mut().for_each(|c| c.status = Status::Untracked);
        groups.push(("Untracked".into(), files));
    }
    ChangeSet::grouped(root, groups)
}

/// Local branches: the current one first, then by last commit, most recent first.
pub fn branches(cwd: &Path) -> Result<Vec<Branch>, String> {
    branches_limited(cwd, usize::MAX)
}

/// At most `limit` branches, with HEAD first even when its commit is old.
pub fn branches_limited(cwd: &Path, limit: usize) -> Result<Vec<Branch>, String> {
    if limit==0 {return Ok(Vec::new());}
    let fields = ["%(HEAD)", "%(refname:lstrip=2)", "%(objectname)", "%(committerdate:unix)"];
    let format = format!("--format={}", fields.join("%00"));
    let count = format!("--count={limit}");
    let mut args=vec!["for-each-ref","--sort=-committerdate","--sort=-HEAD",&format];
    if limit != usize::MAX { args.push(&count); }
    args.push("refs/heads");
    let out=git::run(cwd,&args)?;
    let mut list = Vec::new();
    for line in String::from_utf8_lossy(&out).lines() {
        let values:Vec<_>=line.split('\0').collect();
        let [head,name,sha,date]=values[..] else { return Err(format!("lenz: unexpected branch output: {line:?}")); };
        list.push(Branch { name:name.into(), sha:sha.into(), date: date.parse().unwrap_or(0), current: head == "*" });
    }
    Ok(list)
}

/// Remote-tracking branches per remote (by name), each most recent first; `origin/HEAD` and the like left out.
pub fn remotes(cwd: &Path) -> Result<Vec<Remote>, String> {
    remotes_limited(cwd,usize::MAX).map(|(remotes,_)|remotes)
}
/// A bounded prefix of remote refs, plus whether another batch is available.
pub fn remotes_limited(cwd:&Path,limit:usize)->Result<(Vec<Remote>,bool),String>{
    let fields = ["%(symref)", "%(refname:lstrip=2)", "%(objectname)", "%(committerdate:unix)"];
    let mut refs=refs_limited(cwd,"refs/remotes","-committerdate",fields,limit.saturating_add(1))?;
    let more=refs.len()>limit;refs.truncate(limit);
    let mut by_remote: BTreeMap<String, Vec<Branch>> = BTreeMap::new();
    for [symref, name, sha, date] in refs {
        // ponytail: a remote whose name has a `/` splits at the first one; match against `git remote` if that matters.
        let Some((remote, _)) = name.split_once('/').filter(|_| symref.is_empty()) else { continue };
        let branch = Branch { name: name.clone(), sha, date: date.parse().unwrap_or(0), current: false };
        by_remote.entry(remote.to_string()).or_default().push(branch);
    }
    Ok((by_remote.into_iter().map(|(name, branches)| Remote { name, branches }).collect(),more))
}

/// Tags, newest first.
pub fn tags(cwd: &Path) -> Result<Vec<Tag>, String> {
    let fields = ["%(refname:lstrip=2)", "%(*objectname)", "%(objectname)", "%(creatordate:unix)"];
    Ok(refs(cwd, "refs/tags", "-creatordate", fields)?
        .into_iter()
        .map(|[name, peeled, sha, date]| {
            Tag { name, commit: if peeled.is_empty() { sha } else { peeled }, date: date.parse().unwrap_or(0) }
        })
        .collect())
}

/// The repository's worktrees, main first, each with its count of changed paths.
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

/// Stashes, newest first. Listed only: never applied or dropped.
pub fn stashes(cwd: &Path) -> Result<Vec<Stash>, String> {
    let out = git::run(cwd, &["stash", "list", "--format=%H%x00%ct%x00%gs"])?;
    let mut list = Vec::new();
    for (index, line) in String::from_utf8_lossy(&out).lines().enumerate() {
        let [sha, date, message] = line.splitn(3, '\0').collect::<Vec<_>>()[..] else {
            return Err(format!("lenz: unexpected `git stash list` output: {line:?}"));
        };
        list.push(Stash { index, sha: sha.into(), message: message.into(), date: date.parse().unwrap_or(0) });
    }
    Ok(list)
}

/// `git for-each-ref` over `pattern` sorted by `sort`, the `fields` of each ref.
fn refs<const N: usize>(cwd: &Path, pattern: &str, sort: &str, fields: [&str; N]) -> Result<Vec<[String; N]>, String> {
    refs_limited(cwd,pattern,sort,fields,usize::MAX)
}
fn refs_limited<const N:usize>(cwd:&Path,pattern:&str,sort:&str,fields:[&str;N],limit:usize)->Result<Vec<[String;N]>,String>{
    if limit==0{return Ok(Vec::new());}
    let format = format!("--format={}", fields.join("%00"));
    let sort=format!("--sort={sort}");let count=format!("--count={limit}");
    let mut args=vec!["for-each-ref",&sort,&format];
    if limit!=usize::MAX{args.push(&count);}
    args.push(pattern);let out=git::run(cwd,&args)?;
    String::from_utf8_lossy(&out)
        .lines()
        .map(|line| {
            let values: Vec<String> = line.split('\0').map(String::from).collect();
            values.try_into().map_err(|_| format!("lenz: unexpected `git for-each-ref` output: {line:?}"))
        })
        .collect()
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::testing::{commit_all, git, git_at, repo, write};

    /// Each group's name and paths.
    fn groups(set: &ChangeSet) -> Vec<(&str, Vec<&str>)> {
        let paths = |r: &std::ops::Range<usize>| set.comparisons[r.clone()].iter().map(|c| c.path.as_str()).collect();
        set.groups.iter().map(|(name, r)| (name.as_str(), paths(r))).collect()
    }

    fn texts(set: &ChangeSet, i: usize) -> (String, String) {
        set.texts(i).unwrap()
    }

    fn pair(l: &str, r: &str) -> (String, String) {
        (l.to_string(), r.to_string())
    }

    /// Commits everything with committer (and tagger) date `t` seconds after a fixed moment.
    fn commit_at(dir: &Path, message: &str, t: i64) {
        git(dir, &["add", "-A"]);
        git_at(dir, &["commit", "-q", "--no-verify", "-m", message], Some(&date(t)));
    }

    fn date(t: i64) -> String {
        format!("{} +0000", 1_700_000_000 + t)
    }

    fn head(dir: &Path) -> String {
        git(dir, &["rev-parse", "HEAD"]).trim().to_string()
    }

    #[test]
    fn local_changes_are_grouped_staged_unstaged_and_untracked() {
        let r = repo();
        write(r.path(), "a.txt", "one\n");
        write(r.path(), "b.txt", "b\n");
        commit_all(r.path(), "1");
        write(r.path(), "a.txt", "staged\n");
        write(r.path(), "new.txt", "added\n");
        git(r.path(), &["add", "-A"]);
        write(r.path(), "a.txt", "worktree\n");
        write(r.path(), "b.txt", "b2\n");
        write(r.path(), "dir/u.txt", "u\n");
        let set = local_changes(r.path(), true).unwrap();
        assert_eq!(
            groups(&set),
            [("Staged", vec!["a.txt", "new.txt"]), ("Unstaged", vec!["a.txt", "b.txt"]), ("Untracked", vec!["dir/u.txt"])]
        );
        assert_eq!(texts(&set, 0), pair("one\n", "staged\n"));
        assert_eq!(texts(&set, 1), pair("", "added\n"));
        assert_eq!(texts(&set, 2), pair("staged\n", "worktree\n"));
        assert_eq!(set.comparisons[4].status, Status::Untracked);
        assert_eq!(texts(&set, 4), pair("", "u\n"));
        // from a subdirectory too
        assert_eq!(groups(&local_changes(&r.path().join("dir"), true).unwrap()), groups(&set));
    }

    #[test]
    fn local_changes_leave_conflicted_files_to_the_conflicts_entry() {
        let r = repo();
        let d = r.path();
        write(d, "c.txt", "base\n");
        commit_all(d, "base");
        git(d, &["checkout", "-qb", "topic"]);
        write(d, "c.txt", "topic\n");
        commit_all(d, "topic");
        git(d, &["checkout", "-q", "main"]);
        write(d, "c.txt", "main\n");
        commit_all(d, "main");
        assert!(!crate::git::testing::try_git(d, &["merge", "topic"]));
        write(d, "edited.txt", "new\n");
        let paths = |set: &ChangeSet| set.comparisons.iter().map(|c| c.path.clone()).collect::<Vec<_>>();
        assert_eq!(paths(&local_changes(d, false).unwrap()), ["edited.txt"]);
        assert!(paths(&local_changes(d, true).unwrap()).contains(&"c.txt".to_string()));
    }

    #[test]
    fn an_unborn_branch_stages_against_nothing_and_a_clean_tree_has_no_groups() {
        let r = repo();
        write(r.path(), "a.txt", "first\n");
        git(r.path(), &["add", "a.txt"]);
        write(r.path(), "a.txt", "second\n");
        write(r.path(), "u.txt", "u\n");
        let set = local_changes(r.path(), true).unwrap();
        assert_eq!(groups(&set), [("Staged", vec!["a.txt"]), ("Unstaged", vec!["a.txt"]), ("Untracked", vec!["u.txt"])]);
        assert_eq!(set.comparisons[0].status, Status::Added);
        assert_eq!(texts(&set, 0), pair("", "first\n"));
        assert_eq!(texts(&set, 1), pair("first\n", "second\n"));
        assert!(branches(r.path()).unwrap().is_empty());
        assert!(stashes(r.path()).unwrap().is_empty());

        commit_all(r.path(), "1");
        let set = local_changes(r.path(), true).unwrap();
        assert!(set.groups.is_empty() && set.comparisons.is_empty());
    }

    #[test]
    fn a_stash_shows_its_changes_its_staged_part_and_its_untracked_files() {
        let r = repo();
        write(r.path(), "a.txt", "one\n");
        write(r.path(), "b.txt", "b\n");
        commit_all(r.path(), "1");
        write(r.path(), "a.txt", "staged\n");
        git(r.path(), &["add", "a.txt"]);
        write(r.path(), "a.txt", "worktree\n");
        write(r.path(), "b.txt", "b2\n");
        write(r.path(), "u.txt", "u\n");
        git(r.path(), &["stash", "push", "-q", "-u", "-m", "parked"]);

        let list = stashes(r.path()).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!((list[0].index, list[0].message.as_str()), (0, "On main: parked"));
        assert_eq!(list[0].sha, git(r.path(), &["rev-parse", "stash@{0}"]).trim());
        assert!(list[0].date > 0);

        for stash in ["stash@{0}", list[0].sha.as_str()] {
            let set = stash_changes(r.path(), stash).unwrap();
            assert_eq!(
                groups(&set),
                [("Changes", vec!["a.txt", "b.txt"]), ("Staged", vec!["a.txt"]), ("Untracked", vec!["u.txt"])]
            );
            assert_eq!(texts(&set, 0), pair("one\n", "worktree\n"));
            assert_eq!(texts(&set, 2), pair("one\n", "staged\n"));
            assert_eq!(set.comparisons[3].status, Status::Untracked);
            assert_eq!(texts(&set, 3), pair("", "u\n"));
        }
        // looking never applies or drops it
        assert_eq!(stashes(r.path()).unwrap(), list);
        assert!(local_changes(r.path(), true).unwrap().comparisons.is_empty());
    }

    #[test]
    fn a_stash_without_staged_changes_has_no_staged_group_and_stashes_list_newest_first() {
        let r = repo();
        write(r.path(), "a.txt", "one\n");
        commit_all(r.path(), "1");
        write(r.path(), "a.txt", "first\n");
        git(r.path(), &["stash", "push", "-q", "-m", "first"]);
        write(r.path(), "a.txt", "second\n");
        git(r.path(), &["stash", "push", "-q", "-m", "second"]);

        let list = stashes(r.path()).unwrap();
        let summary: Vec<_> = list.iter().map(|s| (s.index, s.message.as_str())).collect();
        assert_eq!(summary, [(0, "On main: second"), (1, "On main: first")]);
        let set = stash_changes(r.path(), &list[1].sha).unwrap();
        assert_eq!(groups(&set), [("Changes", vec!["a.txt"])]);
        assert_eq!(texts(&set, 0), pair("one\n", "first\n"));
    }

    #[test]
    fn worktrees_list_branch_head_and_dirty_count_and_show_their_own_changes() {
        let r = repo();
        write(r.path(), "a.txt", "one\n");
        commit_all(r.path(), "1");
        let elsewhere = tempfile::tempdir().unwrap();
        let wt = elsewhere.path().join("wt");
        git(r.path(), &["worktree", "add", "-q", "-b", "feature", wt.to_str().unwrap()]);
        write(&wt, "a.txt", "changed\n");
        write(&wt, "new.txt", "n\n");

        let list = worktrees(r.path()).unwrap();
        let summary: Vec<_> = list.iter().map(|w| (w.branch.as_deref(), w.current, w.dirty)).collect();
        assert_eq!(summary, [(Some("main"), true, Some(0)), (Some("feature"), false, Some(2))]);
        assert_eq!(list[1].path.canonicalize().unwrap(), wt.canonicalize().unwrap());
        assert_eq!(list[1].head.as_deref(), Some(head(r.path()).as_str()));
        let from_wt: Vec<_> = worktrees(&wt).unwrap().iter().map(|w| w.current).collect();
        assert_eq!(from_wt, [false, true]);

        let set = local_changes(&wt, true).unwrap();
        assert_eq!(groups(&set), [("Unstaged", vec!["a.txt"]), ("Untracked", vec!["new.txt"])]);
        assert_eq!(texts(&set, 0), pair("one\n", "changed\n"));
        assert!(local_changes(r.path(), true).unwrap().comparisons.is_empty());
    }

    #[test]
    fn branches_put_the_current_one_first_then_the_most_recent() {
        let r = repo();
        write(r.path(), "a.txt", "1\n");
        commit_at(r.path(), "1", 0);
        let main = head(r.path());
        for (branch, t) in [("recent", 30), ("middle", 20)] {
            git(r.path(), &["checkout", "-q", "-b", branch, "main"]);
            write(r.path(), "a.txt", branch);
            commit_at(r.path(), branch, t);
        }
        git(r.path(), &["checkout", "-q", "main"]);
        let list = branches(r.path()).unwrap();
        let summary: Vec<_> = list.iter().map(|b| (b.name.as_str(), b.current)).collect();
        assert_eq!(summary, [("main", true), ("recent", false), ("middle", false)]);
        assert_eq!((list[0].sha.as_str(), list[0].date), (main.as_str(), 1_700_000_000));
        assert_eq!(list[1].date, 1_700_000_030);
    }

    #[test]
    fn tags_are_newest_first_with_annotated_ones_peeled_to_their_commit() {
        let r = repo();
        write(r.path(), "a.txt", "1\n");
        commit_at(r.path(), "1", 0);
        let first = head(r.path());
        git(r.path(), &["tag", "v1"]);
        write(r.path(), "a.txt", "2\n");
        commit_at(r.path(), "2", 10);
        let second = head(r.path());
        git(r.path(), &["tag", "v2"]);
        // annotated later, on the older commit
        git_at(r.path(), &["tag", "-a", "-m", "release", "v3", &first], Some(&date(20)));
        let summary: Vec<_> = tags(r.path()).unwrap().into_iter().map(|t| (t.name, t.commit, t.date)).collect();
        assert_eq!(
            summary,
            [
                ("v3".into(), first.clone(), 1_700_000_020),
                ("v2".into(), second, 1_700_000_010),
                ("v1".into(), first, 1_700_000_000)
            ]
        );
    }

    #[test]
    fn remote_branches_are_grouped_per_remote() {
        let r = repo();
        write(r.path(), "a.txt", "1\n");
        commit_at(r.path(), "1", 0);
        git(r.path(), &["checkout", "-q", "-b", "topic"]);
        write(r.path(), "a.txt", "2\n");
        commit_at(r.path(), "2", 10);
        let (origin, backup) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        for (name, dir, branches) in [("origin", &origin, &["main", "topic"][..]), ("backup", &backup, &["main"][..])] {
            git(dir.path(), &["init", "-q", "--bare", "-b", "main"]);
            git(r.path(), &["remote", "add", name, dir.path().to_str().unwrap()]);
            git(r.path(), &[&["push", "-q", name][..], branches].concat());
            git(r.path(), &["fetch", "-q", name]);
        }
        git(r.path(), &["remote", "set-head", "origin", "main"]); // origin/HEAD, which isn't listed
        let summary: Vec<_> = remotes(r.path())
            .unwrap()
            .into_iter()
            .map(|remote| (remote.name, remote.branches.into_iter().map(|b| b.name).collect::<Vec<_>>()))
            .collect();
        assert_eq!(
            summary,
            [("backup".into(), vec!["backup/main".to_string()]), ("origin".into(), vec!["origin/topic".into(), "origin/main".into()])]
        );
        assert!(remotes(repo().path()).unwrap().is_empty());
    }
}
