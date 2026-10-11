//! Seeded workflow and pending-result scenarios using the real GUI routes.
use super::*;

fn flow_fixture(cx: &mut TestAppContext, case: &user_flows::Case) -> Fixture {
    let f = fixture(cx, case.left, case.right);
    let left = f.dir.path().join("left");
    let right = f.dir.path().join("right");
    for (directory, content, sentinel) in [
        (&left, case.left, "sentinel left\n"),
        (&right, case.right, "sentinel right\n"),
    ] {
        std::fs::create_dir(directory).unwrap();
        std::fs::write(directory.join("a.txt"), content).unwrap();
        std::fs::write(directory.join("b.txt"), sentinel).unwrap();
    }
    let spec = Spec::Plain {
        left: left.clone(),
        right: right.clone(),
    };
    let set = Arc::new(ChangeSet::load(&spec, f.dir.path()).unwrap());
    let doc =
        Arc::new(Doc::load(&set, 0, &left.join("a.txt"), &right.join("a.txt"), false).unwrap());
    let view = doc.view(&doc.line_diff(&Options::default()), 30.);
    f.viewer.update(cx, |v, cx| {
        *v = Viewer::new(
            set,
            spec,
            false,
            None,
            doc,
            view,
            false,
            Options::default(),
            gpui::font("monospace"),
            Instant::now(),
            false,
            cx,
        );
    });
    f
}

#[gpui::test]
fn gui_seeded_user_flows_match_independent_history_model(cx: &mut TestAppContext) {
    use user_flows::{RandomAction, Snapshot, Step};
    for case in user_flows::CASES {
        for seed in user_flows::seeds() {
            let f = flow_fixture(cx, case);
            let mut size = 0;
            user_flows::randomized(case, seed, |action| {
                match action {
                    RandomAction::Edit(step) => match step {
                        Step::Inspect => {}
                        Step::ApplyRight => {
                            block(cx, &f, 0);
                            key(cx, &f, "right", false, true, true);
                        }
                        Step::Undo => key(cx, &f, "z", true, false, false),
                        Step::Redo => key(cx, &f, "z", true, true, false),
                        Step::Save => key(cx, &f, "s", true, false, false),
                        Step::Discard => run(cx,&f,crate::catalog::Action::Edit(crate::edit_session::EditMenuCommand::Discard)),
                        Step::Reload => run(cx,&f,crate::catalog::Action::Interaction(commands::Interaction::Reload)),
                        Step::SwitchRoundTrip => {
                            key(cx, &f, "]", false, false, false);
                            workflow_wait(cx, &f, |v| {
                                v.current == 1 && v.doc().is_some() && v.diffed == v.generation
                            });
                            assert_eq!(
                                f.viewer.read_with(cx, |v, _| v
                                    .doc()
                                    .unwrap()
                                    .text(Side::Right)
                                    .to_owned()),
                                "sentinel right\n"
                            );
                            key(cx, &f, "[", false, false, false);
                        }
                        step => unreachable!("not generated: {step:?}"),
                    },
                    RandomAction::FindBlockedSave => {
                        key(cx, &f, "f", true, false, false);
                        key(cx, &f, "s", true, false, false);
                        assert!(f.viewer.read_with(cx, |v, _| v.find.is_some()));
                        key(cx, &f, "escape", false, false, false);
                    }
                    RandomAction::Resize => {
                        let [width, height] = [[900., 650.], [400., 240.], [1., 1.]][size % 3];
                        size += 1;
                        cx.simulate_window_resize(
                            f.window.into(),
                            gpui::size(px(width), px(height)),
                        );
                    }
                }
                workflow_wait(cx, &f, |v| {
                    v.current == 0 && v.doc().is_some() && v.diffed == v.generation
                });
                f.window
                    .update(cx, |v, w, cx| {
                        let _ = v.render(w, cx);
                    })
                    .unwrap();
                f.viewer.read_with(cx, |v, _| Snapshot {
                    documents: [Side::Left, Side::Right]
                        .map(|side| v.doc().unwrap().text(side).to_owned()),
                    disk: ["left/a.txt", "right/a.txt"]
                        .map(|p| std::fs::read(f.dir.path().join(p)).unwrap()),
                    dirty: v.edits.has_dirty(),
                    last_applied_active: v.edits.applied(0).last().map(|r| r.active),
                })
            });
            assert_eq!(
                std::fs::read(f.dir.path().join("right/b.txt")).unwrap(),
                b"sentinel right\n"
            );
            f.window.update(cx, |_, w, _| w.remove_window()).unwrap();
            cx.executor().run_until_parked();
        }
    }
}

#[gpui::test]
fn gui_pending_switch_reload_resize_rejects_old_requests_and_keeps_draft(cx: &mut TestAppContext) {
    let f = flow_fixture(cx, &user_flows::CASES[0]);
    key(cx, &f, "right", false, true, true);
    workflow_wait(cx, &f, |v| v.diffed == v.generation);
    let mut retired = Vec::new();
    for round in 0..12 {
        // Route inputs without pumping the executor between them, so earlier
        // queued loads cannot settle before the user changes direction.
        f.window
            .update(cx, |v, w, cx| {
                retired.push(Arc::downgrade(&v.comparison.work_id));
                v.on_key(&event("]", false, false, false), w, cx);
                retired.push(Arc::downgrade(&v.comparison.work_id));
                v.on_key(&event("[", false, false, false), w, cx);
                v.open_palette(cx);
                v.on_key(&event("escape", false, false, false), w, cx);
            })
            .unwrap();
        let [width, height] = [[400., 240.], [1., 1.], [900., 650.]][round % 3];
        cx.simulate_window_resize(f.window.into(), gpui::size(px(width), px(height)));
        run(cx,&f,crate::catalog::Action::Interaction(commands::Interaction::Reload));
    }
    workflow_wait(cx, &f, |v| {
        v.current == 0 && v.doc().is_some() && v.diffed == v.generation
    });
    assert!(retired.iter().all(|token| token.strong_count() == 0));
    f.viewer.read_with(cx, |v, _| {
        assert_eq!(v.doc().unwrap().text(Side::Right), "old\n");
        assert!(v.edits.has_dirty());
        assert!(v.find.is_none());
        assert_eq!(v.edits.applied(0).last().map(|r| r.active), Some(true));
    });
    assert_eq!(
        std::fs::read(f.dir.path().join("right/a.txt")).unwrap(),
        b"new\n"
    );
    key(cx, &f, "s", true, false, false);
    assert_eq!(
        std::fs::read(f.dir.path().join("right/a.txt")).unwrap(),
        b"old\n"
    );
    f.window
        .update(cx, |v, w, cx| {
            v.on_key(&event("]", false, false, false), w, cx);
            w.remove_window();
        })
        .unwrap();
    let weak = f.viewer.downgrade();
    drop(f.viewer);
    cx.executor().run_until_parked();
    assert!(
        weak.upgrade().is_none(),
        "pending load retained a closed viewer"
    );
    assert_eq!(
        std::fs::read(f.dir.path().join("right/b.txt")).unwrap(),
        b"sentinel right\n"
    );
}

#[gpui::test]
fn gui_find_preserves_plain_navigation_and_blocks_modified_document_actions(
    cx: &mut TestAppContext,
) {
    let common = "needle row\n".repeat(300);
    for entry in theme::builtin_entries() {
        let name = &entry.id;
        let f = fixture(cx, &(common.clone() + "old\n"), &(common.clone() + "new\n"));
        run(cx,&f,crate::catalog::Action::Interaction(commands::Interaction::ChangeTheme));
        let selected = f.viewer.read_with(cx, |v, _| {
            v.theme_chooser
                .as_ref()
                .unwrap()
                .entries
                .iter()
                .position(|e| e.id == entry.id)
                .unwrap()
        });
        key(cx, &f, "home", false, false, false);
        for _ in 0..selected {
            key(cx, &f, "down", false, false, false);
        }
        key(cx, &f, "enter", false, false, false);
        block(cx, &f, 0);
        key(cx, &f, "right", false, true, true);
        workflow_wait(cx, &f, |v| v.diffed == v.generation);
        f.viewer.update(cx, |v, _| {
            v.comparison.collapse = false;
            v.comparison.view.set_collapsed(false);
            v.comparison.view.go_to_line(Side::Right, 1);
            v.diff_focus = Side::Right;
            v.sel = None;
        });
        let draft = f
            .viewer
            .read_with(cx, |v, _| v.doc().unwrap().text(Side::Right).to_owned());
        let bytes = std::fs::read(f.dir.path().join("right.txt")).unwrap();
        key(cx, &f, "f", true, false, false);
        key(cx, &f, "e", false, false, false);
        let before = f
            .viewer
            .read_with(cx, |v, _| v.comparison.view.top_line_on(Side::Right));
        key(cx, &f, "pagedown", false, false, false);
        assert!(
            f.viewer
                .read_with(cx, |v, _| v.comparison.view.top_line_on(Side::Right)
                    > before),
            "Find blocked PageDown in {name}"
        );
        for navigation in ["pageup", "down", "up", "left", "right", "home", "end", "f3"] {
            key(cx, &f, navigation, false, false, false);
            assert_eq!(
                f.viewer
                    .read_with(cx, |v, _| v.find.as_ref().unwrap().query.clone()),
                "e"
            );
        }
        for modified in ["s", "z", "right", "pagedown"] {
            key(cx, &f, modified, true, false, false);
            assert_eq!(
                f.viewer
                    .read_with(cx, |v, _| v.doc().unwrap().text(Side::Right).to_owned()),
                draft
            );
            assert_eq!(
                std::fs::read(f.dir.path().join("right.txt")).unwrap(),
                bytes
            );
            assert!(
                f.viewer
                    .read_with(cx, |v, _| v.edits.has_dirty() && v.find.is_some())
            );
        }
        key(cx, &f, "escape", false, false, false);
        f.window.update(cx, |_, w, _| w.remove_window()).unwrap();
        cx.executor().run_until_parked();
    }
}
