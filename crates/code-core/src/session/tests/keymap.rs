use super::*;
use crate::actions::{CatalogAction, InterfaceAction};
use crate::keymap::{Chord, KeyPress, KeymapPreset};

fn run(s: &mut Session, id: CommandId) {
    let CatalogAction::Command(command) = s.action(id) else { panic!("{id:?} is not a command: {:?}", s.action(id)) };
    s.dispatch(command);
}

fn press(s: &Session, text: &str) -> Option<CommandId> {
    let chord = Chord::parse(text).unwrap();
    s.key_command(&KeyPress::new(&chord.key, chord.ctrl, chord.shift, chord.alt).unwrap())
}

fn settings_file(dir: &Path) -> PathBuf {
    dir.join("state/lenz-code/settings.json")
}

/// The saved settings, or `null` before the background save has written them.
fn settings(dir: &Path) -> serde_json::Value {
    std::fs::read(settings_file(dir)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(serde_json::Value::Null)
}

fn hint(s: &Session, id: CommandId) -> Option<String> {
    s.actions().into_iter().find(|a| a.id == id).and_then(|a| a.shortcut)
}

#[test]
fn the_default_keymap_is_intellij_and_menus_and_keys_share_its_chords() {
    let d = tempfile::tempdir().unwrap();
    let mut s = session(d.path());
    assert_eq!(s.keymap().preset(), KeymapPreset::IntelliJ);
    assert_eq!(hint(&s, CommandId::FindAction).as_deref(), Some("Ctrl+Shift+A"));
    assert_eq!(hint(&s, CommandId::Reload).as_deref(), Some("Ctrl+R"));
    assert_eq!(hint(&s, CommandId::Save).as_deref(), Some("Ctrl+S"));
    assert_eq!(hint(&s, CommandId::Copy).as_deref(), Some("Ctrl+C"), "editing keys stay where adapters map them");
    // The label of an action never disagrees with what its key runs.
    for id in CommandId::ALL {
        if let Some(chord) = s.keymap().chords(id).first() {
            assert_eq!(s.key_command(&KeyPress::new(&chord.key, chord.ctrl, chord.shift, chord.alt).unwrap()), Some(id), "{id:?}");
        }
    }
    assert_eq!(s.action(CommandId::Keymap), CatalogAction::Interface(InterfaceAction::Keymap));
    assert_eq!(s.on(CommandId::KeymapIntelliJ), Some(true));
    assert_eq!(s.disabled(CommandId::KeymapIntelliJ), Some("The IntelliJ keymap is already in use"));
    assert_eq!(s.disabled(CommandId::ResetKeymap), Some("No shortcut is modified"));

    // VS Code: the same commands, VS Code's chords where it has them; menus follow.
    run(&mut s, CommandId::KeymapVsCode);
    assert_eq!(s.keymap().preset(), KeymapPreset::VsCode);
    assert_eq!(hint(&s, CommandId::FindAction).as_deref(), Some("Ctrl+Shift+P"));
    assert_eq!(hint(&s, CommandId::Close).as_deref(), Some("Ctrl+W"));
    assert_eq!(hint(&s, CommandId::LanguageServer), None, "F2 renames in VS Code; the status stays in the menus");
    assert_eq!(press(&s, "Ctrl+W"), Some(CommandId::Close));
    assert_eq!(press(&s, "Ctrl+F4"), None);
    assert_eq!(s.on(CommandId::KeymapVsCode), Some(true));
    assert_eq!(s.on(CommandId::KeymapIntelliJ), Some(false));
    settle(&mut s, |_| settings(d.path())["keymap"] == serde_json::json!("vs_code"));
}

#[test]
fn a_chord_another_command_runs_is_refused_with_the_reason_and_changes_nothing() {
    let d = tempfile::tempdir().unwrap();
    let mut s = session(d.path());
    s.dispatch(Command::SetKeybinding { command: CommandId::Save, chord: "Ctrl+K".into() });
    assert!(s.state().messages.iter().any(|m| m == "Ctrl+K already runs Search Everywhere…; reset that shortcut before giving it to Save"), "{:?}", s.state().messages);
    assert_eq!(hint(&s, CommandId::Save).as_deref(), Some("Ctrl+S"));
    assert!(s.keymap().overrides().is_empty());
    // A bare key is not a shortcut: it would run while typing.
    s.dispatch(Command::SetKeybinding { command: CommandId::Save, chord: "s".into() });
    assert!(s.state().messages.iter().any(|m| m.starts_with("Save: a shortcut needs Ctrl, Alt or a function key")), "{:?}", s.state().messages);
    assert!(s.keymap().overrides().is_empty());
}

#[test]
fn an_override_persists_with_the_settings_and_returns_with_the_next_session() {
    let d = tempfile::tempdir().unwrap();
    let mut s = session(d.path());
    s.dispatch(Command::SetKeybinding { command: CommandId::Save, chord: "Ctrl+Alt+S".into() });
    assert!(s.state().messages.iter().any(|m| m == "Save: Ctrl+Alt+S"), "{:?}", s.state().messages);
    assert_eq!(hint(&s, CommandId::Save).as_deref(), Some("Ctrl+Alt+S"));
    assert_eq!(press(&s, "Ctrl+Alt+S"), Some(CommandId::Save));
    assert_eq!(press(&s, "Ctrl+S"), None, "the override replaces the preset's chord");
    settle(&mut s, |_| settings(d.path())["keybindings"]["save"] == serde_json::json!("Ctrl+Alt+S"));
    drop(s);

    let mut s = session(d.path());
    assert_eq!(hint(&s, CommandId::Save).as_deref(), Some("Ctrl+Alt+S"));
    assert_eq!(press(&s, "Ctrl+Alt+S"), Some(CommandId::Save));
    // Reset returns the command to the preset and forgets the override.
    s.dispatch(Command::ResetKeybinding(CommandId::Save));
    assert!(s.state().messages.iter().any(|m| m == "Save: Ctrl+S (IntelliJ)"), "{:?}", s.state().messages);
    assert_eq!(press(&s, "Ctrl+S"), Some(CommandId::Save));
    settle(&mut s, |_| settings(d.path()).get("keybindings").is_none());
    // Resetting a command without an override changes nothing.
    s.dispatch(Command::ResetKeybinding(CommandId::Save));
    assert_eq!(press(&s, "Ctrl+S"), Some(CommandId::Save));
}

#[test]
fn a_preset_change_names_the_binding_an_override_displaced_and_reset_brings_it_back() {
    let d = tempfile::tempdir().unwrap();
    let mut s = session(d.path());
    s.dispatch(Command::SetKeybinding { command: CommandId::ShowCommit, chord: "Ctrl+T".into() });
    run(&mut s, CommandId::KeymapVsCode);
    assert_eq!(press(&s, "Ctrl+T"), Some(CommandId::ShowCommit));
    assert_eq!(hint(&s, CommandId::SearchEverywhere), None);
    let message = s.state().messages.last().cloned().unwrap_or_default();
    assert_eq!(message, "Keymap: VS Code (your overrides are kept). Ctrl+T now runs Show Commit…; Search Everywhere… stays in the menus", "{:?}", s.state().messages);
    run(&mut s, CommandId::ResetKeymap);
    assert_eq!(press(&s, "Ctrl+T"), Some(CommandId::SearchEverywhere));
    assert!(s.keymap().overrides().is_empty());
    assert!(s.disabled(CommandId::ResetKeymap).is_some());
}

#[test]
fn keymap_rows_group_commands_and_mark_overrides_and_displaced_bindings() {
    let d = tempfile::tempdir().unwrap();
    let mut s = session(d.path());
    let rows = s.keymap().rows();
    let row = |id: CommandId| rows.iter().find(|r| r.command == id).cloned().unwrap();
    assert_eq!((row(CommandId::Save).chords.as_str(), row(CommandId::Save).note.as_deref()), ("Ctrl+S", None));
    assert_eq!(row(CommandId::FindFile).chords, "Ctrl+Shift+N, Ctrl+P");
    assert!(rows.iter().all(|r| r.command != CommandId::Undo), "editing keys adapters own are not in the keymap");
    // Groups follow the menus: every File row comes before every View row.
    let group_at = |group| rows.iter().position(|r| r.command.group() == group).unwrap();
    assert!(group_at(CommandId::Open.group()) <= group_at(CommandId::SoftWrap.group()));
    let ordered: Vec<_> = rows.iter().map(|r| r.command.group()).collect();
    let mut sorted = ordered.clone();
    sorted.sort_by_key(|g| crate::actions::CommandGroup::ALL.iter().position(|a| a == g));
    assert_eq!(ordered, sorted);

    s.dispatch(Command::SetKeybinding { command: CommandId::Save, chord: "Ctrl+Alt+S".into() });
    let rows = s.keymap().rows();
    let save = rows.iter().find(|r| r.command == CommandId::Save).unwrap();
    assert_eq!((save.chords.as_str(), save.note.as_deref()), ("Ctrl+Alt+S", Some("your choice")));
}

#[test]
fn settings_the_reader_cannot_use_are_left_out_instead_of_failing_the_load() {
    let d = tempfile::tempdir().unwrap();
    let dir = d.path().to_path_buf();
    session(&dir);
    std::fs::create_dir_all(settings_file(&dir).parent().unwrap()).unwrap();
    std::fs::write(settings_file(&dir), serde_json::json!({
        "keymap": "vs_code",
        "keybindings": { "save": "Ctrl+Alt+S", "no_such_command": "Ctrl+F1", "reload": "K", "search_everywhere": "none" },
    }).to_string()).unwrap();
    let s = session(&dir);
    assert_eq!(s.keymap().preset(), KeymapPreset::VsCode);
    assert_eq!(hint(&s, CommandId::Save).as_deref(), Some("Ctrl+Alt+S"));
    assert_eq!(hint(&s, CommandId::Reload).as_deref(), Some("F5"), "a bare key is not a shortcut");
    assert_eq!(hint(&s, CommandId::SearchEverywhere), None, "`none` leaves the command to the menus");
    assert_eq!(s.keymap().overrides().len(), 2, "the unknown command and the bare key are left out");
    // An unknown preset name falls back to the default.
    std::fs::write(settings_file(&dir), serde_json::json!({ "keymap": "emacs" }).to_string()).unwrap();
    assert_eq!(session(&dir).keymap().preset(), KeymapPreset::IntelliJ);
}
