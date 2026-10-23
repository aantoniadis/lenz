use std::sync::Arc;
use reader_core::{document::Document, text::{BufferId, TextEdit, TextSnapshot}};

#[test]
fn editor_text_unicode_transactions_preserve_held_snapshots() {
    let document = Document::decode("unicode.txt".into(), 1, "a😀\r\ne\u{301}\n".as_bytes());
    let original = TextSnapshot::from_document(BufferId(7), &document);
    let changed = original.apply_edits(2, &[
        TextEdit { range: 1..5, text: Arc::from("β") },
        TextEdit { range: 7..10, text: Arc::from("終😀") },
    ]).unwrap();
    assert_eq!(original.copy_range(0..original.len_bytes()).unwrap(), "a😀\r\ne\u{301}\n");
    assert_eq!(changed.copy_range(0..changed.len_bytes()).unwrap(), "aβ\r\n終😀\n");
    assert_eq!(changed.key().version, 2);
    assert_eq!(changed.key().buffer, BufferId(7));
    assert_eq!(changed.line_count(), 3);
    assert_eq!(changed.line_range(0), Some(0..3));
    assert_eq!(changed.line_with_ending(0), Some(0..5));
    assert_eq!(original.byte_to_utf16(5), Some(3));
    assert_eq!(original.utf16_to_byte(2), None);
    assert!(original.apply_edits(2, &[TextEdit { range: 2..5, text: Arc::from("") }]).is_err());
}

#[test]
fn editor_text_metadata_survives_materialization() {
    let mut utf16 = vec![0xff, 0xfe];
    utf16.extend("café\r\n".encode_utf16().flat_map(u16::to_le_bytes));
    let document = Document::decode("legacy.txt".into(), 1, &utf16);
    let snapshot = TextSnapshot::from_document(BufferId(1), &document);
    let changed = snapshot.apply_edits(2, &[TextEdit { range: 0..0, text: Arc::from("😀") }]).unwrap();
    let worker = changed.materialize();
    assert_eq!(worker.text().as_ref(), "😀café\r\n");
    assert_eq!(worker.encoding(), "UTF-16LE BOM");
    assert_eq!(worker.source_meta().original_encoding(), "UTF-16LE");
    assert!(worker.source_meta().had_bom());
    assert_eq!(worker.notices(), document.notices());
    assert!(Arc::ptr_eq(worker.source_meta(), document.source_meta()));
    let binary = Document::decode("bad.png".into(), 1, b"\x89PNG\r\n\x1a\n\0");
    assert!(binary.source_meta().is_binary(), "raster decoding must retain original binary identity");
    assert!(binary.image().is_some());
}

#[test]
fn editor_text_long_line_windows_keep_absolute_unicode_coordinates() {
    let text = format!("{}\n", "aé😀e\u{301}".repeat(90_000));
    assert_eq!(text.len(), 900_001);
    let snapshot = TextSnapshot::from_document(BufferId(8), &Document::decode("long.txt".into(), 1, text.as_bytes()));
    for from in [0, 9999, 10000, 20000, 450000, 899990] {
        let window = snapshot.line_window(0, from, 37).unwrap();
        assert!(window.bytes.len() <= 37);
        assert_eq!(window.text, text[window.bytes.clone()]);
        assert_eq!(window.utf16.start, text[..window.bytes.start].encode_utf16().count());
        assert_eq!(window.utf16.end, text[..window.bytes.end].encode_utf16().count());
        assert_eq!(snapshot.utf16_to_byte(window.utf16.start), Some(window.bytes.start));
        assert_eq!(snapshot.utf16_to_byte(window.utf16.end), Some(window.bytes.end));
    }
    assert!(snapshot.line_window(0, 3, 1).unwrap().text.is_empty());
    assert!(snapshot.line_window(2, 0, 37).is_none());
    assert_eq!(snapshot.line_window(1, usize::MAX, 37).unwrap().bytes, 900001..900001);
    for byte in (4096..text.len()).step_by(4096) {
        for byte in byte - 3..=byte + 3 {
            let floor = text.floor_char_boundary(byte);
            assert_eq!(snapshot.floor_char_boundary(byte), floor);
            assert_eq!(snapshot.byte_to_utf16(byte), text.is_char_boundary(byte).then(|| text[..byte].encode_utf16().count()));
            if text.is_char_boundary(byte) {
                assert_eq!(snapshot.utf16_to_byte(snapshot.byte_to_utf16(byte).unwrap()), Some(byte));
            }
        }
    }
}

#[test]
fn editor_text_chunk_crossings_crlf_and_atomic_range_rejections() {
    let text = format!("{}\r\n😀e\u{301}\n{}\r\n", "x".repeat(4095), "y".repeat(4095));
    let snapshot = TextSnapshot::from_document(BufferId(9), &Document::decode("chunks.txt".into(), 1, text.as_bytes()));
    assert_eq!(snapshot.line_range(0), Some(0..4095));
    assert_eq!(snapshot.line_with_ending(0), Some(0..4097));
    assert_eq!(snapshot.line_range(1), Some(4097..4104));
    assert_eq!(snapshot.line_of(4096), 0);
    assert_eq!(snapshot.line_of(4097), 1);
    assert_eq!(snapshot.line_range(4), None);
    assert!(matches!(snapshot.line_window(0, 0, 4).unwrap().text, std::borrow::Cow::Borrowed(_)));
    let crossing = snapshot.line_window(2, 8190, 16).unwrap();
    assert_eq!(crossing.text, text[crossing.bytes.clone()]);
    let changed = snapshot.apply_edits(2, &[
        TextEdit { range: 4094..4098, text: Arc::from("終") },
    ]);
    assert!(changed.is_err(), "range ends inside emoji");
    let changed = snapshot.apply_edits(2, &[TextEdit { range: 4094..4101, text: Arc::from("終") }]).unwrap();
    assert_eq!(changed.copy_range(4090..4100).unwrap(), "xxxx終e\u{301}");
    for edits in [
        vec![TextEdit { range: 0..4, text: Arc::from("a") }, TextEdit { range: 3..5, text: Arc::from("b") }],
        vec![TextEdit { range: 4..5, text: Arc::from("a") }, TextEdit { range: 0..1, text: Arc::from("b") }],
        vec![TextEdit { range: 0..0, text: Arc::from("a") }, TextEdit { range: 0..0, text: Arc::from("b") }],
        vec![TextEdit { range: text.len()..text.len()+1, text: Arc::from("") }],
    ] {
        assert!(snapshot.apply_edits(2, &edits).is_err());
        assert_eq!(snapshot.key().version, 1);
        assert_eq!(snapshot.copy_range(0..snapshot.len_bytes()).unwrap(), text);
    }
    assert!(snapshot.apply_edits(1, &[]).is_err());
    assert!(snapshot.copy_range(4100..4101).is_none());
    assert_eq!(snapshot.previous_char_boundary(4101), 4097);
    assert_eq!(snapshot.next_char_boundary(4097), 4101);
}

#[test]
fn editor_text_empty_and_final_newline_edits() {
    let empty = TextSnapshot::from_document(BufferId(0), &Document::decode("empty".into(), 1, b""));
    assert_eq!(empty.line_range(0), Some(0..0));
    assert_eq!(empty.line_of(usize::MAX), 0);
    assert_eq!(empty.byte_to_utf16(0), Some(0));
    assert_eq!(empty.byte_to_utf16(1), None);
    assert_eq!(empty.utf16_to_byte(0), Some(0));
    let inserted = empty.apply_edits(2, &[TextEdit { range: 0..0, text: Arc::from("😀\r\n") }]).unwrap();
    assert_eq!(inserted.line_count(), 2);
    assert_eq!(inserted.line_range(1), Some(6..6));
    let removed = inserted.apply_edits(3, &[TextEdit { range: 0..6, text: Arc::from("") }]).unwrap();
    assert_eq!(removed.copy_range(0..0).unwrap(), "");
    assert_eq!(removed.line_count(), 1);
    assert_eq!(inserted.copy_range(0..6).unwrap(), "😀\r\n");
}

#[test]
fn editor_text_declared_encoding_without_bom_is_not_guessed_again() {
    let bytes: Vec<_> = "café\r\n".encode_utf16().flat_map(u16::to_be_bytes).collect();
    let decoded = lenz_content::encoding::decode(&bytes, Some("UTF-16BE"));
    let document = Document::from_decoded("declared.txt".into(), 10, decoded);
    let snapshot = TextSnapshot::from_document(BufferId(3), &document);
    let changed = snapshot.apply_edits(11, &[TextEdit { range: 0..0, text: Arc::from("😀") }]).unwrap().materialize();
    assert_eq!(changed.text().as_ref(), "😀café\r\n");
    assert_eq!(changed.encoding(), "UTF-16BE");
    assert!(!changed.source_meta().had_bom());
    assert_eq!(changed.source_meta().original_encoding(), "UTF-16BE");
}

#[test]
#[ignore = "release measurement; run explicitly with --release --ignored --nocapture"]
fn editor_text_release_workload() {
    let text = format!("{}\n", "aé😀e\u{301}".repeat(90_000));
    let document = Document::decode("long.txt".into(), 1, text.as_bytes());
    let held = TextSnapshot::from_document(BufferId(1), &document);
    let mut current = held.clone();
    let mut samples = Vec::with_capacity(10_000);
    for iteration in 0..10_000 {
        let at = match iteration % 3 { 0 => 0, 1 => 450_000, _ => 900_000 };
        let started = std::time::Instant::now();
        current = current.apply_edits(current.key().version + 1, &[TextEdit { range: at..at, text: Arc::from("😀") }]).unwrap();
        let window = current.line_window(0, at, 64).unwrap();
        assert!(window.text.starts_with("😀"));
        assert!(window.bytes.len() <= 64);
        assert_eq!(current.utf16_to_byte(window.utf16.end), Some(window.bytes.end));
        current = current.apply_edits(current.key().version + 1, &[TextEdit { range: at..at + 4, text: Arc::from("") }]).unwrap();
        samples.push(started.elapsed().as_nanos());
    }
    assert_eq!(current.copy_range(0..current.len_bytes()).unwrap(), text);
    assert_eq!(held.copy_range(0..held.len_bytes()).unwrap(), text);
    eprintln!("raw_cycle_ns={samples:?}");
    samples.sort_unstable();
    eprintln!("900001-byte Unicode line, 10000 insert/window/UTF16/delete cycles; p50={}ns p95={}ns p99={}ns", samples[5000], samples[9500], samples[9900]);
}
