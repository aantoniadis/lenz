//! End-of-epic release measurements. No provider or renderer timing is implied.
use super::*;

#[test]
#[ignore = "release performance/stress: run alone; EDITOR_STRESS_SECONDS defaults to 900"]
fn integrated_editing_costs_and_retention() {
    use crate::text::{EditKind, TextEdit};
    let seconds: u64 = std::env::var("EDITOR_STRESS_SECONDS").ok().and_then(|v| v.parse().ok()).unwrap_or(900);
    assert!(seconds > 0);
    let dir = tempfile::tempdir().unwrap();
    let original: String = (0..100_000).map(|n| format!("line {n:06} α theme\r\n")).collect();
    let path = write(dir.path(), "large.txt", original.as_bytes());
    let sql = "WITH source AS (SELECT 1 AS id, 2 AS name) SELECT s. FROM source s";
    let query = write(dir.path(), "query.sql", sql.as_bytes());
    let mut s = session(&dir.path().join("state"));
    s.enable_text_input_for_test();
    open(&mut s, Command::OpenRetained(query.clone()), "query.sql");
    s.dispatch(Command::SetSqlDialect(Some(crate::sql_dialect::Dialect::PostgreSql)));
    let sql_tab = s.state().active().unwrap().id();
    open(&mut s, Command::OpenRetained(path.clone()), "large.txt");
    let tab = s.state().active().unwrap().id();
    let mut timings = std::collections::BTreeMap::<&str, Vec<f64>>::new();
    let mut record = |name, elapsed: std::time::Duration| {
        let values = timings.entry(name).or_default();
        if values.len() < 1000 { values.push(elapsed.as_secs_f64() * 1000.0); }
    };
    let started = std::time::Instant::now();
    let mut samples = Vec::<(f64, f64)>::new();
    let mut next_sample = started;
    let mut iterations = 0u64;
    while started.elapsed() < Duration::from_secs(seconds) {
        iterations += 1;
        let at = [0, original.find("line 050000").unwrap(), original.len()][iterations as usize % 3];
        let expected = s.state().current_source().unwrap().key();
        let now = std::time::Instant::now();
        s.dispatch(Command::Edit { tab, expected, edits: vec![TextEdit { range: at..at, text: "draft λ\r\n".into() }], kind: EditKind::Replace });
        record("edit/100k", now.elapsed());
        assert_eq!(s.state().current_source().unwrap().copy_range(at..at + 10).unwrap(), "draft λ\r\n");
        if iterations.is_multiple_of(10) {
            let now = std::time::Instant::now();
            markers::settle_markers(&mut s, |s| s.state().markers(tab).is_some_and(|m| !m.pending()));
            record("markers/100k", now.elapsed());
            let line = s.state().current_source().unwrap().line_of(at);
            let now = std::time::Instant::now();
            s.dispatch(Command::InspectChangeAt { tab, line });
            assert!(s.inspected_change_text().unwrap().contains("Prior content (0 bytes):\n[empty]"));
            record("inspect-change/100k", now.elapsed());
            s.dispatch(Command::CloseChange);
            let now = std::time::Instant::now();
            s.dispatch(Command::CompareSaved);
            settle(&mut s, |s| s.state().saved_comparison().is_some_and(|c| !c.pending()));
            assert!(!s.state().saved_comparison().unwrap().changes().is_empty());
            record("compare-saved/100k", now.elapsed());
            s.dispatch(Command::CloseSavedComparison);
        }
        for (name, command) in [("undo/100k", Command::Undo { tab }), ("redo/100k", Command::Redo { tab }), ("undo/100k", Command::Undo { tab })] {
            let now = std::time::Instant::now();
            s.dispatch(command);
            record(name, now.elapsed());
        }
        assert_eq!(s.state().current_source().unwrap().len_bytes(), original.len());
        if iterations.is_multiple_of(20) {
            s.dispatch(Command::ActivateTab(sql_tab));
            settle(&mut s, |s| s.state().active().is_some_and(|t| t.id() == sql_tab) && s.state().current_source().is_some());
            let at = sql.find("s. FROM").unwrap() + 2;
            s.dispatch(Command::Select(Selection { anchor: at, head: at }));
            let now = std::time::Instant::now();
            s.dispatch(Command::Complete);
            settle(&mut s, |s| s.state().completion().is_some());
            assert_eq!(s.state().completion().unwrap().rows().map(|r| r.label.as_str()).collect::<Vec<_>>(), ["id", "name"]);
            record("sql/offline-completion", now.elapsed());
            s.dispatch(Command::AcceptCompletion);
            assert_eq!(source_text(s.state().current_source().unwrap()), sql.replace("s. FROM", "s.id FROM"));
            s.dispatch(Command::Undo { tab: sql_tab });
            s.dispatch(Command::CloseCompletion);
            s.dispatch(Command::ActivateTab(tab));
            settle(&mut s, |s| s.state().active().is_some_and(|t| t.id() == tab) && s.state().current_source().is_some());
        }
        while let Ok(c) = s.completions().try_recv() { s.dispatch(c); }
        if std::time::Instant::now() >= next_sample {
            samples.push((started.elapsed().as_secs_f64(), memory_mib().unwrap()));
            next_sample = std::time::Instant::now() + Duration::from_secs(1);
        }
        assert_eq!(s.state().tabs().len(), 2, "temporary comparison tabs are released");
    }
    assert_eq!(source_text(s.state().current_source().unwrap()), original);
    assert_eq!(std::fs::read(path).unwrap(), original.as_bytes());
    assert_eq!(std::fs::read(query).unwrap(), sql.as_bytes());
    let thirds = samples.len() / 3;
    let median = |values: &[(f64, f64)]| { let mut v: Vec<_> = values.iter().map(|(_, rss)| *rss).collect(); v.sort_by(f64::total_cmp); v[v.len()/2] };
    let growth = if thirds > 0 { median(&samples[samples.len()-thirds..]) - median(&samples[thirds..2*thirds]) } else { 0.0 };
    eprintln!("EDITOR_PERF {}", serde_json::json!({"seconds":started.elapsed().as_secs_f64(),"iterations":iterations,"timings_ms":timings,"rss_timeline":samples,"tail_growth_mib":growth,"memory_kind":memory_kind(),"correctness":"exact original bytes; SQL completion accept/undo; two retained tabs; comparison closes"}));
    assert!(growth <= 16.0, "fixed working set grew by {growth:.1} MiB after warm-up");
}

#[test]
#[ignore = "release catalog costs: explicit disposable PostgreSQL/DuckDB fixture variables required"]
fn explicit_catalog_costs() {
    let pg = std::env::var("LENZ_TEST_PG_SERVICE").expect("disposable service");
    let psql = PathBuf::from(std::env::var_os("LENZ_TEST_PSQL").expect("psql executable"));
    let duck = PathBuf::from(std::env::var_os("LENZ_TEST_DUCKDB").expect("DuckDB executable"));
    let file = PathBuf::from(std::env::var_os("LENZ_TEST_DUCKDB_FILE").expect("disposable DuckDB file"));
    let before = std::fs::read(&file).unwrap();
    let mut samples = std::collections::BTreeMap::<&str, Vec<f64>>::new();
    for trial in 0..33 {
        for name in ["postgres", "duckdb"] {
            let now = std::time::Instant::now();
            let catalog = if name == "postgres" { crate::sql_metadata::postgres(&psql, &pg, &|| true) } else { crate::sql_metadata::duckdb(&duck, &file, &|| true) }.unwrap();
            let ms = now.elapsed().as_secs_f64() * 1000.0;
            assert!(catalog.relations.iter().any(|r| r.name.contains("lenz_metadata_fixture") && r.columns.iter().any(|c| c.name == "id")));
            assert!(catalog.functions.iter().any(|f| f.name.contains("lenz_metadata_function")));
            if trial >= 3 { samples.entry(name).or_default().push(ms); }
        }
    }
    assert_eq!(std::fs::read(file).unwrap(), before);
    eprintln!("EDITOR_PERF {}", serde_json::json!({"catalog_refresh_ms":samples,"scope":"fixed metadata operation and subprocess startup on tiny local disposable catalogs; three warmups, thirty samples; no query execution or remote latency claim"}));
}

#[test]
#[ignore = "release template materialization costs: run alone"]
fn template_materialization_costs() {
    let mut measurements = Vec::new();
    for (extension, line) in [("html.erb", "<p><%= helper(value) %></p>\n"), ("haml", "%p= helper(value)\n")] {
        for lines in [100, 10_000] {
            let text = line.repeat(lines);
            let mut samples = Vec::new();
            let mut generated_bytes = 0;
            let mut spans = 0;
            for version in 0..33 {
                let now = std::time::Instant::now();
                let doc = Document::decode(format!("/synthetic/template.{extension}").into(), version, text.as_bytes());
                let elapsed = now.elapsed().as_secs_f64() * 1000.0;
                let views = doc.templates().unwrap();
                assert!(views.unmapped().is_none(), "{:?}", views.unmapped());
                let ruby = views.primary().unwrap();
                let projection = ruby.embedded().unwrap();
                let at = text.find("helper(value)").unwrap();
                assert_eq!(projection.to_generated(at).and_then(|at| projection.to_source(at)), Some(at));
                generated_bytes = views.served().map(|v| v.text().len()).sum::<usize>();
                spans = views.served().map(|v| v.embedded().unwrap().spans().len()).sum::<usize>();
                if version >= 3 { samples.push(elapsed); }
                let observed = Arc::downgrade(ruby);
                drop(doc);
                assert!(observed.upgrade().is_none(), "dropping the source releases its projections");
            }
            measurements.push(serde_json::json!({"extension":extension,"lines":lines,"source_bytes":text.len(),"materialization_ms":samples,"generated_text_bytes":generated_bytes,"mapping_spans":spans}));
        }
    }
    eprintln!("EDITOR_PERF {}", serde_json::json!({"templates":measurements,"scope":"decode and materialize syntax projections; three warmups and thirty samples; generic templates without Rails context or a running language server"}));
}
