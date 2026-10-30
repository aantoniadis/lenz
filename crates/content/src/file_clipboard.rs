//! Desktop file clipboard publishing shared by the GUI and terminal reader.

use std::{path::Path, sync::atomic::AtomicBool};

/// Publish an entry as a file, for pasting into a desktop file manager.
///
/// Run on a worker: Linux clipboard tools may need up to two seconds to acquire
/// ownership. Cancel and finish an obsolete worker before starting another one.
pub fn copy_file(path: &Path, cancelled: &AtomicBool) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        use std::{process::Command, sync::atomic::Ordering, time::Duration};
        if cancelled.load(Ordering::Relaxed) {
            return Err("File clipboard copy cancelled".into());
        }
        let data = uri_list(path)?;
        std::fs::symlink_metadata(path).map_err(|error| format!("Cannot copy file: {error}"))?;
        let mut command = if std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty()) {
            let mut command = Command::new("wl-copy");
            command.args(["--type", "text/uri-list"]);
            command
        } else if std::env::var_os("DISPLAY").is_some_and(|v| !v.is_empty()) {
            let mut command = Command::new("xclip");
            command.args(["-selection", "clipboard", "-target", "text/uri-list"]);
            command
        } else {
            return Err("File clipboard needs a local Wayland or X11 desktop".into());
        };
        publish(
            &mut command,
            data.as_bytes(),
            cancelled,
            Duration::from_secs(2),
        )
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (path, cancelled);
        Err("Desktop file clipboard is currently supported on Linux only".into())
    }
}

#[cfg(target_os = "linux")]
fn uri_list(path: &Path) -> Result<String, String> {
    use std::os::unix::ffi::OsStrExt;
    if !path.is_absolute() {
        return Err("File clipboard needs an absolute path".into());
    }
    let mut uri = String::from("file://");
    for &byte in path.as_os_str().as_bytes() {
        if byte.is_ascii_alphanumeric() || b"/-._~".contains(&byte) {
            uri.push(char::from(byte));
        } else {
            const HEX: &[u8; 16] = b"0123456789ABCDEF";
            uri.push('%');
            uri.push(char::from(HEX[usize::from(byte >> 4)]));
            uri.push(char::from(HEX[usize::from(byte & 15)]));
        }
    }
    uri.push_str("\r\n");
    Ok(uri)
}

#[cfg(target_os = "linux")]
fn publish(
    command: &mut std::process::Command,
    data: &[u8],
    cancelled: &AtomicBool,
    timeout: std::time::Duration,
) -> Result<(), String> {
    use std::{
        io::{Seek, Write},
        os::unix::process::CommandExt,
        process::Stdio,
        sync::atomic::Ordering,
        time::{Duration, Instant},
    };
    if cancelled.load(Ordering::Relaxed) {
        return Err("File clipboard copy cancelled".into());
    }
    // A private file avoids blocking on a clipboard command that never reads stdin.
    let mut input =
        tempfile::tempfile().map_err(|error| format!("File clipboard input: {error}"))?;
    input
        .write_all(data)
        .and_then(|()| input.rewind())
        .map_err(|error| format!("File clipboard input: {error}"))?;
    let program = command.get_program().to_string_lossy().into_owned();
    let mut child = command
        .stdin(input)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .map_err(|error| format!("File clipboard requires {program}: {error}"))?;
    let started = Instant::now();
    let result = loop {
        if cancelled.load(Ordering::Relaxed) {
            break Err("File clipboard copy cancelled".into());
        }
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break Ok(()),
            Ok(Some(status)) => break Err(format!("File clipboard: {program} failed ({status})")),
            Err(error) => break Err(format!("File clipboard: {program}: {error}")),
            Ok(None) => {}
        }
        if started.elapsed() >= timeout {
            break Err(format!("File clipboard: {program} timed out"));
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    if result.is_err() {
        #[allow(unsafe_code)] // audited kill of this command's isolated process group
        // SAFETY: process_group(0) gives only this command and its children this group ID.
        unsafe {
            libc::kill(-(child.id() as libc::pid_t), libc::SIGKILL);
        }
        let _ = child.kill();
        let _ = child.wait();
    }
    // wl-copy/xclip fork after acquiring the clipboard; the owner exits when replaced.
    result
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::{
        fs, os::unix::ffi::OsStrExt, process::Command, sync::atomic::Ordering, time::Duration,
    };

    #[test]
    fn uri_list_encodes_file_names_without_losing_bytes() {
        let path = Path::new(std::ffi::OsStr::from_bytes(b"/tmp/a #?%\n\r\xff\xc3\xa9"));
        assert_eq!(
            uri_list(path).unwrap(),
            "file:///tmp/a%20%23%3F%25%0A%0D%FF%C3%A9\r\n"
        );
        assert!(uri_list(Path::new("relative")).is_err());
    }

    #[test]
    fn copy_file_uses_the_desktop_backend_and_file_mime_type() {
        use std::os::unix::fs::PermissionsExt;
        const SOURCE: &str = "LENZ_FILE_CLIPBOARD_TEST_SOURCE";
        if let Some(source) = std::env::var_os(SOURCE) {
            copy_file(Path::new(&source), &AtomicBool::new(false)).unwrap();
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("a # %\n$(touch unexpected)");
        fs::write(&source, "source bytes stay unchanged").unwrap();
        let output = temp.path().join("output");
        for tool in ["wl-copy", "xclip"] {
            let program = temp.path().join(tool);
            fs::write(&program, "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$LENZ_FILE_CLIPBOARD_TEST_OUTPUT.args\"\n/bin/cat > \"$LENZ_FILE_CLIPBOARD_TEST_OUTPUT\"\n").unwrap();
            fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
        }
        for (display, expected_args) in [
            ("WAYLAND_DISPLAY", "--type\ntext/uri-list\n"),
            ("DISPLAY", "-selection\nclipboard\n-target\ntext/uri-list\n"),
        ] {
            let result = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", std::thread::current().name().unwrap()])
                .env_remove("WAYLAND_DISPLAY")
                .env_remove("DISPLAY")
                .env(display, "fake-display")
                .env("PATH", temp.path())
                .env(SOURCE, &source)
                .env("LENZ_FILE_CLIPBOARD_TEST_OUTPUT", &output)
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stdout)
            );
            assert_eq!(
                fs::read(&output).unwrap(),
                uri_list(&source).unwrap().as_bytes()
            );
            assert_eq!(
                fs::read_to_string(output.with_extension("args")).unwrap(),
                expected_args
            );
        }
        assert_eq!(
            fs::read_to_string(source).unwrap(),
            "source bytes stay unchanged"
        );
        assert!(!temp.path().join("unexpected").exists());
    }

    #[test]
    fn publisher_streams_exact_payload_without_shell_interpolation() {
        let temp = tempfile::tempdir().unwrap();
        let output = temp.path().join("output");
        let mut command = Command::new("/bin/sh");
        command
            .args([
                "-c",
                "cat > \"$1\"; printf '%s\\n' \"$2\" > \"$1.args\"",
                "fake",
            ])
            .arg(&output)
            .arg("text/uri-list");
        let payload = b"file:///tmp/a%0A%23%25\r\n";
        publish(
            &mut command,
            payload,
            &AtomicBool::new(false),
            Duration::from_secs(1),
        )
        .unwrap();
        assert_eq!(fs::read(&output).unwrap(), payload);
        assert_eq!(
            fs::read(output.with_extension("args")).unwrap(),
            b"text/uri-list\n"
        );
    }

    #[test]
    fn publisher_reports_failure_and_does_not_start_cancelled_commands() {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "exit 7"]);
        assert!(
            publish(
                &mut command,
                b"",
                &AtomicBool::new(false),
                Duration::from_secs(1)
            )
            .is_err()
        );
        let temp = tempfile::tempdir().unwrap();
        let marker = temp.path().join("marker");
        let mut command = Command::new("/usr/bin/touch");
        command.arg(&marker);
        assert!(
            publish(
                &mut command,
                b"",
                &AtomicBool::new(true),
                Duration::from_secs(1)
            )
            .is_err()
        );
        assert!(!marker.exists());
    }

    #[test]
    fn publisher_times_out_and_cancels_the_whole_pending_process_group() {
        let temp = tempfile::tempdir().unwrap();
        for cancel in [false, true] {
            let marker = temp
                .path()
                .join(if cancel { "cancelled" } else { "timed-out" });
            let mut command = Command::new("/bin/sh");
            command
                .args(["-c", "(sleep 0.2; touch \"$1\") & wait", "fake"])
                .arg(&marker);
            let cancelled = AtomicBool::new(false);
            std::thread::scope(|scope| {
                if cancel {
                    scope.spawn(|| {
                        std::thread::sleep(Duration::from_millis(30));
                        cancelled.store(true, Ordering::Relaxed);
                    });
                }
                let error =
                    publish(&mut command, b"", &cancelled, Duration::from_millis(70)).unwrap_err();
                assert!(
                    error.contains(if cancel { "cancelled" } else { "timed out" }),
                    "{error}"
                );
            });
            std::thread::sleep(Duration::from_millis(220));
            assert!(!marker.exists());
        }
    }
}
