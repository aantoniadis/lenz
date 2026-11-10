use super::*;

#[gpui::test]
fn gui_branch_sections_are_visible_inert_and_keep_action_navigation(cx: &mut TestAppContext) {
    use lenz_content::git::testing::git;
    let (f, _wt) = branches_fixture(cx);
    git(f.dir.path(), &["update-ref", "refs/remotes/origin/main", "main"]);
    let mut v = visual(cx, &f);
    for theme in ["darcula", "github-light"] {
        f.view.update(&mut v, |r, cx| r.dispatch(Command::Theme(ThemeCommand::Select(ThemeSelection::from_id(theme).unwrap())), cx));
        v.simulate_resize(size(px(900.0), px(650.0)));
        draw(&mut v);
        v.simulate_keystrokes("ctrl-shift-`");
        settle_for(cx, &f, 10, &|r| r.session().state().branches().is_some_and(|b| !b.loading && b.refs.remote.iter().any(|r| r == "origin/main")));
        draw(&mut v);
        let headings = f.view.read_with(&v, |r, _| r.modal_lines().into_iter().filter(|r| r.0 == usize::MAX).map(|r| r.1).collect::<Vec<_>>());
        for label in ["Recent branches", "Local branches", "Remote branches", "Tags", "Worktrees"] {
            assert!(headings.iter().any(|heading| heading == label), "{theme}: {headings:?}");
        }
        let header = f.view.read_with(&v, |r, _| {
            let layout = r.modal_layout().unwrap();
            let line = layout.lines.iter().position(|(row, text, _)| *row == usize::MAX && text == "Local branches").unwrap();
            assert!((layout.first..layout.first + layout.visible).contains(&line));
            point(layout.bounds.origin.x + px(24.0), layout.bounds.origin.y + px(layout.body_top + (line - layout.first) as f32 * line_h() + line_h() / 2.0))
        });
        v.simulate_click(header, Modifiers::default());
        assert!(f.view.read_with(&v, |r, _| matches!(r.modal, Some(Modal::Branches { at: 0, .. }))));
        assert_eq!(std::fs::read(f.dir.path().join("a.txt")).unwrap(), b"main\n");
        v.simulate_keystrokes("pagedown");
        let down = f.view.read_with(&v, |r, _| match r.modal { Some(Modal::Branches { at, .. }) => at, _ => panic!("branch picker remains open") });
        assert!(down > 0, "PageDown moves to a choice, skipping section headings");
        v.simulate_keystrokes("pageup");
        assert!(f.view.read_with(&v, |r, _| matches!(r.modal, Some(Modal::Branches { at, .. }) if at < down)));
        v.simulate_keystrokes("alt-end up");
        let last = f.view.read_with(&v, |r, _| r.session().state().branch_rows("").len() - 1);
        assert!(f.view.read_with(&v, |r, _| matches!(r.modal, Some(Modal::Branches { at, .. }) if at == last - 1)));
        v.simulate_keystrokes("escape");
    }
}

#[gpui::test]
fn gui_narrow_branch_picker_keeps_a_visible_clickable_result(cx: &mut TestAppContext) {
    let (f, _wt) = branches_fixture(cx);
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(260.0), px(200.0)));
    draw(&mut v);
    v.simulate_keystrokes("ctrl-shift-`");
    settle_for(cx, &f, 10, &|r| r.session().state().branches().is_some_and(|b| !b.loading));
    type_text(&mut v, "oth");
    draw(&mut v);
    let result = f.view.read_with(&v, |r, _| {
        let layout = r.modal_layout().unwrap();
        assert!(layout.bounds.right() <= px(260.0) && layout.bounds.bottom() <= px(200.0));
        assert!(layout.visible > 0, "wrapped help must leave space for a result");
        let line = layout.lines.iter().enumerate().skip(layout.first).take(layout.visible).find(|(_, (row, _, _))| *row == layout.at).map(|(line, _)| line).expect("selected action is visible");
        point(layout.bounds.origin.x + px(24.0), layout.bounds.origin.y + px(layout.body_top + (line - layout.first) as f32 * line_h() + line_h() / 2.0))
    });
    v.simulate_click(result, Modifiers::default());
    settle_for(cx, &f, 10, &|r| branch_of(r).as_deref() == Some("⎇ other"));
    assert!(f.view.read_with(&v, |r, _| r.modal.is_none()));
    assert_eq!(std::fs::read(f.dir.path().join("a.txt")).unwrap(), b"other\n");
}

#[gpui::test]
fn gui_cached_branch_refresh_keeps_the_target_and_rejects_old_pointer_rows(cx: &mut TestAppContext) {
    use code_core::branches::BranchTarget;
    use lenz_content::git::testing::git;
    let (f, _wt) = branches_fixture(cx);
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(900.0), px(650.0)));
    draw(&mut v);
    v.simulate_keystrokes("ctrl-shift-`");
    settle_for(cx, &f, 10, &|r| r.session().state().branches().is_some_and(|b| !b.loading));
    draw(&mut v);
    let selected = f.view.read_with(&v, |r, _| r.session().state().branch_rows("").iter().position(|row| row.section == "Local" && row.target == BranchTarget::Branch("other".into())).unwrap());
    let old_point = f.view.read_with(&v, |r, _| {
        let layout = r.modal_layout().unwrap();
        let line = layout.lines.iter().position(|(row, _, _)| *row == selected + FINDER_HEADER_ROWS).unwrap();
        assert!((layout.first..layout.first + layout.visible).contains(&line));
        point(layout.bounds.center().x, layout.bounds.origin.y + px(layout.body_top + (line - layout.first) as f32 * line_h() + line_h() / 2.0))
    });
    git(f.dir.path(), &["branch", "aaa", "main"]);
    let completions = f.view.update(&mut v, |r, cx| {
        r.dispatch(Command::CloseBranches, cx);
        r.dispatch(Command::OpenBranches, cx);
        assert!(!r.session().state().branches().unwrap().refs.local.iter().any(|name| name == "aaa"), "the retained snapshot is shown immediately");
        r.modal = Some(Modal::Branches { query: String::new(), at: selected });
        r.session.completions()
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let refresh = loop {
        match completions.try_recv() {
            Ok(completion @ Completion::Branches { .. }) => break completion,
            Ok(completion) => f.view.update(&mut v, |r, cx| r.dispatch(completion, cx)),
            Err(_) => { assert!(std::time::Instant::now() < deadline, "branch refresh arrives"); std::thread::sleep(std::time::Duration::from_millis(2)); }
        }
    };
    f.view.update(&mut v, |r, cx| r.dispatch(refresh, cx));
    assert!(f.view.read_with(&v, |r, _| !r.branch_hits_valid));
    let target = f.view.read_with(&v, |r, _| {
        assert!(r.session().state().branches().unwrap().refs.local.iter().any(|name| name == "aaa"));
        let Some(Modal::Branches { query, at }) = &r.modal else { panic!("picker remains open") };
        let selected = &r.session().state().branch_rows(query)[*at];
        (selected.section, selected.target.clone())
    });
    assert_eq!(target, ("Local", BranchTarget::Branch("other".into())));
    // Deliver a pointer event before the next frame: old coordinates must not pick a newly inserted row.
    f.view.update_in(&mut v, |r, window, cx| r.on_mouse_down(&MouseDownEvent { button: MouseButton::Left, position: old_point, modifiers: Modifiers::default(), click_count: 1, first_mouse: false }, window, cx));
    assert!(f.view.read_with(&v, |r, _| matches!(r.modal, Some(Modal::Branches { .. })) && !r.session().state().checking_out()));
    assert_eq!(f.view.read_with(&v, |r, _| branch_of(r)), Some("⎇ main".into()));
    draw(&mut v);
    v.simulate_keystrokes("enter");
    settle_for(cx, &f, 10, &|r| branch_of(r).as_deref() == Some("⎇ other"));
    assert_eq!(std::fs::read(f.dir.path().join("a.txt")).unwrap(), b"other\n");
}

#[gpui::test]
fn gui_branch_filter_edits_reject_pointer_rows_until_repaint(cx: &mut TestAppContext) {
    for native in [false, true] {
        let (f, _wt) = branches_fixture(cx);
        let mut v = visual(cx, &f);
        v.simulate_resize(size(px(900.0), px(650.0)));
        draw(&mut v);
        v.simulate_keystrokes("ctrl-shift-`");
        settle_for(cx, &f, 10, &|r| r.session().state().branches().is_some_and(|b| !b.loading));
        draw(&mut v);
        // This old command row will overlap a branch after filtering, so reusing its coordinates is unsafe.
        let old_point = f.view.read_with(&v, |r, _| {
            let layout = r.modal_layout().unwrap();
            let line = layout.lines.iter().position(|(row, _, _)| *row == FINDER_HEADER_ROWS + 1).unwrap();
            point(layout.bounds.center().x, layout.bounds.origin.y + px(layout.body_top + (line - layout.first) as f32 * line_h() + line_h() / 2.0))
        });
        if native {
            let mut input = ime_handler(&f, &mut v);
            input.replace_text_in_range(None, "oth");
        } else {
            type_text(&mut v, "oth");
        }
        assert!(f.view.read_with(&v, |r, _| !r.branch_hits_valid), "native={native}: edited filters invalidate old coordinates");
        f.view.update_in(&mut v, |r, window, cx| r.on_mouse_down(&MouseDownEvent { button: MouseButton::Left, position: old_point, modifiers: Modifiers::default(), click_count: 1, first_mouse: false }, window, cx));
        assert!(f.view.read_with(&v, |r, _| matches!(&r.modal, Some(Modal::Branches { query, .. }) if query == "oth") && !r.session().state().checking_out()));
        assert_eq!(std::fs::read(f.dir.path().join("a.txt")).unwrap(), b"main\n");
        draw(&mut v);
        let fresh_point = f.view.read_with(&v, |r, _| {
            let layout = r.modal_layout().unwrap();
            let line = layout.lines.iter().enumerate().skip(layout.first).take(layout.visible).find(|(_, (row, _, _))| *row == layout.at).map(|(line, _)| line).unwrap();
            point(layout.bounds.center().x, layout.bounds.origin.y + px(layout.body_top + (line - layout.first) as f32 * line_h() + line_h() / 2.0))
        });
        v.simulate_click(fresh_point, Modifiers::default());
        settle_for(cx, &f, 10, &|r| branch_of(r).as_deref() == Some("⎇ other"));
        assert_eq!(std::fs::read(f.dir.path().join("a.txt")).unwrap(), b"other\n");
    }
}
