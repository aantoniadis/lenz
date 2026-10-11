//! Seeded workflow and pending-result scenarios using the real TUI routes.
use super::*;

fn flow_fixture(case: &user_flows::Case) -> (tempfile::TempDir, App) {
    let dir = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
    for (side, content, sentinel) in [
        ("left", case.left, "sentinel left\n"),
        ("right", case.right, "sentinel right\n"),
    ] {
        std::fs::create_dir(dir.path().join(side)).unwrap();
        std::fs::write(dir.path().join(side).join("a.txt"), content).unwrap();
        std::fs::write(dir.path().join(side).join("b.txt"), sentinel).unwrap();
    }
    let mut app = open(dir.path(), &["--no-index", "left", "right"]);
    draw(&mut app, 120, 36);
    (dir, app)
}

#[test]
fn tui_seeded_user_flows_match_independent_history_model() {
    use user_flows::{RandomAction, Snapshot, Step};
    for case in user_flows::CASES {
        for seed in user_flows::seeds() {
            let (dir, mut app) = flow_fixture(case);
            let mut size = 0;
            let mut dimensions = (120, 36);
            user_flows::randomized(case, seed, |action| {
                match action {
                    RandomAction::Edit(step) => match step {
                        Step::Inspect => {}
                        Step::ApplyRight => {
                            for _ in 0..3 {
                                if app.session.view().current_change() == Some(0) {
                                    break;
                                }
                                key(&mut app, KeyCode::Char('n'), KeyModifiers::NONE);
                            }
                            assert_eq!(app.session.view().current_change(), Some(0));
                            key(
                                &mut app,
                                KeyCode::Right,
                                KeyModifiers::ALT | KeyModifiers::SHIFT,
                            );
                        }
                        Step::Undo => key(&mut app, KeyCode::Char('z'), KeyModifiers::CONTROL),
                        Step::Redo => key(
                            &mut app,
                            KeyCode::Char('z'),
                            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
                        ),
                        Step::Save => key(&mut app, KeyCode::Char('s'), KeyModifiers::CONTROL),
                        Step::Discard => {
                            key(&mut app, KeyCode::F(10), KeyModifiers::NONE);
                            text(&mut app, "Discard unsaved changes");
                            key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
                        }
                        Step::Reload => key(&mut app, KeyCode::F(5), KeyModifiers::NONE),
                        Step::SwitchRoundTrip => {
                            key(&mut app, KeyCode::Char(']'), KeyModifiers::NONE);
                            ready(&mut app);
                            assert_eq!(app.session.current(), 1);
                            assert_eq!(
                                app.session.document().unwrap().text(Side::Right),
                                "sentinel right\n"
                            );
                            key(&mut app, KeyCode::Char('['), KeyModifiers::NONE);
                        }
                        step => unreachable!("not generated: {step:?}"),
                    },
                    RandomAction::FindBlockedSave => {
                        key(&mut app, KeyCode::Char('f'), KeyModifiers::CONTROL);
                        app.handle(Event::Key(KeyEvent::new(
                            KeyCode::Char('s'),
                            KeyModifiers::CONTROL,
                        )));
                        assert!(app.session.find().is_some());
                        key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
                    }
                    RandomAction::Resize => {
                        dimensions = [(120, 36), (48, 16), (1, 1), (0, 0)][size % 4];
                        size += 1;
                        app.handle(Event::Resize(dimensions.0, dimensions.1));
                    }
                }
                ready(&mut app);
                draw(&mut app, dimensions.0, dimensions.1);
                assert_eq!(app.session.current(), 0);
                Snapshot {
                    documents: [Side::Left, Side::Right]
                        .map(|side| app.session.document().unwrap().text(side).to_owned()),
                    disk: ["left/a.txt", "right/a.txt"]
                        .map(|p| std::fs::read(dir.path().join(p)).unwrap()),
                    dirty: app.session.has_dirty(),
                    last_applied_active: app.session.applied().last().map(|r| r.active),
                }
            });
            assert_eq!(
                std::fs::read(dir.path().join("right/b.txt")).unwrap(),
                b"sentinel right\n"
            );
        }
    }
}

#[test]
fn tui_pending_switch_reload_resize_rejects_old_requests_and_keeps_draft() {
    let (dir, mut app) = flow_fixture(&user_flows::CASES[0]);
    key(
        &mut app,
        KeyCode::Right,
        KeyModifiers::ALT | KeyModifiers::SHIFT,
    );
    ready(&mut app);
    for round in 0..12 {
        // Avoid polling between these real events: old jobs remain in flight.
        for code in [KeyCode::Char(']'), KeyCode::Char('['), KeyCode::F(5)] {
            app.handle(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
        }
        let (width, height) = [(48, 16), (0, 0), (120, 36)][round % 3];
        app.handle(Event::Resize(width, height));
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| app.render(frame)).unwrap();
    }
    ready(&mut app);
    assert_eq!(app.session.current(), 0);
    assert_eq!(app.session.document().unwrap().text(Side::Right), "old\n");
    assert!(app.session.has_dirty());
    assert!(app.session.find().is_none());
    assert_eq!(app.session.applied().last().map(|r| r.active), Some(true));
    assert_eq!(
        std::fs::read(dir.path().join("right/a.txt")).unwrap(),
        b"new\n"
    );
    key(&mut app, KeyCode::Char('s'), KeyModifiers::CONTROL);
    assert_eq!(
        std::fs::read(dir.path().join("right/a.txt")).unwrap(),
        b"old\n"
    );
    let old_doc = std::sync::Arc::downgrade(&app.session.document_arc().unwrap());
    app.handle(Event::Key(KeyEvent::new(
        KeyCode::Char(']'),
        KeyModifiers::NONE,
    )));
    drop(app);
    // Closing drops the consumer immediately; a running refinement may own its
    // input until it returns. Drain earlier jobs before asserting final release.
    let barrier = std::sync::Arc::new(());
    assert_eq!(
        lenz::work::document(std::sync::Arc::downgrade(&barrier), || ()),
        Some(())
    );
    assert!(
        old_doc.upgrade().is_none(),
        "closed session retained its document"
    );
    assert_eq!(
        std::fs::read(dir.path().join("right/b.txt")).unwrap(),
        b"sentinel right\n"
    );
}

#[test]
fn tui_committed_find_preserves_navigation_and_editing_find_blocks_save() {
    let common = "needle row\n".repeat(300);
    for entry in lenz::theme::builtin_entries() {
        let dir = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        std::fs::write(dir.path().join("left"), common.clone() + "old\n").unwrap();
        std::fs::write(dir.path().join("right"), common.clone() + "new\n").unwrap();
        let mut app = open(dir.path(), &["--no-index", "left", "right"]);
        app.palette = entry.palette;
        draw(&mut app, 120, 36);
        if app.session.view().current_change() != Some(0) {
            key(&mut app, KeyCode::Char('n'), KeyModifiers::NONE);
        }
        key(
            &mut app,
            KeyCode::Right,
            KeyModifiers::ALT | KeyModifiers::SHIFT,
        );
        ready(&mut app);
        assert!(app.session.has_dirty(), "Apply did not create draft: {:?}", app.session.notice());
        key(&mut app, KeyCode::Char('z'), KeyModifiers::NONE);
        app.selection = None;
        app.session.go_to_line(Side::Right, 1);
        let draft = app.session.document().unwrap().text(Side::Right).to_owned();
        let bytes = std::fs::read(dir.path().join("right")).unwrap();
        key(&mut app, KeyCode::Char('f'), KeyModifiers::CONTROL);
        text(&mut app, "e");
        app.handle(Event::Key(KeyEvent::new(
            KeyCode::Char('s'),
            KeyModifiers::CONTROL,
        )));
        assert_eq!(std::fs::read(dir.path().join("right")).unwrap(), bytes);
        assert!(app.prompt.is_some());
        key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        assert!(app.prompt.is_none());
        let before = app.session.view().top_line_on(Side::Right);
        key(&mut app, KeyCode::PageDown, KeyModifiers::NONE);
        assert!(
            app.session.view().top_line_on(Side::Right) > before,
            "{}",
            entry.id
        );
        for navigation in [
            KeyCode::PageUp,
            KeyCode::Down,
            KeyCode::Up,
            KeyCode::Left,
            KeyCode::Right,
            KeyCode::Home,
            KeyCode::End,
            KeyCode::F(3),
        ] {
            key(&mut app, navigation, KeyModifiers::NONE);
            draw(&mut app, 120, 36);
            assert_eq!(app.session.find().unwrap().query(), "e");
            assert_eq!(app.session.document().unwrap().text(Side::Right), draft);
            assert_eq!(std::fs::read(dir.path().join("right")).unwrap(), bytes);
            assert!(app.session.has_dirty());
        }
    }
}
