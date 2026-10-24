//! Keymap presets, overrides and their terminal delivery, through the real key path.
use super::*;
use reader_core::keymap::{Chord, KeyPress, KeymapPreset};

fn press(app: &App, text: &str) -> Option<CommandId> {
    let chord = Chord::parse(text).unwrap();
    app.session.key_command(&KeyPress::new(&chord.key, chord.ctrl, chord.shift, chord.alt).unwrap())
}

fn open_app(dir: &Path, name: &str) -> (App, std::path::PathBuf) {
    let path = dir.join(name);
    std::fs::write(&path, "one\ntwo\nthree\n").unwrap();
    let mut a = app(dir);
    a.dispatch(Command::OpenRetained(path.clone()));
    settle(&mut a, |a| a.session.state().current_source().is_some());
    draw(&mut a, 80, 24);
    (a, path)
}

fn hint(app: &App, id: CommandId) -> Option<String> {
    app.session.key_hint(id)
}

#[test]
fn keymap_presets_change_live_dispatch_and_the_labels_next_to_commands() {
    let d = tempfile::tempdir().unwrap();
    let (mut a, path) = open_app(d.path(), "keys.txt");
    assert_eq!(a.session.keymap().preset(), KeymapPreset::IntelliJ);
    assert_eq!(hint(&a, CommandId::FindAction).as_deref(), Some("Ctrl+Shift+A"));
    // IntelliJ's chords reach the same commands as before.
    key(&mut a, KeyCode::Char('k'), KeyModifiers::CONTROL);
    assert!(matches!(a.modal.as_ref().map(|m| &m.kind), Some(Prompt::Navigation)), "Ctrl+K searches everywhere");
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    key(&mut a, KeyCode::Char('w'), KeyModifiers::CONTROL);
    assert!(a.session.state().tabs().len() == 1 && a.session.disabled(CommandId::Undo).is_some(), "Ctrl+W extends the selection, not closing");
    assert_eq!(std::fs::read(&path).unwrap(), b"one\ntwo\nthree\n");

    // VS Code: Ctrl+W closes the tab and Ctrl+F4 no longer does; menus follow the same resolution.
    a.run(CommandId::KeymapVsCode);
    assert_eq!(a.session.keymap().preset(), KeymapPreset::VsCode);
    assert_eq!(hint(&a, CommandId::FindAction).as_deref(), Some("Ctrl+Shift+P"));
    assert_eq!(hint(&a, CommandId::Close).as_deref(), Some("Ctrl+W"));
    assert_eq!(press(&a, "Ctrl+F4"), None);
    key(&mut a, KeyCode::Char('w'), KeyModifiers::CONTROL);
    assert!(a.session.state().tabs().is_empty(), "Ctrl+W closes the tab under the VS Code keymap");
    assert_eq!(std::fs::read(&path).unwrap(), b"one\ntwo\nthree\n", "closing a clean tab writes nothing");
    draw(&mut a, 80, 24);
    let json: serde_json::Value = serde_json::from_slice(&std::fs::read(d.path().join("config/settings.json")).unwrap()).unwrap_or(serde_json::Value::Null);
    assert!(json.is_null() || json["keymap"] == serde_json::json!("vs_code"), "{json}");
}

#[test]
fn type_hierarchy_and_the_language_status_keep_deliverable_terminal_chords() {
    let d = tempfile::tempdir().unwrap();
    let (mut a, _) = open_app(d.path(), "types.txt");
    // Ctrl+H arrives as Backspace in terminals, so Alt+H is the route here; both are the same semantic command.
    assert_eq!(hint(&a, CommandId::TypeHierarchy).as_deref(), Some("Ctrl+H"));
    assert_eq!(a.session.keymap().labels(CommandId::TypeHierarchy).as_deref(), Some("Ctrl+H, Alt+H"));
    key(&mut a, KeyCode::Char('h'), KeyModifiers::ALT);
    assert_eq!(a.notice.as_deref(), Some("No language server for this language"));
    // F2 language status and Shift+F2 its details stay distinct chords.
    key(&mut a, KeyCode::F(2), KeyModifiers::SHIFT);
    assert!(matches!(a.modal.as_ref().map(|m| &m.kind), Some(Prompt::Details { .. })));
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    key(&mut a, KeyCode::F(2), KeyModifiers::NONE);
    assert!(matches!(a.modal.as_ref().map(|m| &m.kind), Some(Prompt::LanguageServer)));
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
}

#[test]
fn an_undeliverable_shortcut_still_leaves_the_command_in_the_palette() {
    let d = tempfile::tempdir().unwrap();
    let (mut a, _) = open_app(d.path(), "palette.txt");
    a.run(CommandId::KeymapVsCode);
    assert_eq!(hint(&a, CommandId::Documentation), None, "VS Code has no quick documentation; Lenz does not invent one");
    // Ctrl+Q cannot be taken from the terminal's view close, so the dialog warns and the menu keeps the action.
    a.run(CommandId::Keymap);
    for c in "recent files".chars() {
        key(&mut a, KeyCode::Char(c), KeyModifiers::NONE);
    }
    let at = a.choices().iter().position(|c| c.label.starts_with("Recent Files… — ")).expect("Recent Files row");
    for _ in 0..at { key(&mut a, KeyCode::Down, KeyModifiers::NONE); }
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    key(&mut a, KeyCode::Char('q'), KeyModifiers::CONTROL);
    assert_eq!(a.session.keymap().labels(CommandId::RecentFiles).as_deref(), Some("Ctrl+Q"));
    assert!(a.session.state().notices().iter().any(|n| n.contains("not delivered by most terminals")), "{:?}", a.session.state().notices());
    a.close_modal();
    // The terminal keeps Ctrl+Q for closing the view; Recent Files still runs from the palette.
    key(&mut a, KeyCode::Char('q'), KeyModifiers::CONTROL);
    assert!(a.quit, "the terminal's view close is unchanged");
    a.quit = false;
    assert!(a.session.actions().iter().any(|action| action.id == CommandId::RecentFiles && action.shortcut.as_deref() == Some("Ctrl+Q")), "the menu still names the choice");
}

#[test]
fn the_keymap_dialog_filters_binds_refuses_taken_chords_and_restores() {
    let d = tempfile::tempdir().unwrap();
    let (mut a, path) = open_app(d.path(), "dialog.txt");
    a.run(CommandId::Keymap);
    assert!(matches!(a.modal.as_ref().map(|m| &m.kind), Some(Prompt::Keymap)));
    let before = a.session.state().current_source().unwrap().copy_range(0..a.session.state().current_source().unwrap().len_bytes());
    // Typing filters; it never edits or saves the document behind the dialog.
    for c in "save".chars() {
        key(&mut a, KeyCode::Char(c), KeyModifiers::NONE);
    }
    let at = a.choices().iter().position(|c| c.label.starts_with("Save — ")).expect("Save row");
    for _ in 0..at { key(&mut a, KeyCode::Down, KeyModifiers::NONE); }
    key(&mut a, KeyCode::Char('s'), KeyModifiers::CONTROL);
    assert_eq!(std::fs::read(&path).unwrap(), b"one\ntwo\nthree\n", "Ctrl+S in the dialog saves nothing");
    assert_eq!(a.session.state().current_source().unwrap().copy_range(0..a.session.state().current_source().unwrap().len_bytes()), before);
    // A bare printable key cannot become a shortcut.
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    key(&mut a, KeyCode::Char('x'), KeyModifiers::NONE);
    assert!(a.modal.as_ref().is_some_and(|m| m.error.is_some() && m.capture.is_some()), "the capture stays open with its reason");
    key(&mut a, KeyCode::Char('S'), KeyModifiers::CONTROL | KeyModifiers::ALT);
    assert_eq!(hint(&a, CommandId::Save).as_deref(), Some("Ctrl+Alt+S"));
    // A chord another command runs is refused with its owner named.
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    key(&mut a, KeyCode::Char('k'), KeyModifiers::CONTROL);
    assert!(a.session.state().notices().iter().any(|m| m.starts_with("Ctrl+K already runs")), "{:?}", a.session.state().notices());
    assert_eq!(hint(&a, CommandId::Save).as_deref(), Some("Ctrl+Alt+S"));
    // Delete restores the preset's chord.
    key(&mut a, KeyCode::Delete, KeyModifiers::NONE);
    assert_eq!(hint(&a, CommandId::Save).as_deref(), Some("Ctrl+S"));
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    assert!(a.modal.is_none());
    assert_eq!(std::fs::read(&path).unwrap(), b"one\ntwo\nthree\n");
}

#[test]
fn an_override_persists_and_survives_reopening_the_view() {
    let d = tempfile::tempdir().unwrap();
    let (mut a, _) = open_app(d.path(), "persist.txt");
    a.dispatch(Command::SetKeybinding { command: CommandId::FindFile, chord: "Ctrl+Alt+F".into() });
    assert_eq!(hint(&a, CommandId::FindFile).as_deref(), Some("Ctrl+Alt+F"));
    // The settings file is written in the background.
    let settings = d.path().join("config/settings.json");
    let deadline = Instant::now() + Duration::from_secs(5);
    while !std::fs::read(&settings).ok().and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .is_some_and(|v| v["keybindings"]["find_file"] == serde_json::json!("Ctrl+Alt+F"))
    {
        assert!(Instant::now() < deadline, "the override was not saved");
        std::thread::sleep(Duration::from_millis(5));
    }
    drop(a);
    let a = app(d.path());
    assert_eq!(hint(&a, CommandId::FindFile).as_deref(), Some("Ctrl+Alt+F"));
    assert_eq!(press(&a, "Ctrl+Alt+F"), Some(CommandId::FindFile));
}
