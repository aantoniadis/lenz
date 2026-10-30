use super::*;

#[gpui::test]
fn gui_project_syntax_menu_checks_unopened_files_and_shows_coverage_in_narrow_details(cx: &mut TestAppContext) {
    let f = fixture(cx, None);
    let root = f.dir.path().join("project"); std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("a.rs"), b"fn broken( {\n").unwrap();
    std::fs::write(root.join("bad.xml"), b"<Root></root>\n").unwrap();
    std::fs::write(root.join("note.txt"), b"unparsed\n").unwrap();
    f.view.update(cx, |v, cx| v.open_root(root.clone(), cx)); settle(cx, &f, scanned);
    let mut v = visual(cx, &f); v.simulate_resize(size(px(900.0), px(500.0))); draw(&mut v);
    assert!(f.view.read_with(&v, |r, _| r.session().state().tabs().is_empty() && r.session().state().results().is_none()));
    choose_menu_command(&mut v, &f, CommandId::CheckProjectSyntax);
    drop(v);
    settle(cx, &f, |r| r.session().state().results().is_some_and(|results| results.done()));
    let mut v = visual(cx, &f); draw(&mut v);
    let paths = f.view.read_with(&v, |r, _| {
        assert!(r.results_focus && r.session().state().tabs().is_empty());
        assert!(r.tree_lines(0).iter().any(|row| row.error), "{}; tree {:?}; {}x{}; tree rows {}", r.session().state().results().unwrap().kind(), r.tree_lines(0).iter().map(|row| &row.name).collect::<Vec<_>>(), r.width, r.height, r.tree_rows);
        let results = r.session().state().results().unwrap();
        assert!(results.kind().contains("1 unsupported"));
        results.hits().iter().map(|h| h.path.to_string()).collect::<Vec<_>>()
    });
    assert!(paths.iter().any(|p| p == "a.rs") && paths.iter().any(|p| p == "bad.xml"));
    v.simulate_resize(size(px(220.0), px(120.0))); draw(&mut v);
    choose_menu_command(&mut v, &f, CommandId::StatusDetails); draw(&mut v);
    assert!(f.view.read_with(&v, |r, _| matches!(&r.modal, Some(Modal::StatusDetails { text, .. }) if text.contains("1 unsupported") && text.contains("1024 paths"))));
    v.simulate_keystrokes("end up escape");
    assert!(f.view.read_with(&v, |r, _| r.modal.is_none()));
    assert_eq!(std::fs::read(root.join("a.rs")).unwrap(), b"fn broken( {\n");
}
