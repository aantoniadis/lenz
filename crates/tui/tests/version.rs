//! `--version` prints this executable's installed name and version, and starts nothing.
use std::process::{Command, Stdio};

#[test]
fn version_prints_the_name_and_version_and_starts_nothing() {
    for flag in ["--version", "-V"] {
        let out = Command::new(env!("CARGO_BIN_EXE_lenz-tui")).arg(flag).stdin(Stdio::null()).output().unwrap();
        assert!(out.status.success(), "{flag}: {out:?}");
        assert_eq!(String::from_utf8_lossy(&out.stdout), format!("lenz-diff-tui {}\n", env!("CARGO_PKG_VERSION")), "{flag}");
    }
}

#[test]
fn help_documents_commit_and_new_comparison_without_starting_a_terminal() {
    for flag in ["--help", "-h"] {
        let out = Command::new(env!("CARGO_BIN_EXE_lenz-tui")).arg(flag).stdin(Stdio::null()).output().unwrap();
        assert!(out.status.success(), "{flag}: {out:?}");
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(text.contains("lenz diff --tui show <commit>"), "{text}");
        assert!(text.contains("lenz diff --tui --new-comparison"), "{text}");
        assert!(out.stderr.is_empty(), "{out:?}");
    }
}
