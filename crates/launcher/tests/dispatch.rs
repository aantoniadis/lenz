use std::{
    ffi::OsString,
    fs,
    os::unix::{ffi::OsStringExt, fs::PermissionsExt},
    process::{Command, Stdio},
};

fn fixture() -> tempfile::TempDir {
    let dir = tempfile::Builder::new().prefix("lenz launcher ").tempdir().unwrap();
    fs::copy(env!("CARGO_BIN_EXE_lenz-launcher"), dir.path().join("lenz")).unwrap();
    for name in ["lenz-code", "lenz-diff", "lenz-diff-tui"] {
        let path = dir.path().join(name);
        fs::write(&path, "#!/bin/sh\nprintf '%s\\0' \"$0\" \"$$\" \"$PWD\" \"$LENZ_LAUNCHER_TEST\" \"$@\"\nexit 37\n").unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    dir
}

#[test]
fn dispatch_preserves_arguments_directory_environment_pid_and_exit_status() {
    let dir = fixture();
    let cases: &[(&[&str], &str, &[&str])] = &[
        (&["a b.rs"], "lenz-code", &["a b.rs"]),
        (&[""], "lenz-code", &[""]),
        (&["code"], "lenz-code", &["code"]),
        (&["log"], "lenz-code", &["log"]),
        (&["merge"], "lenz-code", &["merge"]),
        (&["./diff"], "lenz-code", &["./diff"]),
        (&["diff", "--no-index", "left", "right"], "lenz-diff", &["--no-index", "left", "right"]),
        (&["diff", "--tui", "merge", "base", "ours", "theirs", "result"], "lenz-diff-tui", &["merge", "base", "ours", "theirs", "result"]),
        (&["diff", "--name", "--tui", "left", "right"], "lenz-diff", &["--name", "--tui", "left", "right"]),
        (&["diff", "--tui", "--", "--tui"], "lenz-diff-tui", &["--", "--tui"]),
        (&["diff", "--", "--tui"], "lenz-diff", &["--", "--tui"]),
        (&["diff", "log", "--follow", "code"], "lenz-diff", &["log", "--follow", "code"]),
        (&[], "lenz-code", &[]),
    ];
    for (args, binary, forwarded) in cases {
        let mut args: Vec<OsString> = args.iter().map(OsString::from).collect();
        let mut expected: Vec<OsString> = forwarded.iter().map(OsString::from).collect();
        let raw = OsString::from_vec(b"raw-\xff.rs".to_vec());
        if !args.is_empty() {
            args.push(raw.clone());
            expected.push(raw);
        }
        let child = Command::new(dir.path().join("lenz"))
            .args(args).current_dir(dir.path()).env("LENZ_LAUNCHER_TEST", "preserved")
            .stdout(Stdio::piped()).spawn().unwrap();
        let pid = child.id();
        let out = child.wait_with_output().unwrap();
        assert_eq!(out.status.code(), Some(37), "{binary}: dispatch must preserve exit status");
        let fields: Vec<_> = out.stdout.strip_suffix(&[0]).unwrap().split(|&b| b == 0).map(|b| OsString::from_vec(b.to_vec())).collect();
        assert_eq!(fields[0], dir.path().join(binary));
        assert_eq!(fields[1], pid.to_string().as_str(), "exec must preserve the launcher PID");
        assert_eq!(fields[2], dir.path().canonicalize().unwrap());
        assert_eq!(fields[3], "preserved");
        assert_eq!(&fields[4..], expected);
    }
}

#[test]
fn help_does_not_need_payloads_and_missing_or_unexecutable_payloads_fail_clearly() {
    let dir = fixture();
    fs::remove_file(dir.path().join("lenz-code")).unwrap();
    let run = |args: &[&str]| Command::new(dir.path().join("lenz")).args(args).output().unwrap();
    let out = run(&["--help"]);
    assert!(out.status.success());
    let help = String::from_utf8(out.stdout).unwrap();
    for command in ["lenz [file-or-folder]", "lenz diff", "--tui"] { assert!(help.contains(command)); }
    let out = run(&["a.rs"]);
    assert_eq!(out.status.code(), Some(127));
    assert!(String::from_utf8_lossy(&out.stderr).contains("lenz-code"));
    fs::set_permissions(dir.path().join("lenz-diff"), fs::Permissions::from_mode(0o644)).unwrap();
    let out = run(&["diff"]);
    assert_eq!(out.status.code(), Some(126));
    assert!(String::from_utf8_lossy(&out.stderr).contains("lenz-diff"));
}
