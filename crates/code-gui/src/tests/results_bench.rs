//! Results panel cost with a large project search: completion, then a key and a window draw per step.
//! Run with `LENZ_PERF_JSON=1 cargo test --release -p code-gui --features test-support results_panel_perf -- --ignored --nocapture`.
use super::*;

fn percentile(samples: &mut [f64], p: f64) -> f64 {
    samples.sort_by(f64::total_cmp);
    samples[((samples.len() - 1) as f64 * p).round() as usize]
}

/// Up to a minute of wall time: the probe also runs on a loaded machine.
fn wait(cx: &mut TestAppContext, f: &Fixture, done: impl Fn(&ReaderView) -> bool) {
    let until = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while !f.view.read_with(cx, |v, _| done(v)) {
        assert!(std::time::Instant::now() < until, "did not settle");
        cx.run_until_parked();
        f.view.update(cx, |v, cx| v.poll(cx));
        std::thread::sleep(std::time::Duration::from_micros(200));
    }
}

#[gpui::test]
#[ignore = "release-mode performance probe; LENZ_PERF_JSON=1 prints its samples"]
fn results_panel_perf(cx: &mut TestAppContext) {
    let f = fixture(cx, None);
    let root = f.dir.path().join("proj");
    // 1,800 files three folders deep, five matching lines each: 9,000 matches, under the result limit.
    for a in 0..12 {
        for b in 0..10 {
            let dir = root.join(format!("pkg{a:02}/mod{b:02}/src"));
            std::fs::create_dir_all(&dir).unwrap();
            for c in 0..15 {
                let text: String = (0..40).map(|l| if l % 8 == 3 { format!("    let value_{l} = compute({l}); // needle\n") } else { format!("    other_{l}();\n") }).collect();
                std::fs::write(dir.join(format!("file{c:02}.rs")), text).unwrap();
            }
        }
    }
    f.view.update(cx, |v, cx| v.open_root(root.clone(), cx));
    wait(cx, &f, scanned);
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(1440.0), px(900.0)));
    draw(&mut v);
    drop(v);
    let mut search_ms = Vec::new();
    for _ in 0..5 {
        let started = std::time::Instant::now();
        f.view.update(cx, |v, cx| v.dispatch(Command::Search { query: "needle".into(), scope: SearchIn::Project }, cx));
        wait(cx, &f, |v| v.session().state().results().is_some_and(|r| r.done() && r.preview().is_some()));
        let mut v = visual(cx, &f);
        draw(&mut v);
        search_ms.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    let hits = f.view.read_with(cx, |v, _| v.session().state().results().unwrap().hits().len());
    assert_eq!(hits, 9_000);
    // Step through the list: each Down moves the selection; the window draws the panel and the preview.
    let mut v = visual(cx, &f);
    f.view.update(&mut v, |v, _| v.results_focus = true);
    let mut frame_ms = Vec::new();
    for _ in 0..400 {
        let started = std::time::Instant::now();
        v.simulate_keystrokes("down");
        draw(&mut v);
        frame_ms.push(started.elapsed().as_secs_f64() * 1000.0);
        f.view.update(&mut v, |r, cx| r.poll(cx));
    }
    let rss_kib = std::fs::read_to_string("/proc/self/status").ok()
        .and_then(|s| s.lines().find(|l| l.starts_with("VmRSS:")).and_then(|l| l.split_whitespace().nth(1)?.parse::<u64>().ok()));
    if std::env::var_os("LENZ_PERF_JSON").is_some() {
        println!("{}", serde_json::json!({
            "schema": 1, "probe": "gui_results_panel", "files": 1800, "hits": hits,
            "search_complete_ms": search_ms, "search_median_ms": percentile(&mut search_ms.clone(), 0.5),
            "down_frame_median_ms": percentile(&mut frame_ms.clone(), 0.5), "down_frame_p95_ms": percentile(&mut frame_ms.clone(), 0.95),
            "rss_kib": rss_kib,
        }));
    }
}
