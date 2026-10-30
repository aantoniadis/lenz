use super::*;
use crate::actions::{CatalogAction, InterfaceAction};

const PROJECT: [CommandId; 5] = [CommandId::IndentTabs, CommandId::IndentTwoSpaces, CommandId::IndentFourSpaces, CommandId::IndentInherit, CommandId::FormatOnSave];

fn run(s: &mut Session, id: CommandId) {
    let CatalogAction::Command(command) = s.action(id) else { panic!("{id:?} is not a command: {:?}", s.action(id)) };
    s.dispatch(command);
}

fn project_files(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut files: Vec<_> = std::fs::read_dir(root).unwrap().map(|e| { let p = e.unwrap().path(); let b = std::fs::read(&p).unwrap(); (p, b) }).collect();
    files.sort();
    files
}

#[test]
fn project_editing_preferences_show_precedence_persist_outside_the_project_and_keep_explicit_formatting() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("p");
    std::fs::create_dir(&root).unwrap();
    write(&root, ".editorconfig", b"[*.py]\nindent_size = 2\n");
    write(&root, ".env", b"DATABASE_PASSWORD=hunter2\nPATH=$(touch ran)\n");
    let rs = write(&root, "a.rs", b"fn secret_body() {}\n");
    let py = write(&root, "b.py", b"def secret_py(): pass\n");
    let before = project_files(&root);
    let settings = d.path().join("state/lenz-code/settings.json");

    let mut s = session(d.path());
    let providers = std::fs::read(d.path().join("config/lenz-code/providers.json")).ok();
    for id in PROJECT { assert_eq!(s.disabled(id), Some("Open a folder to choose its editing preferences"), "{id:?}"); }
    assert_eq!(s.disabled(CommandId::EditingPreferences), Some("No file is open"));
    s.dispatch(Command::OpenRoot(root.clone()));
    settle(&mut s, scanned);
    s.dispatch(Command::Open(rs.clone()));
    settle(&mut s, |s| s.state().current_source().is_some_and(|d| d.source() == rs));
    assert_eq!(s.action(CommandId::EditingPreferences), CatalogAction::Interface(InterfaceAction::EditingPreferences));
    assert_eq!(PROJECT.map(|id| s.actions().iter().find(|a| a.id == id).and_then(|a| a.on)), [Some(false), Some(false), Some(false), Some(true), Some(false)], "inherited indentation and explicit formatting are the defaults");

    run(&mut s, CommandId::IndentTabs);
    run(&mut s, CommandId::FormatOnSave);
    assert_eq!(PROJECT.map(|id| s.actions().iter().find(|a| a.id == id).and_then(|a| a.on)), [Some(true), Some(false), Some(false), Some(false), Some(true)]);
    let details = s.editing_preferences_details();
    let rows: Vec<&str> = details.lines().collect();
    assert_eq!(rows[..7], [
        format!("Editing preferences for {}", rs.display()).as_str(),
        "Precedence: EditorConfig, then this project's choice in Lenz, then the language default.",
        "Indent style: tabs (this project's choice)",
        "Indent size: 4 (language default)",
        "Tab width: 4 (language default)",
        "Format on save: on for this project (Save asks the language server to format first)",
        &format!("Project preferences: saved by Lenz in {}, never in the project", settings.display()),
    ]);
    assert!(rows[7].starts_with("Formatter: rustfmt (tool defaults); "), "{}", rows[7]);

    s.dispatch(Command::Open(py.clone()));
    settle(&mut s, |s| s.state().current_source().is_some_and(|d| d.source() == py));
    let details = s.editing_preferences_details();
    assert!(details.contains("Indent style: tabs (this project's choice)\n"), "{details}");
    assert!(details.contains(&format!("Indent size: 2 (EditorConfig {})\n", root.join(".editorconfig").display())), "{details}");
    assert!(details.contains("Formatter: none configured for this project. Lenz never installs formatters."), "{details}");
    for private in ["hunter2", "secret_body", "secret_py", "touch ran"] { assert!(!details.contains(private), "{private} in {details}"); }

    // Saved outside the project, read back by the next session; project files and provider settings untouched.
    settle(&mut s, |_| std::fs::read(&settings).ok().and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .is_some_and(|v| v["projects"][root.to_str().unwrap()] == serde_json::json!({ "indent": "Tabs", "format_on_save": true })));
    let saved = std::fs::read_to_string(&settings).unwrap();
    for private in ["hunter2", "secret_body", "secret_py"] { assert!(!saved.contains(private)); }
    drop(s);
    let mut s = session(d.path());
    s.dispatch(Command::OpenRoot(root.clone()));
    settle(&mut s, scanned);
    assert_eq!(PROJECT.map(|id| s.actions().iter().find(|a| a.id == id).and_then(|a| a.on)), [Some(true), Some(false), Some(false), Some(false), Some(true)]);
    run(&mut s, CommandId::IndentTwoSpaces);
    run(&mut s, CommandId::FormatOnSave);
    assert_eq!(PROJECT.map(|id| s.actions().iter().find(|a| a.id == id).and_then(|a| a.on)), [Some(false), Some(true), Some(false), Some(false), Some(false)]);
    run(&mut s, CommandId::IndentInherit);
    settle(&mut s, |_| std::fs::read(&settings).ok().and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .is_some_and(|v| v.get("projects").is_none_or(|p| p.get(root.to_str().unwrap()).is_none())));
    assert_eq!(project_files(&root), before);
    assert_eq!(std::fs::read(d.path().join("config/lenz-code/providers.json")).ok(), providers);
    assert!(!root.join("ran").exists());
}

#[test]
fn yaml_editing_preferences_say_spaces_only_and_which_formatter_runs() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("p");
    std::fs::create_dir(&root).unwrap();
    write(&root, ".prettierrc", b"tabWidth: 8\n");
    let yaml = write(&root, "app.yaml", b"secret_key: hunter2\n");
    let before = project_files(&root);
    let mut s = session(d.path());
    s.dispatch(Command::OpenRoot(root.clone()));
    settle(&mut s, scanned);
    s.dispatch(Command::Open(yaml.clone()));
    settle(&mut s, |s| s.state().current_source().is_some_and(|d| d.source() == yaml));
    run(&mut s, CommandId::IndentTabs);
    let details = s.editing_preferences_details();
    let rows: Vec<&str> = details.lines().collect();
    assert_eq!(rows[2..5], ["Indent style: spaces (YAML indents with spaces only)", "Indent size: 2 (language default)", "Tab width: 2 (language default)"]);
    assert_eq!(rows.last().copied(), Some("Formatter: yaml-language-server's bundled Prettier, with this file's indent size (2) and the `format` settings of its provider settings; a project's own Prettier configuration is not read"));
    assert!(!details.contains("hunter2"), "{details}");
    assert_eq!(project_files(&root), before);
}
