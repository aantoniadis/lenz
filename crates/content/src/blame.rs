//! Blame Annotations: which commit last changed each line, via `git blame --incremental`, shared by Lenz Diff (its
//! Sides) and Lenz Code (the file on disk). Synchronous: callers run it off the UI thread.

use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Write},
    ops::Range,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
};

use crate::git;

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

/// Every line of a file not in HEAD (new, untracked or ignored), as one "Not committed" run.
pub fn not_committed(lines: usize, path: &str) -> Hunk {
    let commit = Commit { sha: NOT_COMMITTED.into(), author: "Not Committed Yet".into(), author_time: now(), summary: String::new() };
    Hunk { lines: 0..lines, commit: Arc::new(commit), original_line: 1, original_path: path.into() }
}

/// Lines as git counts them: each `\n` ends one, and a last line without one counts too.
pub fn line_count(bytes: &[u8]) -> usize {
    bytes.iter().filter(|&&b| b == b'\n').count() + usize::from(!bytes.is_empty() && !bytes.ends_with(b"\n"))
}

/// The repository holding `file` (symlinks followed) and the file's `/`-separated path in it.
pub fn repository(file: &Path) -> Result<(PathBuf, String), String> {
    let file = std::fs::canonicalize(file).map_err(|e| format!("{}: {e}", file.display()))?;
    let dir = file.parent().ok_or("Not in a Git repository")?;
    let out = git::run(dir, &["rev-parse", "--show-toplevel"]).map_err(|_| "Not in a Git repository".to_string())?;
    let top = PathBuf::from(String::from_utf8_lossy(&out).trim_end_matches(['\n', '\r']));
    let root = std::fs::canonicalize(&top).unwrap_or(top);
    let relative = file.strip_prefix(&root).map_err(|_| "Not in a Git repository".to_string())?;
    let path = relative.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/");
    Ok((root, path))
}

/// Blames the work-tree file `path` of the repository at `root`, calling `on_hunk` as git reports each run. Stops,
/// ending git, once `current` says the result is no longer wanted (`Err("cancelled")`).
pub fn blame_worktree(root: &Path, path: &str, current: &dyn Fn() -> bool, mut on_hunk: impl FnMut(Hunk)) -> Result<(), String> {
    if !in_head(root, path) {
        let bytes = std::fs::read(root.join(path)).map_err(|e| format!("{path}: {e}"))?;
        if !bytes.is_empty() {
            on_hunk(not_committed(line_count(&bytes), path));
        }
        return Ok(());
    }
    let mut args = vec!["blame".to_string(), "--incremental".to_string()];
    args.extend(ignore_revs_args(root)?);
    args.extend(["--".into(), path.into()]);
    run_blame(root, &args, None, current, on_hunk)
}

/// The configured `blame.ignoreRevsFile`s, when one is missing: git dies on a missing one (still in 2.55),
/// so the list is replaced by the files that exist. Nothing when all exist: git applies them itself.
pub fn ignore_revs_args(root: &Path) -> Result<Vec<String>, String> {
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
pub fn resolve(root: &Path, rev: &str) -> Result<String, String> {
    let Some((a, b)) = rev.split_once("...") else { return Ok(rev.to_string()) };
    let or_head = |r: &str| if r.is_empty() { "HEAD" } else { r }.to_string();
    let out = git::run(root, &["merge-base", &or_head(a), &or_head(b)])?;
    Ok(String::from_utf8_lossy(&out).trim().to_string())
}

pub fn in_head(root: &Path, path: &str) -> bool {
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

/// Runs `git blame --incremental` with `args`, calling `on_hunk` per run of lines; once `current` turns false (checked
/// per line git writes) git is ended and the result is `Err("cancelled")`.
pub fn run_blame(root: &Path, args: &[String], stdin: Option<Vec<u8>>, current: &dyn Fn() -> bool, on_hunk: impl FnMut(Hunk)) -> Result<(), String> {
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
    let parsed = parse(BufReader::new(child.stdout.take().expect("piped stdout")), current, on_hunk);
    if parsed.as_ref().is_err_and(|e| e.kind() == std::io::ErrorKind::Interrupted) {
        let _ = child.kill();
        let _ = child.wait();
        return Err("cancelled".into());
    }
    // ponytail: stderr is read only after stdout ends; fine for blame's few warnings, a thread if it ever floods.
    let out = child.wait_with_output().map_err(|e| format!("lenz: git blame: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim_end().to_string());
    }
    parsed.map_err(|e| format!("lenz: git blame: {e}"))
}

/// Reads `--incremental` output: per run, `<sha> <original line> <final line> <count>`, the commit's headers
/// the first time that commit appears, and `filename <path>` last.
fn parse(mut out: impl BufRead, current: &dyn Fn() -> bool, mut on_hunk: impl FnMut(Hunk)) -> std::io::Result<()> {
    let mut commits: HashMap<String, Arc<Commit>> = HashMap::new();
    let mut run: Option<(String, usize, Range<usize>)> = None;
    let mut new = Commit::default(); // headers seen since the run's first line
    let mut buf = Vec::new();
    while {
        buf.clear();
        out.read_until(b'\n', &mut buf)? > 0
    } {
        if !current() {
            return Err(std::io::ErrorKind::Interrupted.into());
        }
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


    /// The file's blame: (summary, or "-" when not committed) per line.
    fn worktree(file: &Path) -> Vec<String> {
        let (root, path) = repository(file).unwrap();
        let mut lines: Vec<Option<String>> = Vec::new();
        blame_worktree(&root, &path, &|| true, |h| {
            if lines.len() < h.lines.end {
                lines.resize(h.lines.end, None);
            }
            for n in h.lines.clone() {
                lines[n] = Some(if h.commit.is_committed() { h.commit.summary.clone() } else { "-".into() });
            }
        })
        .unwrap();
        lines.into_iter().map(|l| l.expect("every line blamed")).collect()
    }

    #[test]
    fn worktree_blame_marks_edited_lines_not_committed() {
        let r = repo();
        write(r.path(), "a.txt", "a\nb\n");
        commit_all(r.path(), "first");
        write(r.path(), "a.txt", "a\nB\nc\n");
        assert_eq!(worktree(&r.path().join("a.txt")), ["first", "-", "-"]);
    }

    #[test]
    fn worktree_blame_untracked_and_ignored_files_are_not_committed() {
        let r = repo();
        write(r.path(), ".gitignore", "ignored.txt\n");
        commit_all(r.path(), "ignore");
        write(r.path(), "u.txt", "1\n2\n");
        write(r.path(), "ignored.txt", "x\n");
        assert_eq!(worktree(&r.path().join("u.txt")), ["-", "-"]);
        assert_eq!(worktree(&r.path().join("ignored.txt")), ["-"]);
        write(r.path(), "empty.txt", "");
        assert!(worktree(&r.path().join("empty.txt")).is_empty());
    }

    #[test]
    fn repository_finds_the_root_from_a_subdirectory_and_through_a_symlink() {
        let r = repo();
        write(r.path(), "sub/dir/f.txt", "x\n");
        commit_all(r.path(), "f");
        let root = std::fs::canonicalize(r.path()).unwrap();
        assert_eq!(repository(&r.path().join("sub/dir/f.txt")).unwrap(), (root.clone(), "sub/dir/f.txt".to_string()));
        let outside = tempfile::tempdir().unwrap();
        let link = outside.path().join("link.txt");
        std::os::unix::fs::symlink(r.path().join("sub/dir/f.txt"), &link).unwrap();
        assert_eq!(repository(&link).unwrap(), (root, "sub/dir/f.txt".to_string()));
        assert_eq!(worktree(&link), ["f"]);
    }

    #[test]
    fn repository_outside_git_is_an_error() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("f.txt"), "x\n").unwrap();
        // GIT_CEILING_DIRECTORIES is not set here: a temp dir under a repository would find it; /tmp is not one.
        assert_eq!(repository(&d.path().join("f.txt")), Err("Not in a Git repository".to_string()));
    }

    #[test]
    fn worktree_blame_counts_lines_like_the_document() {
        let r = repo();
        write(r.path(), "crlf.txt", "a\r\nb\r\n");
        write(r.path(), "open.txt", "a\nb");
        commit_all(r.path(), "both");
        assert_eq!(worktree(&r.path().join("crlf.txt")), ["both", "both"]);
        assert_eq!(worktree(&r.path().join("open.txt")), ["both", "both"]);
        assert_eq!((line_count(b"a\r\nb\r\n"), line_count(b"a\nb"), line_count(b"")), (2, 2, 0));
    }

    #[test]
    fn worktree_blame_path_with_spaces_and_non_ascii() {
        let r = repo();
        write(r.path(), "dir with space/é.txt", "x\ny\n");
        commit_all(r.path(), "odd name");
        assert_eq!(worktree(&r.path().join("dir with space/é.txt")), ["odd name", "odd name"]);
    }

    #[test]
    fn worktree_blame_in_a_submodule_uses_its_history() {
        let inner = repo();
        write(inner.path(), "lib.txt", "inner\n");
        commit_all(inner.path(), "inner commit");
        let outer = repo();
        write(outer.path(), "top.txt", "outer\n");
        commit_all(outer.path(), "outer commit");
        let url = inner.path().to_string_lossy().into_owned();
        if !crate::git::testing::try_git(outer.path(), &["-c", "protocol.file.allow=always", "submodule", "add", "-q", &url, "sub"]) {
            eprintln!("skipped: git submodule add is unavailable");
            return;
        }
        assert_eq!(worktree(&outer.path().join("sub/lib.txt")), ["inner commit"]);
    }

    #[test]
    fn blame_stops_and_kills_git_when_no_longer_current() {
        let r = repo();
        // A file touched by many commits: git keeps working (and writing) for a while.
        let mut lines: Vec<String> = (0..400).map(|i| format!("line {i}")).collect();
        for n in 0..300 {
            lines[(n * 37) % 400] = format!("line {} edited in {n}", (n * 37) % 400);
            write(r.path(), "big.txt", lines.join("\n") + "\n");
            git(r.path(), &["commit", "-qam", &format!("c{n}"), "--allow-empty"]);
            if n == 0 {
                git(r.path(), &["add", "big.txt"]);
                git(r.path(), &["commit", "-qm", "add"]);
            }
        }
        let (root, path) = repository(&r.path().join("big.txt")).unwrap();
        let started = std::time::Instant::now();
        let mut full = 0;
        blame_worktree(&root, &path, &|| true, |_| full += 1).unwrap();
        let whole = started.elapsed();
        let wanted = std::sync::atomic::AtomicBool::new(true);
        let mut calls = 0;
        let started = std::time::Instant::now();
        let result = blame_worktree(&root, &path, &|| wanted.load(std::sync::atomic::Ordering::Relaxed), |_| {
            calls += 1;
            wanted.store(false, std::sync::atomic::Ordering::Relaxed);
        });
        assert_eq!(result, Err("cancelled".to_string()));
        assert_eq!(calls, 1, "no hunk after the result stopped being wanted ({full} in a full run)");
        assert!(started.elapsed() < whole, "stopped early: {:?} vs {whole:?}", started.elapsed());
    }
}
