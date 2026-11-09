use super::*;

#[test]
fn word_motion_and_deletion_keep_combining_emoji_flags_and_keycaps_atomic() {
    for token in ["cafe\u{301}", "नमस्ते", "👩🏽‍💻", "🇬🇷🇯🇵", "1\u{fe0f}\u{20e3}"] {
        let d = tempfile::tempdir().unwrap();
        let original = format!("pre {token} tail\r\n");
        let path = write(d.path(), "word.txt", original.as_bytes());
        let mut s = session(d.path());
        open(&mut s, Command::OpenRetained(path.clone()), "word.txt");
        s.enable_text_input_for_test();
        let end = 4 + token.len();
        let tab = s.state().active().unwrap().id();
        s.dispatch(Command::Select(Selection { anchor: end, head: end }));
        s.dispatch(Command::MoveCaret(Motion::WordLeft));
        assert_eq!(s.state().selection().unwrap().head, 4, "{token}");
        s.dispatch(Command::MoveCaret(Motion::WordRight));
        assert_eq!(s.state().selection().unwrap().head, end, "{token}");
        s.dispatch(Command::Delete { backward: true, word: true });
        assert_eq!(source_text(s.state().current_source().unwrap()), "pre  tail\r\n", "{token}");
        s.dispatch(Command::Undo { tab });
        assert_eq!(source_text(s.state().current_source().unwrap()), original);
        s.dispatch(Command::Select(Selection { anchor: 4, head: 4 }));
        s.dispatch(Command::Delete { backward: false, word: true });
        assert_eq!(source_text(s.state().current_source().unwrap()), "pre  tail\r\n", "{token}");
        assert_eq!(std::fs::read(path).unwrap(), original.as_bytes());
    }
}

#[test]
fn word_stops_follow_graphemes_across_original_and_edited_rope_chunks() {
    let d = tempfile::tempdir().unwrap();
    // The base starts at the last byte of the first original chunk; its marks span two further chunks.
    let start = 4095;
    let original = format!("{}e{} tail\n", " ".repeat(start), "\u{301}".repeat(4200));
    let path = write(d.path(), "chunks.txt", original.as_bytes());
    let mut s = session(d.path());
    open(&mut s, Command::OpenRetained(path.clone()), "chunks.txt");
    s.enable_text_input_for_test();
    let end = start + 1 + 8400;
    for at in [end, start] {
        let source = s.state().current_source().unwrap();
        assert_eq!(super::super::word_stop(source, at, at == end, false), if at == end { start } else { end });
    }
    let tab = s.state().active().unwrap().id();
    let expected = s.state().current_source().unwrap().key();
    s.dispatch(Command::Edit { tab, expected, edits: vec![crate::text::TextEdit {
        range: start + 1..start + 1, text: "\u{302}".into(),
    }], kind: crate::text::EditKind::Replace });
    let source = s.state().current_source().unwrap();
    assert_eq!(super::super::word_stop(source, end + 2, true, false), start);
    assert_eq!(super::super::word_stop(source, start, false, false), end + 2);
    s.dispatch(Command::Select(Selection { anchor: end + 2, head: end + 2 }));
    s.dispatch(Command::Delete { backward: true, word: true });
    assert_eq!(source_text(s.state().current_source().unwrap()), format!("{} tail\n", " ".repeat(start)));
    assert_eq!(std::fs::read(path).unwrap(), original.as_bytes());
}
