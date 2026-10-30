//! Bounded, cancellable stdin transport for fixed SQL assistance operations. Arguments never contain drafts.
use std::{io::{Read, Write}, path::Path, process::{Command, Stdio}, time::{Duration, Instant}};

pub(crate) struct Capture { pub limit: usize, pub diagnostics: bool }
impl From<usize> for Capture { fn from(limit: usize) -> Self { Self { limit, diagnostics: false } } }

pub(crate) fn run(executable: &Path, args: &[&str], input: &str, env: &[(String, String)], timeout: Duration, capture_policy: impl Into<Capture>, current: &dyn Fn() -> bool) -> Result<String, String> {
    let policy = capture_policy.into();
    if !current() { return Err("SQL request cancelled".into()); }
    let mut command = Command::new(executable);
    command.args(args).envs(env.iter().map(|(k,v)| (k,v))).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    let mut child = command.spawn().map_err(|_| "cannot start the configured SQL tool".to_owned())?;
    let (Some(mut stdin), Some(stdout), Some(stderr)) = (child.stdin.take(), child.stdout.take(), child.stderr.take()) else {
        crate::semantics::kill_group(&mut child); let _ = child.wait(); return Err("cannot open SQL tool pipes".into());
    };
    fn capture(mut pipe: impl Read + Send + 'static, limit: usize) -> std::sync::mpsc::Receiver<(Vec<u8>, bool)> {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let (mut bytes, mut truncated) = (Vec::new(), false);
            let mut buffer = [0u8; 4096];
            while let Ok(count) = pipe.read(&mut buffer) {
                if count == 0 { break; }
                let kept = count.min(limit.saturating_sub(bytes.len()));
                bytes.extend(buffer.iter().take(kept)); truncated |= count > kept;
            }
            let _ = tx.send((bytes, truncated));
        }); rx
    }
    let output = capture(stdout, policy.limit);
    let errors = capture(stderr, 8192);
    let text = input.to_owned();
    let writer = std::thread::spawn(move || { let _ = stdin.write_all(text.as_bytes()); });
    let until = Instant::now() + timeout;
    let status = loop {
        if !current() { break Err("SQL request cancelled".to_owned()); }
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Err(_) => break Err("cannot wait for SQL tool".to_owned()),
            _ if Instant::now() >= until => break Err("SQL tool request timed out".to_owned()),
            _ => std::thread::sleep(Duration::from_millis(5)),
        }
    };
    crate::semantics::kill_group(&mut child); let _ = child.wait(); let _ = writer.join();
    let status = status?;
    let diagnostic = errors.recv_timeout(Duration::from_secs(1)).ok();
    if !status.success() {
        if policy.diagnostics && let Some((bytes, truncated)) = diagnostic {
            let detail: String = String::from_utf8_lossy(&bytes).chars().filter(|c| !c.is_control() || matches!(c, '\n' | '\t')).collect();
            if !detail.trim().is_empty() { return Err(format!("SQL tool exited with {status}:\n{}{}", detail.trim(), if truncated { "\n[diagnostic truncated]" } else { "" })); }
        }
        return Err("SQL tool failed (no document or connection details included)".into());
    }
    let (bytes, truncated) = output.recv_timeout(Duration::from_secs(1)).map_err(|_| "SQL tool output did not end".to_owned())?;
    if truncated { return Err("SQL tool output exceeded its limit".into()); }
    String::from_utf8(bytes).map_err(|_| "SQL tool output was not UTF-8".into())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[test]
    fn diagnostics_are_explicit_and_bounded() {
        let args = ["-c", "printf 'extension missing\\n' >&2; exit 1"];
        let private = run(Path::new("/bin/sh"), &args, "", &[], Duration::from_secs(2), 1024, &|| true).unwrap_err();
        assert!(!private.contains("extension missing"));
        let detail = run(Path::new("/bin/sh"), &args, "", &[], Duration::from_secs(2), Capture { limit: 1024, diagnostics: true }, &|| true).unwrap_err();
        assert!(detail.contains("extension missing"), "{detail}");
    }
    #[test]
    fn sql_transport_bounds_output_and_cancels_a_blocked_child() {
        let args = ["-c", "head -c 131072 /dev/zero"];
        assert!(run(Path::new("/bin/sh"), &args, "", &[], Duration::from_secs(2), 1024, &|| true).unwrap_err().contains("output exceeded"));
        let start = Instant::now();
        let current = || start.elapsed() < Duration::from_millis(100);
        assert!(run(Path::new("/bin/sh"), &["-c", "sleep 30"], "draft", &[], Duration::from_secs(2), 1024, &current).unwrap_err().contains("cancelled"));
        assert!(start.elapsed() < Duration::from_secs(2));
    }
}
