//! Explorer selection, captured copy requests, worker cancellation and publication through Session.
use super::*;
use crate::workspace::Explorer;

fn highlight(s: &mut Session, path: Option<&str>) {
    s.dispatch(Command::SetExplorer(Explorer { visible: true, selected: path.map(str::to_owned), top: None }));
}

fn copied(s: &mut Session, id: CommandId) -> Effect {
    let crate::actions::CatalogAction::Command(command) = s.action(id) else { panic!("{id:?} is not an enabled command") };
    s.dispatch(command)
}

#[test]
fn explorer_path_copy_uses_the_highlighted_entry_and_represents_the_project_root() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("project");
    let mut s = project_with(&root, &[("src/a.rs", "fn a() {}\n"), ("other/b.rs", "fn b() {}\n")], "src/a.rs");
    let active = s.state().active().unwrap().id();
    highlight(&mut s, Some("other/b.rs"));
    for (id, expected) in [(CommandId::CopyExplorerName, "b.rs".to_string()), (CommandId::CopyExplorerFullPath, root.join("other/b.rs").display().to_string()), (CommandId::CopyExplorerRelativePath, "other/b.rs".to_string())] {
        assert_eq!(copied(&mut s, id).copy, Some(expected));
        assert_eq!(s.state().active().unwrap().id(), active);
    }
    for selected in [None, Some("")] {
        highlight(&mut s, selected);
        assert_eq!(copied(&mut s, CommandId::CopyExplorerRelativePath).copy.as_deref(), Some("."));
        assert_eq!(copied(&mut s, CommandId::CopyExplorerName).copy.as_deref(), Some("project"));
        assert_eq!(copied(&mut s, CommandId::CopyExplorerFullPath).copy, Some(root.display().to_string()));
        assert!(s.disabled(CommandId::CopyExplorerEntry).is_some(), "the project root is not an entry to duplicate");
    }
    for selected in ["gone.rs", "../outside", "/etc/passwd"] {
        highlight(&mut s, Some(selected));
        for id in [CommandId::CopyExplorerName, CommandId::CopyExplorerFullPath, CommandId::CopyExplorerRelativePath, CommandId::CopyExplorerEntry] {
            assert!(s.disabled(id).is_some(), "{selected}: {id:?}");
        }
        assert!(s.dispatch(Command::CopyExplorerPath { kind: PathKind::Relative }).copy.is_none());
    }
    assert_eq!(std::fs::read(root.join("src/a.rs")).unwrap(), b"fn a() {}\n");
}

#[test]
fn explorer_paste_captures_source_and_destination_and_uses_saved_bytes_without_touching_a_dirty_draft() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("project");
    let mut s = project_with(&root, &[("src/a.rs", "fn a() {}\n"), ("dest/b.rs", "fn b() {}\n"), ("other/c.rs", "fn c() {}\n")], "src/a.rs");
    s.disable_desktop_file_clipboard_for_test();
    let tab = s.state().active().unwrap().id();
    let expected = s.state().current_source().unwrap().key();
    s.dispatch(Command::Edit { tab, expected, edits: vec![crate::text::TextEdit { range: 0..0, text: "// unsaved\n".into() }], kind: crate::text::EditKind::Paste });
    let draft = source_text(s.state().current_source().unwrap());
    highlight(&mut s, Some("src/a.rs"));
    copied(&mut s, CommandId::CopyExplorerEntry);
    highlight(&mut s, Some("dest"));
    let request = s.paste_entry_request().unwrap();
    assert_eq!(request.source(), root.join("src/a.rs"));
    assert_eq!(request.folder(), "dest");
    assert_eq!(s.paste_entry_start(&request), "a.rs");
    assert_eq!(s.paste_entry_refusal(&request, "renamed.rs"), None);
    // A later Copy or selection change does not retarget the open Paste prompt.
    highlight(&mut s, Some("other/c.rs"));
    copied(&mut s, CommandId::CopyExplorerEntry);
    s.dispatch(Command::PasteExplorerEntry { request, name: "renamed.rs".into() });
    settle(&mut s, |s| s.explorer_copy.is_none());
    assert_eq!(std::fs::read(root.join("dest/renamed.rs")).unwrap(), b"fn a() {}\n");
    assert_eq!(std::fs::read(root.join("src/a.rs")).unwrap(), b"fn a() {}\n");
    assert_eq!(source_text(s.state().view(tab).unwrap().snapshot()), draft);
    assert!(s.state().tab_dirty(tab));
    s.dispatch(Command::Undo { tab });
    assert_eq!(source_text(s.state().view(tab).unwrap().snapshot()), "fn a() {}\n");
    assert_eq!(std::fs::read(root.join("dest/renamed.rs")).unwrap(), b"fn a() {}\n");
}

#[test]
fn explorer_paste_refuses_a_new_collision_and_a_prompt_from_another_project() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("project");
    let other = d.path().join("other");
    let mut s = project_with(&root, &[("src/a.rs", "fn a() {}\n")], "src/a.rs");
    s.disable_desktop_file_clipboard_for_test();
    highlight(&mut s, Some("src/a.rs"));
    copied(&mut s, CommandId::CopyExplorerEntry);
    let request = s.paste_entry_request().unwrap();
    assert_eq!(s.paste_entry_refusal(&request, "new.rs"), None);
    std::fs::write(root.join("src/new.rs"), b"someone else's bytes").unwrap();
    s.dispatch(Command::PasteExplorerEntry { request: request.clone(), name: "new.rs".into() });
    assert!(s.explorer_copy.is_none());
    assert_eq!(std::fs::read(root.join("src/new.rs")).unwrap(), b"someone else's bytes");
    std::fs::create_dir(&other).unwrap();
    s.dispatch(Command::OpenRoot(other.clone()));
    settle(&mut s, |s| s.state().project().is_some_and(|p| p.root() == other) && scanned(s));
    assert!(s.paste_entry_refusal(&request, "elsewhere.rs").unwrap().contains("project changed"));
    s.dispatch(Command::PasteExplorerEntry { request, name: "elsewhere.rs".into() });
    assert!(s.explorer_copy.is_none());
    assert!(!root.join("src/elsewhere.rs").exists() && !other.join("elsewhere.rs").exists());
}

/// Occupy the existing bounded executor so a cancellation test controls when the copy starts.
fn pause_jobs(s: &Session) -> Vec<std::sync::mpsc::Sender<()>> {
    let (started, ready) = std::sync::mpsc::channel();
    let release = (0..crate::jobs::MAX_JOBS).map(|_| {
        let started = started.clone();
        let (release, wait) = std::sync::mpsc::channel();
        s.jobs.submit(Priority::Visible, move || { let _ = started.send(()); let _ = wait.recv(); });
        release
    }).collect();
    for _ in 0..crate::jobs::MAX_JOBS { ready.recv_timeout(Duration::from_secs(10)).unwrap(); }
    release
}

#[test]
fn explorer_copy_cancellation_prevents_publication_and_obsolete_completions_cannot_finish_the_current_work() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("project");
    let mut s = project_with(&root, &[("src/a.rs", "fn a() {}\n")], "src/a.rs");
    s.disable_desktop_file_clipboard_for_test();
    highlight(&mut s, Some("src/a.rs"));
    copied(&mut s, CommandId::CopyExplorerEntry);
    let request = s.paste_entry_request().unwrap();
    let release = pause_jobs(&s);
    s.dispatch(Command::PasteExplorerEntry { request, name: "cancelled.rs".into() });
    let id = s.save_seq;
    assert!(s.explorer_copy.is_some());
    assert!(s.disabled(CommandId::PasteExplorerEntry).is_some());
    assert_eq!(s.disabled(CommandId::CancelExplorerCopy), None);
    let reveal = s.state().tree_reveal().map(|(p, ticket)| (p.to_string(), ticket));
    s.dispatch(Completion::ExplorerCopied { request: id + 1, result: Ok(()) });
    assert!(s.explorer_copy.is_some(), "an unrelated completion cannot finish this copy");
    assert_eq!(s.state().tree_reveal().map(|(p, ticket)| (p.to_string(), ticket)), reveal);
    copied(&mut s, CommandId::CancelExplorerCopy);
    drop(release);
    settle(&mut s, |s| s.explorer_copy.is_none());
    assert!(!root.join("src/cancelled.rs").exists());
    assert!(s.state().notices().iter().any(|n| n.contains("cancelled")));
    s.dispatch(Completion::ExplorerCopied { request: id, result: Ok(()) });
    assert!(!s.state().project().unwrap().has_file("src/cancelled.rs"));
    assert!(s.disabled(CommandId::CancelExplorerCopy).is_some());
    assert_eq!(std::fs::read(root.join("src/a.rs")).unwrap(), b"fn a() {}\n");
}

#[test]
fn explorer_folder_copy_refreshes_nested_inventory_and_keeps_the_original() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("project");
    let mut s = project_with(&root, &[("src/a.rs", "fn a() {}\n"), ("src/nested/b.rs", "fn b() {}\n"), ("dest/keep.rs", "fn keep() {}\n")], "src/a.rs");
    s.disable_desktop_file_clipboard_for_test();
    highlight(&mut s, Some("src"));
    copied(&mut s, CommandId::CopyExplorerEntry);
    highlight(&mut s, Some("dest"));
    let request = s.paste_entry_request().unwrap();
    s.dispatch(Command::PasteExplorerEntry { request, name: "copied".into() });
    settle(&mut s, |s| s.explorer_copy.is_none() && s.state().project().is_some_and(|p| p.has_file("dest/copied/nested/b.rs") && !p.refreshing()));
    let project = s.state().project().unwrap();
    assert!(project.has_dir("dest/copied") && project.has_dir("dest/copied/nested"));
    assert!(project.has_file("src/a.rs") && project.has_file("src/nested/b.rs"));
    assert_eq!(std::fs::read(root.join("dest/copied/nested/b.rs")).unwrap(), b"fn b() {}\n");
    assert_eq!(std::fs::read(root.join("dest/keep.rs")).unwrap(), b"fn keep() {}\n");
    assert_eq!(s.state().tree_reveal().map(|(p, _)| p), Some("dest/copied".into()));
}

/// Receive the real worker barrier result without using the user's desktop clipboard.
fn clipboard_ready(s: &mut Session) -> Effect {
    let rx = s.completions();
    let until = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        assert!(std::time::Instant::now() < until, "clipboard barrier did not finish");
        if let Ok(completion) = rx.recv_blocking_timeout() {
            let text = matches!(completion, Completion::ClipboardTextReady { .. });
            let effect = s.dispatch(completion);
            if text { return effect; }
        }
    }
}

#[test]
fn a_text_copy_cancels_pending_file_clipboard_work_and_only_latest_ready_text_is_delivered() {
    let d = tempfile::tempdir().unwrap();
    let mut s = session(d.path());
    let service = s.file_clipboard.service.clone();
    let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let file = service.begin(Some(cancelled.clone()));
    let gate = service.gate.lock().unwrap();
    assert!(s.dispatch(Command::CopyText("first text".into())).copy.is_none());
    let first = s.file_clipboard.text.unwrap();
    assert!(cancelled.load(Ordering::Relaxed));
    assert!(!service.current(file));
    assert!(s.dispatch(Command::CopyText("latest text".into())).copy.is_none());
    let latest = s.file_clipboard.text.unwrap();
    assert_ne!(first, latest);
    assert!(s.dispatch(Completion::ClipboardTextReady { ticket: first, text: "first text".into() }).copy.is_none());
    assert_eq!(s.file_clipboard.text, Some(latest));
    drop(gate);
    // Workers may finish in either order; the obsolete barrier cannot publish any text.
    let effects = [clipboard_ready(&mut s), clipboard_ready(&mut s)];
    assert_eq!(effects.iter().filter(|effect| effect.copy.is_some()).count(), 1);
    let effect = effects.into_iter().find(|effect| effect.copy.is_some()).unwrap();
    assert_eq!(effect.copy.as_deref(), Some("latest text"));
    assert_eq!(effect.copy_ticket, Some(latest));
    assert!(s.clipboard_text_current(latest));
    assert_eq!(s.file_clipboard.text, Some(latest), "delivery stays pending until its adapter acknowledges the sink");
    s.dispatch(Command::ClipboardTextPublished(latest));
    assert!(s.file_clipboard.text.is_none());
    let effect = s.dispatch(Command::CopyText("next text".into()));
    assert_eq!(effect.copy.as_deref(), Some("next text"));
    assert!(effect.copy_ticket.is_some_and(|ticket| s.clipboard_text_current(ticket)));
}

#[test]
fn clipboard_tickets_reject_late_file_and_text_results_from_another_reader_session() {
    let d = tempfile::tempdir().unwrap();
    let mut a = session(&d.path().join("a"));
    let mut b = session(&d.path().join("b"));
    let service = a.file_clipboard.service.clone();
    b.file_clipboard.service = service.clone();
    let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let file = service.begin(Some(cancelled.clone()));
    let gate = service.gate.lock().unwrap();
    assert!(a.dispatch(Command::CopyText("older window".into())).copy.is_none());
    let older = a.file_clipboard.text.unwrap();
    assert!(b.dispatch(Command::CopyText("newer window".into())).copy.is_none());
    let newer = b.file_clipboard.text.unwrap();
    assert!(cancelled.load(Ordering::Relaxed));
    assert!(!service.current(file) && !service.current(older) && service.current(newer));
    drop(gate);
    assert!(clipboard_ready(&mut a).copy.is_none(), "the older session cannot publish after another window's Copy");
    assert!(a.file_clipboard.text.is_none());
    let delayed = clipboard_ready(&mut b);
    assert_eq!(delayed.copy.as_deref(), Some("newer window"));
    let delivered = a.dispatch(Command::CopyText("newest request".into()));
    assert_eq!(delivered.copy.as_deref(), Some("newest request"));
    assert!(!b.clipboard_text_current(delayed.copy_ticket.unwrap()), "a result accepted by core can still become obsolete before the adapter writes it");
    assert!(a.clipboard_text_current(delivered.copy_ticket.unwrap()));
    b.dispatch(Command::ClipboardTextPublished(newer));
    assert!(b.file_clipboard.text.is_none());
}

#[test]
fn graceful_close_waits_until_the_adapter_publishes_deferred_clipboard_text() {
    use crate::close::{CloseCoordinator, Progress};
    let d = tempfile::tempdir().unwrap();
    let mut s = session(d.path());
    let service = s.file_clipboard.service.clone();
    let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
    service.begin(Some(cancelled.clone()));
    let gate = service.gate.lock().unwrap();
    assert!(s.dispatch(Command::CopyText("keep after quit".into())).copy.is_none());
    let ticket = s.file_clipboard.text.unwrap();
    assert!(cancelled.load(Ordering::Relaxed));
    let mut close = CloseCoordinator::default();
    close.register(s.id());
    close.begin(&mut [&mut s]);
    assert_eq!(close.progress(&mut [&mut s]), Progress::Pending(s.id()));
    assert!(!s.closed());
    drop(gate);
    let effect = clipboard_ready(&mut s);
    assert_eq!(effect.copy.as_deref(), Some("keep after quit"));
    assert_eq!(effect.copy_ticket, Some(ticket));
    assert!(s.clipboard_text_current(ticket));
    assert_eq!(close.progress(&mut [&mut s]), Progress::Pending(s.id()), "receipt by the adapter is not publication");
    s.dispatch(Command::ClipboardTextPublished(ticket));
    assert_eq!(close.progress(&mut [&mut s]), Progress::Committed(vec![s.id()]));
    assert!(s.closed());
}

#[test]
fn a_superseded_clipboard_request_does_not_hold_its_old_window_open() {
    use crate::close::{CloseCoordinator, Progress};
    let d = tempfile::tempdir().unwrap();
    let mut a = session(&d.path().join("a"));
    let mut b = session(&d.path().join("b"));
    let service = a.file_clipboard.service.clone();
    b.file_clipboard.service = service.clone();
    let gate = service.gate.lock().unwrap();
    let release = pause_jobs(&a);
    assert!(a.dispatch(Command::CopyText("obsolete".into())).copy.is_none());
    assert!(b.dispatch(Command::CopyText("keep".into())).copy.is_none());
    let mut close = CloseCoordinator::default();
    close.register(a.id());
    close.begin(&mut [&mut a]);
    assert_eq!(close.progress(&mut [&mut a]), Progress::Pending(a.id()), "the closing session first drains its own queued worker");
    drop(release);
    assert!(clipboard_ready(&mut a).copy.is_none(), "obsolete work exits while another session still holds the desktop gate");
    assert_eq!(close.progress(&mut [&mut a]), Progress::Committed(vec![a.id()]));
    drop(gate);
    let effect = clipboard_ready(&mut b);
    assert_eq!(effect.copy.as_deref(), Some("keep"));
    b.dispatch(Command::ClipboardTextPublished(effect.copy_ticket.unwrap()));
}


#[test]
fn missing_workers_keep_internal_tree_copy_ready_and_refuse_desktop_publication_and_file_paste() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("project");
    let mut s = project_with(&root, &[("src/a.rs", "fn a() {}\n")], "src/a.rs");
    s.jobs = crate::jobs::Jobs::without_workers_for_test();
    assert!(!s.skip_desktop_file_clipboard, "exercise the production worker check before a desktop helper could start");
    highlight(&mut s, Some("src/a.rs"));
    let service = s.file_clipboard.service.clone();
    let prior = service.begin(None);
    assert!(copied(&mut s, CommandId::CopyExplorerEntry).copy.is_none());
    assert!(service.current(prior), "the refused desktop operation does not reserve a publication ticket");
    assert_eq!(s.explorer_clipboard.as_deref(), Some(root.join("src/a.rs").as_path()));
    assert!(s.state().notices().iter().any(|n| n.contains("Desktop file clipboard needs a background worker")));
    let request = s.paste_entry_request().unwrap();
    assert_eq!(request.source(), root.join("src/a.rs"));
    assert!(s.paste_entry_refusal(&request, "new.rs").unwrap().contains("background worker is unavailable"));
    s.dispatch(Command::PasteExplorerEntry { request, name: "new.rs".into() });
    assert!(s.explorer_copy.is_none() && !root.join("src/new.rs").exists());
    assert_eq!(s.jobs.queued(), 0);
    assert!(s.clipboard_close_ready());
    assert_eq!(std::fs::read(root.join("src/a.rs")).unwrap(), b"fn a() {}\n");
}

#[test]
fn missing_workers_allow_immediate_text_copy_but_refuse_a_busy_clipboard_without_inline_work() {
    let d = tempfile::tempdir().unwrap();
    let mut s = session(d.path());
    s.jobs = crate::jobs::Jobs::without_workers_for_test();
    let free = s.dispatch(Command::CopyText("immediate".into()));
    assert_eq!(free.copy.as_deref(), Some("immediate"));
    assert!(free.copy_ticket.is_some_and(|ticket| s.clipboard_text_current(ticket)));
    let service = s.file_clipboard.service.clone();
    let gate = service.gate.lock().unwrap();
    let started = std::time::Instant::now();
    let busy = s.dispatch(Command::CopyText("must not block".into()));
    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(busy.copy.is_none() && busy.copy_ticket.is_none());
    assert!(s.file_clipboard.text.is_none());
    assert!(s.state().notices().iter().any(|n| n.contains("Clipboard is busy and no background worker is available")));
    assert_eq!(s.jobs.queued(), 0);
    assert!(s.clipboard_close_ready());
    drop(gate);
}

#[test]
fn explorer_search_is_literal_unicode_and_only_selects_visible_names_until_enter() {
    use ExplorerSearchInput as Search;
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("project");
    let mut s = project_with(&root, &[("src/a.rs", "active\r\n"), ("src/Beta.rs", "beta\n"), ("src/beta-two.rs", "two\n"), ("src/Éclair[1].rs", "unicode\n"), ("src/BetaDir/hidden.rs", "hidden\n")], "src/a.rs");
    s.dispatch(Command::RevealInTree("src/a.rs".into()));
    highlight(&mut s, Some("src/a.rs"));
    let active = s.state().active().unwrap().id();
    let snapshot = s.state().current_source().unwrap().key();
    s.dispatch(Command::ExplorerSearch(Search::Text("BETA".into())));
    assert_eq!(s.state().explorer_search().query(), "BETA");
    assert_eq!(s.state().explorer_search().matched(), 3);
    let candidates: Vec<_> = s.state().tree_rows().filter(|r| r.name.to_lowercase().contains("beta")).map(|r| r.path.to_string()).collect();
    assert_eq!(s.state().explorer().selected.as_ref(), candidates.first());
    for expected in candidates.iter().skip(1).chain(candidates.iter().take(1)) {
        s.dispatch(Command::ExplorerSearch(Search::Step { forward: true }));
        assert_eq!(s.state().explorer().selected.as_ref(), Some(expected));
    }
    s.dispatch(Command::ExplorerSearch(Search::Step { forward: false }));
    assert_eq!(s.state().explorer().selected.as_ref(), candidates.last());
    assert_eq!(s.state().active().unwrap().id(), active);
    assert_eq!(s.state().current_source().unwrap().key(), snapshot);
    s.dispatch(Command::ExplorerSearch(Search::Clear));
    assert_eq!(s.state().explorer().selected.as_deref(), Some("src/a.rs"));
    s.dispatch(Command::ExplorerSearch(Search::Text("éCLAIR[".into())));
    assert_eq!(s.state().explorer_search().matched(), 1);
    assert_eq!(s.state().explorer_search().ranges("Éclair[1].rs").collect::<Vec<_>>().as_slice(), std::slice::from_ref(&(0..8)));
    s.dispatch(Command::ExplorerSearch(Search::Backspace));
    assert_eq!(s.state().explorer_search().query(), "éCLAIR");
    s.dispatch(Command::ExplorerSearch(Search::Activate));
    settle(&mut s, |s| s.state().active().is_some_and(|t| t.source() == root.join("src/Éclair[1].rs")));
    assert!(s.state().explorer_search().query().is_empty());
    assert_eq!(std::fs::read(root.join("src/a.rs")).unwrap(), b"active\r\n");
}

#[test]
fn explorer_search_refuses_stale_or_missing_candidates_and_resets_for_another_project() {
    use ExplorerSearchInput as Search;
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("project");
    let mut s = project_with(&root, &[("src/a.rs", "active\n"), ("src/b.rs", "candidate\n"), ("src/sub/hidden.rs", "hidden\n")], "src/a.rs");
    s.dispatch(Command::RevealInTree("src/a.rs".into()));
    highlight(&mut s, Some("src/a.rs"));
    let active = s.state().active().unwrap().id();
    s.dispatch(Command::ExplorerSearch(Search::Text("hidden".into())));
    assert_eq!(s.state().explorer_search().matched(), 0, "collapsed descendants are not searched");
    s.dispatch(Command::ExplorerSearch(Search::Activate));
    assert_eq!(s.state().active().unwrap().id(), active, "no match must not activate an unrelated row");
    s.dispatch(Command::ExplorerSearch(Search::Text("b.rs".into())));
    s.dispatch(Command::ToggleDir("src".into()));
    s.dispatch(Command::ExplorerSearch(Search::Activate));
    assert_eq!(s.state().active().unwrap().id(), active, "a no-longer-visible candidate is refused");
    s.dispatch(Command::ExplorerSearch(Search::Text("x".repeat(300))));
    assert_eq!(s.state().explorer_search().query().len(), 256);
    let walk = s.state.project.as_ref().unwrap().walk;
    s.dispatch(Completion::Scanned { generation: walk + 1, batch: crate::project::Batch::default() });
    assert_eq!(s.state().explorer_search().query().len(), 256, "a stale scan cannot change the current search");
    s.dispatch(Completion::Scanned { generation: walk, batch: crate::project::Batch { done: true, ..Default::default() } });
    assert!(s.state().explorer_search().query().is_empty(), "an accepted inventory refresh resets dependent search state");
    s.dispatch(Command::ExplorerSearch(Search::Text("again".into())));
    let next = d.path().join("next");
    std::fs::create_dir(&next).unwrap();
    s.dispatch(Command::OpenRoot(next.clone()));
    settle(&mut s, |s| s.state().project().is_some_and(|p| p.root() == next));
    assert!(s.state().explorer_search().query().is_empty());
}

#[test]
#[ignore = "release cached 10k-row explorer latency and retained-memory probe; run alone"]
fn explorer_search_cached_inventory_costs() {
    use ExplorerSearchInput as Search;
    let d = tempfile::tempdir().unwrap();
    let mut s = session(d.path());
    let mut project = Project::new(d.path().to_owned(), 1, true);
    project.add(crate::project::Batch { paths: (0..10_000).map(|n| format!("file-{n:05}.rs")).collect(), done: true, ..Default::default() });
    s.state.project = Some(project);
    highlight(&mut s, Some("file-00000.rs"));
    let before = memory_mib().unwrap();
    let started = std::time::Instant::now();
    let mut samples = std::collections::BTreeMap::<&str, Vec<f64>>::new();
    let mut cycles = 0;
    while started.elapsed() < Duration::from_secs(10) {
        for (name, input) in [("query", Search::Text("file-009".into())), ("next", Search::Step { forward: true }), ("previous", Search::Step { forward: false }), ("clear", Search::Clear)] {
            let now = std::time::Instant::now();
            s.dispatch(Command::ExplorerSearch(input));
            let elapsed = now.elapsed().as_secs_f64() * 1000.0;
            let timings = samples.entry(name).or_default();
            if timings.len() < 1000 { timings.push(elapsed); }
            if name == "query" {
                assert_eq!(s.state().explorer_search().matched(), 100);
                assert_eq!(s.state().explorer().selected.as_deref(), Some("file-00900.rs"));
            }
        }
        assert_eq!(s.state().explorer().selected.as_deref(), Some("file-00000.rs"));
        assert_eq!(s.state().tree_rows().count(), 10_000);
        assert!(s.state().active().is_none());
        cycles += 1;
    }
    let after = memory_mib().unwrap();
    eprintln!("EXPLORER_PERF {}", serde_json::json!({"seconds":started.elapsed().as_secs_f64(),"cycles":cycles,"timings_ms":samples,"rss_before_mib":before,"rss_after_mib":after,"memory_kind":memory_kind(),"correctness":"100 visible matches; stable candidate; original restored; no document opened; inventory unchanged"}));
    assert!(after - before < 16.0, "bounded query and unchanged inventory should not retain growing search data");
}
