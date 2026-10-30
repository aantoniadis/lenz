//! Local crash diagnostics shared by the GUI and terminal adapters.
//! Healthy startup does no filesystem I/O. The hook is installed before Ratatui,
//! which chains it after restoring the terminal, including panic=abort builds.
use std::{
    backtrace::Backtrace,
    collections::VecDeque,
    fs::{self, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Instant, SystemTime, UNIX_EPOCH},
};

const ISSUE: &str = "https://github.com/aantoniadis/lenz/issues/new?";
const MAX_REPORTS: usize = 10;
const MAX_BYTES: u64 = 256 * 1024;
const BUILD: &str = match option_env!("LENZ_BUILD_ID") {
    Some(id) => id,
    None => "local build (revision unavailable)",
};
struct Config {
    frontend: &'static str,
    started: Instant,
}
static CONFIG: OnceLock<Config> = OnceLock::new();
static RECENT: Mutex<VecDeque<(u128, &'static str)>> = Mutex::new(VecDeque::new());
static SEQUENCE: AtomicU64 = AtomicU64::new(0);
static FIRST_PANIC: OnceLock<String> = OnceLock::new();
static FIRST_REPORT: OnceLock<PathBuf> = OnceLock::new();
struct WorkerPanic {
    cause: String,
    trace: String,
    report: Option<PathBuf>,
}
static DOCUMENT_PANIC: OnceLock<WorkerPanic> = OnceLock::new();
/// A full-screen terminal app is drawing: stderr would print over it, and its notice already says what failed.
static TERMINAL_OWNED: AtomicBool = AtomicBool::new(false);

/// The terminal adapter owns the screen (`true`) or gave it back (`false`, also before a panic is reported).
pub fn terminal_owned(owned: bool) {
    TERMINAL_OWNED.store(owned, Ordering::Relaxed);
}

/// stderr, unless a terminal app is drawing over it.
fn stderr() -> Option<io::StderrLock<'static>> {
    (!TERMINAL_OWNED.load(Ordering::Relaxed)).then(|| io::stderr().lock())
}

#[derive(Debug)]
pub struct SavedReport {
    pub path: PathBuf,
    pub issue_url: String,
}

/// Install once per executable, before any application or terminal initialization.
pub fn install(frontend: &'static str) {
    if CONFIG
        .set(Config {
            frontend,
            started: Instant::now(),
        })
        .is_err()
    {
        return;
    }
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // A reporter failure must never recursively invoke itself.
        thread_local! { static REPORTING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
        if REPORTING
            .try_with(|busy| busy.replace(true))
            .unwrap_or(true)
        {
            let _ = writeln!(
                io::stderr(),
                "Lenz: recursive panic while recording diagnostics"
            );
            return;
        }
        let message = info
            .payload()
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| info.payload().downcast_ref::<&str>().copied())
            .unwrap_or("non-string panic payload");
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()));
        let original = FIRST_PANIC.get_or_init(|| {
            format!(
                "{}\nLocation: {}",
                clean(message),
                location.as_deref().unwrap_or("unavailable")
            )
        });
        let previous_report = FIRST_REPORT
            .get()
            .map(|path| format!("\nOriginal panic report: {}", path.display()))
            .unwrap_or_default();
        let detail = format!(
            "First panic in this process:\n{original}{previous_report}\n\nCurrent panic:\n{}",
            clean(message)
        );
        let trace = Backtrace::force_capture().to_string();
        let result = report_dir().and_then(|dir| {
            persist(
                &dir,
                frontend,
                "Panic",
                &detail,
                location.as_deref(),
                &trace,
            )
        });
        if let Ok(report) = &result {
            let _ = FIRST_REPORT.set(report.path.clone());
        }
        if std::thread::current().name() == Some("lenz-document") {
            let _ = DOCUMENT_PANIC.set(WorkerPanic {
                cause: format!(
                    "{}\nLocation: {}",
                    clean(message),
                    clean(location.as_deref().unwrap_or("unavailable"))
                ),
                trace: clean(&trace),
                report: result.as_ref().ok().map(|report| report.path.clone()),
            });
        }
        print_result("Lenz encountered an internal error", &result);
        previous(info);
        let _ = REPORTING.try_with(|busy| busy.set(false));
    }));
    breadcrumb("Application startup");
}

/// Only fixed semantic labels belong here: never paths, queries or document text.
pub fn breadcrumb(operation: &'static str) {
    let Some(config) = CONFIG.get() else { return };
    if let Ok(mut recent) = RECENT.try_lock() {
        if recent.len() == 32 {
            recent.pop_front();
        }
        recent.push_back((config.started.elapsed().as_millis(), operation));
    }
}

/// Save a fatal operational error with context and an actionable next step.
/// Expected validation errors should remain in their existing notices.
pub fn failure(operation: &'static str, error: &str, next_step: &'static str) {
    failure_with_trace(
        operation,
        error,
        next_step,
        "Not captured for an operational error",
    );
}

pub fn comparison_failure(error: &str) {
    failure(
        "Read comparison",
        error,
        "Check that the inputs are readable and the requested Git revisions exist, then reload.",
    );
}

pub fn repository_failure(error: &str) {
    failure(
        "Repository operation",
        error,
        "Check Git status, the requested revisions and repository permissions, then retry.",
    );
}

pub fn pull_request_failure(error: &crate::pr::PrError) {
    if error.is_user_error() { return user_error(&error.to_string(), error.next_step()); }
    failure("Open pull request", &error.to_string(), error.next_step());
}

/// A mistake the user fixes (arguments, where to run it): the message and what to do, without a diagnostic report.
pub fn user_error(error: &str, next_step: &str) {
    if let Some(mut stderr) = stderr() { let _ = writeln!(stderr, "{error}\nNext step: {next_step}"); }
}

/// Only a disconnected document-worker receiver should use this association.
/// Ordinary later I/O errors must not be attributed to an earlier panic.
pub fn document_interrupted() {
    let Some(original) = DOCUMENT_PANIC.get() else {
        comparison_failure("Comparison loading was interrupted");
        return;
    };
    let report = original.report.as_ref().map_or_else(
        || "Original worker report: could not be saved".into(),
        |path| format!("Original worker report: {}", path.display()),
    );
    let error = format!(
        "Comparison loading was interrupted\nOriginal worker panic:\n{}\n{report}",
        original.cause
    );
    failure_with_trace(
        "Read comparison",
        &error,
        "Restart Lenz; review the original worker report and report the failure on GitHub.",
        &original.trace,
    );
}

fn failure_with_trace(operation: &'static str, error: &str, next_step: &'static str, trace: &str) {
    breadcrumb(operation);
    let message = format!("{operation} failed: {error}\nNext step: {next_step}");
    if let Some(mut stderr) = stderr() { let _ = writeln!(stderr, "{}", clean(&message)); }
    // Library callers/tests do not opt in to persistent application diagnostics.
    if let Some(config) = CONFIG.get() {
        let result = report_dir().and_then(|dir| {
            persist(
                &dir,
                config.frontend,
                "Operational error",
                &message,
                None,
                trace,
            )
        });
        print_result(operation, &result);
    }
}

fn print_result(context: &str, result: &io::Result<SavedReport>) {
    let Some(mut stderr) = stderr() else { return };
    match result {
        Ok(report) => {
            let _ = writeln!(
                stderr,
                "{context}. Diagnostic report: {}",
                clean(&report.path.to_string_lossy())
            );
            let _ = writeln!(
                stderr,
                "Review the report before sharing; panic messages can contain private data.\nReport on GitHub: {}",
                report.issue_url
            );
            let executable = if CONFIG.get().is_some_and(|c| c.frontend == "TUI") {
                "lenz-tui"
            } else {
                "lenz"
            };
            let _ = writeln!(
                stderr,
                "Open the issue draft later: {executable} report --last"
            );
        }
        Err(error) => {
            let _ = writeln!(
                stderr,
                "{context}. Could not save diagnostics: {}. Set LENZ_LOG_DIR to a writable private directory.\nReport on GitHub: {ISSUE}",
                clean(&error.to_string())
            );
        }
    }
}

fn report_dir() -> io::Result<PathBuf> {
    report_dir_from(
        std::env::var_os("LENZ_LOG_DIR"),
        crate::settings::get().tools.crash_dir.as_deref(),
        std::env::var_os("XDG_STATE_HOME"),
        std::env::var_os("HOME"),
    )
}

/// `LENZ_LOG_DIR`, else the settings file's `crash_dir`, else the platform's state directory.
fn report_dir_from(
    log_dir: Option<std::ffi::OsString>,
    configured: Option<&Path>,
    state: Option<std::ffi::OsString>,
    home: Option<std::ffi::OsString>,
) -> io::Result<PathBuf> {
    if let Some(path) = log_dir.filter(|p| !p.is_empty()) {
        return Ok(path.into());
    }
    if let Some(path) = configured {
        return Ok(path.to_path_buf());
    }
    if !cfg!(target_os = "macos")
        && let Some(path) = state.filter(|p| Path::new(p).is_absolute())
    {
        return Ok(PathBuf::from(path).join("lenz/crashes"));
    }
    let home = home
        .filter(|p| !p.is_empty())
        .ok_or_else(|| io::Error::other("HOME is unavailable; set LENZ_LOG_DIR"))?;
    Ok(PathBuf::from(home).join(if cfg!(target_os = "macos") {
        "Library/Logs/Lenz"
    } else {
        ".local/state/lenz/crashes"
    }))
}

fn clean(text: &str) -> String {
    let bounded = text
        .char_indices()
        .take_while(|(i, _)| *i < 64 * 1024)
        .map(|(_, c)| c);
    let mut result = String::new();
    for c in bounded {
        if result.len() >= 64 * 1024 - 16 {
            result.push_str("\n[truncated]");
            break;
        }
        if c.is_control() && !matches!(c, '\n' | '\t') {
            result.extend(c.escape_default());
        } else {
            result.push(c);
        }
    }
    if text.len() > 64 * 1024 && !result.ends_with("[truncated]") {
        result.push_str("\n[truncated]");
    }
    if let Some(home) = std::env::var_os("HOME").filter(|p| !p.is_empty()) {
        result = result.replace(home.to_string_lossy().as_ref(), "$HOME");
    }
    result
}

fn encode(value: &str) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    for b in value.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

fn persist(
    dir: &Path,
    frontend: &str,
    kind: &str,
    message: &str,
    location: Option<&str>,
    trace: &str,
) -> io::Result<SavedReport> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(dir)?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let id = format!(
        "crash-{:020}-{:010}-{:06}",
        timestamp.as_nanos(),
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    );
    let version = env!("CARGO_PKG_VERSION");
    let platform = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    // The URL contains metadata only. Raw panic/error strings, paths, environment,
    // breadcrumbs and stack traces remain in the local file for manual review.
    let title = format!("[{kind}] Lenz {frontend}");
    let body = format!(
        "## What happened?\nDescribe the action immediately before the error.\n\n## Expected behavior\n\n## Reproduction steps\n1. \n\n## Diagnostics\nLenz: {version}\nBuild: {BUILD}\nFrontend: {frontend}\nPlatform: {platform} ({arch})\nFailure: {kind}\nReport: {id}.log\n\nReview the local report for private data, then attach it or paste the relevant error and backtrace here.\n"
    );
    let issue_url = format!("{ISSUE}title={}&body={}", encode(&title), encode(&body));
    let thread = std::thread::current();
    let mut log = format!(
        "Lenz diagnostic report\nReport: {id}\nTimestamp (Unix seconds, UTC): {}\nVersion: {version}\nBuild: {BUILD}\nPlatform: {platform} ({arch})\nFrontend: {frontend}\nFailure: {kind}\nThread: {} ({:?})\nLocation: {}\n\nCause:\n{}\n\nRecent operations (milliseconds since startup):\n",
        timestamp.as_secs(),
        clean(thread.name().unwrap_or("unnamed")),
        thread.id(),
        clean(location.unwrap_or("unavailable")),
        clean(message)
    );
    if let Ok(recent) = RECENT.try_lock() {
        use std::fmt::Write;
        for (ms, operation) in recent.iter() {
            let _ = writeln!(log, "{ms:>8} ms  {operation}");
        }
    }
    log.push_str(&format!("\nBacktrace:\n{}\n\nReview before sharing: panic messages and stack traces may contain private data.\nNo document contents, command arguments, or credentials are collected explicitly.\nReport on GitHub: {issue_url}\n", clean(trace)));
    if log.len() as u64 > MAX_BYTES {
        return Err(io::Error::other(
            "diagnostic report exceeded its size limit",
        ));
    }
    let temporary = dir.join(format!("{id}.pending"));
    let path = dir.join(format!("{id}.log"));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let written = (|| {
        let mut file = options.open(&temporary)?;
        file.write_all(log.as_bytes())?;
        file.sync_all()?;
        fs::rename(&temporary, &path)
    })();
    if let Err(error) = written {
        let _ = fs::remove_file(temporary);
        return Err(error);
    }
    // Best effort cleanup must not hide a successfully persisted report.
    if let Ok(mut reports) = report_paths(dir) {
        reports.sort();
        let excess = reports.len().saturating_sub(MAX_REPORTS);
        for old in reports.into_iter().take(excess) {
            let _ = fs::remove_file(old);
        }
    }
    Ok(SavedReport { path, issue_url })
}

fn report_paths(dir: &Path) -> io::Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("crash-") && name.ends_with(".log") && entry.file_type()?.is_file() {
            paths.push(entry.path());
        }
    }
    Ok(paths)
}

pub fn last_report() -> io::Result<SavedReport> {
    last_report_in(&report_dir()?)
}
/// Handle diagnostics before normal CLI parsing, repository access or UI setup.
pub fn report_cli(args: &[String]) -> Option<std::process::ExitCode> {
    use std::process::ExitCode;
    if args.first().is_none_or(|s| s != "report") {
        return None;
    }
    if !matches!(args, [_, last] if last == "--last")
        && !matches!(args, [_, last, print] if last == "--last" && print == "--print")
    {
        eprintln!(
            "usage: lenz (or lenz-tui) report --last [--print]\n--print displays the report location and issue link without opening a browser"
        );
        return Some(ExitCode::from(2));
    }
    let result = last_report().and_then(|report| {
        println!(
            "Diagnostic report: {}\nReview the report before sharing.\nReport on GitHub: {}",
            clean(&report.path.to_string_lossy()),
            report.issue_url
        );
        if args.len() == 2 {
            crate::open::spawn(&crate::open::Launch {
                program: if cfg!(target_os = "macos") {
                    "open"
                } else {
                    "xdg-open"
                }
                .into(),
                args: vec![report.issue_url],
                cwd: None,
            })
            .map_err(|error| {
                io::Error::other(format!(
                    "Cannot open browser: {error}; open the printed GitHub link manually"
                ))
            })?;
        }
        Ok(())
    });
    Some(match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{}", clean(&error.to_string()));
            ExitCode::from(2)
        }
    })
}

/// One explicitly requested disk lookup at a time, owned by the shared core.
#[derive(Default)]
pub struct ReportRequest {
    inbox: Option<std::sync::mpsc::Receiver<io::Result<SavedReport>>>,
}
impl ReportRequest {
    pub fn busy(&self) -> bool {
        self.inbox.is_some()
    }
    pub fn start(&mut self) -> io::Result<bool> {
        if self.busy() {
            return Ok(false);
        }
        breadcrumb("Report last error");
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("lenz-report".into())
            .spawn(move || {
                let _ = tx.send(last_report());
            })?;
        self.inbox = Some(rx);
        Ok(true)
    }
    pub fn poll(&mut self) -> Option<io::Result<SavedReport>> {
        use std::sync::mpsc::TryRecvError;
        let result = match self.inbox.as_ref()?.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => Err(io::Error::other(
                "Report lookup was interrupted; use lenz diff report --last --print",
            )),
        };
        self.inbox = None;
        Some(result)
    }
}
fn last_report_in(dir: &Path) -> io::Result<SavedReport> {
    let paths = match report_paths(dir) {
        Ok(paths) => paths,
        Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(e),
    };
    let path = paths.into_iter().max().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "No saved Lenz error report; reports are created when an error occurs",
        )
    })?;
    let mut log = String::new();
    fs::File::open(&path)?
        .take(MAX_BYTES + 1)
        .read_to_string(&mut log)?;
    if log.len() as u64 > MAX_BYTES {
        return Err(io::Error::other("saved report exceeded its size limit"));
    }
    let issue_url = log.lines().rev().find_map(|line| line.strip_prefix("Report on GitHub: "))
        .filter(|url| url.starts_with(ISSUE) && url.len() < 8000 && !url.chars().any(char::is_control))
        .ok_or_else(|| io::Error::other("Saved report has no valid GitHub draft link; review it and report at https://github.com/aantoniadis/lenz/issues"))?.into();
    Ok(SavedReport { path, issue_url })
}

#[cfg(test)]
fn write_report(dir: &Path, frontend: &str, message: &str) -> io::Result<SavedReport> {
    persist(dir, frontend, "Panic", message, None, "test stack")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_dir_prefers_env_then_settings_then_state_then_home() {
        use std::ffi::OsString;
        let os = |s: &str| Some(OsString::from(s));
        let settings = Some(Path::new("/configured"));
        assert_eq!(report_dir_from(os("/env"), settings, os("/state"), os("/home")).unwrap(), PathBuf::from("/env"));
        assert_eq!(report_dir_from(None, settings, os("/state"), os("/home")).unwrap(), PathBuf::from("/configured"));
        if !cfg!(target_os = "macos") {
            assert_eq!(report_dir_from(None, None, os("/state"), os("/home")).unwrap(), PathBuf::from("/state/lenz/crashes"));
            assert_eq!(report_dir_from(None, None, None, os("/home")).unwrap(), PathBuf::from("/home/.local/state/lenz/crashes"));
        }
        assert!(report_dir_from(None, None, None, None).is_err());
    }
    use std::process::Command;

    #[test]
    fn report_cli_child() {
        let Ok(mode) = std::env::var("LENZ_TEST_REPORT") else {
            return;
        };
        if mode == "saved" {
            write_report(&report_dir().unwrap(), "GUI", "SECRET_RAW_PANIC").unwrap();
        }
        let args = ["report", "--last", "--print"].map(String::from);
        std::process::exit(
            if report_cli(&args).unwrap() == std::process::ExitCode::SUCCESS {
                0
            } else {
                2
            },
        );
    }

    #[test]
    fn cli_prints_saved_report_without_initializing_a_repository_or_browser() {
        for (mode, success) in [("saved", true), ("missing", false)] {
            let dir = tempfile::tempdir().unwrap();
            let output = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "diagnostics::tests::report_cli_child",
                    "--nocapture",
                ])
                .env("LENZ_TEST_REPORT", mode)
                .env("LENZ_LOG_DIR", dir.path())
                .output()
                .unwrap();
            assert_eq!(output.status.success(), success);
            let out = String::from_utf8_lossy(&output.stdout);
            if success {
                assert!(out.contains("Diagnostic report:"));
                assert!(out.contains("https://github.com/aantoniadis/lenz/issues/new?"));
                assert!(!out.contains("SECRET_RAW_PANIC"));
            } else {
                assert!(
                    String::from_utf8_lossy(&output.stderr).contains("No saved Lenz error report")
                );
            }
        }
    }

    #[test]
    fn reports_persist_original_failure_and_open_a_private_issue_draft() {
        let dir = tempfile::tempdir().unwrap();
        let saved = write_report(
            dir.path(),
            "GUI",
            "index out of bounds: private/path token=secret",
        )
        .unwrap();
        let log = std::fs::read_to_string(&saved.path).unwrap();
        assert!(log.contains("index out of bounds"));
        assert!(log.contains("GUI"));
        assert!(log.contains("Report on GitHub: https://github.com/aantoniadis/lenz/issues/new?"));
        assert!(!saved.issue_url.contains("private%2Fpath"));
        assert!(!saved.issue_url.contains("secret"));
        assert!(saved.issue_url.contains("%0A"));
        let retrieved = last_report_in(dir.path()).unwrap();
        assert_eq!(retrieved.path, saved.path);
        assert_eq!(retrieved.issue_url, saved.issue_url);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&saved.path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn retention_ignores_unrelated_files_and_incomplete_reports() {
        let dir = tempfile::tempdir().unwrap();
        let unrelated = dir.path().join("keep.log");
        std::fs::write(&unrelated, "keep").unwrap();
        std::fs::write(
            dir.path().join("crash-999999999999999999999999.pending"),
            "incomplete",
        )
        .unwrap();
        let mut latest = None;
        for _ in 0..14 {
            latest = Some(write_report(dir.path(), "TUI", "failure").unwrap());
        }
        assert_eq!(
            last_report_in(dir.path()).unwrap().path,
            latest.unwrap().path
        );
        let count = std::fs::read_dir(dir.path())
            .unwrap()
            .filter(|p| {
                p.as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with("crash-")
                    && p.as_ref()
                        .unwrap()
                        .path()
                        .extension()
                        .is_some_and(|e| e == "log")
            })
            .count();
        assert_eq!(count, 10);
        assert_eq!(std::fs::read_to_string(unrelated).unwrap(), "keep");
    }

    // Run in an isolated process: a hook must never replace the test runner's hook.
    #[test]
    fn panic_child() {
        let Ok(mode) = std::env::var("LENZ_TEST_PANIC") else {
            return;
        };
        install("TUI");
        breadcrumb("Reload comparison");
        if mode == "worker" {
            std::thread::Builder::new()
                .name("lenz-test-worker".into())
                .spawn(|| panic!("original worker failure"))
                .unwrap()
                .join()
                .unwrap();
        } else {
            panic!("original main failure");
        }
    }

    #[test]
    fn main_and_worker_panics_leave_a_report_without_backtrace_environment() {
        for mode in ["main", "worker"] {
            let dir = tempfile::tempdir().unwrap();
            let output = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "diagnostics::tests::panic_child", "--nocapture"])
                .env("LENZ_TEST_PANIC", mode)
                .env("LENZ_LOG_DIR", dir.path())
                .env_remove("RUST_BACKTRACE")
                .output()
                .unwrap();
            assert!(!output.status.success());
            let logs = std::fs::read_dir(dir.path())
                .unwrap()
                .filter_map(Result::ok)
                .map(|entry| std::fs::read_to_string(entry.path()).unwrap())
                .collect::<Vec<_>>()
                .join("\n");
            assert!(logs.contains(&format!("original {mode} failure")), "{logs}");
            assert!(logs.contains("Reload comparison"));
            assert!(logs.contains("diagnostics.rs:"));
            assert!(logs.contains("Backtrace:"));
            if mode == "worker" {
                assert!(logs.contains("lenz-test-worker"));
            }
            let last = last_report_in(dir.path()).unwrap();
            assert!(
                std::fs::read_to_string(last.path)
                    .unwrap()
                    .contains(&format!("original {mode} failure"))
            );
            assert!(String::from_utf8_lossy(&output.stderr).contains("report --last"));
        }
    }
}
