//! Every `git` process Lenz starts goes through here, after a once-per-run check that git is usable.

use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    sync::{Arc, Mutex, OnceLock, PoisonError},
};

const MIN_VERSION: (u32, u32) = (2, 30);
/// The first git with `--no-lazy-fetch` (its 2.45 release notes).
const NO_LAZY_FETCH: (u32, u32) = (2, 45);

/// A `git` command running in `cwd`, or why git can't be used.
pub fn command(cwd: &Path) -> Result<Command, String> {
    let version = usable().clone()?;
    let untrusted = untrusted(cwd);
    if let Some(key) = &untrusted.refused {
        return Err(format!("lenz: this repository's own config sets {key}, a program Lenz cannot switch off; use git yourself if you trust it"));
    }
    let mut cmd = Command::new("git");
    // `status` must not take `index.lock` behind the user's own git. Arguments, not GIT_OPTIONAL_LOCKS or
    // GIT_CONFIG_*: any env override makes std copy the whole environment on every spawn (~70 µs with a typical
    // desktop env).
    cmd.current_dir(cwd).arg("--no-optional-locks");
    // An object a promisor remote holds would be fetched with the repository's own transport settings.
    if version >= NO_LAZY_FETCH {
        cmd.arg("--no-lazy-fetch");
    } else if untrusted.promisor {
        return Err("lenz: this repository's own config names a promisor remote; reading it without fetching needs git 2.45 or later".into());
    }
    // Hooks run only for what the user asked for ([`command_as_user`]); signature checks run `gpg.program`.
    cmd.args(["-c", "core.hooksPath=/dev/null", "-c", "log.showSignature=false"]);
    for setting in &untrusted.args {
        cmd.args(["-c", setting]);
    }
    Ok(cmd)
}

/// Programs a repository's own config (`.git/config`, its worktree config, and what they include) would have
/// git run: textconv, filter and merge drivers, configured hooks, an fsmonitor hook, and a promisor remote's
/// transport. A repository can arrive with its `.git` (an archive, a shared drive, someone else's checkout), and
/// opening it in a viewer must not run its commands. Drivers from the user's global or system config, and git's
/// own fsmonitor daemon, are kept.
#[derive(Debug, Default, PartialEq)]
pub struct Untrusted {
    /// `name=value` overrides passed as `-c` to every git Lenz runs in that directory.
    args: Vec<String>,
    /// A key needing an override `-c` cannot name (git splits it at the first `=`): every command is refused.
    refused: Option<String>,
    /// A promisor remote: reading an absent object would fetch it.
    promisor: bool,
    /// A key that would pick where a fetch connects, what it trusts and sends, or which program carries it.
    network: Option<String>,
    /// A local include: its file can change unseen by [`stamp`], so the answer is never kept.
    includes: bool,
    /// Drivers not run, by name, for the notes on the files that use them.
    pub textconv: Vec<String>,
    pub filters: Vec<String>,
}

/// The nearest repository at or above a directory: its git dir, the identity of its config files, and whether
/// git would recognise it yet (`HEAD`, `objects`, `refs`).
type Stamp = Option<(PathBuf, [Option<[i64; 4]>; 2], [bool; 3])>;

/// Probes by directory, with the [`Stamp`] each was taken under (`None`: trusted, never asked again).
type Seen = HashMap<PathBuf, (Option<Stamp>, Arc<Untrusted>)>;

fn seen() -> &'static Mutex<Seen> {
    static SEEN: OnceLock<Mutex<Seen>> = OnceLock::new();
    SEEN.get_or_init(Default::default)
}

/// Treats `cwd`'s repository config as the user's own, as a driver in their global config would be.
#[cfg(any(test, feature = "git-testing"))]
pub fn trust(cwd: &Path) {
    let mut seen = seen().lock().unwrap_or_else(PoisonError::into_inner);
    seen.insert(cwd.to_path_buf(), (None, Arc::default()));
    // git names the top level by its resolved path (macOS temp dirs sit behind the /var -> /private/var link)
    if let Ok(resolved) = cwd.canonicalize() {
        seen.insert(resolved, (None, Arc::default()));
    }
}

/// [`Untrusted`] for `cwd`, asked again whenever the repository there is not the one it was asked of: one that
/// arrives (extracted, copied, synced) or is replaced at a probed directory.
pub fn untrusted(cwd: &Path) -> Arc<Untrusted> {
    let seen = seen();
    let now = stamp(cwd);
    if let Some((was, found)) = seen.lock().unwrap_or_else(PoisonError::into_inner).get(cwd)
        && was.as_ref().is_none_or(|was| *was == now)
    {
        return found.clone();
    }
    // `.*`: git applies drivers and hooks with an empty name too
    let out = Command::new("git")
        .current_dir(cwd)
        .args(["--no-optional-locks", "config", "--show-scope", "-z", "--get-regexp"])
        .arg(concat!(r"^(core\.(fsmonitor|sshcommand|gitproxy|askpass|alternaterefscommand)",
            r"|filter\..*\.(clean|smudge|process)|diff\..*\.textconv|merge\..*\.driver|hook\..*\.(command|event)",
            r"|extensions\.partialclone|remote\..*\.(promisor|partialclonefilter)|remote\..*[/:].*\.[^.]*",
            r"|(http|credential|url|protocol)\..*|include\.path|includeif\..*\.path)$"))
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output();
    let found = Arc::new(out.map(|o| untrusted_from(&o.stdout)).unwrap_or_default());
    if !found.includes {
        seen.lock().unwrap_or_else(PoisonError::into_inner).insert(cwd.to_path_buf(), (Some(now), found.clone()));
    }
    found
}

/// See [`Stamp`]. A few `stat`s, against the git process each guarded command starts.
fn stamp(cwd: &Path) -> Stamp {
    for dir in cwd.ancestors() {
        let dotgit = dir.join(".git");
        let git = match std::fs::metadata(&dotgit) {
            // a `.git` file names the git dir: a linked worktree's, or an absorbed submodule's
            Ok(m) if !m.is_dir() => match std::fs::read_to_string(&dotgit) {
                Ok(ref text) if let Some(path) = text.strip_prefix("gitdir:") => dir.join(path.trim()),
                _ => return Some((dotgit, [identity(&dir.join(".git")), None], [false; 3])),
            },
            Ok(_) => dotgit,
            Err(_) if dir.join("HEAD").is_file() && dir.join("objects").is_dir() => dir.to_path_buf(),
            Err(_) => continue,
        };
        // a linked worktree keeps its config, objects and refs in the common dir
        let common = std::fs::read_to_string(git.join("commondir")).map_or_else(|_| git.clone(), |c| git.join(c.trim()));
        let recognised = [git.join("HEAD").exists(), common.join("objects").exists(), common.join("refs").exists()];
        return Some((git.clone(), [identity(&common.join("config")), identity(&git.join("config.worktree"))], recognised));
    }
    None
}

/// A file's size, inode and change time: a copy that keeps size and modification time still changes these.
#[cfg(unix)]
fn identity(path: &Path) -> Option<[i64; 4]> {
    use std::os::unix::fs::MetadataExt;
    let m = std::fs::metadata(path).ok()?;
    Some([m.len() as i64, m.ino() as i64, m.ctime(), m.ctime_nsec()])
}

#[cfg(not(unix))]
fn identity(path: &Path) -> Option<[i64; 4]> {
    let m = std::fs::metadata(path).ok()?;
    let t = m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).unwrap_or_default();
    Some([m.len() as i64, 0, t.as_secs() as i64, t.subsec_nanos() as i64])
}

/// git's own text merge, standing in for a repository's merge driver; the conflict style comes from
/// `merge.conflictStyle`, which git passes on to it.
const TEXT_MERGE: &str = "git merge-file -L ours -L base -L theirs --marker-size=%L %A %O %B";

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
        // `section.variable` or `section.<name>.variable`; a name may hold dots, or be empty
        let (section, rest) = key.split_once('.').unwrap_or((key, ""));
        let (name, variable) = rest.rsplit_once('.').map_or((None, rest), |(n, v)| (Some(n), v));
        let overridden = match (section, variable) {
            ("diff", "textconv") | ("filter", "clean" | "smudge" | "process") | ("merge", "driver") | ("hook", "command" | "event") => name,
            _ => None,
        };
        if let Some(name) = overridden && name.contains('=') {
            u.refused.get_or_insert_with(|| key.to_string());
            continue;
        }
        match (section, overridden, variable) {
            ("core", _, "fsmonitor") => {
                // a boolean selects git's own daemon; anything else names a program to run
                let boolean = ["true", "false", "yes", "no", "on", "off", "1", "0", ""].contains(&value.to_ascii_lowercase().as_str());
                if !boolean {
                    u.args.push("core.fsmonitor=false".into());
                }
            }
            ("diff", Some(name), _) => {
                u.args.push(format!("{key}="));
                u.textconv.push(name.to_string());
            }
            ("filter", Some(name), _) => {
                u.args.push(format!("{key}="));
                if !u.filters.iter().any(|f| f == name) {
                    // an empty driver marked required would fail every read of its files
                    u.args.push(format!("filter.{name}.required=false"));
                    u.filters.push(name.to_string());
                }
            }
            ("merge", Some(name), _) => u.args.push(format!("merge.{name}.driver={TEXT_MERGE}")),
            // core.hooksPath does not reach hooks defined in config; a local event can point the user's own hook
            ("hook", Some(name), _) => {
                let off = format!("hook.{name}.enabled=false");
                if !u.args.contains(&off) {
                    u.args.push(off);
                }
            }
            ("include", _, "path") | ("includeif", _, "path") => u.includes = true,
            _ => {}
        }
        if key == "extensions.partialclone" || section == "remote" && matches!(variable, "promisor" | "partialclonefilter") {
            u.promisor = true;
        }
        let network = match section {
            "http" | "credential" | "url" | "protocol" => true,
            "core" => matches!(variable, "sshcommand" | "gitproxy" | "askpass" | "alternaterefscommand"),
            // a remote named by a URL configures a fetch from that URL
            "remote" => name.is_some_and(|n| n.contains(['/', ':'])),
            _ => false,
        };
        if network {
            u.network.get_or_insert_with(|| key.to_string());
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

/// A `git` command in `cwd` as the user's own terminal would run it: the repository's hooks, filters and config all
/// apply. Only for changes the user asked for (a checkout, a new branch); everything else uses [`command`], which keeps
/// a repository's programs from running when it is merely read. Messages in the C locale (callers read git's
/// refusals), paths unquoted, never waiting on a prompt.
pub fn command_as_user(cwd: &Path) -> Result<Command, String> {
    usable().clone()?;
    let mut cmd = Command::new("git");
    cmd.current_dir(cwd)
        .args(["-c", "core.quotePath=false"])
        .env("LC_ALL", "C")
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null());
    Ok(cmd)
}

/// [`run`] as the user: see [`command_as_user`].
pub fn run_as_user(cwd: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    finish(command_as_user(cwd)?.args(args))
}

/// Runs git in `cwd` and returns its stdout; a non-zero exit becomes git's stderr.
pub fn run(cwd: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    finish(command(cwd)?.args(args))
}

/// [`run`] for a command that may reach the network: it fails rather than waits on a credential prompt, which
/// the GUI has no terminal for and the TUI's raw-mode terminal would swallow. Refused while the repository's own
/// config sets a proxy, TLS, cookie, credential, URL-rewrite, protocol or transport-program key: it would hand the
/// user's forge credential to someone else or run the repository's program. The user's own settings still apply.
pub fn run_network(cwd: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    if let Some(key) = &untrusted(cwd).network {
        return Err(format!("lenz: this repository's own config sets {key}, which the fetch would use; fetch it yourself if you trust it"));
    }
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
pub fn path_text(bytes: &[u8]) -> Result<&str, String> {
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
pub fn may_be_in_repository(cwd: &Path) -> bool {
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
        let lazy = cmd.get_args().nth(1) == Some(std::ffi::OsStr::new("--no-lazy-fetch"));
        assert_eq!(lazy, version().unwrap() >= NO_LAZY_FETCH);
        // only config overrides follow, as `-c` pairs
        assert!(cmd.get_args().skip(1 + lazy as usize).step_by(2).all(|a| a == "-c"));
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
        testing::write(repo.path(), "e.dat", "a\n");
        // `filter=` and `diff=` select the drivers named "" ([filter ""], [diff ""]), which git applies too
        testing::write(repo.path(), ".gitattributes", "*.txt filter=evil diff=evil\n*.dat filter= diff=\n");
        testing::commit_all(repo.path(), "f");
        testing::write(repo.path(), "f.txt", "b\n");
        testing::write(repo.path(), "e.dat", "b\n");
        let hook = repo.path().join("fsmonitor.sh");
        std::fs::write(&hook, format!("#!/bin/sh\ntouch {}\n", ran.display())).unwrap();
        std::fs::set_permissions(&hook, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        for (key, value) in [("filter.evil.clean", touch.as_str()), ("filter.evil.smudge", &touch), ("filter.evil.required", "true"),
            ("diff.evil.textconv", &touch), ("core.fsmonitor", hook.to_str().unwrap())] {
            testing::git(repo.path(), &["config", key, value]);
        }
        append_config(repo.path(), &format!("[filter \"\"]\n\tclean = \"{touch}\"\n\tsmudge = \"{touch}\"\n[diff \"\"]\n\ttextconv = \"{touch}\"\n"));
        let u = untrusted(repo.path());
        assert_eq!(u.textconv, ["evil", ""]);
        assert_eq!(u.filters, ["evil", ""]);
        for path in ["f.txt", "e.dat"] {
            for args in [&["status", "--porcelain=v2", "-z"][..], &["diff", "--raw", "-z"], &["blame", "--incremental", "--", path],
                &["cat-file", "--filters", &format!(":{path}")], &["cat-file", "--textconv", &format!("--path={path}"), &format!(":{path}")]] {
                let _ = run(repo.path(), args);
                assert!(!ran.exists(), "git {args:?} ran a repository-config program");
            }
        }
        // git itself would have run them, the empty-named clean filter alone too
        testing::git(repo.path(), &["config", "--unset", "core.fsmonitor"]);
        testing::git(repo.path(), &["config", "--remove-section", "filter.evil"]);
        testing::git(repo.path(), &["add", "e.dat"]);
        assert!(ran.exists());
    }

    fn append_config(repo: &Path, text: &str) {
        let mut config = std::fs::OpenOptions::new().append(true).open(repo.join(".git/config")).unwrap();
        config.write_all(text.as_bytes()).unwrap();
    }

    fn marker_script(dir: &Path, name: &str, marker: &Path) -> PathBuf {
        let script = dir.join(name);
        std::fs::write(&script, format!("#!/bin/sh\ntouch {}\n", marker.display())).unwrap();
        std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        script
    }

    #[test]
    fn driver_names_holding_an_equals_sign_refuse_every_command() {
        let repo = testing::repo();
        let ran = repo.path().join("ran");
        testing::write(repo.path(), "f.txt", "a\n");
        testing::write(repo.path(), ".gitattributes", "*.txt filter=a=b diff=c=d\n");
        testing::commit_all(repo.path(), "f");
        testing::write(repo.path(), "f.txt", "b\n");
        let touch = format!("sh -c 'touch {}; cat'", ran.display());
        // `-c filter.a=b.clean=` would set `filter.a` and leave the driver in force
        append_config(repo.path(), &format!("[filter \"a=b\"]\n\tclean = \"{touch}\"\n[diff \"c=d\"]\n\ttextconv = \"{touch}\"\n"));
        let err = command(repo.path()).unwrap_err();
        assert!(err.contains("sets filter.a=b.clean,"), "{err}");
        for args in [&["status", "--porcelain=v2", "-z"][..], &["diff", "--raw", "-z"], &["blame", "--", "f.txt"]] {
            assert!(run(repo.path(), args).is_err());
        }
        assert!(!ran.exists());
        testing::git(repo.path(), &["add", "f.txt"]);
        assert!(ran.exists(), "git itself runs the driver");
    }

    #[test]
    fn repository_hooks_never_run_on_guarded_commands() {
        let repo = testing::repo();
        testing::write(repo.path(), "f.txt", "a\n");
        testing::commit_all(repo.path(), "f");
        let (file_hook, config_hook) = (repo.path().join("file-hook-ran"), repo.path().join("config-hook-ran"));
        let hooks = repo.path().join(".git/hooks");
        marker_script(&hooks, "post-index-change", &file_hook);
        marker_script(&hooks, "reference-transaction", &file_hook);
        append_config(repo.path(), &format!("[hook \"x\"]\n\tcommand = touch {}\n\tevent = post-index-change\n", config_hook.display()));
        testing::write(repo.path(), "f.txt", "b\n");
        run(repo.path(), &["add", "f.txt"]).unwrap();
        run(repo.path(), &["update-ref", "refs/lenz/x", "HEAD"]).unwrap();
        run(repo.path(), &["update-ref", "-d", "refs/lenz/x"]).unwrap();
        assert!(!file_hook.exists() && !config_hook.exists(), "a guarded write ran a repository hook");
        // git itself would have run them
        testing::write(repo.path(), "f.txt", "c\n");
        testing::git(repo.path(), &["add", "f.txt"]);
        assert!(file_hook.exists());
        if version().unwrap() >= (2, 54) {
            assert!(config_hook.exists(), "git with config-defined hooks runs this one");
        }
    }

    #[test]
    fn signature_checks_never_run_the_repositorys_gpg_program() {
        let repo = testing::repo();
        let ran = repo.path().join("ran");
        testing::write(repo.path(), "f.txt", "a\n");
        testing::commit_all(repo.path(), "f");
        let tree = testing::git(repo.path(), &["rev-parse", "HEAD^{tree}"]);
        let head = testing::git(repo.path(), &["rev-parse", "HEAD"]);
        let commit = format!("tree {}\nparent {}\nauthor t <t@t> 1700000000 +0000\ncommitter t <t@t> 1700000000 +0000\n\
            gpgsig -----BEGIN PGP SIGNATURE-----\n \n AAAA\n -----END PGP SIGNATURE-----\n\nsigned\n", tree.trim(), head.trim());
        let raw = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(raw.path(), commit).unwrap();
        let signed = testing::git(repo.path(), &["hash-object", "-t", "commit", "-w", raw.path().to_str().unwrap()]);
        testing::git(repo.path(), &["update-ref", "HEAD", signed.trim()]);
        let gpg = marker_script(repo.path(), "verify", &ran);
        testing::git(repo.path(), &["config", "log.showSignature", "true"]);
        testing::git(repo.path(), &["config", "gpg.program", gpg.to_str().unwrap()]);
        for args in [&["log", "-1", "--format=%h"][..], &["reflog", "-1"], &["log", "--format=%H", "--all"]] {
            run(repo.path(), args).unwrap();
            assert!(!ran.exists(), "git {args:?} ran the repository's gpg.program");
        }
        testing::git(repo.path(), &["log", "-1"]);
        assert!(ran.exists(), "git itself verifies the signature");
    }

    #[test]
    fn promisor_repositories_never_fetch_on_a_read() {
        for shape in ["[extensions]\n\tpartialClone = origin\n", "[remote \"origin\"]\n\tpromisor = true\n",
            "[remote \"origin\"]\n\tpartialclonefilter = blob:none\n"] {
            let repo = testing::repo();
            let ran = repo.path().join("ran");
            testing::write(repo.path(), "f.txt", "a\n");
            testing::commit_all(repo.path(), "f");
            let blob = testing::git(repo.path(), &["rev-parse", "HEAD:f.txt"]).trim().to_string();
            std::fs::remove_file(repo.path().join(".git/objects").join(&blob[..2]).join(&blob[2..])).unwrap();
            let url = repo.path().display();
            append_config(repo.path(), &format!("[remote \"origin\"]\n\turl = {url}\n\tuploadpack = touch {}; false\n{shape}", ran.display()));
            assert!(untrusted(repo.path()).promisor, "{shape}");
            if version().unwrap() < NO_LAZY_FETCH {
                assert!(command(repo.path()).unwrap_err().contains("promisor remote"));
                continue;
            }
            assert!(run(repo.path(), &["cat-file", "-p", &blob]).is_err());
            let mut sizes = CatFile::spawn(repo.path(), "--batch-check").unwrap();
            assert_eq!(sizes.size(&blob).unwrap(), None);
            assert!(!ran.exists(), "{shape}: a read fetched from the promisor remote");
            assert!(!testing::try_git(repo.path(), &["cat-file", "-p", &blob]));
            assert!(ran.exists(), "{shape}: git itself fetches");
        }
        // an ordinary repository reads as before
        let repo = testing::repo();
        testing::write(repo.path(), "f.txt", "a\n");
        testing::commit_all(repo.path(), "f");
        assert_eq!(run(repo.path(), &["cat-file", "-p", "HEAD:f.txt"]).unwrap(), b"a\n");
    }

    #[test]
    fn a_probe_is_taken_again_when_a_repository_arrives() {
        // a `.git` directory, or a `.git` file naming the git dir
        for behind_a_file in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let ran = dir.path().join("ran");
            assert_eq!(*untrusted(dir.path()), Untrusted::default(), "no repository yet");
            // an extracted repository whose HEAD comes last: git does not see it until then
            let made = testing::repo();
            testing::write(made.path(), "f.txt", "a\n");
            testing::commit_all(made.path(), "f");
            let hook = marker_script(made.path(), "fsmonitor.sh", &ran);
            testing::git(made.path(), &["config", "core.fsmonitor", hook.to_str().unwrap()]);
            let git = dir.path().join(if behind_a_file { "store" } else { ".git" });
            std::fs::rename(made.path().join(".git"), &git).unwrap();
            std::fs::rename(git.join("HEAD"), dir.path().join("HEAD.later")).unwrap();
            if behind_a_file {
                std::fs::write(dir.path().join(".git"), "gitdir: store\n").unwrap();
            }
            assert_eq!(*untrusted(dir.path()), Untrusted::default(), "git does not recognise it yet");
            std::fs::rename(dir.path().join("HEAD.later"), git.join("HEAD")).unwrap();
            testing::write(dir.path(), "f.txt", "b\n");
            run(dir.path(), &["status", "--porcelain=v2"]).unwrap();
            assert!(!ran.exists(), "the repository's fsmonitor hook ran");
            assert!(command(dir.path()).unwrap().get_args().any(|a| a == "core.fsmonitor=false"));
            // replaced in place by one without it: asked again
            testing::git(dir.path(), &["config", "--unset", "core.fsmonitor"]);
            assert_eq!(*untrusted(dir.path()), Untrusted::default());
        }
    }

    #[test]
    fn a_local_include_is_read_again_on_every_command() {
        let repo = testing::repo();
        testing::git(repo.path(), &["config", "include.path", "../extra.cfg"]);
        assert!(untrusted(repo.path()).textconv.is_empty());
        // the included file arrives later, with the repository's own config unchanged
        testing::write(repo.path(), "extra.cfg", "[diff \"evil\"]\n\ttextconv = false\n");
        assert_eq!(untrusted(repo.path()).textconv, ["evil"]);
    }

    #[test]
    fn network_commands_refuse_a_repositorys_own_transport_tls_and_credential_settings() {
        let upstream = testing::repo();
        testing::write(upstream.path(), "f.txt", "a\n");
        testing::commit_all(upstream.path(), "f");
        let url = upstream.path().to_str().unwrap();
        let fetch = ["fetch", url, "+refs/heads/main:refs/x"];
        for (key, value) in [("http.proxy", "http://127.0.0.1:9"), ("http.https://github.com.sslVerify", "false"),
            ("http.sslCAInfo", ".git/ca.pem"), ("http.cookieFile", "~/.gitcookies"), ("credential.helper", "store --file=.git/c"),
            ("url.https://x.invalid/.insteadOf", "https://github.com/"), ("core.sshCommand", "false"),
            ("protocol.ext.allow", "always"), ("remote.https://github.com/o/r.proxy", "http://127.0.0.1:9")] {
            let repo = testing::repo();
            testing::git(repo.path(), &["config", key, value]);
            let err = run_network(repo.path(), &fetch).unwrap_err();
            assert!(err.starts_with("lenz: this repository's own config sets"), "{key}: {err}");
            assert!(!testing::try_git(repo.path(), &["rev-parse", "--verify", "refs/x"]), "{key}: fetched");
        }
        // an ordinary repository fetches; a named remote's own settings don't apply to a fetch by URL
        let repo = testing::repo();
        testing::git(repo.path(), &["config", "remote.origin.url", url]);
        run_network(repo.path(), &fetch).unwrap();
        assert!(testing::try_git(repo.path(), &["rev-parse", "--verify", "refs/x"]));
        // a test's stand-in forge: a trusted fixture's own URL rewrite applies
        let repo = testing::repo();
        testing::git(repo.path(), &["config", &format!("url.{url}.insteadOf"), "https://lenz.invalid/o/r"]);
        assert!(run_network(repo.path(), &["fetch", "https://lenz.invalid/o/r", "+refs/heads/main:refs/x"]).is_err());
        trust(repo.path());
        run_network(repo.path(), &["fetch", "https://lenz.invalid/o/r", "+refs/heads/main:refs/x"]).unwrap();
    }

    #[test]
    fn repository_drivers_hooks_and_transport_settings_are_parsed_once() {
        let out = b"local\0merge.ours=.driver\nx\0local\0merge.m.driver\ny\0local\0hook.h.command\nz\0worktree\0hook.h.event\npre-commit\0\
            global\0hook.g.command\nmine\0local\0remote.origin.promisor\ntrue\0local\0remote.origin.url\nhttps://x\0\
            local\0http.https://x.proxy\np\0local\0include.path\nf\0";
        let u = untrusted_from(out);
        assert_eq!(u.refused.as_deref(), Some("merge.ours=.driver"));
        assert_eq!(u.args, [format!("merge.m.driver={TEXT_MERGE}"), "hook.h.enabled=false".into()]);
        assert!(u.promisor && u.includes);
        assert_eq!(u.network.as_deref(), Some("http.https://x.proxy"));
        let u = untrusted_from(b"local\0filter..clean\nx\0local\0diff..textconv\ny\0local\0remote.git@h:o/r.uploadpack\nz\0");
        assert_eq!(u.args, ["filter..clean=", "filter..required=false", "diff..textconv="]);
        assert_eq!((u.filters.as_slice(), u.textconv.as_slice(), u.refused), (&["".to_string()][..], &["".to_string()][..], None));
        assert_eq!(u.network.as_deref(), Some("remote.git@h:o/r.uploadpack"));
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
#[cfg(any(test, feature = "git-testing"))]
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
            // Detached maintenance can leave an orphan after fixture commits on
            // recent Git. Keep it owned and waited for, including on older Git.
            .args(["-c", "maintenance.autoDetach=false", "-c", "gc.autoDetach=false"])
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

    /// A repository with a nested one at `sub`, committed as a gitlink, whose own config names `hook` (an
    /// executable) as its fsmonitor, and whose file is modified: git's own `status` outside runs the hook.
    pub fn nested_with_fsmonitor(hook: &Path) -> tempfile::TempDir {
        let outer = repo();
        let sub = outer.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        git(&sub, &["init", "-q", "-b", "main"]);
        write(&sub, "f", "a\n");
        commit_all(&sub, "inner");
        write(outer.path(), "top", "x\n");
        commit_all(outer.path(), "outer");
        git(&sub, &["config", "core.fsmonitor", hook.to_str().unwrap()]);
        write(&sub, "f", "b\n");
        outer
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn fixture_commits_do_not_request_detached_maintenance() {
            let dir = repo();
            let trace = tempfile::NamedTempFile::new().unwrap();
            for _ in 0..3 {
                let out = command(dir.path(), &["commit", "-q", "--allow-empty", "--no-verify", "-m", "fixture"])
                    .env("GIT_TRACE2_EVENT", trace.path())
                    .output().unwrap();
                assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
            }
            let events = std::fs::read_to_string(trace.path()).unwrap();
            let mut commits = 0;
            for line in events.lines() {
                let event: serde_json::Value = serde_json::from_str(line).unwrap();
                if event["event"] == "cmd_name" && event["name"] == "commit" { commits += 1; }
                if event["event"] == "child_start" && let Some(args) = event["argv"].as_array() {
                    assert!(!args.iter().any(|arg| arg == "--detach"), "fixture detached maintenance: {event}");
                }
            }
            assert_eq!(commits, 3, "the trace must cover every fixture commit");
        }
    }
}
