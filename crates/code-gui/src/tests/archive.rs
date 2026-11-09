use super::*;

fn row_point(v: &VisualTestContext, f: &Fixture, name: &str) -> gpui::Point<Pixels> {
    f.view.read_with(v, |r, _| {
        let (index, row) = r
            .session
            .state()
            .tree_rows()
            .enumerate()
            .find(|(_, row)| row.name.as_ref() == name)
            .unwrap_or_else(|| {
                panic!(
                    "archive row {name:?} absent: {:?}",
                    r.session
                        .state()
                        .tree_rows()
                        .map(|row| row.name.to_string())
                        .collect::<Vec<_>>()
                )
            });
        point(
            px(6.0 + (row.depth as f32 * 2.0 + 0.5) * r.char_w),
            px(toolbar_h() + line_h() * ((index - r.tree_scroll) as f32 + 1.5)),
        )
    })
}

fn member(bytes: Vec<u8>, archive_name: &str, name: &str) -> Vec<u8> {
    let index = lenz_archive::Index::open(
        lenz_archive::Kind::of(archive_name).unwrap(),
        bytes.into(),
        Default::default(),
        &|| false,
    )
    .unwrap();
    let member = index
        .members()
        .iter()
        .find(|member| member.path.as_deref() == Some(name))
        .unwrap();
    index.read_member(member.id, &|| false).unwrap()
}

fn source(v: &VisualTestContext, f: &Fixture) -> String {
    f.view.read_with(v, |r, _| {
        let source = r.session.state().current_source().unwrap();
        source.copy_range(0..source.len_bytes()).unwrap()
    })
}

fn assert_human_context(v: &VisualTestContext, f: &Fixture, member: &str, archive: &str) {
    f.view.read_with(v, |r, _| {
        let title = r.session.state().active().unwrap().title();
        assert!(title.contains(member) && title.contains(archive), "{title}");
        let status = r.status();
        assert!(status.contains(archive), "{status}");
        let bar = r.session.state().breadcrumbs_for(0).unwrap();
        let labels: Vec<_> = bar.crumbs.iter().map(|crumb| crumb.name()).collect();
        assert!(
            labels.contains(&member) && labels.contains(&archive),
            "{labels:?}"
        );
        for label in std::iter::once(title.as_str())
            .chain(std::iter::once(status.as_str()))
            .chain(labels)
        {
            assert!(
                !label.contains("lenz-archive"),
                "internal identity leaked into presentation: {label}"
            );
        }
    });
}

#[gpui::test]
fn gui_archive_constructor_tree_entry_edit_and_save_preserve_container(cx: &mut TestAppContext) {
    let original = lenz_archive::testing::zip(&[
        ("src/main.txt", b"original\n"),
        ("src/other.txt", b"other member\n"),
        ("keep.txt", b"keep sibling\n"),
    ]);
    let f = fixture(cx, Some(("bundle.zip", &original)));
    settle(cx, &f, |r| {
        r.session
            .state()
            .tree_rows()
            .any(|row| row.name.as_ref() == "src")
    });
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(240.0), px(180.0)));
    draw(&mut v);
    f.view.read_with(&v, |r, _| {
        assert!(
            r.tree_focus && r.sidebar_w == 240.0,
            "a narrow constructor must show the archive browser"
        );
        assert_eq!(r.explorer_title(), "bundle.zip");
    });
    v.simulate_click(row_point(&v, &f, "src"), Modifiers::default());
    settle(cx, &f, |r| {
        r.session
            .state()
            .tree_rows()
            .any(|row| row.name.as_ref() == "main.txt")
    });
    draw(&mut v);
    v.simulate_click(row_point(&v, &f, "main.txt"), Modifiers::default());
    v.simulate_keystrokes("enter");
    settle(cx, &f, |r| {
        r.session
            .state()
            .current_source()
            .is_some_and(|source| source.source_meta().archive().is_some())
    });
    draw(&mut v);
    assert!(!f.view.read_with(&v, |r, _| r.tree_focus));
    assert_eq!(source(&v, &f), "original\n");
    assert_human_context(&v, &f, "main.txt", "bundle.zip");
    v.simulate_resize(size(px(900.0), px(500.0)));
    draw(&mut v);
    let sibling_menu = f.view.read_with(&v, |r, _| {
        let file = r
            .session
            .state()
            .breadcrumbs_for(0)
            .unwrap()
            .crumbs
            .iter()
            .position(|crumb| matches!(crumb, code_core::session::Crumb::File { .. }))
            .unwrap();
        r.crumb_hits
            .iter()
            .find(|hit| hit.crumb == Some(file) && hit.chevron)
            .unwrap()
            .bounds
            .center()
    });
    v.simulate_click(sibling_menu, Modifiers::default());
    assert!(
        f.view
            .read_with(&v, |r, _| matches!(r.modal, Some(Modal::Siblings { .. })))
    );
    v.simulate_keystrokes("down enter");
    settle(cx, &f, |r| {
        r.session
            .state()
            .active()
            .is_some_and(|tab| tab.title().starts_with("other.txt"))
            && r.session.state().current_source().is_some()
    });
    assert_eq!(
        source(&v, &f),
        "other member\n",
        "breadcrumb siblings open typed archive entries"
    );
    v.simulate_keystrokes("alt-left");
    settle(cx, &f, |r| {
        r.session
            .state()
            .active()
            .is_some_and(|tab| tab.title().starts_with("main.txt"))
            && r.session.state().current_source().is_some()
    });
    assert_eq!(source(&v, &f), "original\n");
    f.view
        .update(&mut v, |r, _| r.enable_draft_input_for_test());
    v.update(|_, cx| cx.write_to_clipboard(ClipboardItem::new_string("edited λ\n".into())));
    v.simulate_keystrokes("ctrl-a ctrl-v");
    draw(&mut v);
    assert_eq!(source(&v, &f), "edited λ\n");
    assert!(f.view.read_with(&v, |r, _| r.painted_badges.contains(&"●")));
    assert!(f.view.read_with(&v, |r, _| {
        r.session
            .state()
            .tree_rows()
            .any(|row| row.name.as_ref() == "main.txt" && r.session.state().row_dirty(row))
    }));
    assert_eq!(
        std::fs::read(f.dir.path().join("bundle.zip")).unwrap(),
        original,
        "typing changes only the member draft"
    );
    v.simulate_keystrokes("ctrl-s");
    settle(cx, &f, |r| {
        r.session
            .state()
            .active()
            .is_some_and(|tab| !r.session.state().tab_dirty(tab.id()))
            && r.session
                .state()
                .tree_rows()
                .any(|row| row.name.as_ref() == "main.txt")
    });
    let saved = std::fs::read(f.dir.path().join("bundle.zip")).unwrap();
    assert_eq!(
        member(saved.clone(), "bundle.zip", "src/main.txt"),
        "edited λ\n".as_bytes()
    );
    assert_eq!(member(saved, "bundle.zip", "keep.txt"), b"keep sibling\n");
    for theme in ["darcula", "github-light"] {
        f.view.update(&mut v, |r, cx| {
            r.dispatch(
                Command::Theme(ThemeCommand::Select(
                    ThemeSelection::from_id(theme).unwrap(),
                )),
                cx,
            )
        });
        for (width, height) in [(900.0, 500.0), (240.0, 180.0), (1.0, 1.0), (900.0, 500.0)] {
            v.simulate_resize(size(px(width), px(height)));
            draw(&mut v);
            assert_human_context(&v, &f, "main.txt", "bundle.zip");
        }
    }
    v.simulate_keystrokes("ctrl-w");
    settle(cx, &f, |r| {
        r.session
            .state()
            .tree_rows()
            .any(|row| row.name.as_ref() == "main.txt")
    });
    let at = row_point(&v, &f, "main.txt");
    v.simulate_click(at, Modifiers::default());
    v.simulate_keystrokes("enter");
    settle(cx, &f, |r| r.session.state().current_source().is_some());
    assert_eq!(
        source(&v, &f),
        "edited λ\n",
        "reopening reads the saved archive entry"
    );
}

#[gpui::test]
fn gui_nested_archive_save_preserves_both_containers_and_parent_keeps_open_tab(
    cx: &mut TestAppContext,
) {
    let inner = lenz_archive::testing::zip(&[
        ("main.txt", b"inner\n"),
        ("inner-keep.txt", b"inner sibling"),
    ]);
    let outer = lenz_archive::testing::zip(&[
        ("lib/inner.zip", &inner),
        ("outer-keep.txt", b"outer sibling"),
    ]);
    let f = fixture(cx, None);
    let path = f.dir.path().join("outer.zip");
    std::fs::write(&path, &outer).unwrap();
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(900.0), px(500.0)));
    draw(&mut v);
    v.simulate_keystrokes("ctrl-o");
    assert!(cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(|options| {
        assert!(options.files && !options.directories);
        Some(vec![path])
    });
    settle(cx, &f, |r| {
        r.session
            .state()
            .tree_rows()
            .any(|row| row.name.as_ref() == "lib")
    });
    draw(&mut v);
    v.simulate_click(row_point(&v, &f, "lib"), Modifiers::default());
    settle(cx, &f, |r| {
        r.session
            .state()
            .tree_rows()
            .any(|row| row.name.as_ref() == "inner.zip")
    });
    draw(&mut v);
    v.simulate_click(row_point(&v, &f, "inner.zip"), Modifiers::default());
    settle(cx, &f, |r| {
        r.session
            .state()
            .tree_rows()
            .any(|row| row.name.as_ref() == "main.txt")
    });
    draw(&mut v);
    v.simulate_click(row_point(&v, &f, "main.txt"), Modifiers::default());
    v.simulate_keystrokes("enter");
    settle(cx, &f, |r| r.session.state().current_source().is_some());
    draw(&mut v);
    assert_human_context(&v, &f, "main.txt", "inner.zip");
    assert!(
        f.view
            .read_with(&v, |r, _| r.status().contains("outer.zip"))
    );
    f.view
        .update(&mut v, |r, _| r.enable_draft_input_for_test());
    v.update(|_, cx| cx.write_to_clipboard(ClipboardItem::new_string("nested 😀\n".into())));
    v.simulate_keystrokes("ctrl-a ctrl-v ctrl-s");
    settle(cx, &f, |r| {
        r.session
            .state()
            .active()
            .is_some_and(|tab| !r.session.state().tab_dirty(tab.id()))
    });
    let saved = std::fs::read(f.dir.path().join("outer.zip")).unwrap();
    assert_eq!(
        member(saved.clone(), "outer.zip", "outer-keep.txt"),
        b"outer sibling"
    );
    let inner = member(saved, "outer.zip", "lib/inner.zip");
    assert_eq!(
        member(inner.clone(), "inner.zip", "main.txt"),
        "nested 😀\n".as_bytes()
    );
    assert_eq!(
        member(inner, "inner.zip", "inner-keep.txt"),
        b"inner sibling"
    );
    // Save refreshes both listings independently; the clean draft alone does not make tree coordinates ready.
    settle(cx, &f, |r| {
        r.session
            .state()
            .tree_rows()
            .any(|row| row.name.as_ref() == "inner.zip")
            && r.session
                .state()
                .tree_rows()
                .any(|row| row.name.as_ref() == "main.txt")
    });
    draw(&mut v);
    // Leaving an archive branch does not discard an ordinary editor tab opened from it.
    v.simulate_click(row_point(&v, &f, "inner.zip"), Modifiers::default());
    v.simulate_keystrokes("left");
    draw(&mut v);
    assert_eq!(source(&v, &f), "nested 😀\n");
    assert_human_context(&v, &f, "main.txt", "inner.zip");
}

#[gpui::test]
fn gui_archive_empty_error_retry_and_close_are_reachable_in_narrow_windows(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx, Some(("broken.zip", b"not an archive")));
    settle(cx, &f, |r| {
        r.session
            .state()
            .tree_rows()
            .any(|row| row.name.starts_with("Cannot open archive:"))
    });
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(240.0), px(180.0)));
    draw(&mut v);
    assert!(
        f.view
            .read_with(&v, |r, _| r.tree_focus && r.sidebar_w > 0.0)
    );
    assert!(f.view.read_with(&v, |r, _| {
        r.tree_lines(r.tree_at())
            .iter()
            .any(|row| row.name.contains("Enter to retry"))
    }));
    let empty = lenz_archive::testing::zip(&[]);
    std::fs::write(f.dir.path().join("broken.zip"), &empty).unwrap();
    v.simulate_keystrokes("end enter");
    settle(cx, &f, |r| {
        r.session
            .state()
            .tree_rows()
            .any(|row| row.name.as_ref() == "(Empty archive)")
    });
    draw(&mut v);
    assert!(
        f.view
            .read_with(&v, |r, _| r.session.state().active().is_none())
    );
    v.simulate_keystrokes("home down enter");
    draw(&mut v);
    assert!(!f.view.read_with(&v, |r, _| r.session.state().has_tree()));
    assert_eq!(
        std::fs::read(f.dir.path().join("broken.zip")).unwrap(),
        empty
    );
}

#[gpui::test]
fn gui_archive_loading_selection_stays_with_its_parent_over_project_rows(cx: &mut TestAppContext) {
    for retry in [false, true] {
        let f = fixture(cx, None);
        let project = f.dir.path().join("project");
        std::fs::create_dir(&project).unwrap();
        std::fs::write(project.join("a.txt"), b"physical file\n").unwrap();
        let path = f.dir.path().join("bundle.zip");
        let contents = lenz_archive::testing::zip(&[("entry.txt", b"member\n")]);
        std::fs::write(
            &path,
            if retry {
                b"invalid archive".as_slice()
            } else {
                &contents
            },
        )
        .unwrap();
        f.view.update(cx, |r, cx| r.open_root(project.clone(), cx));
        settle(cx, &f, scanned);
        let parent = f.view.update(cx, |r, cx| {
            assert_eq!(
                r.session.state().tree_rows().next().unwrap().name.as_ref(),
                "a.txt"
            );
            r.open(path.clone(), cx);
            let loading = r
                .session
                .state()
                .tree_rows()
                .position(|row| row.name.as_ref() == "Loading archive…")
                .unwrap();
            r.tree_move(loading);
            r.tree_focus = true;
            r.session
                .state()
                .archive_tree()
                .parent(r.tree_sel.as_deref().unwrap())
                .unwrap()
        });
        if retry {
            settle(cx, &f, |r| {
                r.session
                    .state()
                    .tree_rows()
                    .any(|row| row.name.starts_with("Cannot open archive:"))
            });
            std::fs::write(&path, &contents).unwrap();
            f.view.update(cx, |r, cx| {
                assert!(
                    r.session
                        .state()
                        .tree_rows()
                        .nth(r.tree_at())
                        .unwrap()
                        .name
                        .contains("Enter to retry")
                );
                r.activate_row(r.tree_at(), false, cx);
                assert_eq!(
                    r.session
                        .state()
                        .tree_rows()
                        .nth(r.tree_at())
                        .unwrap()
                        .name
                        .as_ref(),
                    "Loading archive…"
                );
            });
        }
        settle(cx, &f, |r| {
            r.session
                .state()
                .tree_rows()
                .any(|row| row.name.as_ref() == "entry.txt")
        });
        f.view.read_with(cx, |r, _| {
            assert_eq!(
                r.tree_sel.as_ref(),
                Some(&parent),
                "completion must not move to a physical project row"
            );
            assert_eq!(
                r.session
                    .state()
                    .tree_rows()
                    .nth(r.tree_at())
                    .unwrap()
                    .name
                    .as_ref(),
                "bundle.zip"
            );
        });
        let mut v = visual(cx, &f);
        draw(&mut v);
        v.simulate_keystrokes("enter");
        assert!(
            f.view
                .read_with(&v, |r, _| r.session.state().active().is_none()),
            "Enter must act on the archive parent, never open a physical file"
        );
        assert_eq!(
            std::fs::read(project.join("a.txt")).unwrap(),
            b"physical file\n"
        );
    }
}
