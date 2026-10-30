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
    assert_eq!(rows.iter().map(|r| r.0.as_str()).collect::<Vec<_>>(), ["New File in Explorer…", "New Folder in Explorer…", "—", "Copy File Name", "Copy Full Path", "Copy Project-Relative Path", "Paste File Path to Terminal…", "—", "Copy File / Folder", "Paste File / Folder…", "Cancel File Copy", "—", "Rename in Explorer…", "Move in Explorer…", "—", "Delete in Explorer…", "Delete Permanently in Explorer…", "Restore Last Deleted Entry", "—", "Show File / Folder Git History"], "{rows:?}");
    v.simulate_keystrokes("enter");
    assert!(f.view.read_with(&v, |r, _| matches!(&r.modal, Some(Modal::Create { kind: code_core::create::Kind::File, dir, .. }) if dir == "src")));
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
    // The keys go to the new file, not the explorer.
    assert!(f.view.read_with(&v, |r, _| !r.tree_focus && !r.results_focus));
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

#[gpui::test]
fn directory_click_selects_and_only_arrow_or_double_click_toggles(cx: &mut TestAppContext) {
    let f = fixture(cx, None);
    let root = f.dir.path().join("proj");
    std::fs::create_dir_all(root.join("src/nested")).unwrap();
    std::fs::write(root.join("src/nested/keep.txt"), b"keep\n").unwrap();
    std::fs::write(root.join("src/other.txt"), b"other\n").unwrap();
    f.view.update(cx, |r, cx| r.open_root(root.clone(), cx));
    settle(cx, &f, scanned);
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(900.0), px(400.0))); draw(&mut v);
    let open = |v: &VisualTestContext, name: &str| f.view.read_with(v, |r, _| r.session.state().tree_rows().find(|row| &*row.path == name).and_then(|row| row.dir));
    let at = tree_row(&mut v, &f, "src");
    let label = point(px(90.0), at.y);
    v.simulate_click(label, Modifiers::default()); draw(&mut v);
    assert_eq!(open(&v, "src"), Some(false), "a directory label click only selects it");
    assert_eq!(f.view.read_with(&v, |r, _| r.session.state().explorer().selected.as_deref().map(str::to_owned)), Some("src".into()));
    v.simulate_event(gpui::MouseDownEvent { button: MouseButton::Left, position: label, modifiers: Modifiers::default(), click_count: 2, first_mouse: false });
    draw(&mut v);
    assert_eq!(open(&v, "src"), Some(true));
    // The nested arrow follows the drawn depth, not the row's name or icon.
    let nested = tree_row(&mut v, &f, "src/nested");
    let arrow = f.view.read_with(&v, |r, _| point(px(6.0 + 2.0 * r.char_w + r.char_w / 2.0), nested.y));
    v.simulate_click(arrow, Modifiers::default()); draw(&mut v);
    assert_eq!(open(&v, "src/nested"), Some(true));
    v.simulate_click(arrow, Modifiers::default()); draw(&mut v);
    assert_eq!(open(&v, "src/nested"), Some(false));
    let arrow = point(px(10.0), at.y);
    v.simulate_click(arrow, Modifiers::default()); draw(&mut v);
    assert_eq!(open(&v, "src"), Some(false));
    v.simulate_keystrokes("right"); draw(&mut v);
    assert_eq!(open(&v, "src"), Some(true), "keyboard expansion still works");
    assert!(f.view.read_with(&v, |r, _| r.session.state().active().is_none()));
    assert_eq!(std::fs::read(root.join("src/nested/keep.txt")).unwrap(), b"keep\n");
}

#[gpui::test]
fn gui_file_forms_have_fields_named_buttons_and_keep_validation_modal(cx: &mut TestAppContext) {
    let f = fixture(cx, None);
    let root = f.dir.path().join("proj");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("keep.txt"), b"unchanged\n").unwrap();
    f.view.update(cx, |r, cx| { r.open_root(root.clone(), cx); r.open(root.join("keep.txt"), cx); });
    settle(cx, &f, |r| scanned(r) && active_is("keep.txt")(r));
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(900.0), px(500.0)));
    choose_menu_command(&mut v, &f, CommandId::NewExplorerFile);
    let selection = f.view.read_with(&v, |r, _| r.session.state().selection());
    let submit = f.view.read_with(&v, |r, _| {
        let form = r.form_layout().unwrap();
        assert_eq!(form.text.field, Some("Name"));
        assert_eq!(form.text.submit, "Create");
        assert!(form.field.unwrap().bottom() < form.buttons[0].top());
        form.buttons[1].center()
    });
    v.simulate_click(submit, Modifiers::default());
    assert!(f.view.read_with(&v, |r, _| matches!(&r.modal, Some(Modal::Create { error: Some(e), .. }) if e == "Enter a name")));
    type_text(&mut v, "created.txt");
    for theme in ["darcula", "github-light"] {
        f.view.update(&mut v, |r, cx| r.dispatch(Command::Theme(ThemeCommand::Select(ThemeSelection::from_id(theme).unwrap())), cx));
        for (w, h) in [(900.0, 500.0), (480.0, 320.0), (180.0, 140.0), (1.0, 1.0)] {
            v.simulate_resize(size(px(w), px(h))); draw(&mut v);
            f.view.read_with(&v, |r, _| {
                let l = r.form_layout().unwrap();
                for b in [l.bounds, l.field.unwrap(), l.buttons[0], l.buttons[1]] {
                    assert!(b.left() >= px(0.0) && b.top() >= px(0.0) && b.right() <= px(w) && b.bottom() <= px(h), "{b:?}");
                }
                assert_eq!(r.session.state().selection(), selection);
            });
        }
    }
    v.simulate_resize(size(px(900.0), px(500.0))); draw(&mut v);
    let submit = f.view.read_with(&v, |r, _| r.form_layout().unwrap().buttons[1].center());
    v.simulate_click(submit, Modifiers::default());
    settle(cx, &f, active_is("created.txt"));
    assert_eq!(std::fs::read(root.join("created.txt")).unwrap(), b"");
    assert_eq!(std::fs::read(root.join("keep.txt")).unwrap(), b"unchanged\n");
    choose_menu_command(&mut v, &f, CommandId::DeleteExplorerPermanently);
    let cancel = f.view.read_with(&v, |r, _| {
        let l = r.form_layout().unwrap();
        assert!(l.text.destructive && l.field.is_none());
        l.buttons[0].center()
    });
    v.simulate_click(cancel, Modifiers::default());
    assert!(f.view.read_with(&v, |r, _| r.modal.is_none()));
    assert!(root.join("created.txt").exists());
}

#[gpui::test]
fn gui_short_dialogs_keep_errors_and_consequences_above_the_actions(cx: &mut TestAppContext) {
    let f = fixture(cx, Some(("keep.txt", b"saved\n")));
    settle(cx, &f, active_is("keep.txt"));
    let mut v = visual(cx, &f);
    for theme in ["darcula", "github-light"] {
        f.view.update(&mut v, |r, cx| r.dispatch(Command::Theme(ThemeCommand::Select(ThemeSelection::from_id(theme).unwrap())), cx));
        v.simulate_resize(size(px(180.0), px(140.0)));
        f.view.update(&mut v, |r, cx| { r.modal = Some(Modal::NewScratch { input: "sample.txt".into(), selected: false, error: Some("That name is already in use".into()) }); cx.notify(); });
        draw(&mut v);
        f.view.read_with(&v, |r, _| {
            let l = r.form_layout().unwrap();
            assert!(l.visible > 0, "validation remains readable");
            assert!(l.field.unwrap().bottom() <= l.buttons[0].top());
        });
        v.simulate_keystrokes("escape");

    }
    assert_eq!(std::fs::read(f.dir.path().join("keep.txt")).unwrap(), b"saved\n");
}

#[gpui::test]
fn gui_explorer_delete_key_confirms_selected_entry_and_cancel_preserves_bytes(cx: &mut TestAppContext) {
    let f = fixture(cx, None);
    let root = f.dir.path().join("proj");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/keep.txt"), b"keep\n").unwrap();
    f.view.update(cx, |r, cx| {
        r.open_root(root.clone(), cx);
        r.open(root.join("src/keep.txt"), cx);
    });
    settle(cx, &f, |r| scanned(r) && active_is("keep.txt")(r));
    f.view.update(cx, |r, cx| r.dispatch(Command::RevealInTree("src/keep.txt".into()), cx));
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(900.0), px(500.0))); draw(&mut v);
    for path in ["src/keep.txt", "src"] {
        let at = tree_row(&mut v, &f, path);
        right_click(&mut v, at);
        v.simulate_keystrokes("escape delete");
        assert!(f.view.read_with(&v, |r, _| matches!(&r.modal,
            Some(Modal::ConfirmDelete { removal: code_core::trash::Removal::Trash, from, .. }) if from == path)));
        v.simulate_keystrokes("delete");
        assert_eq!(text(&v, &f), "keep\n", "confirmation consumes repeated Delete");
        v.simulate_keystrokes("escape");
        assert!(f.view.read_with(&v, |r, _| r.modal.is_none()));
        assert_eq!(std::fs::read(root.join("src/keep.txt")).unwrap(), b"keep\n");
    }
    f.view.update(&mut v, |r, _| { r.tree_focus = false; r.enable_draft_input_for_test(); });
    v.simulate_keystrokes("delete");
    assert!(f.view.read_with(&v, |r, _| r.modal.is_none()));
    assert_eq!(text(&v, &f), "eep\n", "Delete in the editor still edits the draft");
    assert_eq!(std::fs::read(root.join("src/keep.txt")).unwrap(), b"keep\n");
}
