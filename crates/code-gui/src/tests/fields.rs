use super::*;

#[gpui::test]
fn find_carets_and_selections_do_not_leak_between_tabs(cx: &mut TestAppContext) {
    let f = fixture(cx, Some(("first.txt", b"first source\n")));
    settle(cx, &f, |r| r.session.state().current_source().is_some());
    let second = f.dir.path().join("second.txt");
    std::fs::write(&second, "second source\n").unwrap();
    let mut v = visual(cx, &f);
    let first = f.view.update(&mut v, |r, cx| {
        r.dispatch(Command::Find("abcdef".into()), cx);
        r.find_focus = Some(r.session.state().focused_pane());
        r.session.state().active().unwrap().id()
    });
    v.simulate_keystrokes("home shift-right shift-right shift-right");
    assert_eq!(f.view.read_with(&v, |r, _| {
        let (_, text, cursor) = r.field_source().unwrap();
        cursor.selected(text).map(str::to_owned)
    }).as_deref(), Some("abc"));
    f.view.update(&mut v, |r, cx| r.dispatch(Command::OpenRetained(second.clone()), cx));
    settle(cx, &f, |r| r.session.state().current_source().is_some_and(|s| s.source() == second));
    f.view.update(&mut v, |r, cx| {
        r.dispatch(Command::Find("xy".into()), cx);
        r.find_focus = Some(r.session.state().focused_pane());
    });
    // This first keystroke must append; the previous tab's selected range would replace the whole short query.
    v.simulate_keystrokes("z");
    assert_eq!(f.view.read_with(&v, |r, _| r.session.state().active().unwrap().find().unwrap().query().to_owned()), "xyz");
    f.view.update(&mut v, |r, cx| {
        r.dispatch(Command::ActivateTab(first), cx);
        r.find_focus = Some(r.session.state().focused_pane());
    });
    // Returning to an inactive tab validates its released source on a worker before Find can edit
    // that tab's query. Wait for that load, as the earlier switch to the second tab does above.
    settle(cx, &f, |r| r.session.state().active().is_some_and(|t| t.id() == first)
        && r.session.state().loading().is_none()
        && r.session.state().current_source().is_some_and(|s| s.source() == f.dir.path().join("first.txt")));
    assert_eq!(f.view.read_with(&v, |r, _| {
        let (_, text, cursor) = r.field_source().unwrap();
        (cursor.caret(text), cursor.selection(text))
    }), (6, None), "the returned tab starts at its own query end without the other tab's caret or selection");
    v.simulate_keystrokes("backspace");
    assert_eq!(f.view.read_with(&v, |r, _| r.session.state().active().unwrap().find().unwrap().query().to_owned()), "abcde");
    assert_eq!(std::fs::read(f.dir.path().join("first.txt")).unwrap(), b"first source\n");
    assert_eq!(std::fs::read(second).unwrap(), b"second source\n");
}

#[gpui::test]
fn popup_fields_copy_cut_paste_ranges_and_leave_source_untouched(cx: &mut TestAppContext) {
    let f = fixture(cx, Some(("original.txt", b"original\n")));
    settle(cx, &f, |v| v.session.state().current_source().is_some());
    let mut visual = visual(cx, &f);
    f.view.update(&mut visual, |v, _| {
        v.modal = Some(Modal::NewScratch {
            input: "one é👩🏽‍💻 two".into(),
            selected: false,
            error: None,
        })
    });
    visual.simulate_keystrokes("home ctrl-right right shift-right shift-right ctrl-c");
    assert_eq!(
        visual
            .update(|_, cx| cx.read_from_clipboard().and_then(|c| c.text()))
            .as_deref(),
        Some("é👩🏽‍💻")
    );
    visual.simulate_keystrokes("ctrl-x");
    assert!(f.view.read_with(
        &visual,
        |v, _| matches!(&v.modal,Some(Modal::NewScratch{input,..})if input=="one  two")
    ));
    visual.simulate_keystrokes("ctrl-v");
    assert!(f.view.read_with(
        &visual,
        |v, _| matches!(&v.modal,Some(Modal::NewScratch{input,..})if input=="one é👩🏽‍💻 two")
    ));
    visual.simulate_keystrokes("ctrl-a");
    visual.update(|_, cx| cx.write_to_clipboard(ClipboardItem::new_string(String::new())));
    visual.simulate_keystrokes("ctrl-v");
    assert!(f.view.read_with(
        &visual,
        |v, _| matches!(&v.modal,Some(Modal::NewScratch{input,..})if input=="one é👩🏽‍💻 two")
    ));
    visual.simulate_resize(size(px(240.), px(180.)));
    draw(&mut visual);
    visual.simulate_keystrokes("ctrl-s ctrl-z escape");
    assert!(f.view.read_with(&visual, |v, _| v.modal.is_none()));
    assert_eq!(
        std::fs::read(f.dir.path().join("original.txt")).unwrap(),
        b"original\n"
    );
    assert_eq!(
        f.view.read_with(&visual, |v, _| {
            let s = v.session.state().current_source().unwrap();
            s.copy_range(0..s.len_bytes()).unwrap()
        }),
        "original\n"
    );
}

#[gpui::test]
fn project_search_fields_retain_independent_selections(cx: &mut TestAppContext) {
    let f = fixture(cx, Some(("original.txt", b"original\n")));
    settle(cx, &f, |v| v.session.state().current_source().is_some());
    let mut visual = visual(cx, &f);
    f.view.update(&mut visual, |v, _| {
        v.modal = Some(Modal::Search {
            query: "first".into(),
            files: "*.rs".into(),
            in_files: false,
            project: true,
            selected: false,
            caret: None,
        })
    });
    visual.simulate_keystrokes("home shift-right tab home shift-right ctrl-c");
    assert_eq!(
        visual
            .update(|_, cx| cx.read_from_clipboard().and_then(|c| c.text()))
            .as_deref(),
        Some("*")
    );
    visual.simulate_keystrokes("shift-tab ctrl-c");
    assert_eq!(
        visual
            .update(|_, cx| cx.read_from_clipboard().and_then(|c| c.text()))
            .as_deref(),
        Some("f")
    );
    visual.simulate_keystrokes("ctrl-x");
    assert!(f.view.read_with(
        &visual,
        |v, _| matches!(&v.modal,Some(Modal::Search{query,files,..})if query=="irst"&&files=="*.rs")
    ));
    visual.simulate_keystrokes("escape");
}

#[gpui::test]
fn popup_character_and_word_selection_matches_the_text_editor(cx: &mut TestAppContext) {
    let f = fixture(cx, Some(("original.txt", "one é👩🏽‍💻 two\n".as_bytes())));
    settle(cx, &f, |v| v.session.state().current_source().is_some());
    let mut visual = visual(cx, &f);
    f.view
        .update(&mut visual, |v, _| v.enable_draft_input_for_test());
    visual.simulate_keystrokes("home ctrl-right right shift-right shift-right ctrl-c");
    let from_editor = visual.update(|_, cx| cx.read_from_clipboard().and_then(|c| c.text()));
    f.view.update(&mut visual, |v, _| {
        v.modal = Some(Modal::NewScratch {
            input: "one é👩🏽‍💻 two".into(),
            selected: false,
            error: None,
        })
    });
    visual.simulate_keystrokes("home ctrl-right right shift-right shift-right ctrl-c");
    let from_field = visual.update(|_, cx| cx.read_from_clipboard().and_then(|c| c.text()));
    assert_eq!(from_field, from_editor);
    assert_eq!(from_field.as_deref(), Some("é👩🏽‍💻"));
}

#[gpui::test]
fn option_printable_text_reaches_fields_without_stealing_reserved_find_actions(cx: &mut TestAppContext) {
    let f = fixture(cx, Some(("original.txt", b"original\n")));
    settle(cx, &f, |v| v.session.state().current_source().is_some());
    let mut v = visual(cx, &f);
    v.simulate_keystrokes("ctrl-f"); draw(&mut v);
    let option = |name: &str, text: &str| {
        let mut keystroke = gpui::Keystroke::parse(&format!("alt-{name}")).unwrap();
        keystroke.key_char = Some(text.into());
        gpui::KeyDownEvent { keystroke, is_held: false, prefer_character_input: false }
    };
    v.simulate_event(option("2", "™"));
    #[cfg(target_os = "macos")]
    ime_handler(&f, &mut v).replace_text_in_range(None, "™"); // TestPlatform has no Cocoa input context.
    assert_eq!(f.view.read_with(&v, |r, _| r.session.state().active().unwrap().find().unwrap().query().to_owned()), "™");
    let before = f.view.read_with(&v, |r, _| r.session.find_options());
    v.simulate_event(option("c", "ç"));
    v.simulate_event(option("w", "∑"));
    v.simulate_event(option("r", "®"));
    v.simulate_event(option("a", "å"));
    let after = f.view.read_with(&v, |r, _| r.session.find_options());
    assert_eq!((after.case, after.word, after.regex), (!before.case, !before.word, !before.regex));
    assert_eq!(f.view.read_with(&v, |r, _| r.session.state().active().unwrap().find().unwrap().query().to_owned()), "™");
    v.simulate_keystrokes("escape");
    f.view.update(&mut v, |r, _| { r.modal = Some(Modal::NewScratch { input: String::new(), selected: false, error: None }); });
    draw(&mut v);
    v.simulate_event(option("2", "™"));
    #[cfg(target_os = "macos")]
    ime_handler(&f, &mut v).replace_text_in_range(None, "™");
    assert!(f.view.read_with(&v, |r, _| matches!(&r.modal, Some(Modal::NewScratch { input, .. }) if input == "™")));
    assert_eq!(std::fs::read(f.dir.path().join("original.txt")).unwrap(), b"original\n");
}

#[gpui::test]
fn popup_mouse_places_and_extends_a_unicode_selection(cx: &mut TestAppContext) {
    let f = fixture(cx, Some(("original.txt", b"original\n")));
    settle(cx, &f, |v| v.session.state().current_source().is_some());
    let mut v = visual(cx, &f);
    for width in [240., 800.] {
        v.simulate_resize(size(px(width), px(300.)));
        f.view.update(&mut v, |r, _| {
            r.popup_fields.clear(); r.cancel_field_native();
            r.modal = Some(Modal::NewScratch { input: "aé😀z".into(), selected: false, error: None });
        });
        draw(&mut v);
        let at = |byte: usize, v: &mut VisualTestContext| {
            f.view.update_in(v, |r, window, _| {
                let bounds = r.field_bounds().unwrap();
                let (_, text, cursor) = r.field_source().unwrap();
                let caret = lenz_gpui::field::bounds_for_range(lenz_gpui::field::View { text, cursor, bounds, char_w: r.char_w, active: true }, byte..byte, window, r.metrics()).unwrap();
                point(caret.origin.x, caret.origin.y + px(4.))
            })
        };
        let start = at(1, &mut v); v.simulate_click(start, Modifiers::default());
        let end = at(7, &mut v); v.simulate_click(end, Modifiers { shift: true, ..Default::default() });
        v.simulate_keystrokes("ctrl-c");
        assert_eq!(v.update(|_, cx| cx.read_from_clipboard().and_then(|c| c.text())).as_deref(), Some("é😀"), "width {width}");
        v.simulate_keystrokes("X");
        assert!(f.view.read_with(&v, |r, _| matches!(&r.modal, Some(Modal::NewScratch { input, .. }) if input == "aXz")));
    }
    assert_eq!(std::fs::read(f.dir.path().join("original.txt")).unwrap(), b"original\n");
}

#[gpui::test]
fn native_field_composition_uses_utf16_selection_and_painted_geometry(cx: &mut TestAppContext) {
    let f = fixture(cx, Some(("original.txt", b"original\n")));
    settle(cx, &f, |r| r.session.state().current_source().is_some());
    let mut v = visual(cx, &f);
    v.simulate_keystrokes("ctrl-f"); draw(&mut v);
    let mut input = ime_handler(&f, &mut v);
    input.replace_text_in_range(None, "a😀z");
    input.set_selected_text_range(1..3);
    input.replace_and_mark_text_in_range(None, "界😀", Some(0..1));
    assert_eq!(f.view.read_with(&v, |r, _| r.session.state().active().unwrap().find().unwrap().query().to_owned()), "a😀z");
    assert_eq!(input.marked_text_range(), Some(1..4));
    assert_eq!(input.selected_text_range(false).unwrap().range, 1..2);
    assert_eq!(input.text_length_utf16(), Some(5));
    let mut adjusted = None;
    assert_eq!(input.text_for_range(0..5, &mut adjusted).as_deref(), Some("a界😀z"));
    assert_eq!(adjusted, Some(0..5));
    assert!(input.text_for_range(3..4, &mut adjusted).is_none(), "surrogate interiors are invalid, even in preedit");
    draw(&mut v);
    let caret = input.bounds_for_range(1..1).expect("composition candidate has field geometry");
    assert_eq!(input.character_index_for_point(caret.origin), Some(1));
    input.unmark_text();
    input.replace_text_in_range(None, "界😀");
    assert_eq!(f.view.read_with(&v, |r, _| r.session.state().active().unwrap().find().unwrap().query().to_owned()), "a界😀z");
    assert_eq!(std::fs::read(f.dir.path().join("original.txt")).unwrap(), b"original\n");
    assert_eq!(f.view.read_with(&v, |r, _| { let s = r.session.state().current_source().unwrap(); s.copy_range(0..s.len_bytes()).unwrap() }), "original\n");
}

#[gpui::test]
fn popup_native_input_registers_over_an_activity_log_focus(cx: &mut TestAppContext) {
    let f = fixture(cx, Some(("original.txt", b"original\n")));
    settle(cx, &f, |r| r.session.state().current_source().is_some());
    let mut v = visual(cx, &f);
    f.view.update(&mut v, |r, _| {
        r.log_focus = true;
        r.modal = Some(Modal::NewScratch { input: String::new(), selected: false, error: None });
    });
    draw(&mut v);
    assert!(f.view.read_with(&v, |r, _| r.input_state.registered.get()));
    let mut input = ime_handler(&f, &mut v);
    input.replace_and_mark_text_in_range(None, "仮", None);
    input.replace_text_in_range(None, "確定");
    assert!(f.view.read_with(&v, |r, _| matches!(&r.modal, Some(Modal::NewScratch { input, .. }) if input == "確定")));
    assert_eq!(std::fs::read(f.dir.path().join("original.txt")).unwrap(), b"original\n");
}

#[gpui::test]
fn fresh_field_key_resumes_after_programmatic_query_replacement(cx: &mut TestAppContext) {
    let f = fixture(cx, Some(("original.txt", b"original\n")));
    settle(cx, &f, |r| r.session.state().current_source().is_some());
    let mut v = visual(cx, &f);
    v.simulate_keystrokes("ctrl-f a b c"); draw(&mut v);
    let mut input = ime_handler(&f, &mut v);
    input.replace_and_mark_text_in_range(None, "pending", None);
    f.view.update(&mut v, |r, cx| r.dispatch(Command::Find("new".into()), cx));
    input.replace_text_in_range(None, "obsolete");
    assert_eq!(f.view.read_with(&v, |r, _| r.session.state().active().unwrap().find().unwrap().query().to_owned()), "new");
    v.simulate_keystrokes("end z");
    assert_eq!(f.view.read_with(&v, |r, _| r.session.state().active().unwrap().find().unwrap().query().to_owned()), "newz");
    assert_eq!(std::fs::read(f.dir.path().join("original.txt")).unwrap(), b"original\n");
}
