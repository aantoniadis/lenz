use std::sync::Arc;
use reader_core::{decor::{Hint, HintKind, Hints}, document::Document, projection::{Cursor, WINDOW_WORK}, text::{BufferId, TextEdit, TextSnapshot}};

#[test]
fn bounded_source_projection_reaches_tail_with_exact_unicode_coordinates() {
    let text = format!("{}é😀e\u{301}\t\x07\r\n", "a".repeat(12_000));
    let snapshot = TextSnapshot::from_document(BufferId(1), &Document::decode("long.txt".into(), 1, text.as_bytes()));
    let mut cursor = Cursor::start(&snapshot, 0, None).unwrap();
    let mut drawn = String::new();
    loop {
        let window = cursor.window(&snapshot, 64).unwrap();
        assert!(window.work <= 64 && window.shown.text.len() <= 64 && window.source.len() <= 64);
        assert!(window.work > 0 || window.complete);
        assert_eq!(window.key, snapshot.key());
        for d in 0..=window.shown.text.len() {
            let source = window.source_at(d);
            assert_eq!(snapshot.floor_char_boundary(source), source);
            assert!(window.source.start <= source && source <= window.source.end);
        }
        drawn += &window.shown.text;
        if window.complete { break; }
    }
    assert_eq!(drawn, format!("{}é😀e\u{301}    \u{FFFD}", "a".repeat(12_000)));
    assert_eq!(snapshot.copy_range(0..snapshot.len_bytes()).unwrap(), text);
    assert_eq!(cursor.source_byte(), text.len() - 2);
    assert!(cursor.window(&snapshot, WINDOW_WORK + 1).is_none());
}

#[test]
fn projection_guard_and_source_mapping_survive_every_budget_and_hint_continuation() {
    let source = "a👩\u{200D}".to_string() + &"👩\u{200D}".repeat(600) + "💻 recover\r\n";
    let snapshot = TextSnapshot::from_document(BufferId(2), &Document::decode("emoji.txt".into(), 4, source.as_bytes()));
    let mut expected = None;
    for budget in [16, 17, 31, 64, 257, WINDOW_WORK] {
        let mut cursor = Cursor::start(&snapshot, 0, None).unwrap();
        let mut drawn = String::new();
        loop {
            let window = cursor.window(&snapshot, budget).unwrap();
            assert!(window.work <= budget);
            for d in 0..=window.shown.text.len() {
                let at = window.source_at(d);
                assert_eq!(snapshot.floor_char_boundary(at), at);
            }
            drawn += &window.shown.text;
            if window.complete { break; }
        }
        assert!(drawn.contains("[U+"));
        assert!(drawn.ends_with("💻 recover"));
        if let Some(expected) = &expected { assert_eq!(&drawn, expected); }
        else { expected = Some(drawn); }
    }

    let snapshot = TextSnapshot::from_document(BufferId(3), &Document::decode("hints.txt".into(), 5, b"ab"));
    let hints = Arc::new(Hints::new(vec![(0, Hint { byte: 1, label: "Z".repeat(500).into(), kind: HintKind::Type }), (0, Hint { byte: 1, label: "👩\u{200D}".repeat(200).into(), kind: HintKind::Parameter })]));
    let mut cursor = Cursor::start(&snapshot, 0, Some(hints)).unwrap();
    let (mut selected_a, mut selected_b, mut whole, mut carets) = (String::new(), String::new(), String::new(), Vec::new());
    loop {
        let window = cursor.window(&snapshot, 64).unwrap();
        selected_a += window.shown.text.get(window.range(0..1)).unwrap();
        selected_b += window.shown.text.get(window.range(1..2)).unwrap();
        whole += &window.shown.text;
        if let Some(at) = window.caret(1) { carets.push(window.display.start + at); }
        for (_, range, _) in &window.shown.hints {
            for d in range.clone() { assert_eq!(window.source_at(d), 1); }
        }
        if window.complete { break; }
    }
    assert_eq!(selected_a, "a");
    assert_eq!(selected_b, "b");
    assert!(whole.starts_with("aZZZZ") && whole.ends_with('b') && whole.contains("[U+"));
    assert!(carets.iter().all(|at| *at == 1));
    assert_eq!(snapshot.copy_range(0..2).unwrap(), "ab");
    let changed = snapshot.apply_edits(6, &[TextEdit { range: 0..1, text: Arc::from("changed") }]).unwrap();
    let old = cursor.source_byte();
    assert!(cursor.window(&changed, 64).is_none());
    assert_eq!(cursor.source_byte(), old);
}
