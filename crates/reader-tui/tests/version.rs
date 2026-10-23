//! `--version` prints this executable's installed name and version, and starts nothing.
use std::process::{Command, Stdio};

#[test]
fn version_prints_the_name_and_version_and_starts_nothing() {
    for flag in ["--version", "-V"] {
        let out = Command::new(env!("CARGO_BIN_EXE_lenz-reader-tui")).arg(flag).stdin(Stdio::null()).output().unwrap();
        assert!(out.status.success(), "{flag}: {out:?}");
        assert_eq!(String::from_utf8_lossy(&out.stdout), format!("lenz-code-tui {}\n", env!("CARGO_PKG_VERSION")), "{flag}");
    }
}
