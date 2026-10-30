use super::*;

fn source(v: &VisualTestContext, f: &Fixture) -> String {
    f.view.read_with(v, |r, _| {
        let source = r.session.state().current_source().unwrap();
        source.copy_range(0..source.len_bytes()).unwrap()
    })
}

fn row_menu(v: &mut VisualTestContext, f: &Fixture, name: &str) {
    draw(v);
    let at = f.view.read_with(v, |r, _| {
        let i = r.session.state().tree_rows().position(|row| &*row.path == name).unwrap() - r.tree_scroll;
        point(px(90.0), px(toolbar_h() + line_h() * (i as f32 + 1.5)))
    });
    v.simulate_event(gpui::MouseDownEvent {
        button: MouseButton::Right, position: at, modifiers: Modifiers::default(),
        click_count: 1, first_mouse: false,
    });
    assert!(f.view.read_with(v, |r, _| matches!(r.modal, Some(Modal::Context { .. }))));
}

fn context_command(v: &mut VisualTestContext, f: &Fixture, id: CommandId) {
    let index = f.view.read_with(v, |r, _| {
        r.modal_rows().iter().filter(|row| row.0 != "—").position(|row| row.0 == id.label()).unwrap()
    });
    for _ in 0..index { v.simulate_keystrokes("down"); }
    v.simulate_keystrokes("enter");
}

fn open_project(cx: &mut TestAppContext, f: &Fixture, root: &Path) {
    f.view.update(cx, |r, cx| r.open_root(root.to_owned(), cx));
    settle(cx, f, scanned);
    f.view.update(cx, |r, cx| {
        r.session.disable_desktop_file_clipboard_for_test();
        r.enable_draft_input_for_test();
        r.dispatch(Command::OpenRetained(root.join("src/a.txt")), cx);
        r.dispatch(Command::RevealInTree("src/a.txt".into()), cx);
    });
    settle(cx, f, active_is("a.txt"));
}

#[gpui::test]
fn gui_explorer_copy_text_uses_the_right_clicked_entry_not_the_editor(cx: &mut TestAppContext) {
    let f = fixture(cx, None);
    let root = f.dir.path().join("project");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/a.txt"), b"active\n").unwrap();
    std::fs::write(root.join("src/b.txt"), b"selected\n").unwrap();
    open_project(cx, &f, &root);
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(900.0), px(700.0)));
    for (command, expected) in [
        (CommandId::CopyExplorerName, "b.txt".to_string()),
        (CommandId::CopyExplorerFullPath, root.join("src/b.txt").display().to_string()),
        (CommandId::CopyExplorerRelativePath, "src/b.txt".to_string()),
    ] {
        row_menu(&mut v, &f, "src/b.txt");
        let rows = f.view.read_with(&v, |r, _| r.modal_rows());
        for id in [CommandId::CopyExplorerName, CommandId::CopyExplorerFullPath, CommandId::CopyExplorerRelativePath, CommandId::CopyExplorerEntry, CommandId::PasteExplorerEntry, CommandId::ExplorerHistory] {
            assert!(rows.iter().any(|row| row.0 == id.label()), "{} missing from {rows:?}", id.label());
        }
        assert_eq!(f.view.read_with(&v, |r, _| r.session.state().explorer().selected.clone()), Some("src/b.txt".into()));
        context_command(&mut v, &f, command);
        assert_eq!(v.read_from_clipboard().and_then(|item| item.text()), Some(expected));
        assert_eq!(f.view.read_with(&v, |r, _| r.session.state().active().unwrap().source().to_owned()), root.join("src/a.txt"));
    }
    assert_eq!(source(&v, &f), "active\n");
    assert_eq!(std::fs::read(root.join("src/b.txt")).unwrap(), b"selected\n");
}

#[gpui::test]
fn gui_explorer_copy_paste_keys_keep_the_dirty_editor_and_modal_ownership(cx: &mut TestAppContext) {
    let f = fixture(cx, None);
    let root = f.dir.path().join("project");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/a.txt"), b"active\r\n").unwrap();
    std::fs::write(root.join("src/b.txt"), b"saved selected\r\n").unwrap();
    open_project(cx, &f, &root);
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(900.0), px(700.0))); draw(&mut v);
    let text = f.view.read_with(&v, |r, _| r.text_bounds.unwrap());
    v.simulate_click(text.center(), Modifiers::default());
    v.simulate_keystrokes("ctrl-home x");
    assert_eq!(source(&v, &f), "xactive\r\n");
    row_menu(&mut v, &f, "src/a.txt");
    v.simulate_keystrokes("escape");
    v.write_to_clipboard(ClipboardItem::new_string("text clipboard stays text".into()));
    v.simulate_keystrokes("ctrl-c");
    assert_eq!(v.read_from_clipboard().and_then(|item| item.text()).as_deref(), Some("text clipboard stays text"));
    row_menu(&mut v, &f, "src");
    v.simulate_keystrokes("escape ctrl-v");
    assert!(f.view.read_with(&v, |r, _| matches!(&r.modal, Some(Modal::PasteEntry { request, name, .. }) if request.source() == root.join("src/a.txt") && request.folder() == "src" && name == "a.txt")));
    v.simulate_keystrokes("enter");
    assert!(f.view.read_with(&v, |r, _| matches!(&r.modal, Some(Modal::PasteEntry { error: Some(error), .. }) if error.contains("already exists"))));
    v.simulate_keystrokes("ctrl-s ctrl-z x");
    for (w, h) in [(1.0, 1.0), (200.0, 80.0), (900.0, 700.0)] {
        v.simulate_resize(size(px(w), px(h))); draw(&mut v);
        v.simulate_keystrokes("end up");
    }
    assert_eq!(source(&v, &f), "xactive\r\n");
    assert_eq!(std::fs::read(root.join("src/a.txt")).unwrap(), b"active\r\n");
    v.simulate_keystrokes("ctrl-a"); type_text(&mut v, "copied.txt");
    v.simulate_keystrokes("enter");
    settle(cx, &f, |r| root.join("src/copied.txt").exists() && r.session.disabled(CommandId::CancelExplorerCopy).is_some());
    assert_eq!(std::fs::read(root.join("src/copied.txt")).unwrap(), b"active\r\n");
    assert_eq!(source(&v, &f), "xactive\r\n");
    assert_eq!(std::fs::read(root.join("src/b.txt")).unwrap(), b"saved selected\r\n");
    // The same shortcuts in the editor retain text selection and text paste.
    draw(&mut v);
    let text = f.view.read_with(&v, |r, _| r.text_bounds.unwrap());
    v.simulate_click(text.center(), Modifiers::default());
    v.simulate_keystrokes("ctrl-a ctrl-c");
    assert_eq!(v.read_from_clipboard().and_then(|item| item.text()).as_deref(), Some("xactive\r\n"));
    v.write_to_clipboard(ClipboardItem::new_string("editor paste".into()));
    v.simulate_keystrokes("ctrl-v");
    assert_eq!(source(&v, &f), "editor paste");
    assert!(f.view.read_with(&v, |r, _| r.modal.is_none()));
    v.simulate_keystrokes("ctrl-z");
    assert_eq!(source(&v, &f), "xactive\r\n");
    assert_eq!(std::fs::read(root.join("src/a.txt")).unwrap(), b"active\r\n");
}

#[gpui::test]
fn gui_explorer_history_dispatches_the_selected_file_and_folder(cx: &mut TestAppContext) {
    use lenz_content::git::testing::{repo, write, commit_all};
    let repository = repo();
    write(repository.path(), "src/a.txt", "active\n");
    write(repository.path(), "src/b.txt", "selected\n");
    commit_all(repository.path(), "files");
    let f = fixture(cx, None);
    let exe = f.dir.path().join("lenz-code");
    std::fs::write(f.dir.path().join("lenz-diff"), "").unwrap();
    open_project(cx, &f, repository.path());
    f.view.update(cx, |r, _| r.exe = Some(exe));
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(900.0), px(700.0)));
    row_menu(&mut v, &f, "src/b.txt");
    settle(cx, &f, |r| r.session.explorer_history_disabled().is_none());
    context_command(&mut v, &f, CommandId::ExplorerHistory);
    row_menu(&mut v, &f, "src");
    context_command(&mut v, &f, CommandId::ExplorerHistory);
    let launches = f.view.read_with(&v, |r, _| r.launched.clone());
    assert_eq!(launches.len(), 2);
    assert_eq!(launches[0].args, ["log", "--follow", "src/b.txt"]);
    assert_eq!(launches[1].args, ["log", "--", ":(literal)src"]);
    assert_eq!(launches[0].cwd.as_deref(), Some(repository.path()));
    assert!(launches.iter().all(|launch| launch.program.ends_with("lenz-diff")));
    assert_eq!(source(&v, &f), "active\n");
}

#[gpui::test]
fn gui_explorer_type_search_selects_without_opening_and_escape_restores(cx: &mut TestAppContext) {
    let f = fixture(cx, None);
    let root = f.dir.path().join("project");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/a.txt"), b"active\n").unwrap();
    std::fs::write(root.join("src/Beta.txt"), b"candidate\n").unwrap();
    open_project(cx, &f, &root);
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(900.0), px(700.0)));
    row_menu(&mut v, &f, "src/a.txt");
    v.simulate_keystrokes("escape b");
    assert_eq!(f.view.read_with(&v, |r, _| r.tree_sel.clone()).as_deref(), Some("src/Beta.txt"));
    assert_eq!(f.view.read_with(&v, |r, _| r.session.state().active().unwrap().source().to_owned()), root.join("src/a.txt"));
    v.simulate_keystrokes("escape");
    assert_eq!(f.view.read_with(&v, |r, _| r.tree_sel.clone()).as_deref(), Some("src/a.txt"));
    assert!(f.view.read_with(&v, |r, _| r.tree_focus));
}

#[gpui::test]
fn gui_explorer_search_keys_cover_unicode_directories_no_match_and_modal_precedence(cx: &mut TestAppContext) {
    let f = fixture(cx, None);
    let root = f.dir.path().join("project");
    std::fs::create_dir_all(root.join("src/BetaDir")).unwrap();
    for (name, text) in [("src/a.txt", "active\r\n"), ("src/Éclair.txt", "unicode\n"), ("src/BetaDir/hidden.txt", "hidden\n")] {
        std::fs::write(root.join(name), text).unwrap();
    }
    open_project(cx, &f, &root);
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(900.0), px(700.0)));
    row_menu(&mut v, &f, "src/a.txt");
    v.simulate_keystrokes("escape é c backspace");
    assert_eq!(f.view.read_with(&v, |r, _| r.session.state().explorer_search().query().to_owned()), "é");
    assert_eq!(f.view.read_with(&v, |r, _| r.tree_sel.clone()).as_deref(), Some("src/Éclair.txt"));
    for theme in ["darcula", "github-light"] {
        f.view.update(&mut v, |r, cx| r.dispatch(Command::Theme(ThemeCommand::Select(ThemeSelection::from_id(theme).unwrap())), cx));
        for (w, h) in [(900.0, 700.0), (240.0, 160.0), (1.0, 1.0)] { v.simulate_resize(size(px(w), px(h))); draw(&mut v); }
        v.simulate_resize(size(px(900.0), px(700.0))); draw(&mut v);
        f.view.read_with(&v, |r, _| {
            let line = r.tree_lines(r.tree_at()).into_iter().find(|line| line.name == "Éclair.txt").unwrap();
            assert_eq!(line.highlight.as_slice(), std::slice::from_ref(&(0..2)));
            assert!(line.selected);
        });
    }
    v.simulate_keystrokes("ctrl-c");
    assert_eq!(f.view.read_with(&v, |r, _| r.session.state().explorer_search().query().to_owned()), "é");
    v.simulate_keystrokes("enter");
    drop(v);
    settle(cx, &f, |r| r.session.state().current_source().is_some_and(|t| t.source() == root.join("src/Éclair.txt")));
    let mut v = visual(cx, &f);
    v.simulate_keystrokes("b e t a d i r enter");
    assert!(f.view.read_with(&v, |r, _| r.session.state().tree_rows().any(|row| &*row.path == "src/BetaDir" && row.dir == Some(true))));
    v.simulate_keystrokes("m i s s i n g f10 a b c ctrl-s ctrl-z");
    assert!(f.view.read_with(&v, |r, _| r.modal.is_some()));
    assert_eq!(f.view.read_with(&v, |r, _| r.session.state().explorer_search().query().to_owned()), "missing");
    assert_eq!(source(&v, &f), "unicode\n");
    v.simulate_keystrokes("escape enter");
    assert_eq!(f.view.read_with(&v, |r, _| r.session.state().active().unwrap().source().to_owned()), root.join("src/Éclair.txt"));
    assert_eq!(std::fs::read(root.join("src/a.txt")).unwrap(), b"active\r\n");
}
