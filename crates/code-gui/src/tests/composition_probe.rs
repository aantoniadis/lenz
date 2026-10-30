//! Candidate-only callback probe; no native input, paint, display or worker-completion timing.
//!
//! Validation (no callback clock reads):
//! cargo test -p code-gui --features test-support composition_callback_sync_cost_probe -- --ignored --nocapture
//!
//! Root's quiet release campaign only:
//! LENZ_COMPOSITION_CALLBACK_TIMINGS=1 cargo test --release -p code-gui --features test-support composition_callback_sync_cost_probe -- --ignored --nocapture
//! Output: one LENZ_COMPOSITION_CALLBACK_PROBE JSON record, schema1. Durations are synchronous
//! callback wall time via Instant, not OS thread-CPU time or completed-action/display latency.
use super::*;
use std::time::Instant;

const SOURCE_BYTES: [usize; 2] = [50 * 1024, 1024 * 1024];
const WARMUP: usize = 4;
const SAMPLES: usize = 32;
const MARKED: &str = "界😀\nβ";
const PARTIAL: &str = "界終\nβ";

fn callback(measure: bool, operation: impl FnOnce()) -> Option<u128> {
    let started = measure.then(Instant::now);
    operation();
    started.map(|started| started.elapsed().as_nanos())
}

#[gpui::test]
#[ignore = "explicit Linux callback probe; timings require LENZ_COMPOSITION_CALLBACK_TIMINGS=1 and a quiet campaign"]
fn composition_callback_sync_cost_probe(cx: &mut TestAppContext) {
    let measure = std::env::var_os("LENZ_COMPOSITION_CALLBACK_TIMINGS").is_some_and(|value| value == "1");
    let iterations = if measure { WARMUP + SAMPLES } else { 1 };
    let mut cases = Vec::new();
    let mut fixtures = Vec::new(); // Keep earlier files alive: deletion/watch work must not spill into the next case.
    for source_bytes in SOURCE_BYTES {
        let suffix = "😀tail\r\n";
        let pattern = "aé😀e\u{301}\t\x07 ";
        let prefix_bytes = source_bytes - suffix.len();
        let prefix = pattern.repeat(prefix_bytes / pattern.len()) + &"a".repeat(prefix_bytes % pattern.len());
        let original = prefix.clone() + suffix;
        assert_eq!(original.len(), source_bytes);
        let from = prefix.len();
        let units = prefix.encode_utf16().count();
        let f = fixture(cx, Some(("callback-probe.txt", original.as_bytes())));
        settle(cx, &f, |reader| reader.session.state().current_source().is_some());
        let mut visual = visual(cx, &f);
        draw(&mut visual);
        f.view.update(&mut visual, |reader, cx| {
            reader.enable_draft_input_for_test();
            reader.dispatch(Command::SetSoftWrap(false), cx);
            reader.dispatch(Command::Select(Selection { anchor: from, head: from + 4 }), cx);
        });
        draw(&mut visual);
        let base = f.view.read_with(&visual, |reader, _| reader.session.state().current_source().unwrap().clone());
        let selection = Some(Selection { anchor: from, head: from + 4 });
        assert_eq!(base.copy_range(0..base.len_bytes()).unwrap(), original);
        assert_eq!(base.byte_to_utf16(from), Some(units));
        let conversions = code_core::text::TextSnapshot::conversion_calls_on_thread_for_test();
        let mut input = ime_handler(&f, &mut visual);
        let mut samples = Vec::new();
        for iteration in 0..iterations {
            let mark = callback(measure, || input.replace_and_mark_text_in_range(None, MARKED, Some(0..1)));
            assert_eq!(input.marked_text_range(), Some(units..units + 5));
            f.view.read_with(&visual, |reader, _| {
                let state = reader.session.state();
                let tab = state.active().unwrap().id();
                let shown = state.presentation_view(tab).unwrap().snapshot();
                assert_ne!(shown.key(), base.key());
                assert_eq!(shown.len_bytes(), original.len() - 4 + MARKED.len());
                assert_eq!(shown.copy_range(from..from + MARKED.len()).as_deref(), Some(MARKED));
                assert_eq!(shown.copy_range(from + MARKED.len()..shown.len_bytes()).as_deref(), Some("tail\r\n"));
                assert!(Arc::ptr_eq(state.current_source().unwrap(), &base));
                assert_eq!(state.selection(), selection);
                assert!(!state.view(tab).unwrap().dirty() && !state.view(tab).unwrap().can_undo() && !state.view(tab).unwrap().can_redo());
            });
            let partial = callback(measure, || input.replace_and_mark_text_in_range(Some(units + 1..units + 3), "終", Some(0..1)));
            assert_eq!(input.marked_text_range(), Some(units..units + 4));
            f.view.read_with(&visual, |reader, _| {
                let state = reader.session.state();
                let shown = state.presentation_view(state.active().unwrap().id()).unwrap().snapshot();
                assert_eq!(shown.len_bytes(), original.len() - 4 + PARTIAL.len());
                assert_eq!(shown.copy_range(from..from + PARTIAL.len()).as_deref(), Some(PARTIAL));
                assert_eq!(shown.copy_range(from + PARTIAL.len()..shown.len_bytes()).as_deref(), Some("tail\r\n"));
                assert!(Arc::ptr_eq(state.current_source().unwrap(), &base));
            });
            // Linux's explicit marked-range cancellation; query and assertions are outside the clock.
            let marked = input.marked_text_range().unwrap();
            let cancel = callback(measure, || input.replace_text_in_range(Some(marked), ""));
            assert!(input.marked_text_range().is_none());
            f.view.read_with(&visual, |reader, _| {
                let state = reader.session.state(); let tab = state.active().unwrap().id();
                assert!(Arc::ptr_eq(state.current_source().unwrap(), &base));
                assert!(Arc::ptr_eq(state.presentation_view(tab).unwrap().snapshot(), &base));
                assert_eq!(state.selection(), selection);
                assert!(!state.view(tab).unwrap().dirty() && !state.view(tab).unwrap().can_undo() && !state.view(tab).unwrap().can_redo());
            });
            assert_eq!(code_core::text::TextSnapshot::conversion_calls_on_thread_for_test(), conversions);
            if measure {
                samples.push(serde_json::json!({
                    "iteration": iteration,
                    "population": if iteration < WARMUP { "warmup" } else { "sample" },
                    "set_marked_text_synchronous_callback_wall_ns": mark.unwrap(),
                    "partial_marked_update_synchronous_callback_wall_ns": partial.unwrap(),
                    "cancel_marked_text_synchronous_callback_wall_ns": cancel.unwrap(),
                }));
            } else { assert!(mark.is_none() && partial.is_none() && cancel.is_none()); }
        }
        assert_eq!(std::fs::read(f.dir.path().join("callback-probe.txt")).unwrap(), original.as_bytes());
        cases.push(serde_json::json!({
            "source_bytes": source_bytes, "source_utf16_units": base.len_utf16(), "source_storage_bytes": base.storage_bytes(),
            "target_byte": from, "target_utf16_unit": units, "validated_iterations": iterations, "soft_wrap": false,
            "preedit_utf8_bytes": MARKED.len(), "partial_preedit_utf8_bytes": PARTIAL.len(),
            "warmup_count": if measure { WARMUP } else { 0 }, "sample_count": if measure { SAMPLES } else { 0 },
            "ui_thread_document_conversion_calls": 0, "dirty": false, "undo": false, "redo": false, "disk_unchanged": true,
            "callback_samples": samples,
        }));
        fixtures.push(f);
    }
    println!("LENZ_COMPOSITION_CALLBACK_PROBE {}", serde_json::json!({
        "schema": 1, "mode": if measure { "measurement" } else { "validation" }, "candidate_only": true,
        "clock": if measure { Some("std::time::Instant") } else { None },
        "scope": "synchronous TestWindow PlatformInputHandler callback wall time; excludes setup, polling, assertions, paint, worker completion and native display",
        "os_thread_cpu_time_measured": false, "native_delivery_measured": false, "worker_completion_measured": false, "paint_measured": false,
        "cases": cases,
    }));
}
