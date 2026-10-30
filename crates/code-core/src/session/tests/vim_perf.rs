//! Candidate-only explicit Insert-group publication probe; the ordinary case has different undo semantics.
use super::*;
use crate::{keymap::KeyPress, text::EditKind};
use std::sync::atomic::AtomicU64;
use std::time::Instant;

fn vim_key(s: &mut Session, key: &str, ctrl: bool) {
    s.dispatch(Command::VimKey(KeyPress::new(key, ctrl, false, false).unwrap()));
}

#[test]
#[ignore = "release Vim Insert-group timings: run alone after the build, no native renderer or provider"]
fn vim_insert_group_publication_perf() {
    let original: String = (0..10_000).map(|n| format!("line {n:05} α source\r\n")).collect();
    let at = original.find("line 05000").unwrap();
    let tokens = ["λ", "界", "a"];
    let typed = tokens.into_iter().cycle().take(300).collect::<String>();
    let changed = format!("{}{typed}{}", &original[..at], &original[at..]);
    let workload_started = Instant::now();
    let mut cases = Vec::new();
    for vim in [false, true] {
        for relative in [false, true] {
            for trial in 0..7 {
                let dir = tempfile::tempdir().unwrap();
                let path = write(dir.path(), "typing.txt", original.as_bytes());
                let millis = Arc::new(AtomicU64::new(0));
                let timer = millis.clone();
                let now = Instant::now();
                let mut s = session_clock(&dir.path().join("state"), Arc::new(move || now + Duration::from_millis(timer.load(Ordering::Relaxed))));
                open(&mut s, Command::OpenRetained(path.clone()), "typing.txt");
                s.enable_text_input_for_test();
                s.dispatch(Command::Select(Selection { anchor: at, head: at }));
                s.dispatch(Command::SetVimEnabled(vim));
                s.dispatch(Command::SetRelativeLineNumbers(relative));
                let tab = s.state().active().unwrap().id();
                let opened = memory_mib();
                if vim { vim_key(&mut s, "i", false); }
                let mut typing_ms = Vec::new();
                let typing_started = Instant::now();
                for (event, token) in tokens.into_iter().cycle().take(300).enumerate() {
                    // Exceed ordinary typing's 500 ms merge window without introducing real delays.
                    millis.store((event as u64 + 1) * 601, Ordering::Relaxed);
                    let started = Instant::now();
                    s.dispatch(Command::InsertText { text: token.into(), kind: EditKind::Typing });
                    typing_ms.push(started.elapsed().as_secs_f64() * 1000.0);
                }
                let typing_total_ms = typing_started.elapsed().as_secs_f64() * 1000.0;
                assert_eq!(source_text(s.state().current_source().unwrap()), changed);
                if vim { vim_key(&mut s, "escape", false); }
                let grouped_transactions = s.state().view(tab).unwrap().draft.unwrap().undo_steps(false);
                assert_eq!(grouped_transactions, if vim { 300 } else { 1 });
                let typed_rss = memory_mib();
                assert_eq!(std::fs::read(&path).unwrap(), original.as_bytes(), "typing and Escape never save");
                let commands = if vim { 1 } else { 300 };
                let started = Instant::now();
                for _ in 0..commands {
                    if vim { vim_key(&mut s, "u", false); } else { s.dispatch(Command::Undo { tab }); }
                }
                let undo_ms = started.elapsed().as_secs_f64() * 1000.0;
                assert_eq!(source_text(s.state().current_source().unwrap()), original, "one Vim u reverses the entire paused Insert group");
                let undone_rss = memory_mib();
                let started = Instant::now();
                for _ in 0..commands {
                    if vim { vim_key(&mut s, "r", true); } else { s.dispatch(Command::Redo { tab }); }
                }
                let redo_ms = started.elapsed().as_secs_f64() * 1000.0;
                assert_eq!(source_text(s.state().current_source().unwrap()), changed);
                let redone_rss = memory_mib();
                assert_eq!(std::fs::read(&path).unwrap(), original.as_bytes(), "Undo/Redo never save");
                let started = Instant::now();
                for _ in 0..100 {
                    if vim { vim_key(&mut s, "j", false); vim_key(&mut s, "k", false); }
                    else { s.dispatch(Command::MoveCaret(Motion::Down)); s.dispatch(Command::MoveCaret(Motion::Up)); }
                }
                let navigation_ms = started.elapsed().as_secs_f64() * 1000.0;
                let selected = s.state().selection().unwrap().head;
                assert_eq!(s.state().current_source().unwrap().line_of(selected), 5000);
                assert_eq!(s.state().line_number(tab, 5000), 5001);
                assert_eq!(s.state().line_number(tab, 4999), if relative { 1 } else { 5000 });
                let started = Instant::now();
                s.dispatch(Command::Save { tab });
                settle(&mut s, |s| !s.state().tab_dirty(tab));
                let save_ms = started.elapsed().as_secs_f64() * 1000.0;
                assert_eq!(std::fs::read(&path).unwrap(), changed.as_bytes());
                let saved_rss = memory_mib();
                drop(s);
                cases.push(serde_json::json!({
                    "vim":vim,"relative_numbers":relative,"trial":trial,
                    "typing_dispatch_ms":typing_ms,"typing_total_ms":typing_total_ms,
                    "undo_ms":undo_ms,"redo_ms":redo_ms,"undo_commands":commands,"redo_commands":commands,
                    "transactions_per_undo":grouped_transactions,"navigation_200_commands_ms":navigation_ms,
                    "save_roundtrip_ms":save_ms,
                    "rss_mib":{"opened":opened,"typed":typed_rss,"undone":undone_rss,"redone":redone_rss,"saved":saved_rss,"dropped":memory_mib()}
                }));
            }
        }
    }
    eprintln!("EDITOR_PERF {}", serde_json::json!({
        "name":"vim/paused-insert-group-publication-v1","candidate_only":true,
        "lines":10000,"source_bytes":original.len(),"events_per_case":300,"virtual_event_spacing_ms":601,
        "actual_duration_seconds":workload_started.elapsed().as_secs_f64(),"memory_kind":memory_kind(),"cases":cases,
        "scope":"direct shared Session dispatch with disposable files; no parser, provider, native paint or physical storage durability latency. Ordinary editing has 300 independent paused transactions and therefore uses 300 Undo/Redo commands; Vim uses one explicit group of 300. RSS is phase sampled and does not establish peak allocation or leak freedom. Save includes completion polling. Seven samples per configuration, with no baseline or significance claim."
    }));
}
