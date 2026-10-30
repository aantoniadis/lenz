use super::*;

#[gpui::test]
fn gui_problems_menu_lists_all_known_files_with_visible_badges(cx: &mut TestAppContext) {
    let f = fixture(cx, None);
    let root = f.dir.path().join("project");
    std::fs::create_dir_all(&root).unwrap();
    let first = root.join("bad.java");
    let second = root.join("also.rs");
    std::fs::write(&first, b"class Bad { void f( { }\n").unwrap();
    std::fs::write(&second, b"fn broken( {\n").unwrap();
    f.view.update(cx, |v, cx| v.open_root(root.clone(), cx));
    settle(cx, &f, scanned);
    for path in [&first, &second] {
        f.view.update(cx, |v, cx| v.dispatch(Command::OpenRetained(path.clone()), cx));
        settle(cx, &f, |v| v.session().state().active().is_some_and(|t| t.source() == path && v.session().state().diagnostics(t.id()).is_some()));
    }
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(800.0), px(400.0)));
    draw(&mut v);
    assert!(f.view.read_with(&v, |r, _| r.session().state().tabs().iter().any(|t| r.session().state().problem_badge(t.id()).starts_with('!'))));
    assert!(f.view.read_with(&v, |r, _| r.tree_lines(0).iter().any(|row| row.error && row.name == "bad.java")));
    choose_menu_command(&mut v, &f, CommandId::ShowProblems);
    let paths = f.view.read_with(&v, |r, _| {
        assert!(r.results_focus && r.modal.is_none());
        assert!(r.results_status().unwrap().contains("Problems"));
        r.session().state().results().unwrap().hits().iter().map(|h| h.path.to_string()).collect::<Vec<_>>()
    });
    assert!(paths.iter().any(|p| p.ends_with("bad.java")), "{paths:?}");
    assert!(paths.iter().any(|p| p.ends_with("also.rs")), "{paths:?}");
    v.simulate_resize(size(px(180.0), px(100.0)));
    draw(&mut v);
    v.simulate_keystrokes("escape");
    assert!(f.view.read_with(&v, |r, _| r.session().state().results().is_none()));
    assert_eq!(std::fs::read(second).unwrap(), b"fn broken( {\n");
}
