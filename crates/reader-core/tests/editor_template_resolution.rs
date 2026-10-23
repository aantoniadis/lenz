//! Disposable template/provider experiment; no production editor route is installed.
#![cfg(unix)]

use std::{path::Path, process::Command};

fn run_python(arguments: &[&str]) {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/template_probe");
    let output = Command::new("python3")
        .current_dir(directory)
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .args(arguments)
        .output()
        .expect("Python 3 is required for the template experiment");
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn template_projection_source_edit_contract() {
    run_python(&["-m", "unittest", "discover", "-v"]);
}

#[test]
#[ignore = "requires private Haml 7.5.1 gems, Ruby LSP 0.26.11, bundled HTML server and explicit evidence directory"]
fn template_real_provider_resolution() {
    let evidence = std::env::var("EDITOR_TEMPLATE_EVIDENCE_DIR")
        .expect("set EDITOR_TEMPLATE_EVIDENCE_DIR to a fresh owned directory");
    run_python(&[
        "providers.py",
        &evidence,
        "utf-8",
        "--private-fix",
        "--require-complete-signatures",
    ]);
}
