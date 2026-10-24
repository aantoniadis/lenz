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
    assert_eq!(rows.iter().map(|r| r.0.as_str()).collect::<Vec<_>>(), ["New File in Explorer…", "New Folder in Explorer…"], "{rows:?}");
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
