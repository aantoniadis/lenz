//! Every `git` process Lenz starts goes through here, after a once-per-run check that git is usable.

use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    sync::{Arc, Mutex, OnceLock, PoisonError},
};

const MIN_VERSION: (u32, u32) = (2, 30);

/// A `git` command running in `cwd`, or why git can't be used.
pub fn command(cwd: &Path) -> Result<Command, String> {
    usable().clone()?;
    let mut cmd = Command::new("git");
    // `status` must not take `index.lock` behind the user's own git. An argument, not GIT_OPTIONAL_LOCKS: any
    // env override makes std copy the whole environment on every spawn (~70 µs with a typical desktop env).
    cmd.current_dir(cwd).arg("--no-optional-locks");
    for setting in &untrusted(cwd).args {
        cmd.args(["-c", setting]);
    }
    Ok(cmd)
}

/// Programs a repository's own config (`.git/config`, its worktree config, and what they include) would have
/// git run on a read: textconv and filter drivers, and an fsmonitor hook. A repository can arrive with its `.git`
/// (an archive, a shared drive, someone else's checkout), and opening it in a viewer must not run its commands.
/// Drivers from the user's global or system config, and git's own fsmonitor daemon, are kept.
#[derive(Debug, Default, PartialEq)]
pub struct Untrusted {
    /// `name=value` overrides passed as `-c` to every git Lenz runs in that directory.
    args: Vec<String>,
    /// Drivers not run, by name, for the notes on the files that use them.
    pub textconv: Vec<String>,
    pub filters: Vec<String>,
}

fn seen() -> &'static Mutex<HashMap<PathBuf, Arc<Untrusted>>> {
    static SEEN: OnceLock<Mutex<HashMap<PathBuf, Arc<Untrusted>>>> = OnceLock::new();
    SEEN.get_or_init(Default::default)
}

/// Treats `cwd`'s repository config as the user's own, as a driver in their global config would be.
#[cfg(test)]
pub(crate) fn trust(cwd: &Path) {
    let mut seen = seen().lock().unwrap_or_else(PoisonError::into_inner);
    seen.insert(cwd.to_path_buf(), Arc::default());
    // git names the top level by its resolved path (macOS temp dirs sit behind the /var -> /private/var link)
    if let Ok(resolved) = cwd.canonicalize() {
        seen.insert(resolved, Arc::default());
    }
}

/// [`Untrusted`] for `cwd`, asked once per directory and process.
pub fn untrusted(cwd: &Path) -> Arc<Untrusted> {
    let seen = seen();
    if let Some(found) = seen.lock().unwrap_or_else(PoisonError::into_inner).get(cwd) {
        return found.clone();
    }
    let out = Command::new("git")
        .current_dir(cwd)
        .args(["--no-optional-locks", "config", "--show-scope", "-z", "--get-regexp"])
        .arg(r"^(core\.fsmonitor|filter\..+\.(clean|smudge|process)|diff\..+\.textconv)$")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output();
    let found = Arc::new(out.map(|o| untrusted_from(&o.stdout)).unwrap_or_default());
    seen.lock().unwrap_or_else(PoisonError::into_inner).insert(cwd.to_path_buf(), found.clone());
    found
}

/// Parses `git config --show-scope -z` output: `<scope>\0<key>\n<value>\0` per entry.
fn untrusted_from(out: &[u8]) -> Untrusted {
    let mut u = Untrusted::default();
    let mut fields = out.split(|&b| b == 0);
    while let (Some(scope), Some(entry)) = (fields.next(), fields.next()) {
        if !matches!(scope, b"local" | b"worktree") {
            continue;
        }
        let entry = String::from_utf8_lossy(entry);
        let (key, value) = entry.split_once('\n').unwrap_or((&entry, ""));
        if key.eq_ignore_ascii_case("core.fsmonitor") {
            // a boolean selects git's own daemon; anything else names a program to run
            let boolean = ["true", "false", "yes", "no", "on", "off", "1", "0", ""].contains(&value.to_ascii_lowercase().as_str());
            if !boolean {
                u.args.push("core.fsmonitor=false".into());
            }
        } else if let Some((name, _)) = key.strip_prefix("diff.").and_then(|k| k.rsplit_once('.')) {
            u.args.push(format!("{key}="));
            u.textconv.push(name.to_string());
        } else if let Some((name, _)) = key.strip_prefix("filter.").and_then(|k| k.rsplit_once('.')) {
            u.args.push(format!("{key}="));
            if !u.filters.iter().any(|f| f == name) {
                // an empty driver marked required would fail every read of its files
                u.args.push(format!("filter.{name}.required=false"));
                u.filters.push(name.to_string());
            }
        }
    }
    u
}

/// Objects over this many bytes are refused rather than read into memory: Sides are read only after the
/// `LARGE` check or on the user's say-so, and a corrupt size header must not become an allocation.
pub const MAX_OBJECT: usize = 1 << 30;

/// The installed git's (major, minor), or `None` when git isn't usable.
#[allow(dead_code)] // for `conflicts::merge_file`
pub fn version() -> Option<(u32, u32)> {
    usable().as_ref().ok().copied()
}

fn usable() -> &'static Result<(u32, u32), String> {
    static USABLE: OnceLock<Result<(u32, u32), String>> = OnceLock::new();
    USABLE.get_or_init(check)
}

/// Runs git in `cwd` and returns its stdout; a non-zero exit becomes git's stderr.
pub fn run(cwd: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    finish(command(cwd)?.args(args))
}

/// [`run`] for a command that may reach the network: it fails rather than waits on a credential prompt, which
/// the GUI has no terminal for and the TUI's raw-mode terminal would swallow.
pub fn run_network(cwd: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    finish(command(cwd)?.args(args).env("GIT_TERMINAL_PROMPT", "0").stdin(Stdio::null()))
}

fn finish(cmd: &mut Command) -> Result<Vec<u8>, String> {
    let out = cmd.output().map_err(|e| format!("lenz: git: {e}"))?;
    if out.status.success() {
        Ok(out.stdout)
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim_end().to_string())
    }
}

/// Operational paths must keep their exact identity. Display-only decoding may be
/// lossy, but a replacement character can name a different real file.
pub(crate) fn path_text(bytes: &[u8]) -> Result<&str, String> {
    std::str::from_utf8(bytes).map_err(|_| "lenz: non-UTF-8 file paths are unsupported".into())
}

/// [`run`] with `input` on git's stdin.
pub fn run_with_input(cwd: &Path, args: &[&str], input: Vec<u8>) -> Result<Vec<u8>, String> {
    let mut child = command(cwd)?
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("lenz: git: {e}"))?;
    let mut stdin = child.stdin.take().expect("piped stdin");
    // From a thread: git may fill its stdout pipe before it has read all of its input.
    let writer = std::thread::spawn(move || stdin.write_all(&input));
    let out = child.wait_with_output().map_err(|e| format!("lenz: git: {e}"))?;
    let _ = writer.join();
    if out.status.success() {
        Ok(out.stdout)
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim_end().to_string())
    }
}

/// Is `rev` a commit in the repository around `cwd`? False outside a repo or without a usable git.
pub fn is_commit(cwd: &Path, rev: &str) -> bool {
    let Ok(mut cmd) = command(cwd) else { return false };
    cmd.args(["rev-parse", "--verify", "--quiet", "--end-of-options", &format!("{rev}^{{commit}}")])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Does any of `revs` name a commit in the repository around `cwd`? One git process for all of them (each
/// costs ~20 ms on macOS, ahead of the first frame), and none outside a repository.
pub fn any_commit(cwd: &Path, revs: &[&str]) -> bool {
    if std::env::var_os("GIT_DIR").is_none() && !may_be_in_repository(cwd) {
        return false;
    }
    // a line per name: a name with a line break isn't a revision
    let input: String = revs.iter().filter(|r| !r.contains(['\n', '\r'])).map(|r| format!("{r}^{{commit}}\n")).collect();
    if input.is_empty() {
        return false;
    }
    // `%(objecttype)` prints only `commit` for a found name; a missing one echoes the name, so no name can pass
    run_with_input(cwd, &["cat-file", "--batch-check=%(objecttype)"], input.into_bytes())
        .is_ok_and(|out| out.split(|&b| b == b'\n').any(|line| line == b"commit"))
}

/// False only when no directory from `cwd` up holds a `.git` or is itself a bare repository.
fn may_be_in_repository(cwd: &Path) -> bool {
    cwd.ancestors().any(|dir| dir.join(".git").exists() || (dir.join("HEAD").is_file() && dir.join("objects").is_dir()))
}

fn check() -> Result<(u32, u32), String> {
    if cfg!(target_os = "macos") {
        check_macos_stub()?;
    }
    let out = Command::new("git").arg("--version").output().map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => "lenz: git not found; install git 2.30 or later".to_string(),
        _ => format!("lenz: cannot run git: {e}"),
    })?;
    let text = String::from_utf8_lossy(&out.stdout);
    match parse_version(&text) {
        Some(v) if v >= MIN_VERSION => Ok(v),
        Some((major, minor)) => Err(format!("lenz: git 2.30 or later is required (found {major}.{minor})")),
        None => Err(format!("lenz: cannot read the git version from {:?}", text.trim())),
    }
}

/// On macOS `/usr/bin/git` is a stub that pops up an install dialog unless the Command Line Tools exist.
fn check_macos_stub() -> Result<(), String> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let resolved = std::env::split_paths(&path).map(|d| d.join("git")).find(|p| p.is_file());
    if resolved.as_deref() != Some(Path::new("/usr/bin/git")) {
        return Ok(());
    }
    let hint = "lenz: git is not installed (macOS Command Line Tools missing); \
                run `xcode-select --install` or `brew install git`";
    let out = Command::new("xcode-select").arg("-p").output().map_err(|_| hint.to_string())?;
    let dev_dir = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
    if out.status.success() && dev_dir.join("usr/bin/git").is_file() { Ok(()) } else { Err(hint.into()) }
}

/// `git version 2.39.3 (Apple Git-146)` → (2, 39).
fn parse_version(text: &str) -> Option<(u32, u32)> {
    let mut nums = text.split_whitespace().nth(2)?.split('.').map(|n| n.parse().ok());
    Some((nums.next()??, nums.next()??))
}

/// One long-lived `git cat-file --batch` (object bytes) or `--batch-check` (sizes only): object name in.
pub struct CatFile {
    root: PathBuf,
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    /// An unread body is in the pipe: nothing more can be read in step. The owner respawns.
    broken: bool,
}

impl CatFile {
    /// `mode` is `--batch` for [`CatFile::get`], or `--batch-check` for [`CatFile::size`] alone.
    pub fn spawn(cwd: &Path, mode: &str) -> Result<CatFile, String> {
        let mut child = command(cwd)?
            .args(["cat-file", mode])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("lenz: git cat-file: {e}"))?;
        let stdin = child.stdin.take();
        let stdout = BufReader::new(child.stdout.take().expect("piped stdout"));
        Ok(CatFile { root: cwd.to_path_buf(), child, stdin, stdout, broken: false })
    }

    /// The object's size in bytes, or `None` if git says it is missing; under `--batch` its bytes follow.
    pub fn size(&mut self, name: &str) -> std::io::Result<Option<usize>> {
        if self.broken {
            return Err(std::io::Error::other("cat-file stream out of step after an oversized object"));
        }
        // The supported Git versions include ones without NUL-delimited batch input.
        // Resolve exceptional path names through argv so they cannot inject batch requests.
        let resolved;
        let name = if name.contains(['\n', '\r']) {
            let out = command(&self.root).map_err(std::io::Error::other)?
                .args(["rev-parse", "--verify", "--quiet", "--end-of-options", name]).output()?;
            if out.status.code() == Some(1) { return Ok(None); }
            if !out.status.success() { return Err(std::io::Error::other(String::from_utf8_lossy(&out.stderr).into_owned())); }
            resolved = String::from_utf8(out.stdout).map_err(std::io::Error::other)?;
            resolved.trim_end()
        } else { name };
        let stdin = self.stdin.as_mut().expect("stdin open until drop");
        writeln!(stdin, "{name}")?;
        stdin.flush()?;
        let mut header = String::new();
        self.stdout.read_line(&mut header)?;
        if header.ends_with(" missing\n") || header.ends_with(" ambiguous\n") {
            return Ok(None);
        }
        let size = header.split_whitespace().nth(2).and_then(|s| s.parse().ok());
        size.map(Some).ok_or_else(|| std::io::Error::other(format!("unexpected cat-file reply {header:?}")))
    }

    /// The object's size, and its bytes when they are at most `max`, else only its first `head` bytes (the rest is
    /// read past without being kept). `None` if git says it is missing.
    pub fn get_bounded(&mut self, name: &str, max: usize, head: usize) -> std::io::Result<Option<(usize, Vec<u8>)>> {
        let Some(size) = self.size(name)? else { return Ok(None) };
        let keep = if size <= max { size } else { head.min(size) };
        let mut bytes = vec![0; keep];
        self.stdout.read_exact(&mut bytes)?;
        // the rest of the body and its trailing LF, streamed, never held
        let rest = (size - keep) as u64 + 1;
        if std::io::copy(&mut (&mut self.stdout).take(rest), &mut std::io::sink())? != rest {
            self.broken = true;
            return Err(std::io::Error::other("cat-file reply ended early"));
        }
        Ok(Some((size, bytes)))
    }

    /// The object's bytes, or `None` if git says it is missing.
    pub fn get(&mut self, name: &str) -> std::io::Result<Option<Vec<u8>>> {
        let Some(size) = self.size(name)? else { return Ok(None) };
        if size > MAX_OBJECT {
            // The body is on its way and won't be read: stop git writing it, and this stream with it.
            self.broken = true;
            let _ = self.child.kill();
            return Err(std::io::Error::other(format!("{name} is {size} bytes, over the {} MiB limit", MAX_OBJECT >> 20)));
        }
        let mut bytes = vec![0; size + 1]; // + the trailing LF
        self.stdout.read_exact(&mut bytes)?;
        bytes.pop();
        Ok(Some(bytes))
    }
}

impl Drop for CatFile {
    fn drop(&mut self) {
        drop(self.stdin.take()); // EOF ends the batch
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_read_from_plain_and_vendor_output() {
        assert_eq!(parse_version("git version 2.55.0\n"), Some((2, 55)));
        assert_eq!(parse_version("git version 2.39.3 (Apple Git-146)"), Some((2, 39)));
        assert_eq!(parse_version("git version 2.30.2.windows.1"), Some((2, 30)));
        assert!(parse_version("git version 2.29.2").unwrap() < MIN_VERSION);
        assert_eq!(parse_version("nonsense"), None);
    }

    #[test]
    fn batch_paths_with_newlines_do_not_shift_following_replies() {
        let repo = testing::repo();
        let path = "line\nHEAD:normal";
        testing::write(repo.path(), path, b"newline path");
        testing::write(repo.path(), "normal", b"ordinary path");
        testing::commit_all(repo.path(), "files");
        let mut cat = CatFile::spawn(repo.path(), "--batch").unwrap();
        assert_eq!(cat.get(&format!("HEAD:{path}")).unwrap().unwrap(), b"newline path");
        assert_eq!(cat.get("HEAD:normal").unwrap().unwrap(), b"ordinary path");
        assert_eq!(cat.get("HEAD:missing\nHEAD:normal").unwrap(), None);
        assert_eq!(cat.get("HEAD:normal").unwrap().unwrap(), b"ordinary path");
        let mut sizes = CatFile::spawn(repo.path(), "--batch-check").unwrap();
        assert_eq!(sizes.size(&format!("HEAD:{path}")).unwrap(), Some(12));
        assert_eq!(sizes.size("HEAD:normal").unwrap(), Some(13));
    }

    #[test]
    fn any_commit_asks_git_once_for_every_name() {
        let dir = testing::repo();
        testing::write(dir.path(), "commit", "a file named like git's answer");
        testing::commit_all(dir.path(), "first");
        assert!(any_commit(dir.path(), &["commit", "HEAD"]) && any_commit(dir.path(), &["HEAD", "commit"]));
        assert!(!any_commit(dir.path(), &["commit", "absent", "HEAD\nHEAD"]), "only names, not git's output, can match");
        let outside = tempfile::tempdir().unwrap();
        assert!(!any_commit(outside.path(), &["HEAD"]));
    }

    #[test]
    fn installed_git_passes_the_startup_check() {
        assert!(check().is_ok());
    }

    #[test]
    fn commands_take_no_optional_locks_and_set_no_env() {
        let cmd = command(Path::new(".")).unwrap();
        assert_eq!(cmd.get_args().next(), Some(std::ffi::OsStr::new("--no-optional-locks")));
        // only repository-config overrides follow, as `-c` pairs
        assert!(cmd.get_args().skip(1).step_by(2).all(|a| a == "-c"));
        assert_eq!(cmd.get_envs().count(), 0, "an env override copies the environment on every spawn");
        // the oldest supported git accepts the flag before any subcommand
        assert!(run(Path::new("."), &["--version"]).is_ok());
    }

    #[test]
    fn repository_config_programs_are_not_run() {
        let repo = testing::repo();
        let ran = repo.path().join("ran");
        let touch = format!("sh -c 'touch {}; cat'", ran.display());
        testing::write(repo.path(), "f.txt", "a\n");
        testing::write(repo.path(), ".gitattributes", "*.txt filter=evil diff=evil\n");
        testing::commit_all(repo.path(), "f");
        testing::write(repo.path(), "f.txt", "b\n");
        let hook = repo.path().join("fsmonitor.sh");
        std::fs::write(&hook, format!("#!/bin/sh\ntouch {}\n", ran.display())).unwrap();
        std::fs::set_permissions(&hook, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        for (key, value) in [("filter.evil.clean", touch.as_str()), ("filter.evil.smudge", &touch), ("filter.evil.required", "true"),
            ("diff.evil.textconv", &touch), ("core.fsmonitor", hook.to_str().unwrap())] {
            testing::git(repo.path(), &["config", key, value]);
        }
        let u = untrusted(repo.path());
        assert_eq!((u.textconv.as_slice(), u.filters.as_slice()), (&["evil".to_string()][..], &["evil".to_string()][..]));
        for args in [&["status", "--porcelain=v2", "-z"][..], &["diff", "--raw", "-z"], &["blame", "--incremental", "--", "f.txt"],
            &["cat-file", "--filters", ":f.txt"], &["cat-file", "--textconv", "--path=f.txt", ":f.txt"]] {
            let _ = run(repo.path(), args);
            assert!(!ran.exists(), "git {args:?} ran a repository-config program");
        }
        // git itself would have run them
        testing::git(repo.path(), &["status"]);
        assert!(ran.exists());
    }

    #[test]
    fn the_users_own_drivers_and_gits_fsmonitor_daemon_are_kept() {
        let out = b"global\0filter.lfs.clean\ngit-lfs clean -- %f\0local\0core.fsmonitor\ntrue\0system\0diff.exif.textconv\nexiftool\0";
        assert_eq!(untrusted_from(out), Untrusted::default());
        let out = b"local\0filter.a.b.process\nx\0worktree\0filter.a.b.clean\ny\0local\0core.fsmonitor\n/tmp/hook\0";
        let u = untrusted_from(out);
        assert_eq!(u.filters, ["a.b"]);
        assert_eq!(u.args, ["filter.a.b.process=", "filter.a.b.required=false", "filter.a.b.clean=", "core.fsmonitor=false"]);
    }

    #[test]
    fn bounded_reads_keep_a_head_and_stay_in_step() {
        let repo = testing::repo();
        testing::write(repo.path(), "big", "x".repeat(200_000));
        testing::write(repo.path(), "small", "small");
        testing::commit_all(repo.path(), "files");
        let mut cat = CatFile::spawn(repo.path(), "--batch").unwrap();
        assert_eq!(cat.get_bounded("HEAD:big", 1000, 8).unwrap(), Some((200_000, b"xxxxxxxx".to_vec())));
        assert_eq!(cat.get_bounded("HEAD:small", 1000, 8).unwrap(), Some((5, b"small".to_vec())));
        assert_eq!(cat.get_bounded("HEAD:missing", 1000, 8).unwrap(), None);
        assert_eq!(cat.get("HEAD:small").unwrap().unwrap(), b"small");
    }
}

/// Throwaway repositories made with the real `git`, isolated from the user's config.
#[cfg(test)]
pub mod testing {
    use std::{path::Path, process::Command};

    pub fn git(dir: &Path, args: &[&str]) -> String {
        git_at(dir, args, None)
    }

    /// [`git`] with the committer (and tagger) date set, e.g. `1700000000 +0000`.
    pub fn git_at(dir: &Path, args: &[&str], date: Option<&str>) -> String {
        let mut cmd = command(dir, args);
        if let Some(date) = date {
            cmd.env("GIT_COMMITTER_DATE", date);
        }
        let out = cmd.output().expect("run git");
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    /// [`git`] for a command that may fail, e.g. a merge stopping at a conflict: did it succeed?
    pub fn try_git(dir: &Path, args: &[&str]) -> bool {
        command(dir, args).output().expect("run git").status.success()
    }

    fn command(dir: &Path, args: &[&str]) -> Command {
        let mut cmd = Command::new("git");
        cmd.current_dir(dir)
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t");
        cmd
    }

    /// An empty repository on branch `main`.
    pub fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "-q", "-b", "main"]);
        dir
    }

    pub fn write(dir: &Path, path: &str, text: impl AsRef<[u8]>) {
        let path = dir.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    pub fn commit_all(dir: &Path, message: &str) {
        git(dir, &["add", "-A"]);
        git(dir, &["commit", "-q", "--no-verify", "-m", message]);
    }
}
