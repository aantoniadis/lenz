use super::*;

#[test]
fn branch_picker_groups_are_nonselectable_and_filter_to_their_rows() {
    use lenz_content::{git::testing::git, theme::{ThemeCommand, ThemeSelection}};
    let (r, _wt, mut a) = branches_app();
    git(r.path(), &["update-ref", "refs/remotes/origin/topic", "HEAD"]);
    open_popup(&mut a);
    for theme in ["github-light", "darcula"] {
        a.dispatch(Command::Theme(ThemeCommand::Select(ThemeSelection::from_id(theme).unwrap())));
        let screen = draw_screen(&mut a, 100, 32);
        for heading in ["Local branches", "Remote branches", "Tags", "Worktrees"] {
            assert_eq!(screen.matches(heading).count(), 1, "{theme}: {heading}\n{screen}");
        }
        let selected = a.modal.as_ref().unwrap().at;
        for row in 0..a.layout.modal_items.len() {
            if a.layout.modal_items[row] != usize::MAX { continue; }
            let (x, y) = (a.layout.modal_rows.x, a.layout.modal_rows.y + row as u16);
            click(&mut a, x, y);
            mouse(&mut a, MouseButton::Right, x, y);
            assert_eq!(prompt_kind(&a), Some(Prompt::Branches));
            assert_eq!(a.modal.as_ref().unwrap().at, selected, "headers do not select or run a row");
        }
    }
    typed(&mut a, "origin/topic");
    let screen = draw_screen(&mut a, 100, 24);
    assert!(screen.contains("Remote branches") && screen.contains("origin/topic"));
    assert!(!screen.contains("Local branches") && !screen.contains("Worktrees"));
    assert_eq!(a.choices().len(), 1);
    assert!(matches!(&a.choices()[0].target, ChoiceTarget::Branch(code_core::branches::BranchTarget::Remote(name)) if name == "origin/topic"));
    key(&mut a, KeyCode::Enter, KeyModifiers::ALT);
    assert!(matches!(prompt_kind(&a), Some(Prompt::BranchMenu { .. })));
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    assert_eq!(a.modal.as_ref().unwrap().input, "origin/topic");
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    assert!(a.modal.is_none());
    assert_eq!(git(r.path(), &["branch", "--show-current"]).trim(), "main");
}

#[test]
fn branch_picker_selection_and_headers_survive_tiny_resizes_and_scrolling() {
    let (r, _wt, mut a) = branches_app();
    open_popup(&mut a);
    let count = a.choices().len();
    for (width, height) in [(100, 32), (48, 12), (24, 10), (8, 5), (1, 1), (0, 0), (100, 32)] {
        draw(&mut a, width, height);
        key(&mut a, KeyCode::End, KeyModifiers::ALT);
        draw(&mut a, width, height);
        assert_eq!(a.modal.as_ref().unwrap().at, count - 1);
        assert!(a.modal.as_ref().unwrap().top <= a.layout.modal_content_rows.saturating_sub(a.layout.modal_rows.height as usize));
        if a.layout.modal_rows.height > 0 {
            assert!(a.layout.modal_items.contains(&(count - 1)), "selected actionable row remains visible at {width}×{height}");
        }
        key(&mut a, KeyCode::Up, KeyModifiers::NONE);
        draw(&mut a, width, height);
        assert_eq!(a.modal.as_ref().unwrap().at, count - 2);
    }
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    assert!(a.modal.is_none());
    assert_eq!(std::fs::read(r.path().join("a.txt")).unwrap(), b"main\n");
}

#[test]
fn branch_filter_right_opens_actions_only_after_caret_motion_and_selection() {
    let (_r, _wt, mut a) = branches_app();
    open_popup(&mut a);
    typed(&mut a, "oth");
    key(&mut a, KeyCode::Left, KeyModifiers::NONE);
    assert_eq!(App::field_cursor(a.modal.as_ref().unwrap()).caret("oth"), 2);
    key(&mut a, KeyCode::Right, KeyModifiers::NONE);
    assert_eq!(prompt_kind(&a), Some(Prompt::Branches));
    assert_eq!(App::field_cursor(a.modal.as_ref().unwrap()).caret("oth"), 3);
    key(&mut a, KeyCode::Char('a'), KeyModifiers::CONTROL);
    key(&mut a, KeyCode::Right, KeyModifiers::NONE);
    assert_eq!(prompt_kind(&a), Some(Prompt::Branches), "Right collapses selection before opening actions");
    key(&mut a, KeyCode::Right, KeyModifiers::NONE);
    assert!(matches!(prompt_kind(&a), Some(Prompt::BranchMenu { query, .. }) if query == "oth"));
    let screen = draw_screen(&mut a, 100, 24);
    assert!(screen.contains("Esc back") && !screen.contains("Tab field"));
    assert_eq!(a.layout.modal_items.first(), Some(&0), "row actions start immediately without a fake text field");
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    key(&mut a, KeyCode::Backspace, KeyModifiers::NONE);
    assert_eq!(a.modal.as_ref().unwrap().input, "ot");
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
}

#[test]
fn branch_picker_shows_loading_and_no_match_messages() {
    let (_r, _wt, mut a) = branches_app();
    a.run(CommandId::GitBranches);
    assert!(draw_screen(&mut a, 100, 24).contains("loading…"));
    settle(&mut a, |a| a.session.state().branches().is_some_and(|b| !b.loading));
    typed(&mut a, "no_such_ref_or_worktree_2537");
    let screen = draw_screen(&mut a, 100, 24);
    assert!(screen.contains("Nothing matches this filter"), "{screen}");
    assert!(a.choices().is_empty());
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(prompt_kind(&a), Some(Prompt::Branches));
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
}

#[test]
fn branch_picker_renders_inventory_failures_in_the_scrollable_body() {
    let (r, _wt, mut a) = branches_app();
    std::fs::rename(r.path().join(".git"), r.path().join("saved.git")).unwrap();
    a.run(CommandId::GitBranches);
    settle(&mut a, |a| a.session.state().branches().is_some_and(|b| !b.loading));
    let error = a.session.state().branches().unwrap().error.as_ref().expect("Git inventory fails without its repository metadata").clone();
    let prefix: String = error.chars().take(40).collect();
    let screen = draw_screen(&mut a, 100, 24);
    assert!(screen.contains(&prefix), "inventory errors are visible, not dropped with the compact title: {screen}");
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    std::fs::rename(r.path().join("saved.git"), r.path().join(".git")).unwrap();
}

#[test]
fn cached_branch_refresh_keeps_target_identity_and_invalidates_stale_mouse_rows() {
    use lenz_content::git::testing::git;
    let (r, _wt, mut a) = branches_app();
    git(r.path(), &["update-ref", "refs/remotes/origin/topic", "HEAD"]);
    open_popup(&mut a);
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    git(r.path(), &["branch", "aaa"]);
    a.run(CommandId::GitBranches);
    assert!(a.session.state().branches().unwrap().loading);
    draw(&mut a, 100, 32);
    let remote = code_core::branches::BranchTarget::Remote("origin/topic".into());
    let old_at = a.choices().iter().position(|c| matches!(&c.target, ChoiceTarget::Branch(t) if *t == remote))
        .unwrap_or_else(|| panic!("cached remote missing: prompt {:?}, view {:?}, choices {:?}", prompt_kind(&a), a.session.state().branches(), a.choices().iter().map(|c| &c.label).collect::<Vec<_>>()));
    for _ in 0..old_at { key(&mut a, KeyCode::Down, KeyModifiers::NONE); }
    draw(&mut a, 100, 32);
    assert!(!a.layout.modal_items.is_empty());
    let old_mouse = (a.layout.modal_rows.x, a.layout.modal_rows.y);
    settle(&mut a, |a| !a.session.state().branches().unwrap().loading);
    let at = a.modal.as_ref().unwrap().at;
    assert!(at > old_at, "a newly inserted local branch moves the remote row");
    assert!(matches!(&a.choices()[at].target, ChoiceTarget::Branch(t) if *t == remote));
    assert!(a.modal.as_ref().unwrap().input.is_empty());
    assert!(a.layout.modal_items.is_empty(), "old hit indices are unavailable until repaint");
    click(&mut a, old_mouse.0, old_mouse.1);
    assert_eq!(prompt_kind(&a), Some(Prompt::Branches));
    assert_eq!(git(r.path(), &["branch", "--show-current"]).trim(), "main");
    draw(&mut a, 100, 32);
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);

    git(r.path(), &["update-ref", "-d", "refs/remotes/origin/topic"]);
    a.run(CommandId::GitBranches);
    draw(&mut a, 100, 32);
    typed(&mut a, "topic");
    assert_eq!(a.choices().len(), 1, "the cached target remains usable during refresh");
    settle(&mut a, |a| !a.session.state().branches().unwrap().loading);
    assert_eq!(a.modal.as_ref().unwrap().input, "topic");
    assert_eq!(a.modal.as_ref().unwrap().at, 0);
    assert!(a.choices().is_empty(), "removed target is not replaced by an unrelated branch");
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(prompt_kind(&a), Some(Prompt::Branches));
    assert_eq!(git(r.path(), &["branch", "--show-current"]).trim(), "main");
}
