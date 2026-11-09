use super::{Command, Cursor, Native};

#[test]
fn native_replacements_keep_the_same_utf8_byte_limit_as_paste() {
    let mut inserted = "a😀z".to_string();
    let mut replaced = inserted.clone();
    let mut cursor = Cursor {
        anchor: Some(1),
        caret: Some(5),
    };
    let mut replacement_cursor = Cursor::default();
    cursor.execute_limited(&mut inserted, Command::Insert("ééé".into()), 7);
    replacement_cursor.execute_limited(
        &mut replaced,
        Command::Replace {
            range: 1..5,
            text: "ééé".into(),
            selection: None,
        },
        7,
    );
    assert_eq!(inserted, "aééz");
    assert_eq!(replaced, inserted);
    assert_eq!(replacement_cursor.caret(&replaced), cursor.caret(&inserted));
    assert!(replaced.len() <= 7);

    let before = replaced.clone();
    let cursor_before = replacement_cursor;
    let result = replacement_cursor.execute_limited(
        &mut replaced,
        Command::Replace {
            range: 2..3,
            text: "too long".into(),
            selection: None,
        },
        7,
    );
    assert!(
        !result.changed,
        "a replacement inside a UTF-8 codepoint is rejected"
    );
    assert_eq!(replaced, before);
    assert_eq!(replacement_cursor, cursor_before);
}

#[test]
fn stale_mark_callbacks_cannot_reinterpret_a_later_commit_as_fresh_input() {
    let original = "alpha";
    let cursor = Cursor::default();
    for (current, current_cursor) in [
        ("ALPHA", cursor),
        (
            original,
            Cursor {
                caret: Some(0),
                anchor: None,
            },
        ),
    ] {
        let mut native = Native::default();
        assert!(native.mark(original, cursor, None, "仮", None, false));
        assert!(!native.mark(current, current_cursor, None, "obsolete", None, false));
        assert!(
            native
                .commit(current, current_cursor, None, "late commit", false)
                .is_none()
        );
        assert!(
            native
                .set_selection(current, current_cursor, 0..0)
                .is_none()
        );
        assert!(native.unmark(current, current_cursor).is_none());
        native.cancel();
        assert!(
            native.mark(current, current_cursor, None, "fresh", None, false),
            "an explicitly cancelled phase can begin a new composition"
        );
    }
}

#[test]
fn invalid_utf16_selection_rejects_preedit_without_replacing_the_current_projection() {
    let cursor = Cursor::default();
    let mut native = Native::default();
    assert!(!native.mark("", cursor, None, "😀", Some(1..1), false));
    assert!(!native.is_marked());
    assert!(native.mark("", cursor, None, "仮", Some(0..1), false));
    assert!(!native.mark("", cursor, None, "😀", Some(1..1), false));
    assert_eq!(native.snapshot("", cursor, 20).text, "仮");
    assert_eq!(native.selected("", cursor), Some((0..1, false)));
    assert!(native.set_selection("", cursor, 100..100).is_none());
    assert_eq!(native.selected("", cursor), Some((0..1, false)));
}

#[test]
fn partial_preedit_selection_is_relative_to_the_new_replacement_on_both_platforms() {
    let text = "prefix suffix";
    let cursor = Cursor {
        caret: Some(7),
        anchor: None,
    };
    for (local, range) in [(true, 1..3), (false, 8..10)] {
        let mut native = Native::default();
        assert!(native.mark(text, cursor, None, "a😀b", None, local));
        assert!(native.mark(text, cursor, Some(range), "界", Some(0..1), local));
        assert_eq!(
            native.selected(text, cursor),
            Some((8..9, false)),
            "selection starts at the replaced substring rather than the whole preedit"
        );
        assert_eq!(native.snapshot(text, cursor, 40).text, "prefix a界bsuffix");
        assert_eq!(native.marked_range(text, cursor), Some(7..10));
        let mut adjusted = None;
        assert_eq!(
            native
                .text_for_range(text, cursor, 8..9, &mut adjusted)
                .as_deref(),
            Some("界")
        );
        assert_eq!(adjusted, Some(8..9));
    }
}

#[test]
fn a_partial_linux_preedit_commit_preserves_its_surrounding_text_and_insertion_caret() {
    let mut text = "prefix suffix".to_string();
    let mut cursor = Cursor {
        caret: Some(7),
        anchor: None,
    };
    let mut native = Native::default();
    assert!(native.mark(&text, cursor, None, "a😀b", None, false));
    let command = native
        .commit(&text, cursor, Some(8..10), "界", false)
        .unwrap();
    cursor.execute(&mut text, command);
    assert_eq!(text, "prefix a界bsuffix");
    assert_eq!(cursor.caret(&text), "prefix a界".len());
    assert!(cursor.selection(&text).is_none());
    assert!(native.marked_range(&text, cursor).is_none());
}

#[test]
fn a_mac_preedit_commit_keeps_the_platforms_whole_marked_text_replacement() {
    let mut text = "prefix suffix".to_string();
    let mut cursor = Cursor {
        caret: Some(7),
        anchor: None,
    };
    let mut native = Native::default();
    assert!(native.mark(&text, cursor, None, "a😀b", None, true));
    let command = native
        .commit(&text, cursor, Some(1..3), "確定", true)
        .unwrap();
    cursor.execute(&mut text, command);
    assert_eq!(text, "prefix 確定suffix");
    assert_eq!(cursor.caret(&text), "prefix 確定".len());
    assert!(cursor.selection(&text).is_none());
}

#[test]
fn native_surrounding_text_and_paint_stay_bounded_for_a_million_byte_query() {
    let text = "a".repeat(1_000_000);
    let cursor = Cursor {
        caret: Some(500_000),
        anchor: None,
    };
    let mut native = Native::default();
    assert!(native.mark(&text, cursor, None, "😀界", None, false));
    let shown = native.snapshot(&text, cursor, 32);
    assert!(shown.text.contains("😀界"));
    assert!(shown.text.len() <= 32 * 4);
    assert!(shown.text.capacity() <= 32 * 4);
    let mut adjusted = None;
    let surrounding = native
        .text_for_range(&text, cursor, 0..1_000_003, &mut adjusted)
        .unwrap();
    assert!(surrounding.len() <= 4096);
    assert!(
        surrounding.contains("😀界"),
        "native context includes the current preedit"
    );
    let adjusted = adjusted.unwrap();
    assert!(adjusted.start <= 500_000 && adjusted.end >= 500_003);
    assert_eq!(
        adjusted.end - adjusted.start,
        surrounding.encode_utf16().count()
    );
    let projected_units = [&text[..500_000], "😀界", &text[500_000..]]
        .into_iter()
        .flat_map(str::encode_utf16)
        .skip(adjusted.start)
        .take(adjusted.end - adjusted.start)
        .collect::<Vec<_>>();
    assert_eq!(
        surrounding,
        String::from_utf16(&projected_units).unwrap(),
        "adjusted UTF-16 endpoints identify exactly the bytes returned from the projected field"
    );
    assert!(!native.mark(&text, cursor, None, &"x".repeat(4097), None, false));
    assert!(native.snapshot(&text, cursor, 32).text.contains("😀界"));
    native.cancel();
    assert!(native.marked_range(&text, cursor).is_none());
    assert_eq!(
        text.len(),
        1_000_000,
        "preedit never mutates the application query"
    );
}
