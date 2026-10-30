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
    assert_eq!(labels, ["New File in Explorer…", "New Folder in Explorer…", "Copy File Name", "Copy Full Path", "Copy Project-Relative Path", "Paste File Path to Terminal…", "Copy File / Folder", "Paste File / Folder…", "Cancel File Copy", "Rename in Explorer…", "Move in Explorer…", "Delete in Explorer…", "Delete Permanently in Explorer…", "Restore Last Deleted Entry", "Show File / Folder Git History"]);
    // Copy/Paste, cancellation, history and Restore retain their shared prerequisites.
    assert!(app.choices().iter().all(|c| c.disabled.is_none() == !matches!(c.target, ChoiceTarget::Action(CommandId::RestoreDeleted | CommandId::PasteExplorerEntry | CommandId::CancelExplorerCopy | CommandId::ExplorerHistory | CommandId::PasteTerminalPath))));
    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    assert!(matches!(app.modal.as_ref().map(|m| &m.kind), Some(Prompt::Create { kind: code_core::create::Kind::File, dir }) if dir == "src"));
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
    // The keys go to the new file: typing edits it, not the explorer's search.
    assert_eq!(app.focus, Focus::Text);
    key(&mut app, KeyCode::Char('x'), KeyModifiers::NONE);
    assert_eq!(text(&app), "x");
    key(&mut app, KeyCode::Char('z'), KeyModifiers::CONTROL);
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

fn p_or_dir(row: &code_core::project::TreeRow) -> (String, Option<bool>) { (row.path.to_string(), row.dir) }

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
    assert!(matches!(app.modal.as_ref().map(|m| &m.kind), Some(Prompt::Relocate { kind: code_core::relocate::Kind::Rename, from }) if from == "src/a.txt"));
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

#[test]
fn directory_click_selects_and_only_arrow_or_double_click_toggles() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src/nested")).unwrap();
    std::fs::write(dir.path().join("src/nested/keep.txt"), b"keep\n").unwrap();
    std::fs::write(dir.path().join("src/other.txt"), b"other\n").unwrap();
    let mut app = App::new(config(dir.path()), Some(dir.path().to_owned()));
    settle(&mut app, |a| a.session.state().project().is_some_and(|p| !p.refreshing() && p.rows().iter().any(|r| &*r.path == "src")));
    draw(&mut app, 100, 20);
    let at = |a: &App, path: &str| a.layout.tree.y + (a.session.state().tree_rows().position(|r| &*r.path == path).unwrap() - a.tree_top) as u16;
    let open = |a: &App, path: &str| a.session.state().tree_rows().find(|r| &*r.path == path).and_then(|r| r.dir);
    let x = app.layout.tree.x;
    let y = at(&app, "src");
    mouse(&mut app, MouseButton::Left, x + 8, y);
    assert_eq!(open(&app, "src"), Some(false), "a directory label click only selects it");
    assert_eq!(app.session.state().tree_rows().nth(app.tree_at).map(|r| r.path.as_ref()), Some("src"));
    mouse(&mut app, MouseButton::Left, x + 8, y); draw(&mut app, 100, 20);
    assert_eq!(open(&app, "src"), Some(true));
    let y_nested = at(&app, "src/nested");
    mouse(&mut app, MouseButton::Left, x + 3, y_nested); draw(&mut app, 100, 20);
    assert_eq!(open(&app, "src/nested"), Some(true));
    mouse(&mut app, MouseButton::Left, x + 3, y_nested); draw(&mut app, 100, 20);
    assert_eq!(open(&app, "src/nested"), Some(false));
    mouse(&mut app, MouseButton::Left, x + 1, y); draw(&mut app, 100, 20);
    assert_eq!(open(&app, "src"), Some(false));
    key(&mut app, KeyCode::Right, KeyModifiers::NONE); draw(&mut app, 100, 20);
    assert_eq!(open(&app, "src"), Some(true));
    assert!(app.session.state().active().is_none());
    assert_eq!(std::fs::read(dir.path().join("src/nested/keep.txt")).unwrap(), b"keep\n");
}

#[test]
fn file_form_buttons_submit_validate_and_cancel_without_leaking_input() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("keep.txt"), b"unchanged\n").unwrap();
    let mut a = App::new(config(dir.path()), Some(dir.path().to_owned()));
    settle(&mut a, |a| a.session.state().project().is_some_and(|p| !p.refreshing()));
    a.run(CommandId::NewExplorerFile);
    draw(&mut a, 100, 24);
    let b = a.layout.form_buttons.unwrap()[0];
    mouse(&mut a, MouseButton::Left, b.x, b.y);
    assert_eq!(a.modal.as_ref().and_then(|m| m.error.as_deref()), Some("Enter a name"));
    for c in "created.txt".chars() { key(&mut a, KeyCode::Char(c), KeyModifiers::NONE); }
    for (w, h) in [(100, 24), (40, 14), (18, 8), (1, 1), (0, 0)] {
        draw(&mut a, w, h);
        if let Some(buttons) = a.layout.form_buttons {
            for b in buttons { assert!(b.right() <= w && b.bottom() <= h); }
        }
        assert!(!dir.path().join("created.txt").exists());
    }
    draw(&mut a, 100, 24);
    let b = a.layout.form_buttons.unwrap()[0];
    mouse(&mut a, MouseButton::Left, b.x, b.y);
    settle(&mut a, |a| a.session.state().active().is_some_and(|t| t.title() == "created.txt"));
    assert_eq!(std::fs::read(dir.path().join("created.txt")).unwrap(), b"");
    a.run(CommandId::DeleteExplorerPermanently);
    draw(&mut a, 100, 24);
    assert!(a.form_text().unwrap().destructive);
    let b = a.layout.form_buttons.unwrap()[1];
    mouse(&mut a, MouseButton::Left, b.x, b.y);
    assert!(a.modal.is_none() && dir.path().join("created.txt").exists());
    assert_eq!(std::fs::read(dir.path().join("keep.txt")).unwrap(), b"unchanged\n");
}

#[test]
fn explorer_delete_key_confirms_selected_entry_and_cancel_preserves_bytes() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/keep.txt"), b"keep\n").unwrap();
    let mut a = App::new(config(dir.path()), Some(dir.path().to_owned()));
    settle(&mut a, |a| a.session.state().project().is_some_and(|p| !p.refreshing()));
    a.dispatch(Command::OpenRetained(dir.path().join("src/keep.txt")));
    settle(&mut a, |a| a.session.state().current_source().is_some() && a.session.state().project().is_some_and(|p| !p.refreshing() && p.rows().iter().any(|r| &*r.path == "src")));
    a.dispatch(Command::RevealInTree("src/keep.txt".into()));
    draw(&mut a, 100, 24);
    for path in ["src/keep.txt", "src"] {
        a.tree_at = a.session.state().tree_rows().position(|r| r.path.as_ref() == path).unwrap();
        a.focus = Focus::Tree;
        key(&mut a, KeyCode::Delete, KeyModifiers::NONE);
        assert!(matches!(&a.modal.as_ref().unwrap().kind,
            Prompt::ConfirmDelete { removal: code_core::trash::Removal::Trash, from } if from == path));
        key(&mut a, KeyCode::Delete, KeyModifiers::NONE);
        key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
        assert!(a.modal.is_none());
        assert_eq!(std::fs::read(dir.path().join("src/keep.txt")).unwrap(), b"keep\n");
        let source = a.session.state().current_source().unwrap();
        assert_eq!(source.copy_range(0..source.len_bytes()).unwrap(), "keep\n");
    }
    a.focus = Focus::Text;
    key(&mut a, KeyCode::Delete, KeyModifiers::NONE);
    assert!(a.modal.is_none());
    let source = a.session.state().current_source().unwrap();
    assert_eq!(source.copy_range(0..source.len_bytes()).unwrap(), "eep\n");
    assert_eq!(std::fs::read(dir.path().join("src/keep.txt")).unwrap(), b"keep\n");
}
