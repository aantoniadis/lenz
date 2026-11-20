//! The folder-trust dialog (Restricted mode) through the terminal's real input: the held-back server's trigger, the
//! Restricted status item and the command menu open it; it takes input before the editor; Enter and Escape stay
//! restricted; only Alt+T trusts.
use super::*;
use crate::app::{Details, Prompt};
use code_core::semantics::{provider::Trust, Status};

/// A Rust project in a folder never asked about, whose rust-analyzer (the fake) leaves `marker` when it runs.
fn restricted_app(dir: &Path) -> (App, std::path::PathBuf, std::path::PathBuf) {
    let root = dir.join("project");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"p\"\n").unwrap();
    let file = root.join("src/main.rs");
    std::fs::write(&file, "fn main() {}\n").unwrap();
    let profile = std::env::current_exe().unwrap().parent().unwrap().parent().unwrap().to_owned();
    let mut build = std::process::Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    build.args(["build", "-q", "-p", "code-core", "--example", "fake_lsp"]).current_dir(env!("CARGO_MANIFEST_DIR"));
    if profile.ends_with("release") { build.arg("--release"); }
    assert!(build.status().unwrap().success());
    let marker = dir.join("server-ran");
    let server = dir.join("server");
    std::fs::write(&server, format!("#!/bin/sh\ntouch '{}'\nexec '{}' \"$@\"\n", marker.display(), profile.join("examples/fake_lsp").display())).unwrap();
    std::fs::set_permissions(&server, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let cfg = config(dir);
    std::fs::create_dir_all(cfg.providers.parent().unwrap()).unwrap();
    std::fs::write(&cfg.providers, serde_json::json!({ "providers": { "rust-analyzer": { "executable": server, "env": { "FAKE_LSP_NO_DECOR": "1" } } } }).to_string()).unwrap();
    let mut a = App::with_text_input(cfg, Some(root), true);
    settle(&mut a, |a| a.session.state().project().is_some());
    (a, file, marker)
}

fn trust_open(a: &App) -> bool {
    matches!(a.modal.as_ref().map(|m| &m.kind), Some(Prompt::Details { kind: Details::Trust, .. }))
}

fn text(a: &App) -> String {
    a.session.state().current_source().map(|s| s.copy_range(0..s.len_bytes()).unwrap()).unwrap_or_default()
}

#[test]
fn the_folder_trust_dialog_takes_input_first_and_enter_or_escape_stay_restricted() {
    let dir = tempfile::tempdir().unwrap();
    let (mut a, file, marker) = restricted_app(dir.path());
    assert!(!trust_open(&a), "opening a folder runs nothing, so asks nothing");
    // The real trigger: the held-back server.
    a.dispatch(Command::Open(file.clone()));
    settle(&mut a, |a| trust_open(a) && a.session.state().current_source().is_some());
    // It may open while the user types: keys and pastes reach the dialog, never the document behind it, and a plain
    // t (or T) does not trust.
    for (c, m) in [('x', KeyModifiers::NONE), ('t', KeyModifiers::NONE), ('T', KeyModifiers::SHIFT), ('p', KeyModifiers::NONE)] { key(&mut a, KeyCode::Char(c), m); }
    key(&mut a, KeyCode::Backspace, KeyModifiers::NONE);
    a.handle(Event::Paste("pasted".into()));
    assert!(trust_open(&a));
    assert_eq!(a.session.folder_trust(), Some(Trust::Unasked));
    assert_eq!(text(&a), "fn main() {}\n");
    // A 40-column terminal: the text wraps, the buttons stay, and the end scrolls into view.
    let screen = draw_screen(&mut a, 40, 16);
    assert!(screen.contains("Stay Restricted") && screen.contains("Lenz Code runs"), "{screen}");
    key(&mut a, KeyCode::End, KeyModifiers::NONE);
    let screen = draw_screen(&mut a, 40, 16);
    assert!(screen.contains("status bar asks") && screen.contains("again.") && screen.contains("[Trust Alt+T]"), "{screen}");
    assert!(trust_open(&a));
    // Enter stays restricted, remembered as declined; nothing ran and the file is as it was.
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    assert!(!trust_open(&a), "{:?} {:?} {:?}", a.session.state().notices(), a.session.folder_trust(), a.session.trust_text());
    assert_eq!(a.session.folder_trust(), Some(Trust::Declined));
    settle(&mut a, |a| a.session.state().provider().is_some_and(|v| matches!(v.status, Status::Restricted(_))));
    assert_eq!(text(&a), "fn main() {}\n");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "fn main() {}\n");
    assert!(!marker.exists());
    // The Restricted status item opens it again; Escape stays restricted.
    settle(&mut a, |a| a.session.disabled(CommandId::FolderTrust).is_none());
    draw(&mut a, 100, 24);
    let (_, rect) = *a.layout.controls.iter().find(|(id, _)| *id == CommandId::FolderTrust).expect("the Restricted status item");
    assert!(draw_screen(&mut a, 100, 24).contains("Restricted"));
    click(&mut a, rect.x + 1, rect.y);
    draw(&mut a, 100, 24);
    assert!(trust_open(&a), "the status item opens the dialog");
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    assert!(!trust_open(&a));
    assert_eq!(a.session.folder_trust(), Some(Trust::Declined));
    assert!(!marker.exists());
    // The command menu opens it (once the answer is saved); Alt+T trusts, and the held-back server starts without
    // reopening anything.
    settle(&mut a, |a| a.session.disabled(CommandId::FolderTrust).is_none());
    key(&mut a, KeyCode::F(10), KeyModifiers::NONE);
    a.handle(Event::Paste("Folder Trust".into()));
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    draw(&mut a, 100, 24);
    assert!(trust_open(&a), "the command menu opens the dialog: {}", a.modal_title());
    // Ctrl+T is not Alt+T: only Alt with the letter trusts.
    key(&mut a, KeyCode::Char('t'), KeyModifiers::CONTROL);
    assert!(trust_open(&a) && a.session.folder_trust() == Some(Trust::Declined));
    key(&mut a, KeyCode::Char('t'), KeyModifiers::ALT);
    assert!(!trust_open(&a));
    assert_eq!(a.session.folder_trust(), Some(Trust::Trusted));
    settle(&mut a, |a| marker.exists() && a.session.state().provider().is_some_and(|v| matches!(v.status, Status::Ready | Status::Indexing(_))));
    draw(&mut a, 100, 24);
    assert!(!a.layout.controls.iter().any(|(id, _)| *id == CommandId::FolderTrust), "no Restricted item in a trusted folder");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "fn main() {}\n");
}
