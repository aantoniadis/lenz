//! Blame Annotations: which commit last changed each line of a git-backed Side, via `git blame --incremental`.
//! gpui-free and synchronous: the UI runs it on a background executor.

use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Write},
    ops::Range,
    path::Path,
    process::Stdio,
    sync::Arc,
};

use crate::{
    changeset::{ChangeSet, Comparison, Content, Spec, Status, right_is_worktree},
    git,
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

/// A commit as blame reports it; shared by every line it annotates.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Commit {
    pub sha: String,
    pub author: String,
    /// Unix seconds.
    pub author_time: i64,
    pub summary: String,
}

/// A run of a Side's lines from one commit, as git reports them (not in line order).
#[derive(Debug, Clone, PartialEq)]
pub struct Hunk {
    /// 0-based lines of the Side.
    pub lines: Range<usize>,
    pub commit: Arc<Commit>,
    /// 1-based line, in `commit`, of the first line.
    pub original_line: usize,
    /// The file's path in `commit`.
    pub original_path: Arc<str>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BlameLine {
    pub commit: Arc<Commit>,
    /// 1-based line in `commit`: where "Annotate previous revision" scrolls to.
    pub original_line: usize,
    pub original_path: Arc<str>,
}

/// What the commit popup shows.
#[derive(Debug, Clone, PartialEq)]
pub struct CommitDetails {
    pub sha: String,
    pub author: String,
    pub email: String,
    /// Unix seconds.
    pub author_time: i64,
    pub message: String,
    /// (`git diff-tree --name-status` letter, path) against the first parent.
    pub files: Vec<(char, String)>,
}

const NOT_COMMITTED: &str = "0000000000000000000000000000000000000000";

impl Commit {
    /// False for working-tree and staged lines: shown as "Not committed".
    pub fn is_committed(&self) -> bool {
        self.sha != NOT_COMMITTED
    }
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
                let lines = bytes.iter().filter(|&&b| b == b'\n').count() + usize::from(!bytes.ends_with(b"\n"));
                let commit = Commit {
                    sha: NOT_COMMITTED.into(),
                    author: "Not Committed Yet".into(),
                    author_time: now(),
                    summary: String::new(),
                };
                on_hunk(Hunk { lines: 0..lines, commit: Arc::new(commit), original_line: 1, original_path: path.as_str().into() });
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
    run_blame(root, &args, stdin, on_hunk)
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

/// The configured `blame.ignoreRevsFile`s, when one is missing: git dies on a missing one (still in 2.55),
/// so the list is replaced by the files that exist. Nothing when all exist: git applies them itself.
fn ignore_revs_args(root: &Path) -> Result<Vec<String>, String> {
    let out = git::command(root)?
        .args(["config", "--path", "--get-all", "blame.ignoreRevsFile"])
        .stderr(Stdio::null())
        .output()
        .map_err(|e| format!("lenz: git config: {e}"))?;
    if !out.status.success() {
        return Ok(Vec::new()); // unset
    }
    let files = String::from_utf8_lossy(&out.stdout);
    // Relative paths resolve against git's cwd: blame always runs at the root.
    let exists = |f: &str| root.join(f.strip_prefix(":(optional)").unwrap_or(f)).is_file();
    if files.lines().all(|f| f.is_empty() || exists(f)) {
        return Ok(Vec::new());
    }
    let mut kept = Vec::new();
    for f in files.lines() {
        match f {
            "" => kept.clear(), // an empty entry resets the list
            f if exists(f) => kept.push(format!("--ignore-revs-file={}", f.strip_prefix(":(optional)").unwrap_or(f))),
            _ => {}
        }
    }
    Ok(["--no-ignore-revs-file".to_string()].into_iter().chain(kept).collect())
}

/// `A...B` → their merge base; other revisions as they are.
fn resolve(root: &Path, rev: &str) -> Result<String, String> {
    let Some((a, b)) = rev.split_once("...") else { return Ok(rev.to_string()) };
    let or_head = |r: &str| if r.is_empty() { "HEAD" } else { r }.to_string();
    let out = git::run(root, &["merge-base", &or_head(a), &or_head(b)])?;
    Ok(String::from_utf8_lossy(&out).trim().to_string())
}

fn in_head(root: &Path, path: &str) -> bool {
    git::command(root).is_ok_and(|mut cmd| {
        cmd.args(["cat-file", "-e", &format!("HEAD:{path}")])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    })
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

fn run_blame(root: &Path, args: &[String], stdin: Option<Vec<u8>>, on_hunk: impl FnMut(Hunk)) -> Result<(), String> {
    let mut child = git::command(root)?
        .args(args)
        .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("lenz: git blame: {e}"))?;
    if let (Some(bytes), Some(mut pipe)) = (stdin, child.stdin.take()) {
        // git reads all of it before writing anything; if it died instead, its stderr says why.
        let _ = pipe.write_all(&bytes);
    }
    let parsed = parse(BufReader::new(child.stdout.take().expect("piped stdout")), on_hunk);
    // ponytail: stderr is read only after stdout ends; fine for blame's few warnings, a thread if it ever floods.
    let out = child.wait_with_output().map_err(|e| format!("lenz: git blame: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim_end().to_string());
    }
    parsed.map_err(|e| format!("lenz: git blame: {e}"))
}

/// Reads `--incremental` output: per run, `<sha> <original line> <final line> <count>`, the commit's headers
/// the first time that commit appears, and `filename <path>` last.
fn parse(mut out: impl BufRead, mut on_hunk: impl FnMut(Hunk)) -> std::io::Result<()> {
    let mut commits: HashMap<String, Arc<Commit>> = HashMap::new();
    let mut run: Option<(String, usize, Range<usize>)> = None;
    let mut new = Commit::default(); // headers seen since the run's first line
    let mut buf = Vec::new();
    while {
        buf.clear();
        out.read_until(b'\n', &mut buf)? > 0
    } {
        let line = buf.strip_suffix(b"\n").unwrap_or(&buf);
        let Some((sha, original_line, lines)) = &run else {
            let header = String::from_utf8_lossy(line);
            let fields: Vec<&str> = header.split(' ').collect();
            let num = |i: usize| fields.get(i).and_then(|n| n.parse::<usize>().ok());
            let (Some(original), Some(fin @ 1..), Some(count)) = (num(1), num(2), num(3)) else {
                return Err(std::io::Error::other(format!("unexpected line {header:?}")));
            };
            run = Some((fields[0].to_string(), original, fin - 1..fin - 1 + count));
            continue;
        };
        let (key, value) = line.split_at(line.iter().position(|&b| b == b' ').unwrap_or(line.len()));
        let value = value.strip_prefix(b" ").unwrap_or(value);
        match key {
            b"author" => new.author = String::from_utf8_lossy(value).into_owned(),
            b"author-time" => new.author_time = String::from_utf8_lossy(value).parse().unwrap_or(0),
            b"summary" => new.summary = String::from_utf8_lossy(value).into_owned(),
            b"filename" => {
                let commit = commits
                    .entry(sha.clone())
                    .or_insert_with(|| Arc::new(Commit { sha: sha.clone(), ..std::mem::take(&mut new) }))
                    .clone();
                on_hunk(Hunk {
                    lines: lines.clone(),
                    commit,
                    original_line: *original_line,
                    original_path: unquote(value).into(),
                });
                run = None;
            }
            _ => {} // committer, previous, boundary, ignored, ...
        }
    }
    Ok(())
}

/// Undoes git's C-style quoting of a path (`"t\"ab\t\303\251.txt"`).
fn unquote(s: &[u8]) -> String {
    let Some(inner) = s.strip_prefix(b"\"").and_then(|s| s.strip_suffix(b"\"")) else {
        return String::from_utf8_lossy(s).into_owned();
    };
    let mut out = Vec::with_capacity(inner.len());
    let mut bytes = inner.iter().copied();
    while let Some(b) = bytes.next() {
        if b != b'\\' {
            out.push(b);
            continue;
        }
        match bytes.next() {
            Some(d @ b'0'..=b'7') => {
                let digits = [d, bytes.next().unwrap_or(b'0'), bytes.next().unwrap_or(b'0')];
                out.push(digits.iter().fold(0u8, |n, d| n.wrapping_mul(8).wrapping_add(d - b'0')));
            }
            Some(b'a') => out.push(7),
            Some(b'b') => out.push(8),
            Some(b't') => out.push(b'\t'),
            Some(b'n') => out.push(b'\n'),
            Some(b'v') => out.push(11),
            Some(b'f') => out.push(12),
            Some(b'r') => out.push(b'\r'),
            Some(c) => out.push(c),
            None => {}
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn commit_details(repo: &Path, sha: &str) -> Result<CommitDetails, String> {
    // The --no-* flags keep user config (signatures, notes) out of the output.
    let format = "--format=%H%x00%aN%x00%aE%x00%at%x00%P%x00%B";
    let show = git::run(repo, &["show", "--no-patch", "--no-show-signature", "--no-notes", format, sha, "--"])?;
    let show = String::from_utf8_lossy(&show);
    let fields: Vec<&str> = show.splitn(6, '\0').collect();
    let [sha, author, email, time, parents, message] = fields[..] else {
        return Err(format!("lenz: unexpected `git show` output: {show:?}"));
    };
    // Against the first parent, as "Show this commit's changes" (`<sha>~ <sha>`) shows it.
    let mut args = vec!["diff-tree", "-r", "-z", "--name-status"];
    match parents.split(' ').next().filter(|p| !p.is_empty()) {
        Some(first) => args.push(first),
        None => args.extend(["--root", "--no-commit-id"]),
    }
    args.push(sha);
    let tree = git::run(repo, &args)?;
    let fields: Vec<String> =
        tree.split(|&b| b == 0).filter(|f| !f.is_empty()).map(|f| String::from_utf8_lossy(f).into_owned()).collect();
    let files = fields.chunks(2).filter_map(|p| Some((p[0].chars().next()?, p.get(1)?.clone()))).collect();
    Ok(CommitDetails {
        sha: sha.to_string(),
        author: author.to_string(),
        email: email.to_string(),
        author_time: time.parse().unwrap_or(0),
        message: message.trim_end().to_string(),
        files,
    })
}

/// How new `author_time` is between the oldest and newest shown: 0 = oldest … 1 = newest.
pub fn age_shade(author_time: i64, newest: i64, oldest: i64) -> f32 {
    if newest <= oldest {
        return 1.0;
    }
    ((author_time - oldest) as f64 / (newest - oldest) as f64).clamp(0.0, 1.0) as f32
}

/// What to compare commit `sha` against: its first parent (`sha^1`, spelled as the log spells it so the same
/// comparison compares equal), or for a root commit the empty tree.
pub fn parent_or_empty_tree(repo: &Path, sha: &str) -> Result<String, String> {
    let parent = format!("{sha}^1");
    if git::run(repo, &["rev-parse", "--verify", "-q", &format!("{parent}^{{commit}}")]).is_ok() {
        return Ok(parent);
    }
    // the empty tree's id depends on the repository's hash (SHA-1 or SHA-256)
    let id = git::run_with_input(repo, &["hash-object", "-t", "tree", "--stdin"], Vec::new())?;
    Ok(String::from_utf8_lossy(&id).trim().to_string())
}

/// A Unix time as a UTC calendar date, `2026-10-02` (Howard Hinnant's `civil_from_days`).
pub fn date(unix: i64) -> String {
    let z = unix.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

/// `2026-10-08 14:32` for `unix` seconds at `offset` minutes east of UTC.
pub fn date_time(unix: i64, offset: i16) -> String {
    let local = unix + i64::from(offset) * 60;
    let minute = local.rem_euclid(86_400) / 60;
    format!("{} {:02}:{:02}", date(local), minute / 60, minute % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_time_uses_the_given_offset_across_midnight() {
        assert_eq!(date_time(0, 0), "1970-01-01 00:00");
        assert_eq!(date_time(1_791_489_600, 180), "2026-10-08 23:00");
        assert_eq!(date_time(1_791_489_600, 250), "2026-10-09 00:10");
        assert_eq!(date_time(1_791_489_600, -480), "2026-10-08 12:00");
        assert_eq!(date_time(-60, 0), "1969-12-31 23:59");
    }
    use crate::git::testing::{commit_all, git, repo, write};

    #[test]
    fn the_base_of_a_root_commit_is_the_empty_tree() {
        let r = repo();
        write(r.path(), "a.txt", "one\n");
        commit_all(r.path(), "root");
        write(r.path(), "a.txt", "two\n");
        commit_all(r.path(), "second");
        let head = git(r.path(), &["rev-parse", "HEAD"]).trim().to_string();
        let root = git(r.path(), &["rev-parse", "HEAD~"]).trim().to_string();
        assert_eq!(parent_or_empty_tree(r.path(), &head).unwrap(), format!("{head}^1"));
        let empty = parent_or_empty_tree(r.path(), &root).unwrap();
        assert_eq!(git(r.path(), &["cat-file", "-t", &empty]).trim(), "tree");
        assert_eq!(git(r.path(), &["ls-tree", &empty]).trim(), "");
    }

    #[test]
    fn unix_times_become_calendar_dates() {
        assert_eq!(date(0), "1970-01-01");
        assert_eq!(date(951_782_400), "2000-02-29");
        assert_eq!(date(1_791_000_000), "2026-10-03");
        assert_eq!(date(-86_400), "1969-12-31");
    }

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
    fn commit_details_carry_the_full_message_and_changed_files() {
        let r = repo();
        write(r.path(), "a.txt", "a\n");
        write(r.path(), "gone.txt", "g\n");
        commit_all(r.path(), "root");
        write(r.path(), "a.txt", "b\n");
        write(r.path(), "dir/new file.txt", "n\n");
        std::fs::remove_file(r.path().join("gone.txt")).unwrap();
        commit_all(r.path(), "Subject line\n\nBody paragraph one.\nSecond body line.");
        let sha = git(r.path(), &["rev-parse", "HEAD"]).trim().to_string();
        let d = commit_details(r.path(), &sha).unwrap();
        assert_eq!((d.sha.as_str(), d.author.as_str(), d.email.as_str()), (sha.as_str(), "t", "t@t"));
        assert_eq!(d.message, "Subject line\n\nBody paragraph one.\nSecond body line.");
        assert!(d.author_time > 1_600_000_000);
        assert_eq!(d.files, [('M', "a.txt".into()), ('A', "dir/new file.txt".into()), ('D', "gone.txt".into())]);
        // The root commit lists everything as added.
        let root = git(r.path(), &["rev-parse", "HEAD~1"]).trim().to_string();
        let d = commit_details(r.path(), &root).unwrap();
        assert_eq!(d.files, [('A', "a.txt".into()), ('A', "gone.txt".into())]);
        assert!(commit_details(r.path(), "0123456789012345678901234567890123456789").is_err());
    }

    #[test]
    fn a_merge_lists_the_files_it_changed_against_its_first_parent() {
        let r = repo();
        write(r.path(), "a.txt", "a\n");
        commit_all(r.path(), "root");
        git(r.path(), &["checkout", "-q", "-b", "topic"]);
        write(r.path(), "topic.txt", "t\n");
        commit_all(r.path(), "topic");
        git(r.path(), &["checkout", "-q", "main"]);
        write(r.path(), "main.txt", "m\n");
        commit_all(r.path(), "main");
        git(r.path(), &["merge", "-q", "--no-edit", "topic"]);
        let sha = git(r.path(), &["rev-parse", "HEAD"]).trim().to_string();
        assert_eq!(commit_details(r.path(), &sha).unwrap().files, [('A', "topic.txt".into())]);
    }

    #[test]
    fn age_shade_spans_oldest_to_newest() {
        assert_eq!(age_shade(100, 200, 100), 0.0);
        assert_eq!(age_shade(200, 200, 100), 1.0);
        assert_eq!(age_shade(150, 200, 100), 0.5);
        assert_eq!(age_shade(300, 200, 100), 1.0);
        // One commit only: all newest.
        assert_eq!(age_shade(5, 5, 5), 1.0);
    }

    #[test]
    fn quoted_file_names_are_unquoted() {
        assert_eq!(unquote(br"plain.txt"), "plain.txt");
        assert_eq!(unquote(br#""t\"ab\t\303\251\\.txt""#), "t\"ab\té\\.txt");
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
