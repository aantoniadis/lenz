use super::*;

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
