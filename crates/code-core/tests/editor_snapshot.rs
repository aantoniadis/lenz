use std::sync::Arc;
use code_core::{document::Document, text::{BufferId, TextEdit, TextSnapshot}};

#[test]
fn retained_snapshot_storage_counts_shared_backing_once_after_a_large_deletion() {
    let text = "x\n".repeat(100_000);
    let snapshot = TextSnapshot::from_document(BufferId(81), &Document::decode("backing.txt".into(), 1, text.as_bytes()));
    let backing = text.len() + 100_001 * size_of::<usize>();
    assert!(snapshot.storage_bytes() >= backing, "current text owns its original text and newline allocations");
    assert!(snapshot.storage_bytes() < backing + 100_000, "shared backing is charged once rather than per chunk");
    let tail = snapshot.apply_edits(2, &[TextEdit { range: 8192..text.len(), text: Arc::from("") }]).unwrap();
    assert_eq!(tail.len_bytes(), 8192);
    assert!(tail.storage_bytes() >= backing, "surviving original chunks retain the complete backing allocations");
    let empty = TextSnapshot::from_document(BufferId(82), &Document::decode("empty.txt".into(), 1, b""));
    assert!(empty.storage_bytes() >= size_of::<TextSnapshot>() + "empty.txt".len());
}

#[test]
fn encoded_large_policy_survives_snapshot_and_worker_conversion() {
    let mut bytes = vec![0xff, 0xfe];
    bytes.extend("a".repeat(3 << 20).encode_utf16().flat_map(u16::to_le_bytes));
    let document = Document::decode("large.rs".into(), 1, &bytes);
    assert!(document.is_large());
    let snapshot = TextSnapshot::from_document(BufferId(83), &document);
    assert!(snapshot.is_large(), "decoded UTF8 size must not silently enable restricted reader features");
    assert!(snapshot.materialize_while(|| true).unwrap().is_large());
    assert_eq!(snapshot.notices(), document.notices());
}

#[test]
fn decoded_expansion_keeps_the_loaded_reader_policy_on_rematerialization() {
    let mut bytes = vec![0xff, 0xfe];
    bytes.extend(std::iter::repeat_n(0x4e00_u16, 2_500_000).flat_map(u16::to_le_bytes));
    let document = Document::decode("expanded.rs".into(), 1, &bytes);
    assert!(!document.is_large(), "encoded load remains below the existing limit");
    let snapshot = TextSnapshot::from_document(BufferId(84), &document);
    assert!(!snapshot.is_large(), "normalizing loaded bytes must not change existing reader eligibility");
    drop(document);
    assert!(!snapshot.materialize_while(|| true).unwrap().is_large(), "worker reuse must not be required to preserve policy");
}

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
fn editor_text_materialization_observes_cancellation_after_assembly_starts() {
    let text = "aé😀e\u{301}\n".repeat(10_000);
    let snapshot = TextSnapshot::from_document(BufferId(71), &Document::decode("cancel.txt".into(), 1, text.as_bytes()));
    let checks = std::cell::Cell::new(0);
    let worker = snapshot.materialize_while(|| { let next = checks.get() + 1; checks.set(next); next <= 2 });
    assert!(worker.is_none(), "cancellation after assembly begins must discard the obsolete conversion");
    assert_eq!(snapshot.copy_range(0..snapshot.len_bytes()).unwrap(), text);
}

#[test]
fn editor_text_cancellable_materialization_keeps_unicode_chunk_line_indices_and_metadata() {
    let text = format!("{}\r\n😀e\u{301}\n{}\r\n", "x".repeat(4095), "y".repeat(4095));
    let mut bytes = vec![0xff, 0xfe]; bytes.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
    let original = Document::decode("legacy.txt".into(), 1, &bytes);
    let snapshot = TextSnapshot::from_document(BufferId(72), &original).apply_edits(2, &[TextEdit { range: 0..0, text: Arc::from("Ω\n") }]).unwrap();
    let worker = snapshot.materialize_while(|| true).unwrap();
    assert_eq!(worker.text().as_ref(), format!("Ω\n{text}"));
    assert_eq!(worker.line_count(), 5);
    assert_eq!(worker.line_range(0), 0..2);
    assert_eq!(worker.line_range(1), 3..4098);
    assert_eq!(worker.line_range(2), 4100..4107);
    assert_eq!(worker.line_range(3), 4108..8203);
    assert_eq!(worker.line_range(4), 8205..8205);
    assert_eq!(worker.encoding(), "UTF-16LE BOM");
    assert!(Arc::ptr_eq(worker.source_meta(), original.source_meta()));
    assert_eq!(worker.is_large(), snapshot.materialize().is_large());
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

/// Primitive worker conversion, distinct from UI feedback or parser readiness. Run release and alone.
#[test]
#[ignore = "timing: snapshot conversion and cancellation; run in release, alone"]
fn editor_text_materialization_release_workload() {
    use std::{sync::atomic::{AtomicBool, Ordering}, time::Instant};
    let unit = "/// Café 😀\r\npub fn item() { let name = \"é\"; }\r\n";
    let text = unit.repeat(20_000);
    let mut bytes = vec![0xff, 0xfe]; bytes.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
    let original = Document::decode("materialize.rs".into(), 1, &bytes);
    let snapshot = TextSnapshot::from_document(BufferId(73), &original).apply_edits(2, &[TextEdit { range: 0..0, text: Arc::from("// draft\n") }]).unwrap();
    let live = AtomicBool::new(true);
    let (mut old, mut admitted, mut cancelled) = (Vec::new(), Vec::new(), Vec::new());
    for trial in 0..321 {
        for mode in if trial % 2 == 0 { [0, 1] } else { [1, 0] } {
            let started = Instant::now();
            let worker = if mode == 0 { snapshot.materialize() } else { snapshot.materialize_while(|| live.load(Ordering::Acquire)).unwrap() };
            let elapsed = started.elapsed().as_secs_f64() * 1_000_000.0;
            assert_eq!(worker.line_count(), 40_002);
            assert_eq!(worker.line(0), "// draft");
            assert_eq!(worker.line(20_000), "pub fn item() { let name = \"é\"; }");
            assert_eq!(worker.line_range(20_000), snapshot.line_range(20_000).unwrap());
            assert!(Arc::ptr_eq(worker.source_meta(), original.source_meta()));
            if trial > 20 { if mode == 0 { old.push(elapsed); } else { admitted.push(elapsed); } }
        }
        let checks = std::cell::Cell::new(0);
        let started = Instant::now();
        let result = snapshot.materialize_while(|| { let next = checks.get() + 1; checks.set(next); next <= 2 });
        let elapsed = started.elapsed().as_secs_f64() * 1_000_000.0;
        assert!(result.is_none()); assert_eq!(checks.get(), 3);
        if trial > 20 { cancelled.push(elapsed); }
    }
    eprintln!("READER_CONFIG {}", serde_json::json!({"bytes":snapshot.len_bytes(),"lines":snapshot.line_count(),"original_encoding":original.encoding(),"samples":300,"unit":"us","note":"conversion only; one atomic cancellation predicate; interleaved old/new order"}));
    for (name, samples) in [("reader/materialize-original-api", old), ("reader/materialize-cancellable-indexed", admitted), ("reader/materialize-cancel-after-first-chunk", cancelled)] {
        eprintln!("READER_PERF {}", serde_json::json!({"name":name,"unit":"us","samples":samples}));
    }
}

#[test]
fn line_ending_selection_covers_lf_crlf_but_never_the_final_empty_line() {
    for source in ["\n\nend\n", "\r\n\r\nend\r\n"] {
        let doc = TextSnapshot::from_document(BufferId(85), &Document::decode("selection.txt".into(), 1, source.as_bytes()));
        let end = source.find('e').unwrap();
        assert!(doc.line_ending_selected(0, 0..end));
        assert!(doc.line_ending_selected(1, 0..end));
        assert!(!doc.line_ending_selected(2, 0..end), "selection endpoints are exclusive");
        assert!(!doc.line_ending_selected(0, 0..0), "a caret selects no line ending");
        assert!(!doc.line_ending_selected(2, end..end + 3), "text selection does not include the newline");
        assert!(doc.line_ending_selected(2, end..source.len()));
        assert!(!doc.line_ending_selected(3, 0..source.len()), "EOF has no selected source byte");
        assert!(!doc.line_ending_selected(4, 0..source.len()), "invalid rows are ignored");
        if source.starts_with("\r\n") {
            assert!(doc.line_ending_selected(0, 1..2), "selecting only LF still shows the CRLF ending");
        }
        assert_eq!(doc.copy_range(0..doc.len_bytes()).as_deref(), Some(source));
    }
}
