use super::*;

fn query(f: &Fixture, v: &VisualTestContext) -> String {
    f.view.read_with(v, |view, _| {
        view.session
            .state()
            .active()
            .unwrap()
            .find()
            .unwrap()
            .query()
            .to_owned()
    })
}

fn assert_source_untouched(f: &Fixture, v: &VisualTestContext) {
    assert_eq!(
        std::fs::read(f.dir.path().join("source.txt")).unwrap(),
        b"source stays\n"
    );
    f.view.read_with(v, |view, _| {
        let source = view.session.state().current_source().unwrap();
        assert_eq!(
            source.copy_range(0..source.len_bytes()).unwrap(),
            "source stays\n"
        );
    });
}

#[gpui::test]
fn native_find_unmark_and_enter_commit_once_even_with_trailing_callbacks(cx: &mut TestAppContext) {
    for enter in [false, true] {
        let f = fixture(cx, Some(("source.txt", b"source stays\n")));
        settle(cx, &f, |view| {
            view.session.state().current_source().is_some()
        });
        let mut v = visual(cx, &f);
        v.simulate_keystrokes("ctrl-f ctrl-a backspace");
        draw(&mut v);
        let mut input = ime_handler(&f, &mut v);
        input.replace_and_mark_text_in_range(None, "界😀", None);
        assert_eq!(query(&f, &v), "", "preedit is not the authoritative query");
        if enter {
            v.simulate_keystrokes("enter");
        } else {
            input.unmark_text();
        }
        assert_eq!(query(&f, &v), "界😀");
        input.unmark_text();
        input.replace_text_in_range(None, "界😀");
        input.replace_text_in_range(None, "obsolete candidate");
        assert_eq!(
            query(&f, &v),
            "界😀",
            "ended composition callbacks must not commit again"
        );
        input.replace_and_mark_text_in_range(None, "次", None);
        assert_eq!(
            query(&f, &v),
            "界😀",
            "a fresh preedit remains a projection"
        );
        input.replace_text_in_range(None, "次");
        input.unmark_text();
        assert_eq!(
            query(&f, &v),
            "界😀次",
            "native IME may start its next phase before another raw key reaches the application"
        );
        v.simulate_keystrokes("end z");
        assert_eq!(
            query(&f, &v),
            "界😀次z",
            "a fresh printable key starts a new native phase"
        );
        assert_source_untouched(&f, &v);
    }
}

#[gpui::test]
fn native_find_movement_or_field_switch_cancels_old_preedit_and_commit(cx: &mut TestAppContext) {
    for movement in ["right", "tab", "backspace", "delete"] {
        let f = fixture(cx, Some(("source.txt", b"source stays\n")));
        settle(cx, &f, |view| {
            view.session.state().current_source().is_some()
        });
        let mut v = visual(cx, &f);
        v.simulate_keystrokes("ctrl-f ctrl-a backspace a b c");
        draw(&mut v);
        let mut input = ime_handler(&f, &mut v);
        input.replace_and_mark_text_in_range(None, "pending", None);
        v.simulate_keystrokes(movement);
        input.replace_and_mark_text_in_range(None, "obsolete preedit", None);
        input.replace_text_in_range(None, "obsolete commit");
        input.unmark_text();
        assert_eq!(query(&f, &v), "abc");
        assert_eq!(
            f.view
                .read_with(&v, |view, _| view.session.replace_text().to_owned()),
            ""
        );
        assert!(
            f.view
                .read_with(&v, |view, _| !view.field_native.is_marked())
        );
        v.simulate_keystrokes("z");
        if movement == "tab" {
            assert_eq!(query(&f, &v), "abc");
            assert_eq!(
                f.view
                    .read_with(&v, |view, _| view.session.replace_text().to_owned()),
                "z"
            );
        } else {
            assert_eq!(query(&f, &v), "abcz");
        }
        assert_source_untouched(&f, &v);
    }
}

#[gpui::test]
fn native_find_ignores_queued_callbacks_after_focus_moves_away(cx: &mut TestAppContext) {
    let f = fixture(cx, Some(("source.txt", b"source stays\n")));
    settle(cx, &f, |view| {
        view.session.state().current_source().is_some()
    });
    let mut v = visual(cx, &f);
    v.simulate_keystrokes("ctrl-f ctrl-a backspace");
    draw(&mut v);
    let mut input = ime_handler(&f, &mut v);
    input.replace_and_mark_text_in_range(None, "pending", None);
    let other_focus = f.view.update(&mut v, |_, cx| cx.focus_handle());
    v.update(|window, cx| window.focus(&other_focus, cx));
    input.replace_and_mark_text_in_range(None, "obsolete preedit", None);
    input.replace_text_in_range(None, "obsolete commit");
    input.unmark_text();
    assert_eq!(query(&f, &v), "");
    let focus = f.view.read_with(&v, |view, _| view.focus_handle());
    v.update(|window, cx| window.focus(&focus, cx));
    v.simulate_keystrokes("z");
    assert_eq!(query(&f, &v), "z");
    assert_source_untouched(&f, &v);
}

#[gpui::test]
fn native_find_registration_preserves_clipboard_shortcuts(cx: &mut TestAppContext) {
    let f = fixture(cx, Some(("source.txt", b"source stays\n")));
    settle(cx, &f, |view| {
        view.session.state().current_source().is_some()
    });
    let mut v = visual(cx, &f);
    v.simulate_keystrokes("ctrl-f ctrl-a backspace a b c ctrl-a");
    draw(&mut v);
    assert!(
        f.view
            .read_with(&v, |view, _| view.input_state.registered.get())
    );
    v.update(|_, cx| cx.write_to_clipboard(ClipboardItem::new_string("pasted 😀".into())));
    v.simulate_keystrokes("ctrl-v");
    assert_eq!(query(&f, &v), "pasted 😀");
    v.simulate_keystrokes("ctrl-a");
    let mut input = ime_handler(&f, &mut v);
    let selected = input.selected_text_range(false).unwrap().range;
    for empty in ["", "\nignored second line"] {
        input.paste(ClipboardItem::new_string(empty.into()));
        assert_eq!(query(&f, &v), "pasted 😀");
        assert_eq!(input.selected_text_range(false).unwrap().range, selected);
    }
    input.paste(ClipboardItem::new_string(
        "native paste\nignored second line".into(),
    ));
    assert_eq!(query(&f, &v), "native paste");
    assert_source_untouched(&f, &v);
}

#[gpui::test]
fn closing_a_composing_field_keeps_queued_native_callbacks_out_of_the_source(
    cx: &mut TestAppContext,
) {
    for popup in [false, true] {
        let f = fixture(cx, Some(("source.txt", b"source stays\n")));
        settle(cx, &f, |view| {
            view.session.state().current_source().is_some()
        });
        let mut v = visual(cx, &f);
        if popup {
            f.view.update(&mut v, |view, _| {
                view.modal = Some(Modal::NewScratch {
                    input: String::new(),
                    selected: false,
                    error: None,
                })
            });
        } else {
            v.simulate_keystrokes("ctrl-f ctrl-a backspace");
        }
        draw(&mut v);
        let mut input = ime_handler(&f, &mut v);
        input.replace_and_mark_text_in_range(None, "pending", None);
        v.simulate_keystrokes("escape escape");
        assert!(f.view.read_with(&v, |view, _| view.modal.is_none()
            && view.find_focus.is_none()));
        input.replace_and_mark_text_in_range(None, "obsolete preedit", None);
        input.replace_text_in_range(None, "obsolete commit");
        input.unmark_text();
        assert_source_untouched(&f, &v);
    }
}

#[gpui::test]
fn ended_find_callbacks_before_the_next_paint_cannot_become_document_input(cx: &mut TestAppContext) {
    let f = fixture(cx, Some(("source.txt", b"source stays\n")));
    settle(cx, &f, |view| view.session.state().current_source().is_some());
    let mut v = visual(cx, &f);
    f.view.update(&mut v, |view, _| view.enable_draft_input_for_test());
    v.simulate_keystrokes("ctrl-f");
    draw(&mut v);
    let mut input = ime_handler(&f, &mut v);
    input.replace_text_in_range(None, "needle");
    input.replace_and_mark_text_in_range(None, "仮", None);
    input.unmark_text();
    f.view.update_in(&mut v, |view, window, cx| {
        use gpui::EntityInputHandler as _;
        let escape = gpui::KeyDownEvent { keystroke: gpui::Keystroke::parse("escape").unwrap(), is_held: false, prefer_character_input: false };
        assert!(view.find_key(&escape, cx));
        assert!(view.find_focus.is_none());
        // These callbacks run in this same update, before render can invalidate the old field owner.
        view.replace_text_in_range(None, "late", window, cx);
        view.replace_and_mark_text_in_range(None, "late preedit", None, window, cx);
        view.unmark_text(window, cx);
        view.set_selected_text_range(0..6, window, cx);
        view.paste(ClipboardItem::new_string("late paste".into()), window, cx);
        let source = view.session.state().current_source().unwrap();
        assert_eq!(source.copy_range(0..source.len_bytes()).as_deref(), Some("source stays\n"));
        let fresh = gpui::KeyDownEvent { keystroke: gpui::Keystroke::parse("z").unwrap(), is_held: false, prefer_character_input: false };
        view.on_key(&fresh, window, cx);
        view.replace_text_in_range(None, "z", window, cx);
        let source = view.session.state().current_source().unwrap();
        assert_eq!(source.copy_range(0..source.len_bytes()).as_deref(), Some("zsource stays\n"));
    });
    assert_eq!(std::fs::read(f.dir.path().join("source.txt")).unwrap(), b"source stays\n");
}

#[cfg(not(target_os = "macos"))]
#[gpui::test]
fn native_linux_empty_marked_replacement_cancels_without_deleting_the_query(cx: &mut TestAppContext) {
    let f = fixture(cx, Some(("source.txt", b"source stays\n")));
    settle(cx, &f, |view| view.session.state().current_source().is_some());
    let mut v = visual(cx, &f);
    v.simulate_keystrokes("ctrl-f ctrl-a backspace k e e p ctrl-a");
    draw(&mut v);
    let cursor = f.view.read_with(&v, |view, _| view.field_source().unwrap().2);
    assert_eq!(cursor.selection("keep"), Some(0..4));
    let mut input = ime_handler(&f, &mut v);
    input.replace_and_mark_text_in_range(None, "候補", None);
    let marked = input.marked_text_range().unwrap();
    assert_eq!(marked, 0..2);
    assert_eq!(query(&f, &v), "keep");
    // X11 and Wayland send this exact packet to cancel preedit; it is not a committed empty edit.
    input.replace_text_in_range(Some(marked), "");
    assert_eq!(query(&f, &v), "keep");
    assert_eq!(f.view.read_with(&v, |view, _| view.field_source().unwrap().2), cursor);
    assert!(input.marked_text_range().is_none());
    input.unmark_text();
    input.replace_text_in_range(None, "late");
    assert_eq!(query(&f, &v), "keep");
    assert_source_untouched(&f, &v);
    v.simulate_keystrokes("x");
    assert_eq!(query(&f, &v), "x", "the original select-all survives cancellation");
    assert_source_untouched(&f, &v);
}

#[gpui::test]
fn raw_dead_key_without_committed_text_never_inserts_its_physical_name(cx: &mut TestAppContext) {
    let f = fixture(cx, Some(("source.txt", b"source stays\n")));
    settle(cx, &f, |view| view.session.state().current_source().is_some());
    let mut v = visual(cx, &f);
    v.simulate_keystrokes("ctrl-f ctrl-a backspace");
    draw(&mut v);
    let mut input = ime_handler(&f, &mut v);
    input.replace_and_mark_text_in_range(None, "previous", None);
    v.simulate_keystrokes("right");
    assert!(!input.query_prefers_ime_for_printable_keys());
    // Pinned XKB emits preedit first, then the physical apostrophe key with no committed character.
    input.replace_and_mark_text_in_range(None, "´", None);
    let mut dead = gpui::Keystroke::parse("'").unwrap();
    dead.key_char = None;
    v.simulate_event(gpui::KeyDownEvent { keystroke: dead, is_held: false, prefer_character_input: false });
    assert_eq!(query(&f, &v), "");
    assert!(input.query_prefers_ime_for_printable_keys());
    input.replace_text_in_range(None, "é");
    assert_eq!(query(&f, &v), "é", "only the native composed commit reaches the field");
    v.simulate_keystrokes("left");
    assert_eq!(f.view.read_with(&v, |view, _| view.field_source().unwrap().2.caret("é")), 0);
    assert_source_untouched(&f, &v);
}
