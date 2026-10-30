use code_core::{document::Document, layout::{Cache, Query, ROW_BYTES}, projection::Cursor, text::{BufferId, TextSnapshot}};

fn snapshot(text: &str) -> TextSnapshot {
    TextSnapshot::from_document(BufferId(1), &Document::decode("layout.txt".into(), 1, text.as_bytes()))
}

#[test]
fn hanging_whitespace_and_hint_rows_have_a_real_projected_byte_ceiling() {
    use std::sync::Arc;
    use code_core::decor::{Hint, HintKind, Hints};
    const LIMIT: usize = ROW_BYTES;
    assert_eq!(LIMIT, 4096);
    let mut sources = vec![
        " ".repeat(50_000) + "END",
        "x".to_string() + &" ".repeat(50_000) + "END",
        " \u{301}\u{301}".repeat(10_000) + "END",
    ];
    sources.push(" ".repeat(4095) + "e\u{301}" + &" ".repeat(4095) + "END");
    for source in sources {
        let doc = snapshot(&source);
        let mut cache = Cache::default(); cache.configure(&doc, Some(8), None);
        let mut cursor = Cursor::start(&doc, 0, None).unwrap();
        let mut projected = String::new();
        loop {
            let window = cursor.window(&doc, 64).unwrap(); projected += &window.shown.text;
            if window.complete { break; }
        }
        let mut joined = String::new();
        let mut previous = 0;
        for index in 0..1000 {
            let Some(row) = ready(&mut cache, &doc, |cache| cache.row(&doc, 0, index, 32)) else { break };
            assert!(row.complete && row.display.len() <= LIMIT, "row {} has {} projected bytes", row.index, row.display.len());
            assert_eq!(row.display.start, previous);
            assert_eq!(doc.floor_char_boundary(row.source.start), row.source.start);
            assert_eq!(doc.floor_char_boundary(row.source.end), row.source.end);
            joined += projected.get(row.display.clone()).unwrap();
            previous = row.display.end;
            if row.last { break; }
        }
        assert_eq!(joined, projected, "a byte ceiling wraps; it never hides a tail");
        assert_eq!(doc.copy_range(0..doc.len_bytes()).unwrap(), source);
    }

    let doc = snapshot("abEND");
    for labels in [vec![" ".repeat(50_000)], vec![" ".repeat(500); 100]] {
        let hints = Arc::new(Hints::new(labels.into_iter().map(|label| (0, Hint { byte: 1, label: label.into(), kind: HintKind::Parameter })).collect()));
        let mut cache = Cache::default(); cache.configure(&doc, Some(8), Some(&hints));
        let mut end = 0;
        for index in 0..1000 {
            let Some(row) = ready(&mut cache, &doc, |cache| cache.row(&doc, 0, index, 32)) else { break };
            assert!(row.display.len() <= LIMIT);
            assert_eq!(row.display.start, end); end = row.display.end;
            assert!(row.source.end <= doc.len_bytes());
            if row.last { break; }
        }
        assert_eq!(end, 50_005);
    }

    let doc = snapshot(&(" ".repeat(20_000) + "END"));
    let mut cache = Cache::default(); cache.configure(&doc, Some(8), None);
    let row = ready(&mut cache, &doc, |cache| cache.row(&doc, 0, 1, 32)).unwrap();
    assert_eq!(row.display, LIMIT..LIMIT * 2);
    assert_eq!(row.end_byte(), LIMIT * 2 - 1);
    let at = ready(&mut cache, &doc, |cache| cache.position(&doc, 0, LIMIT + 1)).unwrap();
    assert_eq!((at.row, at.column), (1, row.indent + 1));
    assert_eq!(ready(&mut cache, &doc, |cache| cache.byte_at(&doc, &row, row.indent + 1)), Some(LIMIT + 1));
    assert_eq!(doc.copy_range(LIMIT - 1..LIMIT + 1).unwrap(), "  ");
}

#[test]
fn wide_columns_keep_the_continuation_indent_inside_the_native_byte_budget() {
    let doc = snapshot(&(" ".repeat(20_000) + "END"));
    let mut cache = Cache::default(); cache.configure(&doc, Some(100_000), None);
    let row = ready(&mut cache, &doc, |cache| cache.row(&doc, 0, 1, 32)).unwrap();
    assert!(row.indent <= ROW_BYTES / 2, "continuation indentation must be bounded even at enormous column widths: {}", row.indent);
    assert!(row.indent + row.display.len() <= ROW_BYTES + ROW_BYTES / 2);
    assert_eq!(row.display, ROW_BYTES..ROW_BYTES * 2);
}

#[track_caller]
fn ready<T>(cache: &mut Cache, snapshot: &TextSnapshot, mut query: impl FnMut(&mut Cache) -> Query<T>) -> T {
    for _ in 0..512 {
        match query(cache) {
            Query::Ready(value) => return value,
            Query::Pending(work) => { assert!(cache.apply(work.run(snapshot, 4096, || true).unwrap())); }
        }
    }
    panic!("layout did not make progress");
}

#[test]
fn shared_wrap_rows_and_far_coordinates_do_not_stop_at_the_old_prefix() {
    let doc = snapshot("\tab cd ef\n");
    let mut cache = Cache::default();
    cache.configure(&doc, Some(6), None);
    let rows: Vec<_> = (0..3).map(|i| ready(&mut cache, &doc, |c| c.row(&doc, 0, i, 3)).unwrap()).collect();
    assert_eq!(rows.iter().map(|r| r.display.clone()).collect::<Vec<_>>(), [0..7, 7..10, 10..12]);
    assert_eq!(rows.iter().map(|r| r.source.clone()).collect::<Vec<_>>(), [0..4, 4..7, 7..9]);
    assert_eq!(rows.iter().map(|r| r.indent).collect::<Vec<_>>(), [0, 3, 3]);
    assert!(rows[2].last && rows.iter().all(|r| r.complete));

    let doc = snapshot(&("header\r\n".to_string() + &"x".repeat(20_000) + "\r\n"));
    cache.configure(&doc, Some(10), None);
    let row = ready(&mut cache, &doc, |c| c.row(&doc, 1, 1200, 20)).unwrap();
    assert_eq!(row.display, 12_000..12_010);
    assert_eq!(row.source, 12_008..12_018);
    let p = ready(&mut cache, &doc, |c| c.position(&doc, 1, 12_013)).unwrap();
    assert_eq!((p.row, p.display, p.column), (1200, 12_005, 5));
    let cursor = ready(&mut cache, &doc, |c| c.cursor(&doc, 1, row.display.start)).unwrap();
    assert!(cursor.display_byte() <= row.display.start && row.display.start - cursor.display_byte() <= 4096);
    let byte = ready(&mut cache, &doc, |c| c.byte_at(&doc, &row, 5)).unwrap();
    assert_eq!(byte, 12_013);
}

#[test]
fn logical_wrap_never_splits_an_ordinary_projected_grapheme() {
    let text = "🇬🇷 👩🏽‍💻 e\u{301}";
    let doc = snapshot(text);
    let mut cache = Cache::default();
    cache.configure(&doc, Some(4), None);
    let mut actual = Vec::new();
    for i in 0..10 {
        let Some(row) = ready(&mut cache, &doc, |c| c.row(&doc, 0, i, 10)) else { break };
        actual.push(text.get(row.display).unwrap().to_string());
    }
    assert_eq!(actual, ["🇬🇷 ", "👩🏽‍💻 ", "e\u{301}"]);
    let doc = snapshot("👩‍💻X");
    cache.configure(&doc, Some(1), None);
    let row = ready(&mut cache, &doc, |c| c.row(&doc, 0, 0, 3)).unwrap();
    assert_eq!(row.display, 0.."👩‍💻".len());
    assert_eq!(ready(&mut cache, &doc, |c| c.row(&doc, 0, 1, 3)).unwrap().display, "👩‍💻".len().."👩‍💻X".len());
    let _ = Cursor::start(&doc, 0, None).unwrap();
}

#[test]
fn session_resumes_cold_rows_on_the_shared_worker_lane() {
    use std::{sync::Arc, time::{Duration, Instant}};
    use code_core::session::{Command, Completion, Config, Session};
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let path = root.join("cold.txt");
    std::fs::write(&path, "x".repeat(200_000)).unwrap();
    let mut session = Session::new(Config {
        themes: lenz_content::theme::ThemePaths::under(&root.join("config"), &root.join("state"), "lenz-code"),
        recent: root.join("state/recent.json"), settings: root.join("state/settings.json"),
        providers: root.join("config/providers.json"), cache: root.join("cache"),
        idle: code_core::semantics::IDLE, clock: Arc::new(Instant::now), overrides: Default::default(), dark: true,
    });
    let rx = session.completions();
    session.dispatch(Command::Open(path));
    let deadline = Instant::now() + Duration::from_secs(3);
    while !session.state().active().is_some_and(|tab| tab.loaded()) && Instant::now() < deadline {
        while let Ok(completion) = rx.try_recv() { session.dispatch(completion); }
        std::thread::sleep(Duration::from_millis(1));
    }
    let tab = session.state().active().unwrap().id();
    session.dispatch(Command::SetTextSize { pane: 0, cols: 10, rows: 10 });
    session.dispatch(Command::SetSoftWrap(true));
    let mut completed = 0;
    loop {
        assert!(Instant::now() < deadline, "cold layout must make progress");
        match session.state().view(tab).unwrap().layout_row(0, 19_000, Some(10)) {
            Query::Ready(Some(row)) => { assert_eq!(row.source, 190_000..190_010); break; }
            Query::Ready(None) => panic!("missing current source row"),
            Query::Pending(work) => session.dispatch(Command::PrepareLayout { pane: 0, tab, work }),
        };
        loop {
            assert!(Instant::now() < deadline);
            if let Ok(completion) = rx.try_recv() {
                let layout = matches!(&completion, Completion::LaidOut(_));
                session.dispatch(completion);
                if layout { completed += 1; break; }
            } else { std::thread::sleep(Duration::from_millis(1)); }
        }
    }
    assert!(completed >= 2, "distant rows must resume through bounded batches");
}
