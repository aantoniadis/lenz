use super::*;

fn text(app: &App) -> String { let s = app.session.state().current_source().unwrap(); s.copy_range(0..s.len_bytes()).unwrap() }

/// A right-click on the explorer row showing `name`.
fn tree_right_click(app: &mut App, name: &str) {
    let (row, area) = {
        let at = app.session.state().tree_rows().position(|r| &*r.path == name).unwrap();
        (at - app.tree_top, app.layout.tree)
    };
    mouse(app, MouseButton::Right, area.x + 1, area.y + row as u16);
}

#[test]
fn explorer_new_file_and_folder_use_real_keys_and_the_core_rules() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/a.txt"), b"keep\n").unwrap();
    let mut app = App::new(config(dir.path()), Some(dir.path().to_owned()));
    app.enable_draft_input_for_test();
    settle(&mut app, |a| a.session.state().project().is_some_and(|p| !p.refreshing() && p.rows().iter().any(|r| &*r.name == "src")));
    draw(&mut app, 100, 20);
    // The explorer's row menu offers both creates; the highlighted row is where they go.
    tree_right_click(&mut app, "src");
    assert!(matches!(app.modal.as_ref().map(|m| &m.kind), Some(Prompt::Context { .. })), "{:?}", app.modal.as_ref().map(|m| &m.kind));
    let labels: Vec<String> = app.choices().into_iter().map(|c| c.label).collect();
    assert_eq!(labels, ["New File in Explorer…", "New Folder in Explorer…", "Rename in Explorer…", "Move in Explorer…"]);
    assert!(app.choices().iter().all(|c| c.disabled.is_none()));
    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    assert!(matches!(app.modal.as_ref().map(|m| &m.kind), Some(Prompt::Create { kind: reader_core::create::Kind::File, dir }) if dir == "src"));
    assert_eq!(app.modal_title(), "New file in src · Enter creates · Esc cancels");
    // The dialog owns the keys: a paste reaches its name field, never the document.
    app.handle(Event::Paste("ignored".into()));
    assert_eq!(app.modal.as_ref().unwrap().input, "ignored");
    assert_eq!(app.session.state().selection(), None, "the document saw nothing");
    key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert!(app.modal.is_none() && !dir.path().join("src/ignored").exists());
    // A name: created on disk, opened as that file, selected in the explorer.
    tree_right_click(&mut app, "src");
    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    for c in "new.txt".chars() { key(&mut app, KeyCode::Char(c), KeyModifiers::NONE); }
    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    settle(&mut app, |a| a.session.state().active().is_some_and(|t| t.source() == dir.path().join("src/new.txt")) && a.session.state().current_source().is_some());
    assert_eq!(std::fs::read(dir.path().join("src/new.txt")).unwrap(), b"");
    let tab = { let t = app.session.state().active().unwrap(); (t.title(), t.source().to_owned()) };
    assert_eq!(tab, ("new.txt".to_string(), dir.path().join("src/new.txt")));
    let selected = app.session.state().tree_rows().nth(app.tree_at).map(|r| r.path.clone());
    assert_eq!(selected.map(|p| p.to_string()), Some("src/new.txt".to_string()));
    let screen = draw_screen(&mut app, 100, 20);
    assert!(screen.contains("new.txt"), "{screen}");
    // An existing name is refused in the dialog; the file and the document stay as they were.
    tree_right_click(&mut app, "src/a.txt");
    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    for c in "a.txt".chars() { key(&mut app, KeyCode::Char(c), KeyModifiers::NONE); }
    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(app.modal.as_ref().unwrap().error.as_deref(), Some("src/a.txt already exists"));
    for (w, h) in [(20, 5), (1, 1), (100, 20)] { key(&mut app, KeyCode::End, KeyModifiers::NONE); key(&mut app, KeyCode::Up, KeyModifiers::NONE); draw(&mut app, w, h); }
    assert_eq!(std::fs::read(dir.path().join("src/a.txt")).unwrap(), b"keep\n");
    assert_eq!(text(&app), "");
    key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert!(app.modal.is_none());
    // A folder from the same menu: shown open and selected, the document untouched.
    tree_right_click(&mut app, "src");
    key(&mut app, KeyCode::Down, KeyModifiers::NONE);
    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    for c in "docs".chars() { key(&mut app, KeyCode::Char(c), KeyModifiers::NONE); }
    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    assert!(dir.path().join("src/docs").is_dir());
    assert!(std::fs::read_dir(dir.path().join("src/docs")).unwrap().next().is_none());
    let selected = app.session.state().tree_rows().nth(app.tree_at).map(p_or_dir);
    assert_eq!(selected, Some(("src/docs".to_string(), Some(true))), "the new folder is selected, open");
    assert_eq!(app.session.state().active().unwrap().title(), "new.txt");
    // Escape cancels without creating anything.
    tree_right_click(&mut app, "src");
    key(&mut app, KeyCode::Down, KeyModifiers::NONE);
    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    for c in "wip".chars() { key(&mut app, KeyCode::Char(c), KeyModifiers::NONE); }
    key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert!(app.modal.is_none() && !dir.path().join("src/wip").exists());
    assert_eq!(std::fs::read(dir.path().join("src/a.txt")).unwrap(), b"keep\n");
}

fn p_or_dir(row: &reader_core::project::TreeRow) -> (String, Option<bool>) { (row.path.to_string(), row.dir) }

#[test]
fn explorer_rename_and_move_use_the_row_menu_and_survive_tiny_terminals() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/a.txt"), b"keep\n").unwrap();
    let mut app = App::new(config(dir.path()), Some(dir.path().to_owned()));
    app.enable_draft_input_for_test();
    settle(&mut app, |a| a.session.state().project().is_some_and(|p| !p.refreshing() && p.rows().iter().any(|r| &*r.name == "src")));
    app.dispatch(Command::OpenRetained(dir.path().join("src/a.txt")));
    app.dispatch(Command::RevealInTree("src/a.txt".into()));
    settle(&mut app, |a| a.session.state().current_source().is_some());
    draw(&mut app, 100, 20);
    // The file's row menu: Rename is its third entry.
    tree_right_click(&mut app, "src/a.txt");
    let at = app.choices().iter().position(|c| c.label == "Rename in Explorer…").unwrap();
    for _ in 0..at { key(&mut app, KeyCode::Down, KeyModifiers::NONE); }
    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    assert!(matches!(app.modal.as_ref().map(|m| &m.kind), Some(Prompt::Relocate { kind: reader_core::relocate::Kind::Rename, from }) if from == "src/a.txt"));
    assert_eq!(app.modal.as_ref().unwrap().input, "a.txt");
    for (w, h) in [(1, 1), (12, 4), (100, 20)] {
        draw(&mut app, w, h);
        key(&mut app, KeyCode::End, KeyModifiers::NONE);
        key(&mut app, KeyCode::Up, KeyModifiers::NONE);
    }
    key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert!(app.modal.is_none(), "Escape dismisses the prompt");
    assert_eq!(std::fs::read(dir.path().join("src/a.txt")).unwrap(), b"keep\n");
    assert_eq!(text(&app), "keep\n", "nothing behind the prompt changed");
    // Typed for real: the file and its tab move together.
    app.run(CommandId::RenameExplorerEntry);
    for _ in 0..5 { key(&mut app, KeyCode::Backspace, KeyModifiers::NONE); }
    for c in "b.txt".chars() { key(&mut app, KeyCode::Char(c), KeyModifiers::NONE); }
    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    draw(&mut app, 100, 20);
    assert!(app.modal.is_none());
    assert_eq!(app.session.state().active().unwrap().source(), dir.path().join("src/b.txt"));
    assert_eq!(std::fs::read(dir.path().join("src/b.txt")).unwrap(), b"keep\n");
    assert!(!dir.path().join("src/a.txt").exists());
}
