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
            // each heading with its section's size
            assert!(headings.iter().any(|heading| heading.strip_prefix(label).is_some_and(|n| n.trim().parse::<usize>().is_ok())), "{theme}: {headings:?}");
        }
        let header = f.view.read_with(&v, |r, _| {
            let layout = r.modal_layout().unwrap();
            let line = layout.lines.iter().position(|(row, text, _)| *row == usize::MAX && text.starts_with("Local branches")).unwrap();
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
fn gui_branch_picker_rows_stay_one_line_and_show_more_opens_a_section_by_key_and_click(cx: &mut TestAppContext) {
    use lenz_content::git::testing::git;
    let (f, _wt) = branches_fixture(cx);
    for i in 0..14 { git(f.dir.path(), &["branch", &format!("topic-a-very-long-branch-name-number-{i:02}-for-the-settings-dialog")]); }
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(520.0), px(700.0)));
    draw(&mut v);
    v.simulate_keystrokes("ctrl-shift-`");
    settle_for(cx, &f, 10, &|r| r.session().state().branches().is_some_and(|b| !b.loading && b.refs.local.len() > 10));
    draw(&mut v);
    // Every row is one line: no row's text spills onto a second line, long names keep both ends. (The test window
    // shapes no text: give it a character width to count columns with.)
    f.view.update(&mut v, |r, _| r.char_w = 8.0);
    let (lines, rows) = f.view.read_with(&v, |r, _| (r.modal_lines(), r.session().state().branch_rows("")));
    let branch_lines: Vec<_> = lines.iter().filter(|l| l.0 != usize::MAX && l.0 >= FINDER_HEADER_ROWS).collect();
    assert_eq!(branch_lines.len(), rows.len(), "one line per row");
    assert!(branch_lines.iter().any(|l| l.1.contains('…') && l.1.ends_with("dialog")), "{branch_lines:?}");
    // Enter on Show more lists the rest of the section and keeps the highlight on the first row it brings in.
    let more = rows.iter().position(|r| r.target == code_core::branches::BranchTarget::More("Local")).expect("a Show more row");
    f.view.update(&mut v, |r, _| r.modal = Some(Modal::Branches { query: String::new(), at: more }));
    v.simulate_keystrokes("enter");
    let (at, rows) = f.view.read_with(&v, |r, _| (match r.modal { Some(Modal::Branches { at, .. }) => at, _ => panic!("the picker stays open") }, r.session().state().branch_rows("")));
    assert_eq!(at, more);
    assert!(!rows.iter().any(|r| r.target == code_core::branches::BranchTarget::More("Local")));
    assert_eq!(rows[at].section, "Local");
    // Reopened, it is short again; a click on Show more opens it as Enter does.
    v.simulate_keystrokes("escape ctrl-shift-`");
    settle_for(cx, &f, 10, &|r| r.session().state().branches().is_some_and(|b| !b.loading));
    draw(&mut v);
    let click = f.view.read_with(&v, |r, _| {
        let layout = r.modal_layout().unwrap();
        let more = r.session().state().branch_rows("").iter().position(|r| r.target == code_core::branches::BranchTarget::More("Local")).unwrap();
        let line = layout.lines.iter().position(|(row, ..)| *row == more + FINDER_HEADER_ROWS).unwrap();
        assert!((layout.first..layout.first + layout.visible).contains(&line));
        point(layout.bounds.origin.x + px(40.0), layout.bounds.origin.y + px(layout.body_top + (line - layout.first) as f32 * line_h() + line_h() / 2.0))
    });
    v.simulate_click(click, Modifiers::default());
    assert!(f.view.read_with(&v, |r, _| r.modal.is_some() && !r.session().state().branch_rows("").iter().any(|r| r.target == code_core::branches::BranchTarget::More("Local"))));
    assert_eq!(std::fs::read(f.dir.path().join("a.txt")).unwrap(), b"main\n", "nothing was checked out");
}

#[gpui::test]
fn gui_branch_folders_open_by_key_and_a_star_from_the_menu_keeps_the_picker(cx: &mut TestAppContext) {
    use code_core::branches::BranchTarget;
    use lenz_content::git::testing::git;
    let (f, _wt) = branches_fixture(cx);
    for b in ["team/a", "team/b"] { git(f.dir.path(), &["branch", b]); }
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(900.0), px(700.0)));
    draw(&mut v);
    v.simulate_keystrokes("ctrl-shift-`");
    settle_for(cx, &f, 10, &|r| r.session().state().branches().is_some_and(|b| !b.loading && b.refs.local.iter().any(|l| l == "team/a")));
    let rows = |v: &VisualTestContext| f.view.read_with(v, |r, _| r.session().state().branch_rows(""));
    let folder = rows(&v).iter().position(|r| matches!(&r.target, BranchTarget::Folder("Local", n) if n == "team")).expect("a team folder");
    f.view.update(&mut v, |r, _| r.modal = Some(Modal::Branches { query: String::new(), at: folder }));
    // Enter opens it, ← closes it, → opens it again; the highlight stays on the folder.
    v.simulate_keystrokes("enter");
    assert!(rows(&v)[folder].open && rows(&v)[folder + 1].label == "a" && rows(&v)[folder + 1].depth == 1);
    v.simulate_keystrokes("left");
    assert!(!rows(&v)[folder].open);
    v.simulate_keystrokes("right");
    assert!(rows(&v)[folder].open);
    assert!(f.view.read_with(&v, |r, _| matches!(r.modal, Some(Modal::Branches { at, .. }) if at == folder)));
    // → on a branch lists its actions; Add to Favorites stars it and returns to the list, on that branch.
    let team_b = folder + 2;
    f.view.update(&mut v, |r, _| r.modal = Some(Modal::Branches { query: String::new(), at: team_b }));
    v.simulate_keystrokes("right");
    let star = f.view.read_with(&v, |r, _| match &r.modal { Some(Modal::BranchMenu { target, .. }) => r.session().branch_actions(target).iter().position(|(l, _)| *l == "Add to Favorites"), _ => None }).expect("the menu offers a star");
    for _ in 0..star { v.simulate_keystrokes("down"); }
    v.simulate_keystrokes("enter");
    let (at, rows) = f.view.read_with(&v, |r, _| (match r.modal { Some(Modal::Branches { at, .. }) => at, _ => panic!("back to the list") }, r.session().state().branch_rows("")));
    assert!(rows[at].favorite && matches!(&rows[at].target, BranchTarget::Branch(b) if b == "team/b"), "{:?}", rows[at]);
    assert_eq!(std::fs::read(f.dir.path().join("a.txt")).unwrap(), b"main\n", "nothing was checked out");
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
    f.view.update_in(&mut v, |r, window, cx| {
        r.dispatch(refresh, cx);
        assert!(!r.branch_hits_valid);
        assert!(r.session().state().branches().unwrap().refs.local.iter().any(|name| name == "aaa"));
        let Some(Modal::Branches { query, at }) = &r.modal else { panic!("picker remains open") };
        let selected = &r.session().state().branch_rows(query)[*at];
        assert_eq!((selected.section, selected.target.clone()), ("Local", BranchTarget::Branch("other".into())));
        // Deliver the pointer event in this transaction, before GPUI can repaint the changed list.
        r.on_mouse_down(&MouseDownEvent { button: MouseButton::Left, position: old_point, modifiers: Modifiers::default(), click_count: 1, first_mouse: false }, window, cx);
        assert!(matches!(r.modal, Some(Modal::Branches { .. })) && !r.session().state().checking_out());
    });
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
        f.view.update_in(&mut v, |r, window, cx| {
            if native {
                // Use the actual native callback in the same transaction as the click: the
                // PlatformInputHandler wrapper otherwise lets TestWindow repaint on return.
                <ReaderView as gpui::EntityInputHandler>::replace_text_in_range(r, None, "oth", window, cx);
            } else {
                for text in ["o", "t", "h"] {
                    let mut key = gpui::Keystroke::parse(text).unwrap();
                    key.key_char = Some(text.into());
                    r.on_key(&KeyDownEvent { keystroke: key, is_held: false, prefer_character_input: false }, window, cx);
                    // Registered printable keys are committed by the platform callback after
                    // the raw event; on_key deliberately defers their insertion to this callback.
                    <ReaderView as gpui::EntityInputHandler>::replace_text_in_range(r, None, text, window, cx);
                }
            }
            assert!(matches!(&r.modal, Some(Modal::Branches { query, .. }) if query == "oth"));
            assert!(!r.branch_hits_valid, "native={native}: edited filters invalidate old coordinates");
            r.on_mouse_down(&MouseDownEvent { button: MouseButton::Left, position: old_point, modifiers: Modifiers::default(), click_count: 1, first_mouse: false }, window, cx);
            assert!(matches!(&r.modal, Some(Modal::Branches { query, .. }) if query == "oth") && !r.session().state().checking_out());
        });
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

#[gpui::test]
fn gui_ignored_stale_branch_completion_keeps_painted_choices_clickable(cx: &mut TestAppContext) {
    use code_core::branches::BranchTarget;
    let (f, _wt) = branches_fixture(cx);
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(900.0), px(650.0)));
    draw(&mut v);
    v.simulate_keystrokes("ctrl-shift-`");
    settle_for(cx, &f, 10, &|r| r.session().state().branches().is_some_and(|b| !b.loading));
    draw(&mut v);
    let result = f.view.read_with(&v, |r, _| {
        assert!(r.branch_hits_valid);
        let selected = r.session().state().branch_rows("").iter().position(|row| row.target == BranchTarget::Branch("other".into())).unwrap();
        let layout = r.modal_layout().unwrap();
        let line = layout.lines.iter().position(|(row, _, _)| *row == selected + FINDER_HEADER_ROWS).unwrap();
        assert!((layout.first..layout.first + layout.visible).contains(&line));
        point(layout.bounds.center().x, layout.bounds.origin.y + px(layout.body_top + (line - layout.first) as f32 * line_h() + line_h() / 2.0))
    });
    // Opening the popup allocated a positive request. This obsolete reply is ignored by the core
    // and must not disable a current frame's hit map without requesting a replacement frame.
    f.view.update(&mut v, |r, cx| r.dispatch(Completion::Branches { request: 0, root: f.dir.path().to_path_buf(), result: Err("obsolete inventory".into()) }, cx));
    assert!(f.view.read_with(&v, |r, _| r.branch_hits_valid && r.session().state().branches().unwrap().error.is_none()));
    // No draw between the ignored completion and the first click.
    f.view.update_in(&mut v, |r, window, cx| r.on_mouse_down(&MouseDownEvent { button: MouseButton::Left, position: result, modifiers: Modifiers::default(), click_count: 1, first_mouse: false }, window, cx));
    settle_for(cx, &f, 10, &|r| branch_of(r).as_deref() == Some("⎇ other"));
    assert_eq!(std::fs::read(f.dir.path().join("a.txt")).unwrap(), b"other\n");
}
