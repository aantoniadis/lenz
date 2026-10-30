use super::*;

fn scan_session() -> (tempfile::TempDir, PathBuf, Session) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    std::fs::create_dir_all(&root).unwrap();
    (dir, root.clone(), session(&root.parent().unwrap().join("private-state")))
}
fn finish_scan(s: &mut Session) { settle(s, |s| s.state().results().is_some_and(|r| r.results_kind() == ResultsKind::ProjectSyntax && r.done())); }

#[test]
fn project_syntax_checks_unopened_ignored_filtered_files_without_opening_tabs_or_servers() {
    let (_dir, root, mut s) = scan_session();
    write(&root, ".ignore", b"ignored.rs\n");
    write(&root, "ignored.rs", b"fn ignored( {\n");
    write(&root, "bad.rs", b"fn broken( {\n");
    write(&root, "Bad.java", b"class A { void f( { }\n");
    write(&root, "bad.xml", b"<Root></root>\n");
    write(&root, "good.rs", b"fn main() {}\n");
    write(&root, "note.txt", b"plain text\n");
    write(&root, "query.sql", b"SELECT 1\n");
    let huge = std::fs::File::create(root.join("huge.rs")).unwrap();
    huge.set_len(crate::document::LARGE_BYTES as u64 + 1).unwrap();
    s.dispatch(Command::OpenRoot(root.clone()));
    settle(&mut s, scanned);
    assert!(s.state().results().is_none() && s.state().tabs().is_empty(), "no eager syntax scan");
    s.dispatch(Command::CheckProjectSyntax);
    assert!(s.disabled(CommandId::CancelProjectSyntax).is_none());
    finish_scan(&mut s);
    let results = s.state().results().unwrap();
    for file in ["bad.rs", "Bad.java", "bad.xml"] { assert!(results.hits().iter().any(|h| h.path.as_ref() == file), "{file}: {:?}", results.hits()); }
    assert!(!results.hits().iter().any(|h| h.path.as_ref() == "ignored.rs"));
    assert!(results.kind().contains("4 checked"), "{}", results.kind());
    assert!(results.kind().contains("1 too large"), "{}", results.kind());
    assert!(results.kind().contains("3 unsupported"), "{}", results.kind());
    assert!(s.state().tabs().is_empty() && !s.state.semantics.running(), "scanning never opens tabs or servers");
    assert!(s.state().path_problem_count(&root.join("bad.rs"), false) > 0);
    let index = results.hits().iter().position(|h| h.path.as_ref() == "Bad.java").unwrap();
    s.dispatch(Command::OpenResult(index));
    settle(&mut s, |s| s.state().active().is_some_and(|t| t.source() == root.join("Bad.java")));
    assert_eq!(std::fs::read(root.join("bad.xml")).unwrap(), b"<Root></root>\n");
}

#[test]
fn project_syntax_prefers_open_drafts_and_stops_on_edit_cancel_or_root_change() {
    let (_dir, root, mut s) = scan_session();
    let path = write(&root, "a.rs", b"fn broken( {\n");
    s.dispatch(Command::OpenRoot(root.clone())); settle(&mut s, scanned);
    open(&mut s, Command::OpenRetained(path.clone()), "a.rs");
    let tab = s.state().active().unwrap().id();
    let snapshot = s.state().current_source().unwrap();
    let (expected, len) = (snapshot.key(), snapshot.len_bytes());
    s.dispatch(Command::Edit { tab, expected, edits: vec![crate::text::TextEdit { range: 0..len, text: "fn main() {}\n".into() }], kind: crate::text::EditKind::Replace });
    s.dispatch(Command::CheckProjectSyntax); finish_scan(&mut s);
    assert!(s.state().results().unwrap().hits().is_empty(), "the clean unsaved draft wins over broken disk text");
    assert_eq!(std::fs::read(&path).unwrap(), b"fn broken( {\n");
    s.dispatch(Command::CheckProjectSyntax);
    let expected = s.state().current_source().unwrap().key();
    s.dispatch(Command::Edit { tab, expected, edits: vec![crate::text::TextEdit { range: 0..0, text: "// newer\n".into() }], kind: crate::text::EditKind::Replace });
    assert!(s.state().results().unwrap().done());
    assert!(s.state().results().unwrap().kind().contains("Stopped"));
    s.dispatch(Command::CheckProjectSyntax);
    s.dispatch(Command::CancelProjectSyntax);
    let id = s.state().results().unwrap().id();
    assert!(s.state().results().unwrap().kind().contains("Cancelled"));
    while let Ok(completion) = s.completions().try_recv() { s.dispatch(completion); }
    assert!(s.state().results().is_none_or(|r| r.id() != id || r.kind().contains("Cancelled")));
    s.dispatch(Command::CheckProjectSyntax);
    let next = root.parent().unwrap().join("other"); std::fs::create_dir(&next).unwrap();
    s.dispatch(Command::OpenRoot(next.clone()));
    if let Some(prompt) = s.close_prompt() { s.dispatch(Command::ChooseClose { ticket: prompt.ticket, buffer: prompt.buffer, choice: crate::close::Choice::Discard }); }
    settle(&mut s, |s| scanned(s) && s.state().project().is_some_and(|p| p.root() == next));
    while let Ok(completion) = s.completions().try_recv() { s.dispatch(completion); }
    assert!(s.syntax_scan.is_none());
    assert_eq!(s.state().project().unwrap().root(), next);
}

#[cfg(unix)]
#[test]
fn project_syntax_skips_links_outside_root_and_reports_xml_validation_limits_without_error_badges() {
    let (dir, root, mut s) = scan_session();
    let outside = write(dir.path(), "outside.rs", b"fn broken( {\n");
    std::os::unix::fs::symlink(&outside, root.join("linked.rs")).unwrap();
    write(&root, "entities.xml", b"<!DOCTYPE r [<!ENTITY custom 'safe'>]><r>&custom;</r>");
    write(&root, "prompt.zsh", b"print -r -- ${(@kv)options}\n");
    s.dispatch(Command::OpenRoot(root.clone())); settle(&mut s, scanned);
    s.dispatch(Command::CheckProjectSyntax); finish_scan(&mut s);
    let results = s.state().results().unwrap();
    assert!(results.kind().contains("1 unreadable/changed"), "{}", results.kind());
    assert!(results.kind().contains("1 limited"), "{}", results.kind());
    assert!(results.kind().contains("1 unsupported"), "{}", results.kind());
    assert!(!results.hits().iter().any(|hit| hit.path.as_ref() == "prompt.zsh"));
    assert!(results.hits().iter().any(|h| h.text.contains("Info:") && h.text.contains("validation limit")));
    assert_eq!(s.state().path_problem_count(&root.join("entities.xml"), false), 0);
    assert_eq!(std::fs::read(outside).unwrap(), b"fn broken( {\n");
}

#[test]
fn project_syntax_rejects_a_disk_file_changed_after_its_worker_finished() {
    let (_dir, root, mut s) = scan_session();
    let path = write(&root, "a.rs", b"fn broken( {\n");
    s.dispatch(Command::OpenRoot(root.clone())); settle(&mut s, scanned);
    s.watcher = Watcher::new(|_| true);
    s.dispatch(Command::CheckProjectSyntax);
    let reply = loop {
        let completion = s.completions().recv_blocking_timeout().unwrap();
        if matches!(&completion, Completion::Analyzed { result, .. } if matches!(result.as_ref(), Completion::ProjectSyntax(_))) { break completion; }
        s.dispatch(completion);
    };
    std::fs::write(&path, b"fn main() {}\n").unwrap();
    s.dispatch(reply); finish_scan(&mut s);
    assert!(s.state().results().unwrap().hits().is_empty());
    assert!(s.state().results().unwrap().kind().contains("1 unreadable/changed"));
    assert_eq!(s.state().path_problem_count(&path, false), 0);
}

#[test]
fn project_syntax_keeps_large_xml_error_messages_compact() {
    let (_dir, root, mut s) = scan_session();
    let xml = format!("<{}></{}>", "a".repeat(6000), "b".repeat(6000));
    let path = write(&root, "large-message.xml", xml.as_bytes());
    s.dispatch(Command::OpenRoot(root)); settle(&mut s, scanned);
    s.dispatch(Command::CheckProjectSyntax); finish_scan(&mut s);
    let results = s.state().results().unwrap();
    assert_eq!(results.hits().len(), 1);
    assert!(results.hits()[0].text.chars().count() <= 520, "compact reports must not retain whole tag names");
    assert!(results.hits()[0].text.ends_with('…'));
    assert_eq!(s.state().path_problem_count(&path, false), 1);
}

#[test]
#[ignore = "release project-syntax/helper/RSS probe: run alone after builds finish"]
fn project_syntax_release_probe() {
    use std::time::Instant;
    const BAD: &[u8] = b"<Root></root>\n";
    let (_dir, root, mut s) = scan_session();
    let mut paths = Vec::with_capacity(1024);
    for group in 0..32 {
        let folder = root.join(format!("g{group:02}"));
        std::fs::create_dir(&folder).unwrap();
        for file in 0..32 { paths.push(write(&folder, &format!("f{file:02}.xml"), BAD)); }
    }
    s.dispatch(Command::OpenRoot(root.clone())); settle(&mut s, scanned);
    // The fixed private fixture does not need filesystem monitoring during the in-process probe.
    s.watcher = Watcher::new(|_| true);
    s.dispatch(Command::ToggleDir("g00".into()));
    let rows: Vec<_> = s.state().tree_rows().take(48).cloned().collect();
    assert_eq!(rows.len(), 48);
    assert_eq!(rows.iter().filter(|row| row.dir.is_some()).count(), 16);
    assert_eq!(s.state.syntax_locations.len(), 0);
    assert!(s.state().tabs().is_empty() && !s.state.semantics.running());
    let counts = |session: &Session, expected: usize| -> Vec<f64> {
        let mut samples = Vec::new();
        for sample in 0..8 {
            let started = Instant::now();
            let mut sum = 0;
            for _ in 0..100 {
                for row in &rows { sum += std::hint::black_box(session.state().row_problem_count(row)); }
            }
            assert_eq!(sum, expected * 100);
            if sample > 0 { samples.push(started.elapsed().as_secs_f64() * 10.0); }
        }
        samples
    };
    let before_helpers = counts(&s, 0);
    let before_rss = memory_mib();
    let mut scan_ms = Vec::new();
    let mut rss = Vec::new();
    for _ in 0..5 {
        let started = Instant::now(); s.dispatch(Command::CheckProjectSyntax); finish_scan(&mut s);
        scan_ms.push(started.elapsed().as_secs_f64() * 1000.0);
        let results = s.state().results().unwrap();
        assert_eq!(results.hits().len(), 1024);
        assert_eq!(results.searched(), 1024);
        assert!(!results.limited());
        assert!(results.kind().contains("1024 checked"), "{}", results.kind());
        assert_eq!(s.state.syntax_locations.len(), 1024);
        assert_eq!(s.state().path_problem_count(&root, true), 1024);
        assert!(s.state().tabs().is_empty() && !s.state.semantics.running());
        rss.push(memory_mib());
    }
    // First 48 actual expanded-tree rows: 16 folders of 32 errors each, and 32 files with one each.
    let known_helpers = counts(&s, 16 * 32 + 32);
    let mut peak_known_messages = 0;
    for items in s.state.syntax_locations.values() {
        peak_known_messages += items.iter().map(|d| d.message.capacity()).sum::<usize>();
    }
    let stress = Instant::now();
    let (mut completed, mut cancelled) = (0, 0);
    while stress.elapsed() < Duration::from_secs(5) {
        s.dispatch(Command::CheckProjectSyntax); finish_scan(&mut s); completed += 1;
        assert_eq!(s.state().results().unwrap().hits().len(), 1024);
        s.dispatch(Command::CheckProjectSyntax);
        s.dispatch(Command::CancelProjectSyntax); cancelled += 1;
        let id = s.state().results().unwrap().id();
        while let Ok(completion) = s.completions().try_recv() { s.dispatch(completion); }
        let results = s.state().results().unwrap();
        assert_eq!(results.id(), id, "late replies cannot replace the cancelled scan");
        assert!(results.done() && results.kind().contains("Cancelled"));
        assert!(results.hits().is_empty());
        assert_eq!(s.state.syntax_locations.len(), 1024);
        assert!(s.state().tabs().is_empty() && !s.state.semantics.running());
    }
    let actual_stress_seconds = stress.elapsed().as_secs_f64();
    for path in &paths { assert_eq!(std::fs::read(path).unwrap(), BAD, "scanning/cancellation never writes source bytes"); }
    assert_eq!(s.state().project().unwrap().files(), 1024);
    eprintln!("PROJECT_SYNTAX_PERF {}", serde_json::json!({
        "schema": 1, "files": 1024, "folders": 32, "fixture": "14-byte malformed XML files; 32 files per folder", "source_bytes": BAD.len() * paths.len(),
        "scan_completed_ms": scan_ms, "rss_mib_before": before_rss, "rss_mib_after_scans": rss, "rss_mib_after_stress": memory_mib(), "memory_kind": memory_kind(),
        "visible_rows": rows.len(), "visible_folder_rows": 16, "visible_file_rows": 32,
        "helper_batch_ms_without_reports": before_helpers, "helper_batch_ms_with_1024_error_paths": known_helpers,
        "known_error_paths": s.state.syntax_locations.len(), "known_message_capacity_bytes": peak_known_messages,
        "stress_completed_scans": completed, "stress_cancelled_scans": cancelled, "actual_stress_seconds": actual_stress_seconds,
        "scope": "candidate-only explicit scans and shared count helpers; helper samples normalize 100 batches of 48 actual tree rows, one warmup/seven samples; zero-report comparison changes data volume, not code; excludes adapter rendering, startup, display and GPU memory; short stress does not establish leak freedom"
    }));
}
