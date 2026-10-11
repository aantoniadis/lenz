//! The merge / rebase / cherry-pick / revert / stash pop in progress and its Conflicted Files, read from git; the only
//! index writes Lenz makes (Mark resolved, Re-open).

use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    io::Write,
};

use crate::{
    changeset::looks_binary,
    git::{self, CatFile},
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Operation {
    Merge,
    Rebase,
    /// `git am`.
    Am,
    CherryPick,
    Revert,
    /// Unmerged entries without a state file: `git stash pop` / `apply` (also `checkout -m`, `merge --squash`).
    StashPop,
}

impl Operation {
    /// What finishes it once every file is resolved; Lenz never runs it. Nothing for a stash pop.
    pub fn continue_command(self) -> Option<&'static str> {
        Some(match self {
            Operation::Merge => "git merge --continue",
            Operation::Rebase => "git rebase --continue",
            Operation::Am => "git am --continue",
            Operation::CherryPick => "git cherry-pick --continue",
            Operation::Revert => "git revert --continue",
            Operation::StashPop => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct InProgress {
    pub operation: Operation,
    /// What stage 2 holds: the current branch, or the upstream in a rebase.
    pub ours: String,
    /// What stage 3 holds, e.g. `topic` or `76c12fd (topic: edit two)`.
    pub theirs: String,
    /// Rebase / am progress: commit k of n.
    pub step: Option<(u32, u32)>,
}

/// The operation in progress in the repository at `root` (its top level), checked in git's own order.
pub fn in_progress(root: &Path) -> Result<Option<InProgress>, String> {
    let [merge_head, merge_msg, apply, merge, pick_head, revert_head, sequencer] =
        git_paths(root, ["MERGE_HEAD", "MERGE_MSG", "rebase-apply", "rebase-merge", "CHERRY_PICK_HEAD", "REVERT_HEAD", "sequencer"])?;
    let head = || head_name(root);
    let found = |operation, ours, theirs, step| Ok(Some(InProgress { operation, ours, theirs, step }));
    let step = |dir: &Path, k: &str, n: &str| Some((read(&dir.join(k))?.parse().ok()?, read(&dir.join(n))?.parse().ok()?));

    if let Some(sha) = read(&merge_head) {
        let sha = sha.lines().next().unwrap_or_default(); // an octopus lists several
        // "Merge branch 'topic'", "Merge remote-tracking branch 'origin/x' into main"
        let msg = read(&merge_msg).unwrap_or_default();
        let name = msg.lines().next().and_then(|l| l.split('\'').nth(1)).filter(|_| msg.starts_with("Merge "));
        return found(Operation::Merge, head(), name.map_or_else(|| describe(root, sha), str::to_string), None);
    }
    if apply.is_dir() {
        let step = step(&apply, "next", "last");
        if apply.join("applying").exists() {
            let subject = read(&apply.join("final-commit")).and_then(|m| m.lines().next().map(str::to_string));
            return found(Operation::Am, head(), subject.unwrap_or_else(|| "Patch".into()), step);
        }
        return found(Operation::Rebase, onto(root, &apply), describe(root, "REBASE_HEAD"), step);
    }
    if merge.is_dir() {
        return found(Operation::Rebase, onto(root, &merge), describe(root, "REBASE_HEAD"), step(&merge, "msgnum", "end"));
    }
    if pick_head.exists() {
        return found(Operation::CherryPick, head(), describe(root, "CHERRY_PICK_HEAD"), None);
    }
    if revert_head.exists() {
        return found(Operation::Revert, head(), format!("parent of {}", describe(root, "REVERT_HEAD")), None);
    }
    // A multi-commit cherry-pick or revert whose stopped pick was committed by hand: still the todo's first line.
    if let Some(todo) = read(&sequencer.join("todo")) {
        let mut words = todo.lines().next().unwrap_or_default().split_whitespace();
        let (verb, sha) = (words.next().unwrap_or_default(), words.next().unwrap_or("HEAD"));
        return match verb {
            "revert" => found(Operation::Revert, head(), format!("parent of {}", describe(root, sha)), None),
            _ => found(Operation::CherryPick, head(), describe(root, sha), None),
        };
    }
    if !git::run(root, &["ls-files", "-u", "-z"])?.is_empty() {
        return found(Operation::StashPop, "Updated upstream".into(), "Stashed changes".into(), None);
    }
    Ok(None)
}

/// Which side a one-sided conflict lost its version on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Side {
    Us,
    Them,
    Both,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    Text,
    Binary,
    /// Text with no Base.
    AddedByBoth,
    /// Ours or Theirs (or both) missing: modify/delete, rename/delete, and the `path~HEAD` / `path` pair git (ort)
    /// splits a file-vs-symlink or file-vs-directory conflict into.
    DeletedBy(Side),
    /// A symbolic link on our side or theirs.
    Symlink,
    Submodule,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Stage {
    /// e.g. `0o100644`, `0o120000`, `0o160000`.
    pub mode: u32,
    pub sha: String,
    /// The blob's size as stored; `None` for a submodule commit.
    pub size: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConflictedFile {
    /// From the top level.
    pub path: String,
    /// git's `XY` code: `UU`, `AA`, `DU`, `UD`, `AU`, `UA` or `DD`.
    pub xy: String,
    /// Base (stage 1), Ours (2), Theirs (3).
    pub stages: [Option<Stage>; 3],
    pub kind: Kind,
    /// rerere replayed a recorded resolution into the file (still unmerged unless `rerere.autoupdate`).
    pub rerere: bool,
}

impl ConflictedFile {
    /// `git status`'s own words for the conflict, e.g. "deleted by them".
    #[allow(dead_code)] // for the Version Picker's header (ticket 08)
    pub fn wording(&self) -> &'static str {
        match self.xy.as_str() {
            "DD" => "both deleted",
            "AU" => "added by us",
            "UD" => "deleted by them",
            "UA" => "added by them",
            "DU" => "deleted by us",
            "AA" => "both added",
            _ => "both modified",
        }
    }
}

/// The Conflicted Files of the repository at `root` (its top level), in path order.
pub fn list(root: &Path) -> Result<Vec<ConflictedFile>, String> {
    let out = git::run(root, &["status", "--porcelain=v2", "-z", "--untracked-files=no"])?;
    let mut fields = out.split(|&b| b == 0).filter(|f| !f.is_empty());
    let mut files = Vec::new();
    while let Some(entry) = fields.next() {
        match entry[0] {
            b'2' => _ = fields.next(), // a rename's original path
            // u <XY> <sub> <m1> <m2> <m3> <mW> <h1> <h2> <h3> <path>
            b'u' => {
                let entry = git::path_text(entry)?;
                let f: Vec<&str> = entry.splitn(11, ' ').collect();
                let [_, xy, _, m1, m2, m3, _, h1, h2, h3, path] = f[..] else {
                    return Err(format!("lenz: unexpected git status entry {entry:?}"));
                };
                let stage = |mode: &str, sha: &str| {
                    let mode = u32::from_str_radix(mode, 8).unwrap_or(0);
                    (mode != 0).then(|| Stage { mode, sha: sha.to_string(), size: None })
                };
                let stages = [stage(m1, h1), stage(m2, h2), stage(m3, h3)];
                files.push(ConflictedFile { path: path.into(), xy: xy.into(), stages, kind: Kind::Text, rerere: false });
            }
            _ => {}
        }
    }
    if files.is_empty() {
        return Ok(files);
    }

    // One batch stream: the size from each header, and for the NUL check the bytes of a blob up to `LARGE` or the
    // first 8000 of a bigger one, the rest streamed past. A conflicted multi-gigabyte binary is never held.
    let mut cat_file = CatFile::spawn(root, "--batch")?;
    let mut nul = HashSet::new();
    for file in &mut files {
        for stage in file.stages.iter_mut().flatten().filter(|s| s.mode != 0o160000) {
            let (size, head) = cat_file
                .get_bounded(&stage.sha, crate::document::LARGE as usize, 8000)
                .map_err(|e| format!("lenz: git cat-file: {e}"))?
                .unwrap_or_default();
            stage.size = Some(size as u64);
            if looks_binary(&head) {
                nul.insert(file.path.clone());
            }
        }
    }
    drop(cat_file);
    let merge = merge_attrs(root, &files)?;
    let tracked = rerere_tracked(root)?;

    for file in &mut files {
        let mode = |i: usize| file.stages[i].as_ref().map(|s| s.mode);
        let [base, ours, theirs] = [mode(0), mode(1), mode(2)];
        file.kind = if ours == Some(0o160000) || theirs == Some(0o160000) {
            Kind::Submodule
        } else if ours.is_none() || theirs.is_none() {
            Kind::DeletedBy(match (ours, theirs) {
                (None, None) => Side::Both,
                (None, _) => Side::Us,
                _ => Side::Them,
            })
        } else if ours == Some(0o120000) || theirs == Some(0o120000) {
            Kind::Symlink
        } else if match merge.get(&file.path).map(String::as_str) {
            // `binary` is a macro for `-diff -merge -text`; `-diff` alone doesn't stop git merging the text.
            Some("unset" | "binary") => true,
            Some("set" | "text") => false,
            _ => nul.contains(&file.path),
        } {
            Kind::Binary
        } else if base.is_none() {
            Kind::AddedByBoth
        } else {
            Kind::Text
        };
        // rerere takes up every text conflict, and drops from `MERGE_RR` the ones it replays a resolution into
        file.rerere = matches!(file.kind, Kind::Text | Kind::AddedByBoth) && tracked.as_ref().is_some_and(|t| !t.contains(&file.path));
    }
    Ok(files)
}

/// The paths marked resolved during the operation in progress: git's resolve-undo record (`git add` / `git rm` of
/// an unmerged path, here or in a terminal), less what is unmerged again. A commit keeps the record; the next merge,
/// checkout or reset drops it, so ask only while [`in_progress`] says something is.
pub fn marked(root: &Path) -> Result<Vec<String>, String> {
    let out = git::run(root, &["ls-files", "--resolve-undo", "-z"])?;
    let unmerged = git::run(root, &["ls-files", "-u", "-z"])?;
    // <mode> <sha> <stage>\t<path>
    let path = |e: &str| e.split_once('\t').map(|(_, path)| path.to_owned());
    let unmerged: HashSet<String> = git::path_text(&unmerged)?.split('\0').filter_map(path).collect();
    let mut paths: Vec<String> = git::path_text(&out)?.split('\0').filter_map(path).filter(|p| !unmerged.contains(p)).collect();
    paths.dedup();
    Ok(paths)
}

/// Stage `n` (1 Base, 2 Ours, 3 Theirs) of `path` in working-tree form: smudged (LFS, `working-tree-encoding`, CRLF).
// ponytail: one process per read; through a long-lived `cat-file --batch --filters` (`<object> <path>` lines) if
// reading many versions shows.
pub fn read_stage(root: &Path, path: &str, n: u8) -> Result<Vec<u8>, String> {
    git::run(root, &["cat-file", "--filters", &format!(":{n}:{path}")])
}

/// Does `path` (from `root`, or absolute, e.g. mergetool's `$MERGED`) have a Base, stage 1? Not when added on both
/// sides — never guess that from mergetool's empty `$BASE`.
pub fn has_base(root: &Path, path: &str) -> Result<bool, String> {
    let out = git::run(root, &["--literal-pathspecs", "ls-files", "-u", "-z", "--", path])?;
    // <mode> <sha> <stage>\t<path>
    Ok(out.split(|&b| b == 0).any(|e| e.split(|&b| b == b'\t').next().is_some_and(|h| h.ends_with(b" 1"))))
}

/// `merge.conflictStyle`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Style {
    Merge,
    Diff3,
    Zdiff3,
}

impl Style {
    pub fn name(self) -> &'static str {
        match self {
            Style::Merge => "merge",
            Style::Diff3 => "diff3",
            Style::Zdiff3 => "zdiff3",
        }
    }
}

/// The user's `merge.conflictStyle`; `merge` when unset or unknown.
pub fn conflict_style(root: &Path) -> Style {
    match git::run(root, &["config", "merge.conflictStyle"]).map(|v| String::from_utf8_lossy(&v).trim().to_string()) {
        Ok(v) if v == "diff3" => Style::Diff3,
        Ok(v) if v == "zdiff3" => Style::Zdiff3,
        _ => Style::Merge,
    }
}

/// The `conflict-marker-size` attribute of `path`; 7 when unset.
pub fn marker_size(root: &Path, path: &str) -> usize {
    let out = git::run(root, &["check-attr", "-z", "conflict-marker-size", "--", path]).unwrap_or_default();
    // <path>\0<attr>\0<value>\0
    let value = out.split(|&b| b == 0).nth(2).map(String::from_utf8_lossy);
    value.and_then(|v| v.parse().ok()).filter(|&n| n > 0).unwrap_or(7)
}

/// git's own marker output for `file`, rebuilt from its stages without touching the working tree, labelled
/// `[ours, base, theirs]`; in index form (no smudge filters). With the number of Conflicts.
#[allow(dead_code)] // for resuming a part-resolved file (ticket 06)
pub fn merge_file(root: &Path, file: &ConflictedFile, labels: [&str; 3], style: Style, marker_size: usize) -> Result<(Vec<u8>, usize), String> {
    merge_file_with(root, file, labels, style, marker_size, git::version().is_some_and(|v| v >= (2, 43)))
}

#[allow(dead_code)] // see `merge_file`
fn merge_file_with(
    root: &Path,
    file: &ConflictedFile,
    [ours, base, theirs]: [&str; 3],
    style: Style,
    marker_size: usize,
    object_ids: bool,
) -> Result<(Vec<u8>, usize), String> {
    let sha = |i: usize| file.stages[i].as_ref().map(|s| s.sha.as_str());
    let mut cmd = merge_file_command(root, [ours, base, theirs], style, marker_size)?;
    let mut temps = Vec::new();
    match [sha(1), sha(0), sha(2)] {
        // `--object-id` needs git 2.43, and all three objects
        [Some(o), Some(b), Some(t)] if object_ids => _ = cmd.args(["--object-id", o, b, t]),
        shas => {
            // a missing stage merges as empty, as git does for add/add
            for sha in shas {
                let bytes = sha.map_or(Ok(Vec::new()), |sha| git::run(root, &["cat-file", "blob", sha]))?;
                temps.push(merge_temp(&bytes).map_err(|e| format!("lenz: merge temp: {e}"))?);
            }
            cmd.args(temps.iter().map(|temp| temp.path()));
        }
    }
    merge_file_output(cmd)
}

fn merge_temp(bytes: &[u8]) -> std::io::Result<tempfile::NamedTempFile> {
    let mut temp = tempfile::Builder::new().prefix("lenz-merge-").tempfile()?;
    temp.write_all(bytes)?;
    Ok(temp)
}

/// git's own marker output for three files as they are: `git mergetool`'s temp files, in working-tree form like the
/// `$MERGED` git wrote. Labelled `[ours, base, theirs]`; with the number of Conflicts.
pub fn merge_files(cwd: &Path, [ours, base, theirs]: [&Path; 3], labels: [&str; 3], style: Style, marker_size: usize) -> Result<(Vec<u8>, usize), String> {
    let mut cmd = merge_file_command(cwd, labels, style, marker_size)?;
    cmd.arg("--").args([ours, base, theirs]);
    merge_file_output(cmd)
}

fn merge_file_command(cwd: &Path, [ours, base, theirs]: [&str; 3], style: Style, marker_size: usize) -> Result<std::process::Command, String> {
    let mut cmd = git::command(cwd)?;
    // merge-file reads `merge.conflictStyle` itself; the style asked for wins.
    cmd.args(["-c", &format!("merge.conflictStyle={}", style.name()), "merge-file", "-p"]);
    cmd.arg(format!("--marker-size={marker_size}"));
    match style {
        Style::Merge => {}
        Style::Diff3 => _ = cmd.arg("--diff3"),
        Style::Zdiff3 => _ = cmd.arg("--zdiff3"),
    }
    cmd.args(["-L", ours, "-L", base, "-L", theirs]);
    Ok(cmd)
}

fn merge_file_output(mut cmd: std::process::Command) -> Result<(Vec<u8>, usize), String> {
    let out = cmd.output().map_err(|e| format!("lenz: git merge-file: {e}"))?;
    // the exit code counts the Conflicts (capped at 127); an error is negative, i.e. 255
    match out.status.code() {
        Some(n @ 0..=127) => Ok((out.stdout, n as usize)),
        _ => Err(String::from_utf8_lossy(&out.stderr).trim_end().to_string()),
    }
}

/// **Mark resolved**: stage the working-tree file (`git add`), or the deletion (`git rm`) for a Delete.
pub fn mark_resolved(root: &Path, path: &str, delete: bool) -> Result<(), String> {
    let verb: &[&str] = if delete { &["rm", "-q"] } else { &["add"] };
    git::run(root, &[&["--literal-pathspecs"], verb, &["--", path]].concat()).map(drop)
}

/// **Re-open**, until the commit: put the conflict back in the index and git's markers back in the file
/// (`checkout --conflict`). A one-sided conflict has no markers to write: only its stages come back.
pub fn reopen(root: &Path, path: &str, style: Style) -> Result<(), String> {
    let conflict = format!("--conflict={}", style.name());
    let Err(e) = git::run(root, &["--literal-pathspecs", "checkout", &conflict, "--", path]) else { return Ok(()) };
    // `--unresolve` succeeds silently when the index has nothing to restore
    git::run(root, &["update-index", "--unresolve", "--", path]).map_err(|_| e.clone())?;
    let restored = !git::run(root, &["--literal-pathspecs", "ls-files", "-u", "--", path])?.is_empty();
    if restored { Ok(()) } else { Err(e) }
}

/// How many lines of `text` are conflict markers by `git diff --check`'s rule: `<`, `=`, `>` or `|` exactly `size`
/// times, then whitespace — the line's newline counts, so a marker on an unterminated last line doesn't.
pub fn marker_lines(text: &[u8], size: usize) -> usize {
    text.split_inclusive(|&b| b == b'\n')
        .filter(|line| {
            let c = line[0];
            matches!(c, b'<' | b'=' | b'>' | b'|')
                && line.len() > size
                && line[..size].iter().all(|&b| b == c)
                && line[size].is_ascii_whitespace()
        })
        .count()
}

/// `git rev-parse --git-path` for each name, joined to `root`: right in linked worktrees too.
fn git_paths<const N: usize>(root: &Path, names: [&str; N]) -> Result<[PathBuf; N], String> {
    let mut args = vec!["rev-parse"];
    for name in names {
        args.extend(["--git-path", name]);
    }
    let bytes = git::run(root, &args)?;
    let out = git::path_text(&bytes)?;
    let paths: Vec<PathBuf> = out.strip_suffix('\n').unwrap_or(out).split('\n').map(|l| root.join(l)).collect();
    paths.try_into().map_err(|_| format!("lenz: unexpected git rev-parse output {out:?}"))
}

/// A state file's contents, trimmed; `None` when absent.
fn read(path: &Path) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

/// The current branch, or the short sha when detached.
fn head_name(root: &Path) -> String {
    let name = |args: &[&str]| git::run(root, args).ok().map(|o| String::from_utf8_lossy(&o).trim().to_string());
    name(&["symbolic-ref", "--short", "-q", "HEAD"])
        .or_else(|| name(&["rev-parse", "--short", "HEAD"]))
        .unwrap_or_else(|| "HEAD".into())
}

/// `76c12fd (topic: edit two)`, as git labels a picked commit; `rev` itself when it can't be read.
fn describe(root: &Path, rev: &str) -> String {
    let out = git::run(root, &["log", "-1", "--no-show-signature", "--format=%h (%s)", rev, "--"]);
    out.map(|o| String::from_utf8_lossy(&o).trim().to_string()).unwrap_or_else(|_| rev.to_string())
}

/// What a rebase replays onto: a branch pointing at `<dir>/onto`, else its short sha.
fn onto(root: &Path, dir: &Path) -> String {
    let Some(sha) = read(&dir.join("onto")) else { return "HEAD".into() };
    let refs = git::run(root, &["for-each-ref", "--points-at", &sha, "--format=%(refname:short)", "refs/heads", "refs/remotes"]);
    let name = refs.ok().and_then(|o| String::from_utf8_lossy(&o).lines().next().map(str::to_string));
    name.unwrap_or_else(|| sha[..7.min(sha.len())].to_string())
}

/// Each file's `merge` attribute, when specified.
fn merge_attrs(root: &Path, files: &[ConflictedFile]) -> Result<HashMap<String, String>, String> {
    let input: Vec<u8> = files.iter().flat_map(|f| f.path.bytes().chain([0])).collect();
    let out = git::run_with_input(root, &["check-attr", "-z", "--stdin", "merge"], input)?;
    let fields: Vec<&[u8]> = out.split(|&b| b == 0).collect();
    let mut attrs = HashMap::new();
    for [path, _, value] in fields.as_chunks().0 {
        let path = git::path_text(path)?;
        let value = String::from_utf8_lossy(value);
        if value != "unspecified" {
            attrs.insert(path.to_owned(), value.into_owned());
        }
    }
    Ok(attrs)
}

/// The paths in `MERGE_RR` (`<hash>\t<path>\0` records): the conflicts rerere tracks and has no resolution for;
/// `None` when rerere didn't run.
fn rerere_tracked(root: &Path) -> Result<Option<HashSet<String>>, String> {
    let [merge_rr] = git_paths(root, ["MERGE_RR"])?;
    let Ok(bytes) = fs::read(merge_rr) else { return Ok(None) };
    let records = git::path_text(&bytes)?.split('\0').filter_map(|r| r.split_once('\t').map(|(_, path)| path));
    Ok(Some(records.map(str::to_owned).collect()))
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::symlink;

    use super::*;
    use crate::git::testing::{commit_all, git, repo, try_git, write};

    /// `base` committed on main, then `theirs` on branch topic and `ours` on main, each as one commit.
    fn diverged(base: impl Fn(&Path), ours: impl Fn(&Path), theirs: impl Fn(&Path)) -> tempfile::TempDir {
        let dir = repo();
        let d = dir.path();
        base(d);
        commit_all(d, "base");
        git(d, &["checkout", "-qb", "topic"]);
        theirs(d);
        commit_all(d, "topic: edit");
        git(d, &["checkout", "-q", "main"]);
        ours(d);
        commit_all(d, "main: edit");
        dir
    }

    /// `f` edited on both branches: one text conflict.
    fn text_conflict() -> tempfile::TempDir {
        diverged(|d| write(d, "f", "a\nb\nc\n"), |d| write(d, "f", "a\nmain\nc\n"), |d| write(d, "f", "a\ntopic\nc\n"))
    }

    fn sha(d: &Path, rev: &str) -> String {
        git(d, &["rev-parse", rev]).trim().to_string()
    }

    /// `label` is `<short sha of rev> (<subject>)`; the abbreviation length is the user's.
    fn assert_describes(label: &str, d: &Path, rev: &str, subject: &str) {
        let (short, rest) = label.split_once(' ').unwrap();
        assert!(sha(d, rev).starts_with(short) && short.len() >= 7, "{label}");
        assert_eq!(rest, format!("({subject})"));
    }

    fn file<'a>(files: &'a [ConflictedFile], path: &str) -> &'a ConflictedFile {
        files.iter().find(|f| f.path == path).unwrap_or_else(|| panic!("{path} not in {files:#?}"))
    }

    #[test]
    fn nothing_in_progress() {
        let dir = text_conflict();
        assert_eq!(in_progress(dir.path()), Ok(None));
        assert_eq!(list(dir.path()), Ok(vec![]));
    }

    #[test]
    fn merge_labels_ours_by_branch_and_theirs_from_merge_msg() {
        let dir = text_conflict();
        let d = dir.path();
        assert!(!try_git(d, &["merge", "topic"]));
        let p = in_progress(d).unwrap().unwrap();
        assert_eq!((p.operation, p.ours.as_str(), p.theirs.as_str(), p.step), (Operation::Merge, "main", "topic", None));
        assert_eq!(p.operation.continue_command(), Some("git merge --continue"));
    }

    #[test]
    fn merge_of_a_commit_labels_theirs_by_its_sha() {
        let dir = text_conflict();
        let d = dir.path();
        let topic = sha(d, "topic");
        assert!(!try_git(d, &["merge", &topic]));
        let theirs = in_progress(d).unwrap().unwrap().theirs;
        assert!(topic.starts_with(&theirs), "{theirs}"); // "Merge commit '<sha>'"
    }

    #[test]
    fn rebase_swaps_the_sides_and_counts_steps() {
        let dir = text_conflict();
        let d = dir.path();
        git(d, &["checkout", "-q", "topic"]);
        write(d, "g", "later\n");
        commit_all(d, "topic: later");
        assert!(!try_git(d, &["rebase", "main"]));
        let p = in_progress(d).unwrap().unwrap();
        assert_eq!((p.operation, p.ours.as_str(), p.step), (Operation::Rebase, "main", Some((1, 2))));
        assert_describes(&p.theirs, d, "topic~1", "topic: edit");
        assert_eq!(p.operation.continue_command(), Some("git rebase --continue"));
    }

    #[test]
    fn rebase_apply_backend() {
        let dir = text_conflict();
        let d = dir.path();
        git(d, &["checkout", "-q", "topic"]);
        assert!(!try_git(d, &["rebase", "--apply", "main"]));
        let p = in_progress(d).unwrap().unwrap();
        assert_eq!((p.operation, p.ours.as_str(), p.step), (Operation::Rebase, "main", Some((1, 1))));
        assert_describes(&p.theirs, d, "topic", "topic: edit");
    }

    #[test]
    fn am_labels_theirs_by_the_patch_subject() {
        let dir = text_conflict();
        let d = dir.path();
        write(d, "topic.patch", git(d, &["format-patch", "-1", "--stdout", "topic"]));
        assert!(!try_git(d, &["am", "-3", "topic.patch"]));
        let p = in_progress(d).unwrap().unwrap();
        assert_eq!((p.operation, p.ours.as_str(), p.theirs.as_str(), p.step), (Operation::Am, "main", "topic: edit", Some((1, 1))));
        assert_eq!(p.operation.continue_command(), Some("git am --continue"));
    }

    #[test]
    fn cherry_pick_labels_theirs_by_the_picked_commit() {
        let dir = text_conflict();
        let d = dir.path();
        assert!(!try_git(d, &["cherry-pick", "topic"]));
        let p = in_progress(d).unwrap().unwrap();
        assert_eq!((p.operation, p.ours.as_str()), (Operation::CherryPick, "main"));
        assert_describes(&p.theirs, d, "topic", "topic: edit");
        assert_eq!(p.operation.continue_command(), Some("git cherry-pick --continue"));
    }

    #[test]
    fn a_multi_pick_committed_by_hand_still_names_the_stopped_pick() {
        let dir = text_conflict();
        let d = dir.path();
        git(d, &["checkout", "-q", "topic"]);
        write(d, "g", "later\n");
        commit_all(d, "topic: later");
        git(d, &["checkout", "-q", "main"]);
        assert!(!try_git(d, &["cherry-pick", "topic~1", "topic"]));
        write(d, "f", "resolved\n");
        commit_all(d, "resolved"); // a plain commit leaves the sequencer waiting for --continue
        let p = in_progress(d).unwrap().unwrap();
        assert_eq!(p.operation, Operation::CherryPick);
        assert_describes(&p.theirs, d, "topic~1", "topic: edit");
    }

    #[test]
    fn revert_labels_theirs_as_the_parent_of_the_reverted_commit() {
        let dir = repo();
        let d = dir.path();
        write(d, "f", "a\n");
        commit_all(d, "one");
        write(d, "f", "b\n");
        commit_all(d, "two");
        write(d, "f", "c\n");
        commit_all(d, "three");
        assert!(!try_git(d, &["revert", "--no-edit", "HEAD~1"]));
        let p = in_progress(d).unwrap().unwrap();
        assert_eq!((p.operation, p.ours.as_str()), (Operation::Revert, "main"));
        let parent_of = p.theirs.strip_prefix("parent of ").unwrap();
        assert_describes(parent_of, d, "HEAD~1", "two");
        assert_eq!(p.operation.continue_command(), Some("git revert --continue"));
    }

    #[test]
    fn stash_pop_has_no_state_file_and_no_continue() {
        let dir = repo();
        let d = dir.path();
        write(d, "f", "a\n");
        commit_all(d, "one");
        write(d, "f", "stashed\n");
        git(d, &["stash", "-q"]);
        write(d, "f", "upstream\n");
        commit_all(d, "two");
        assert!(!try_git(d, &["stash", "pop", "-q"]));
        let p = in_progress(d).unwrap().unwrap();
        assert_eq!(
            (p.operation, p.ours.as_str(), p.theirs.as_str(), p.step),
            (Operation::StashPop, "Updated upstream", "Stashed changes", None)
        );
        assert_eq!(p.operation.continue_command(), None);
        assert_eq!(list(d).unwrap().len(), 1);
    }

    #[test]
    fn kinds_by_stage_shape_and_attributes() {
        let bin = |tag: &str| format!("\0bin {tag}");
        let dir = diverged(
            |d| {
                write(d, ".gitattributes", "nomerge.txt -merge\n*.crlf text eol=crlf\n");
                for f in ["text.txt", "nomerge.txt", "del-us.txt", "del-them.txt", "type.txt", "w.crlf"] {
                    write(d, f, "a\nb\nc\n");
                }
                write(d, "bin.dat", bin("base"));
                symlink("target-base", d.join("link")).unwrap();
            },
            |d| {
                for f in ["text.txt", "nomerge.txt", "del-them.txt", "type.txt", "w.crlf"] {
                    write(d, f, "a\nmain\nc\n");
                }
                write(d, "bin.dat", bin("main"));
                write(d, "added.txt", "main\n");
                fs::remove_file(d.join("del-us.txt")).unwrap();
                fs::remove_file(d.join("link")).unwrap();
                symlink("target-main", d.join("link")).unwrap();
            },
            |d| {
                for f in ["text.txt", "nomerge.txt", "del-us.txt", "w.crlf"] {
                    write(d, f, "a\ntopic\nc\n");
                }
                write(d, "bin.dat", bin("topic"));
                write(d, "added.txt", "topic\n");
                fs::remove_file(d.join("del-them.txt")).unwrap();
                fs::remove_file(d.join("type.txt")).unwrap();
                symlink("elsewhere", d.join("type.txt")).unwrap();
                fs::remove_file(d.join("link")).unwrap();
                symlink("target-topic", d.join("link")).unwrap();
            },
        );
        let d = dir.path();
        assert!(!try_git(d, &["merge", "topic"]));
        let files = list(d).unwrap();
        let kind = |path| {
            let f = file(&files, path);
            (f.kind, f.xy.as_str(), f.wording())
        };
        assert_eq!(kind("text.txt"), (Kind::Text, "UU", "both modified"));
        assert_eq!(kind("w.crlf"), (Kind::Text, "UU", "both modified"));
        assert_eq!(kind("bin.dat"), (Kind::Binary, "UU", "both modified"));
        assert_eq!(kind("nomerge.txt"), (Kind::Binary, "UU", "both modified"));
        assert_eq!(kind("added.txt"), (Kind::AddedByBoth, "AA", "both added"));
        assert_eq!(kind("del-us.txt"), (Kind::DeletedBy(Side::Us), "DU", "deleted by us"));
        assert_eq!(kind("del-them.txt"), (Kind::DeletedBy(Side::Them), "UD", "deleted by them"));
        assert_eq!(kind("link"), (Kind::Symlink, "UU", "both modified"));
        // file vs symlink: git keeps our file aside as `type.txt~HEAD`
        assert_eq!(kind("type.txt"), (Kind::DeletedBy(Side::Us), "UA", "added by them"));
        assert_eq!(kind("type.txt~HEAD"), (Kind::DeletedBy(Side::Them), "UD", "deleted by them"));
        assert_eq!(file(&files, "type.txt").stages[2].as_ref().map(|s| s.mode), Some(0o120000));
        assert!(files.iter().all(|f| !f.rerere));

        let text = file(&files, "text.txt");
        let stage = |i: usize| text.stages[i].as_ref().map(|s| (s.mode, s.size));
        assert_eq!([stage(0), stage(1), stage(2)], [Some((0o100644, Some(6))), Some((0o100644, Some(9))), Some((0o100644, Some(10)))]);
        assert_eq!(file(&files, "added.txt").stages[0], None);
        assert_eq!(file(&files, "del-them.txt").stages[2], None);
        assert_eq!(file(&files, "link").stages[1].as_ref().map(|s| s.mode), Some(0o120000));

        // versions in working-tree form
        assert_eq!(read_stage(d, "text.txt", 3).unwrap(), b"a\ntopic\nc\n");
        assert_eq!(read_stage(d, "w.crlf", 2).unwrap(), b"a\r\nmain\r\nc\r\n");
        assert_eq!(read_stage(d, "link", 2).unwrap(), b"target-main");
        assert!(read_stage(d, "added.txt", 1).is_err());

        // mergetool's no-base check
        assert_eq!(has_base(d, "text.txt"), Ok(true));
        assert_eq!(has_base(d, "added.txt"), Ok(false));
        assert_eq!(has_base(d, d.join("text.txt").to_str().unwrap()), Ok(true));
    }

    #[test]
    fn huge_binary_stages_are_sized_without_being_read_whole() {
        let big = |tag: &[u8]| {
            let mut bytes = vec![0u8; crate::document::LARGE as usize + 1];
            bytes[..tag.len()].copy_from_slice(tag);
            bytes
        };
        let dir = diverged(|d| write(d, "big.bin", big(b"\0base")), |d| write(d, "big.bin", big(b"\0main")), |d| write(d, "big.bin", big(b"\0topic")));
        assert!(!try_git(dir.path(), &["merge", "topic"]));
        let files = list(dir.path()).unwrap();
        assert_eq!(files.len(), 1);
        assert!(files[0].path == "big.bin" && matches!(files[0].kind, Kind::Binary));
        for stage in files[0].stages.iter().flatten() {
            assert_eq!(stage.size, Some(crate::document::LARGE + 1));
        }
    }

    #[test]
    fn submodule_conflict() {
        let sub = repo();
        let s = sub.path();
        write(s, "s", "1\n");
        commit_all(s, "s1");
        git(s, &["checkout", "-qb", "x"]);
        write(s, "s", "x\n");
        commit_all(s, "sx");
        git(s, &["checkout", "-q", "main"]);
        write(s, "s", "main\n");
        commit_all(s, "smain");
        let (s1, sx, smain) = (sha(s, "main~1"), sha(s, "x"), sha(s, "main"));

        let dir = repo();
        let d = dir.path();
        git(d, &["-c", "protocol.file.allow=always", "submodule", "add", "-q", s.to_str().unwrap(), "sub"]);
        git(&d.join("sub"), &["checkout", "-q", &s1]);
        commit_all(d, "add sub");
        git(d, &["checkout", "-qb", "topic"]);
        git(&d.join("sub"), &["checkout", "-q", &sx]);
        commit_all(d, "topic: sub x");
        git(d, &["checkout", "-q", "main"]);
        git(&d.join("sub"), &["checkout", "-q", &smain]);
        commit_all(d, "main: sub main");
        assert!(!try_git(d, &["merge", "topic"]));

        let files = list(d).unwrap();
        let f = file(&files, "sub");
        assert_eq!((f.kind, f.xy.as_str()), (Kind::Submodule, "UU"));
        let stage = |i: usize| f.stages[i].as_ref().map(|s| (s.mode, s.sha.clone(), s.size));
        assert_eq!([stage(1), stage(2)], [Some((0o160000, smain, None)), Some((0o160000, sx, None))]);
    }

    #[test]
    fn mark_resolved_and_reopen() {
        let dir = text_conflict();
        let d = dir.path();
        assert!(!try_git(d, &["merge", "topic"]));
        assert_eq!(marked(d), Ok(vec![]));
        write(d, "f", "a\nboth\nc\n");
        mark_resolved(d, "f", false).unwrap();
        assert_eq!(list(d), Ok(vec![]));
        assert_eq!(marked(d), Ok(vec!["f".to_string()]));

        reopen(d, "f", Style::Diff3).unwrap();
        let files = list(d).unwrap();
        assert_eq!((files.len(), files[0].kind), (1, Kind::Text));
        assert_eq!(marker_lines(&fs::read(d.join("f")).unwrap(), 7), 4); // <<< ||| === >>>
        assert_eq!(marked(d), Ok(vec![]));
    }

    #[test]
    fn marked_files_are_forgotten_by_the_next_merge() {
        let dir = text_conflict();
        let d = dir.path();
        git(d, &["branch", "later", "topic"]);
        git(d, &["checkout", "-q", "later"]);
        write(d, "g", "later\n");
        commit_all(d, "later");
        git(d, &["checkout", "-q", "main"]);
        write(d, "g", "main\n");
        commit_all(d, "main: g");
        assert!(!try_git(d, &["merge", "topic"]));
        write(d, "f", "a\nboth\nc\n");
        mark_resolved(d, "f", false).unwrap();
        git(d, &["commit", "-q", "--no-edit"]);
        // the record outlives the commit…
        assert_eq!(marked(d), Ok(vec!["f".to_string()]));
        // …but not the next merge
        assert!(!try_git(d, &["merge", "later"]));
        assert_eq!(marked(d), Ok(vec![]));
    }

    #[test]
    fn mark_resolved_as_a_delete_and_reopen() {
        let dir = diverged(|d| write(d, "f", "a\n"), |d| write(d, "f", "main\n"), |d| fs::remove_file(d.join("f")).unwrap());
        let d = dir.path();
        assert!(!try_git(d, &["merge", "topic"]));
        let before = list(d).unwrap();
        mark_resolved(d, "f", true).unwrap();
        assert_eq!(list(d), Ok(vec![]));
        assert!(!d.join("f").exists());

        reopen(d, "f", Style::Merge).unwrap();
        assert_eq!(list(d), Ok(before));
        // nothing to undo
        assert!(reopen(d, "nope", Style::Merge).is_err());
    }

    #[test]
    fn reopen_a_symlink_conflict() {
        let link = |target: &'static str| {
            move |d: &Path| {
                let _ = fs::remove_file(d.join("l"));
                symlink(target, d.join("l")).unwrap();
            }
        };
        let dir = diverged(link("base"), link("main"), link("topic"));
        let d = dir.path();
        assert!(!try_git(d, &["merge", "topic"]));
        let before = list(d).unwrap();
        mark_resolved(d, "l", false).unwrap();
        assert_eq!(list(d), Ok(vec![]));
        reopen(d, "l", Style::Merge).unwrap();
        assert_eq!(list(d), Ok(before));
    }

    // APFS refuses non-UTF-8 file names (EILSEQ), so the fixture can't exist on macOS.
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn non_utf8_delete_conflict_cannot_target_a_unicode_sibling() {
        use std::os::unix::ffi::OsStringExt;
        let dir = repo();
        let invalid = dir.path().join(std::ffi::OsString::from_vec(b"f\xff".to_vec()));
        let valid = dir.path().join("f�");
        fs::write(&invalid, "base\n").unwrap();
        fs::write(&valid, "unrelated file\n").unwrap();
        commit_all(dir.path(), "base");
        git(dir.path(), &["checkout", "-qb", "topic"]);
        fs::write(&invalid, "theirs\n").unwrap();
        commit_all(dir.path(), "modify invalid path");
        git(dir.path(), &["checkout", "-q", "main"]);
        fs::remove_file(&invalid).unwrap();
        commit_all(dir.path(), "delete invalid path");
        assert!(!try_git(dir.path(), &["merge", "topic"]));
        // The old lossy loader exposed Delete for `f�`, removing the unrelated file.
        assert!(list(dir.path()).unwrap_err().contains("non-UTF-8 file paths are unsupported"));
        assert!(marked(dir.path()).is_err());
        assert!(crate::conflict_session::read(dir.path(), &[]).is_err());
        assert_eq!(fs::read_to_string(&valid).unwrap(), "unrelated file\n");
        assert_eq!(fs::read_to_string(&invalid).unwrap(), "theirs\n");
    }

    #[test]
    fn paths_are_literal_and_may_have_spaces() {
        let names = ["a b.txt", "a*.txt"];
        let edit = |text: &'static str| move |d: &Path| names.iter().for_each(|f| write(d, f, text));
        let dir = diverged(edit("a\n"), edit("main\n"), edit("topic\n"));
        let d = dir.path();
        assert!(!try_git(d, &["merge", "topic"]));
        mark_resolved(d, "a*.txt", false).unwrap();
        let files = list(d).unwrap();
        assert_eq!(files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), ["a b.txt"]);
        assert_eq!(has_base(d, "a*.txt"), Ok(false)); // resolved, and the glob doesn't reach `a b.txt`
        assert_eq!(has_base(d, "a b.txt"), Ok(true));
    }

    #[test]
    fn rerere_tags_paths_it_has_a_resolution_for() {
        let dir = text_conflict();
        let d = dir.path();
        git(d, &["config", "rerere.enabled", "true"]);
        assert!(!try_git(d, &["merge", "topic"]));
        assert!(!list(d).unwrap()[0].rerere, "tracked but not yet resolved");
        write(d, "f", "a\nboth\nc\n");
        commit_all(d, "merged");

        git(d, &["reset", "-q", "--hard", "HEAD~1"]);
        assert!(!try_git(d, &["merge", "topic"]));
        assert!(list(d).unwrap()[0].rerere);
        assert_eq!(fs::read_to_string(d.join("f")).unwrap(), "a\nboth\nc\n"); // replayed, still unmerged
    }

    #[test]
    fn merge_inputs_are_private_and_cleaned_up() {
        use std::os::unix::fs::PermissionsExt;
        let temp = merge_temp(b"private merge contents").unwrap();
        let path = temp.path().to_path_buf();
        assert_eq!(fs::read(&path).unwrap(), b"private merge contents");
        assert_eq!(temp.as_file().metadata().unwrap().permissions().mode() & 0o777, 0o600);
        let second = merge_temp(b"other merge").unwrap();
        assert_ne!(temp.path(), second.path());
        drop(temp);
        assert!(!path.exists());
    }

    #[test]
    fn merge_file_rebuilds_gits_markers() {
        let dir = text_conflict();
        let d = dir.path();
        assert!(!try_git(d, &["merge", "topic"]));
        let files = list(d).unwrap();
        let worktree = String::from_utf8(fs::read(d.join("f")).unwrap()).unwrap();
        for object_ids in [true, false] {
            let (text, n) = merge_file_with(d, &files[0], ["HEAD", "base", "topic"], Style::Merge, 7, object_ids).unwrap();
            assert_eq!((String::from_utf8(text).unwrap(), n), (worktree.clone(), 1));
        }
        let (text, _) = merge_file(d, &files[0], ["ours", "base", "theirs"], Style::Diff3, 9).unwrap();
        assert_eq!(text, b"a\n<<<<<<<<< ours\nmain\n||||||||| base\nb\n=========\ntopic\n>>>>>>>>> theirs\nc\n");
    }

    #[test]
    fn merge_file_without_a_base() {
        let dir = diverged(|d| write(d, "g", "g\n"), |d| write(d, "f", "x\nmain\n"), |d| write(d, "f", "x\ntopic\n"));
        let d = dir.path();
        assert!(!try_git(d, &["merge", "topic"]));
        let files = list(d).unwrap();
        let (text, n) = merge_file(d, &files[0], ["o", "b", "t"], Style::Zdiff3, 7).unwrap();
        assert_eq!((String::from_utf8(text).unwrap().as_str(), n), ("x\n<<<<<<< o\nmain\n||||||| b\n=======\ntopic\n>>>>>>> t\n", 1));
    }

    #[test]
    fn style_and_marker_size_from_config_and_attributes() {
        let dir = repo();
        let d = dir.path();
        for (value, style) in [("diff3", Style::Diff3), ("zdiff3", Style::Zdiff3), ("merge", Style::Merge)] {
            git(d, &["config", "merge.conflictStyle", value]);
            assert_eq!(conflict_style(d), style);
        }
        write(d, ".gitattributes", "*.md conflict-marker-size=12\n");
        assert_eq!((marker_size(d, "a.md"), marker_size(d, "a.txt")), (12, 7));
    }

    #[test]
    fn marker_scan_follows_git_diff_check() {
        let text = b"<<<<<<< ours\nx\n|||||||\n=======\n>>>>>>> theirs\n";
        assert_eq!(marker_lines(text, 7), 4);
        assert_eq!(marker_lines(text, 8), 0);
        // too long, no space after, not at the line start, unterminated last line
        assert_eq!(marker_lines(b"========\n<<<<<<<x\n <<<<<<< a\n=======", 7), 0);
        assert_eq!(marker_lines(b"=======\r\n=======\tx\n", 7), 2);
        assert_eq!(marker_lines(b"", 7), 0);
    }
}
