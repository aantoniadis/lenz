//! `--version` prints this executable's installed name and version, and starts nothing.
use std::process::{Command, Stdio};

#[test]
fn version_prints_the_name_and_version_and_starts_nothing() {
    for flag in ["--version", "-V"] {
        let out = Command::new(env!("CARGO_BIN_EXE_lenz-code")).arg(flag).stdin(Stdio::null()).output().unwrap();
        assert!(out.status.success(), "{flag}: {out:?}");
        assert_eq!(String::from_utf8_lossy(&out.stdout), format!("lenz-code {}\n", env!("CARGO_PKG_VERSION")), "{flag}");
    }
}

#[test]
fn help_and_unknown_options_never_open_a_window() {
    let run = |args: &[&str]| Command::new(env!("CARGO_BIN_EXE_lenz-code")).args(args).stdin(Stdio::null()).output().unwrap();
    let help = run(&["--help"]);
    assert!(help.status.success() && String::from_utf8_lossy(&help.stdout).starts_with("Usage: lenz-code"), "{help:?}");
    let unknown = run(&["--bogus"]);
    assert_eq!(unknown.status.code(), Some(2), "{unknown:?}");
    assert!(String::from_utf8_lossy(&unknown.stderr).contains("lenz-code: unknown option --bogus"), "{unknown:?}");
    let two = run(&["a", "b"]);
    assert_eq!(two.status.code(), Some(2), "{two:?}");
}
