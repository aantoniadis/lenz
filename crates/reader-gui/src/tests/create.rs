use super::*;

fn text(v: &VisualTestContext, f: &Fixture) -> String {
    f.view.read_with(v, |view, _| { let s = view.session().state().current_source().unwrap(); s.copy_range(0..s.len_bytes()).unwrap() })
}

/// The point of explorer row `name` as drawn.
fn tree_row(v: &mut VisualTestContext, f: &Fixture, name: &str) -> gpui::Point<gpui::Pixels> {
    f.view.read_with(v, |r, _| {
        let i = r.session().state().tree_rows().position(|row| &*row.path == name).unwrap() - r.tree_scroll;
        point(px(20.0), px(toolbar_h() + line_h() * (i as f32 + 1.5)))
    })
}

fn right_click(v: &mut VisualTestContext, at: gpui::Point<gpui::Pixels>) {
    v.simulate_event(gpui::MouseDownEvent { button: MouseButton::Right, position: at, modifiers: Modifiers::default(), click_count: 1, first_mouse: false });
}

#[gpui::test]
fn gui_the_explorer_creates_a_file_and_a_folder_through_real_input(cx: &mut TestAppContext) {
    let f = fixture(cx, None);
    let root = f.dir.path().join("proj");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/a.txt"), b"keep\n").unwrap();
    f.view.update(cx, |v, cx| v.open_root(root.clone(), cx));
    settle(cx, &f, scanned);
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(900.0), px(400.0)));
    draw(&mut v);
    // The explorer's row menu offers what can be made in the highlighted folder.
    let at = tree_row(&mut v, &f, "src");
    right_click(&mut v, at);
    let rows: Vec<(String, Option<&'static str>)> = f.view.read_with(&v, |r, _| {
        assert!(matches!(r.modal, Some(Modal::Context { .. })));
        r.modal_rows()
    });
    assert_eq!(rows.iter().map(|r| r.0.as_str()).collect::<Vec<_>>(), ["New File in Explorer…", "New Folder in Explorer…", "—", "Rename in Explorer…", "Move in Explorer…", "—", "Delete in Explorer…", "Delete Permanently in Explorer…", "Restore Last Deleted Entry"], "{rows:?}");
    v.simulate_keystrokes("enter");
    assert!(f.view.read_with(&v, |r, _| matches!(&r.modal, Some(Modal::Create { kind: reader_core::create::Kind::File, dir, .. }) if dir == "src")));
    // The dialog owns the keys: typing goes to the name field, not the document or the explorer.
    v.simulate_keystrokes("ctrl-v");
    type_text(&mut v, "new.txt");
    let titles = f.view.read_with(&v, |r, _| tab_titles(r));
    assert!(titles.is_empty(), "{titles:?}");
    v.simulate_keystrokes("enter");
    settle(cx, &f, |view| view.session().state().active().is_some_and(|t| t.source() == root.join("src/new.txt")) && view.session().state().current_source().is_some());
    assert_eq!(std::fs::read(root.join("src/new.txt")).unwrap(), b"", "creating makes an empty file");
    let (title, selected) = f.view.read_with(&v, |r, _| (r.session().state().active().unwrap().title(), r.tree_sel.clone()));
    assert_eq!(title, "new.txt");
    assert_eq!(selected.map(|p| p.to_string()), Some("src/new.txt".to_string()), "the new file is selected in the explorer");
    draw(&mut v);
    assert!(f.view.read_with(&v, |r, _| { let at = r.tree_at(); r.tree_lines(at).iter().any(|l| l.name == "new.txt" && l.selected) }), "the new file is drawn selected");
    // An existing name is refused in the dialog and nothing changes on disk.
    choose_menu_command(&mut v, &f, CommandId::NewExplorerFile);
    type_text(&mut v, "a.txt");
    v.simulate_keystrokes("enter");
    let error = f.view.read_with(&v, |r, _| match &r.modal { Some(Modal::Create { error, .. }) => error.clone(), _ => None });
    assert_eq!(error.as_deref(), Some("src/a.txt already exists"));
    for (w, h) in [(1.0, 1.0), (200.0, 80.0), (900.0, 400.0)] { v.simulate_resize(size(px(w), px(h))); draw(&mut v); }
    v.simulate_keystrokes("end up backspace");
    assert_eq!(std::fs::read(root.join("src/a.txt")).unwrap(), b"keep\n");
    assert_eq!(text(&v, &f), "");
    v.simulate_keystrokes("escape");
    assert!(f.view.read_with(&v, |r, _| r.modal.is_none()));
    assert_eq!(std::fs::read(root.join("src/a.txt")).unwrap(), b"keep\n");
    // A folder, from the same row menu; the new folder is selected and the file stays the active document.
    let at = tree_row(&mut v, &f, "src");
    right_click(&mut v, at);
    v.simulate_keystrokes("down enter");
    type_text(&mut v, "docs");
    v.simulate_keystrokes("enter");
    assert!(root.join("src/docs").is_dir());
    assert!(std::fs::read_dir(root.join("src/docs")).unwrap().next().is_none(), "nothing else is made");
    let selected = f.view.read_with(&v, |r, _| r.tree_sel.clone());
    assert_eq!(selected.map(|p| p.to_string()), Some("src/docs".to_string()));
    assert_eq!(f.view.read_with(&v, |r, _| r.session().state().active().unwrap().title()), "new.txt", "a folder create keeps the document");
    // Escape cancels a name without creating anything.
    choose_menu_command(&mut v, &f, CommandId::NewExplorerFolder);
    type_text(&mut v, "wip");
    v.simulate_keystrokes("escape");
    assert!(f.view.read_with(&v, |r, _| r.modal.is_none()) && !root.join("src/wip").exists());
    assert_eq!(std::fs::read(root.join("src/a.txt")).unwrap(), b"keep\n");
}

#[gpui::test]
fn gui_the_explorer_renames_and_moves_an_open_dirty_file_through_real_input(cx: &mut TestAppContext) {
    let f = fixture(cx, None);
    let root = f.dir.path().join("proj");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/a.txt"), b"keep\n").unwrap();
    std::fs::write(root.join("src/b.txt"), b"other\n").unwrap();
    f.view.update(cx, |v, cx| v.open_root(root.clone(), cx));
    settle(cx, &f, scanned);
    f.view.update(cx, |v, cx| { v.dispatch(Command::OpenRetained(root.join("src/a.txt")), cx); v.dispatch(Command::RevealInTree("src/a.txt".into()), cx); });
    settle(cx, &f, |view| view.session().state().current_source().is_some_and(|s| s.source().ends_with("src/a.txt")));
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(900.0), px(400.0)));
    draw(&mut v);
    f.view.update(&mut v, |r, _| r.enable_draft_input_for_test());
    v.simulate_keystrokes("x");
    assert_eq!(text(&v, &f), "xkeep\n");
    // The row menu of a file offers its rename and move; the right-click highlights that row.
    let at = tree_row(&mut v, &f, "src/a.txt");
    right_click(&mut v, at);
    let rows: Vec<String> = f.view.read_with(&v, |r, _| r.modal_rows().into_iter().map(|row| row.0).collect());
    assert!(rows.iter().any(|r| r == "Rename in Explorer…") && rows.iter().any(|r| r == "Move in Explorer…"), "{rows:?}");
    v.simulate_keystrokes("escape");
    choose_menu_command(&mut v, &f, CommandId::RenameExplorerEntry);
    let input = f.view.read_with(&v, |r, _| match &r.modal { Some(Modal::Relocate { input, .. }) => Some(input.clone()), _ => None });
    assert_eq!(input.as_deref(), Some("a.txt"), "the dialog starts with the current name");
    // The dialog owns the keys in tiny windows too; nothing moves and the draft is untouched.
    for (w, h) in [(1.0, 1.0), (200.0, 80.0), (900.0, 400.0)] { v.simulate_resize(size(px(w), px(h))); draw(&mut v); v.simulate_keystrokes("end up"); }
    for _ in 0..5 { v.simulate_keystrokes("backspace"); }
    type_text(&mut v, "b.txt");
    v.simulate_keystrokes("enter");
    let error = f.view.read_with(&v, |r, _| match &r.modal { Some(Modal::Relocate { error, .. }) => error.clone(), _ => None });
    assert_eq!(error.as_deref(), Some("src/b.txt already exists"));
    assert_eq!((std::fs::read(root.join("src/a.txt")).unwrap(), std::fs::read(root.join("src/b.txt")).unwrap()), (b"keep\n".to_vec(), b"other\n".to_vec()));
    for _ in 0..5 { v.simulate_keystrokes("backspace"); }
    type_text(&mut v, "c.txt");
    v.simulate_keystrokes("enter");
    assert!(f.view.read_with(&v, |r, _| r.modal.is_none()));
    let (title, dirty, selected) = f.view.read_with(&v, |r, _| { let s = r.session().state(); let t = s.active().unwrap(); (t.title(), s.tab_dirty(t.id()), r.tree_sel.clone()) });
    assert_eq!((title.as_str(), dirty), ("c.txt", true));
    assert_eq!(selected.map(|p| p.to_string()), Some("src/c.txt".to_string()), "the renamed entry is selected");
    assert_eq!(text(&v, &f), "xkeep\n");
    assert_eq!(std::fs::read(root.join("src/c.txt")).unwrap(), b"keep\n", "a rename moves the file, not the draft");
    // Move to the project root: the dialog starts with the current folder.
    choose_menu_command(&mut v, &f, CommandId::MoveExplorerEntry);
    for _ in 0..3 { v.simulate_keystrokes("backspace"); }
    v.simulate_keystrokes("enter");
    settle(cx, &f, |view| view.session().state().active().is_some_and(|t| t.source() == root.join("c.txt")));
    assert!(!root.join("src/c.txt").exists());
    v.simulate_keystrokes("ctrl-s");
    settle(cx, &f, |view| view.session().state().active().is_some_and(|t| !view.session().state().tab_dirty(t.id())));
    assert_eq!(std::fs::read(root.join("c.txt")).unwrap(), b"xkeep\n", "Save writes the moved file where it is now");
}
