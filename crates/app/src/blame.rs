//! Blame Annotations: which commit last changed each line of a git-backed Side, via `git blame --incremental`.
//! gpui-free and synchronous: the UI runs it on a background executor.

pub use lenz_content::blame::*;

use crate::{
    changeset::{ChangeSet, Comparison, Content, Spec, Status, right_is_worktree},
    view::Side,
};

/// Which version of a file a Side shows; with the Side's path, the cache key for its blame.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Version {
    /// A revision as the user named it; `A...B` means their merge base.
    Rev(String),
    Index,
    WorkTree,
}

/// The version a Side shows, or `None` when it isn't git-backed (plain files, an empty Side, a symlink or a
/// submodule): no Blame Annotation then.
pub fn version(spec: &Spec, c: &Comparison, side: Side) -> Option<Version> {
    let Spec::Git { cached, revs, .. } = spec else { return None };
    let content = match side {
        Side::Left => &c.left,
        Side::Right => &c.right,
    };
    if !matches!(content, Content::Blob(_) | Content::File(_)) {
        return None;
    }
    let rev = |r: &str| Version::Rev(if r.is_empty() { "HEAD" } else { r }.to_string());
    Some(match (side, &revs[..]) {
        (Side::Left, _) if c.status == Status::Conflicted => rev("HEAD"),
        (Side::Right, _) if c.status == Status::Conflicted => Version::WorkTree,
        (Side::Right, _) if *cached => Version::Index,
        (Side::Right, _) if right_is_worktree(*cached, revs) => Version::WorkTree,
        (Side::Right, [r]) => rev(r.split_once("..").map_or(r, |(_, b)| b.trim_start_matches('.'))),
        (Side::Right, [_, b]) => rev(b),
        (Side::Left, []) if *cached => rev("HEAD"),
        (Side::Left, []) => Version::Index,
        // `A...B` keeps both ends: resolved to the merge base when blamed.
        (Side::Left, [r]) if r.contains("...") => rev(r),
        (Side::Left, [r]) => rev(r.split_once("..").map_or(r, |(a, _)| a)),
        (Side::Left, [a, _]) => rev(a),
        _ => return None,
    })
}

/// Blames one Side, calling `on_hunk` as git reports each run of lines.
pub fn blame_incremental(
    set: &ChangeSet,
    c: &Comparison,
    side: Side,
    version: &Version,
    mut on_hunk: impl FnMut(Hunk),
) -> Result<(), String> {
    let root = set.git_root().ok_or("lenz: blame needs a git Change Set")?;
    let (path, content) = match side {
        Side::Left => (&c.old_path, &c.left),
        Side::Right => (&c.path, &c.right),
    };
    let bytes = set.read(content)?;
    if bytes.is_empty() {
        return Ok(()); // no lines; also a conflicted file HEAD doesn't have
    }
    let mut args = vec!["blame".to_string(), "--incremental".to_string()];
    args.extend(ignore_revs_args(root)?);
    let mut stdin = None;
    match version {
        Version::Rev(rev) => args.extend([resolve(root, rev)?, "--".into(), path.clone()]),
        Version::Index | Version::WorkTree => {
            // Blame starts from HEAD: a staged rename is found under its old path, a new file not at all.
            let Some(head_path) = [path, &c.old_path].into_iter().find(|p| in_head(root, p)) else {
                on_hunk(not_committed(line_count(&bytes), path));
                return Ok(());
            };
            // `--contents` with no revision: blames that content against HEAD (git 2.30; with a revision needs 2.41).
            if *version == Version::Index || head_path != path {
                args.extend(["--contents".into(), "-".into()]);
                stdin = Some(bytes);
            }
            args.extend(["--".into(), head_path.clone()]);
        }
    }
    run_blame(root, &args, stdin, &|| true, on_hunk)
}

/// Every line's blame, in line order (the tests' view of the stream).
#[cfg(test)]
pub fn blame(set: &ChangeSet, c: &Comparison, side: Side, version: &Version) -> Result<Vec<BlameLine>, String> {
    let mut lines = Vec::new();
    blame_incremental(set, c, side, version, |h| {
        if lines.len() < h.lines.end {
            lines.resize(h.lines.end, None);
        }
        for (i, n) in h.lines.enumerate() {
            let original_path = h.original_path.clone();
            lines[n] = Some(BlameLine { commit: h.commit.clone(), original_line: h.original_line + i, original_path });
        }
    })?;
    lines.into_iter().collect::<Option<Vec<_>>>().ok_or_else(|| "lenz: git blame skipped lines".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{path::Path, sync::Arc};

    use crate::git::testing::{commit_all, git, repo, write};

    fn spec(cached: bool, revs: &[&str]) -> Spec {
        Spec::Git { cached, revs: revs.iter().map(|s| s.to_string()).collect(), paths: vec![], untracked: true }
    }

    fn load(dir: &Path, spec: &Spec) -> ChangeSet {
        ChangeSet::load(spec, dir).unwrap()
    }

    /// Blames comparison `path` of `spec` on `side`: (summary or "-" when not committed, original line) per line.
    fn annotate(dir: &Path, spec: &Spec, path: &str, side: Side) -> Vec<(String, usize)> {
        let set = load(dir, spec);
        let c = set.comparisons.iter().find(|c| c.path == path).unwrap_or_else(|| panic!("no {path}"));
        let v = version(spec, c, side).expect("a git Side");
        let lines = blame(&set, c, side, &v).unwrap();
        let label = |l: &BlameLine| if l.commit.is_committed() { l.commit.summary.clone() } else { "-".into() };
        lines.iter().map(|l| (label(l), l.original_line)).collect()
    }

    fn rows(r: &[(&str, usize)]) -> Vec<(String, usize)> {
        r.iter().map(|(s, n)| (s.to_string(), *n)).collect()
    }

    #[test]
    fn each_side_is_blamed_at_its_own_version() {
        let file = |left, right| Comparison {
            old_path: "a".into(),
            path: "a".into(),
            status: Status::Modified,
            left,
            right,
        };
        let blob = || Content::Blob("abc".into());
        let wt = || Content::File("/r/a".into());
        let rev = |r: &str| Some(Version::Rev(r.into()));
        let c = file(blob(), wt());
        assert_eq!(version(&spec(false, &[]), &c, Side::Left), Some(Version::Index));
        assert_eq!(version(&spec(false, &[]), &c, Side::Right), Some(Version::WorkTree));
        assert_eq!(version(&spec(false, &["v1"]), &c, Side::Left), rev("v1"));
        assert_eq!(version(&spec(false, &["v1"]), &c, Side::Right), Some(Version::WorkTree));
        let c = file(blob(), blob());
        assert_eq!(version(&spec(true, &[]), &c, Side::Left), rev("HEAD"));
        assert_eq!(version(&spec(true, &[]), &c, Side::Right), Some(Version::Index));
        assert_eq!(version(&spec(true, &["v1"]), &c, Side::Left), rev("v1"));
        assert_eq!(version(&spec(false, &["a", "b"]), &c, Side::Left), rev("a"));
        assert_eq!(version(&spec(false, &["a", "b"]), &c, Side::Right), rev("b"));
        assert_eq!(version(&spec(false, &["a..b"]), &c, Side::Left), rev("a"));
        assert_eq!(version(&spec(false, &["a..b"]), &c, Side::Right), rev("b"));
        assert_eq!(version(&spec(false, &["a.."]), &c, Side::Right), rev("HEAD"));
        assert_eq!(version(&spec(false, &["..b"]), &c, Side::Left), rev("HEAD"));
        assert_eq!(version(&spec(false, &["a...b"]), &c, Side::Left), rev("a...b"));
        assert_eq!(version(&spec(false, &["a...b"]), &c, Side::Right), rev("b"));
        // Conflicts are always HEAD against the working tree.
        let c = Comparison { status: Status::Conflicted, ..file(Content::Blob("HEAD:a".into()), wt()) };
        assert_eq!(version(&spec(true, &[]), &c, Side::Left), rev("HEAD"));
        assert_eq!(version(&spec(true, &[]), &c, Side::Right), Some(Version::WorkTree));
        // Nothing to blame.
        let c = file(Content::Empty, Content::Submodule("abc".into()));
        assert_eq!(version(&spec(false, &["a", "b"]), &c, Side::Left), None);
        assert_eq!(version(&spec(false, &["a", "b"]), &c, Side::Right), None);
        let plain = Spec::Plain { left: "l".into(), right: "r".into() };
        assert_eq!(version(&plain, &file(wt(), wt()), Side::Left), None);
    }

    #[test]
    fn a_revision_side_is_blamed_at_that_revision() {
        let r = repo();
        write(r.path(), "a.txt", "one\ntwo\n");
        commit_all(r.path(), "first");
        write(r.path(), "a.txt", "one\nTWO\nthree\n");
        commit_all(r.path(), "second");
        write(r.path(), "a.txt", "changed in third\n");
        commit_all(r.path(), "third");
        let s = spec(false, &["HEAD~2", "HEAD~1"]);
        assert_eq!(annotate(r.path(), &s, "a.txt", Side::Left), rows(&[("first", 1), ("first", 2)]));
        assert_eq!(annotate(r.path(), &s, "a.txt", Side::Right), rows(&[("first", 1), ("second", 2), ("second", 3)]));
    }

    #[test]
    fn working_tree_lines_not_yet_committed_are_marked() {
        let r = repo();
        write(r.path(), "a.txt", "one\ntwo\n");
        commit_all(r.path(), "first");
        write(r.path(), "a.txt", "one\nnew\ntwo\n");
        let s = spec(false, &["HEAD"]);
        assert_eq!(annotate(r.path(), &s, "a.txt", Side::Right), rows(&[("first", 1), ("-", 2), ("first", 2)]));
    }

    #[test]
    fn the_index_side_is_blamed_with_its_staged_content() {
        let r = repo();
        write(r.path(), "a.txt", "one\ntwo\n");
        commit_all(r.path(), "first");
        write(r.path(), "a.txt", "staged\none\ntwo\n");
        git(r.path(), &["add", "a.txt"]);
        write(r.path(), "a.txt", "staged\none\ntwo\nworktree\n");
        // `git diff`: the index on the left.
        let index = rows(&[("-", 1), ("first", 1), ("first", 2)]);
        assert_eq!(annotate(r.path(), &spec(false, &[]), "a.txt", Side::Left), index);
        // `git diff --cached`: the index on the right.
        assert_eq!(annotate(r.path(), &spec(true, &[]), "a.txt", Side::Right), index);
    }

    #[test]
    fn files_head_lacks_are_wholly_not_committed() {
        let r = repo();
        write(r.path(), "a.txt", "a\n");
        commit_all(r.path(), "first");
        write(r.path(), "staged.txt", "s1\ns2");
        git(r.path(), &["add", "staged.txt"]);
        write(r.path(), "untracked.txt", "u\n");
        let s = spec(true, &[]);
        assert_eq!(annotate(r.path(), &s, "staged.txt", Side::Right), rows(&[("-", 1), ("-", 2)]));
        let s = spec(false, &[]);
        assert_eq!(annotate(r.path(), &s, "untracked.txt", Side::Right), rows(&[("-", 1)]));
    }

    #[test]
    fn renames_blame_the_old_path_at_the_old_revision() {
        let r = repo();
        let body: String = (1..=10).map(|i| format!("line {i}\n")).collect();
        write(r.path(), "old.txt", &body);
        commit_all(r.path(), "first");
        git(r.path(), &["mv", "old.txt", "new.txt"]);
        write(r.path(), "new.txt", format!("{body}added\n"));
        commit_all(r.path(), "rename");
        let s = spec(false, &["HEAD~1", "HEAD"]);
        let left = annotate(r.path(), &s, "new.txt", Side::Left);
        assert_eq!(left.len(), 10);
        assert!(left.iter().all(|(s, _)| s == "first"), "{left:?}");
        let right = annotate(r.path(), &s, "new.txt", Side::Right);
        assert_eq!(right[9], ("first".to_string(), 10));
        assert_eq!(right[10], ("rename".to_string(), 11));
        // A rename staged but not committed: blamed through the old path in HEAD.
        git(r.path(), &["mv", "new.txt", "newer.txt"]);
        let staged = annotate(r.path(), &spec(true, &[]), "newer.txt", Side::Right);
        assert_eq!(staged[0], ("first".to_string(), 1));
    }

    #[test]
    fn original_lines_follow_lines_that_moved_down() {
        let r = repo();
        write(r.path(), "a.txt", "a\nb\n");
        commit_all(r.path(), "first");
        write(r.path(), "a.txt", "x\ny\na\nb\n");
        commit_all(r.path(), "second");
        let s = spec(false, &["HEAD~1", "HEAD"]);
        let set = load(r.path(), &s);
        let c = &set.comparisons[0];
        let lines = blame(&set, c, Side::Right, &version(&s, c, Side::Right).unwrap()).unwrap();
        let got: Vec<_> = lines.iter().map(|l| (l.commit.summary.as_str(), l.original_line, &*l.original_path)).collect();
        assert_eq!(got, [("second", 1, "a.txt"), ("second", 2, "a.txt"), ("first", 1, "a.txt"), ("first", 2, "a.txt")]);
        // One commit's metadata is shared, not copied.
        assert!(Arc::ptr_eq(&lines[2].commit, &lines[3].commit));
    }

    #[test]
    fn hunks_arrive_progressively_in_git_order_and_cover_every_line_once() {
        let r = repo();
        write(r.path(), "a.txt", "1\n2\n3\n4\n5\n6\n");
        commit_all(r.path(), "first");
        write(r.path(), "a.txt", "1\nB\n3\n4\nE\n6\n");
        commit_all(r.path(), "second");
        write(r.path(), "a.txt", "1\nB\nC\n4\nE\n6\nnew\n");
        let s = spec(false, &[]);
        let set = load(r.path(), &s);
        let c = &set.comparisons[0];
        let mut hunks = Vec::new();
        blame_incremental(&set, c, Side::Right, &Version::WorkTree, |h| hunks.push(h)).unwrap();
        let raw = git(r.path(), &["blame", "--incremental", "--", "a.txt"]);
        let headers: Vec<(String, usize)> = raw
            .lines()
            .filter_map(|l| {
                let f: Vec<_> = l.split(' ').collect();
                (f.len() == 4 && f[0].len() == 40).then(|| (f[0].to_string(), f[2].parse::<usize>().unwrap() - 1))
            })
            .collect();
        let got: Vec<_> = hunks.iter().map(|h| (h.commit.sha.clone(), h.lines.start)).collect();
        assert_eq!(got, headers);
        assert!(hunks.len() > 3);
        let mut seen = vec![0; 7];
        hunks.iter().flat_map(|h| h.lines.clone()).for_each(|i| seen[i] += 1);
        assert_eq!(seen, [1; 7]);
    }

    #[test]
    fn an_ignore_revs_file_hides_a_formatting_commit() {
        let r = repo();
        write(r.path(), "a.txt", "fn  a ( ) {}\nfn  b ( ) {}\n");
        commit_all(r.path(), "real");
        write(r.path(), "a.txt", "fn a() {}\nfn b() {}\n");
        commit_all(r.path(), "reformat");
        let s = spec(false, &["HEAD~1", "HEAD"]);
        assert_eq!(annotate(r.path(), &s, "a.txt", Side::Right), rows(&[("reformat", 1), ("reformat", 2)]));
        let sha = git(r.path(), &["rev-parse", "HEAD"]);
        write(r.path(), ".git-blame-ignore-revs", sha);
        git(r.path(), &["config", "blame.ignoreRevsFile", ".git-blame-ignore-revs"]);
        assert_eq!(annotate(r.path(), &s, "a.txt", Side::Right), rows(&[("real", 1), ("real", 2)]));
        // A configured file that doesn't exist (git itself dies on it) is skipped; the others still apply.
        git(r.path(), &["config", "--add", "blame.ignoreRevsFile", "missing-file"]);
        assert_eq!(annotate(r.path(), &s, "a.txt", Side::Right), rows(&[("real", 1), ("real", 2)]));
        git(r.path(), &["config", "--unset-all", "blame.ignoreRevsFile"]);
        git(r.path(), &["config", "blame.ignoreRevsFile", "missing-file"]);
        assert_eq!(annotate(r.path(), &s, "a.txt", Side::Right), rows(&[("reformat", 1), ("reformat", 2)]));
    }

    #[test]
    fn an_empty_side_blames_to_nothing() {
        let r = repo();
        write(r.path(), "empty.txt", "");
        write(r.path(), "a.txt", "a\n");
        commit_all(r.path(), "first");
        write(r.path(), "empty.txt", "now\n");
        let s = spec(false, &[]);
        assert!(annotate(r.path(), &s, "empty.txt", Side::Left).is_empty());
    }

    #[test]
    #[ignore = "timing: cargo test -p lenz blame_timing -- --ignored --nocapture"]
    fn blame_timing_on_a_long_history() {
        let r = repo();
        let mut lines: Vec<String> = (0..3000).map(|i| format!("line {i} of a long file")).collect();
        let mut seed = 42u64;
        let started = std::time::Instant::now();
        for n in 0..200 {
            for _ in 0..15 {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                let i = (seed >> 33) as usize % lines.len();
                lines[i] = format!("line {i} edited in commit {n}");
            }
            write(r.path(), "big.txt", lines.join("\n") + "\n");
            commit_all(r.path(), &format!("commit {n}"));
        }
        eprintln!("built 200 commits in {:?}", started.elapsed());
        write(r.path(), "big.txt", lines.join("\n") + "\nuncommitted\n");
        let s = spec(false, &["HEAD~100"]);
        let set = load(r.path(), &s);
        let c = &set.comparisons[0];
        for (side, what) in [(Side::Right, "working tree"), (Side::Left, "HEAD~100")] {
            let v = version(&s, c, side).unwrap();
            let started = std::time::Instant::now();
            let mut first = None;
            blame_incremental(&set, c, side, &v, |_| {
                first.get_or_insert(started.elapsed());
            })
            .unwrap();
            let done = started.elapsed();
            crate::perf::record(&format!("blame/{what}/first"), "ms", &[first.unwrap().as_secs_f64() * 1000.0]);
            crate::perf::record(&format!("blame/{what}/complete"), "ms", &[done.as_secs_f64() * 1000.0]);
            eprintln!("{what}: first hunk {:?}, done {:?}", first.unwrap(), done);
        }
    }
}
