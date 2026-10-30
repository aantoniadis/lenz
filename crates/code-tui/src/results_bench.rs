//! Results panel cost with a large project search: completion and per-frame drawing while stepping through rows.
//! Run with `LENZ_PERF_JSON=1 cargo test --release -p code-tui results_panel_perf -- --ignored --nocapture`.
use crate::app::{App, Focus};
use code_core::session::{Command, Config, SearchIn};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use lenz_content::theme::{ThemeOverrides, ThemePaths};
use std::{path::Path, sync::Arc, time::{Duration, Instant}};

/// 1,800 files three folders deep, five matching lines each: 9,000 matches, under the 10,000 result limit.
pub(crate) fn results_fixture(root: &Path) -> usize {
    let mut matches = 0;
    for a in 0..12 {
        for b in 0..10 {
            let dir = root.join(format!("pkg{a:02}/mod{b:02}/src"));
            std::fs::create_dir_all(&dir).unwrap();
            for c in 0..15 {
                let text: String = (0..40).map(|l| if l % 8 == 3 { format!("    let value_{l} = compute({l}); // needle\n") } else { format!("    other_{l}();\n") }).collect();
                std::fs::write(dir.join(format!("file{c:02}.rs")), text).unwrap();
                matches += 5;
            }
        }
    }
    matches
}

fn percentile(samples: &mut [f64], p: f64) -> f64 {
    samples.sort_by(f64::total_cmp);
    samples[((samples.len() - 1) as f64 * p).round() as usize]
}

#[test]
#[ignore = "release-mode performance probe; LENZ_PERF_JSON=1 prints its samples"]
fn results_panel_perf() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("proj");
    let expected = results_fixture(&root);
    let config = Config {
        themes: ThemePaths::under(&dir.path().join("config"), &dir.path().join("state"), "lenz-code"),
        recent: dir.path().join("state/recent.json"),
        settings: dir.path().join("config/settings.json"),
        providers: dir.path().join("config/providers.json"),
        cache: dir.path().join("cache"),
        idle: Duration::ZERO,
        clock: Arc::new(Instant::now),
        overrides: ThemeOverrides::default(),
        dark: true,
    };
    let mut app = App::new(config, None);
    let settle = |app: &mut App, done: &dyn Fn(&App) -> bool| {
        let until = Instant::now() + Duration::from_secs(60);
        while !done(app) {
            assert!(Instant::now() < until, "did not settle");
            match app.session.completions().try_recv() {
                Ok(c) => { app.apply(c); }
                Err(_) => std::thread::sleep(Duration::from_micros(200)),
            }
        }
    };
    app.dispatch(Command::OpenRoot(root.clone()));
    settle(&mut app, &|a| a.session.state().project().is_some_and(|p| matches!(p.coverage(), code_core::project::Coverage::Complete(_))));
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(160, 48)).unwrap();
    let (mut search_ms, mut done_ms, mut preview_ms) = (Vec::new(), Vec::new(), Vec::new());
    for _ in 0..5 {
        let started = Instant::now();
        app.dispatch(Command::Search { query: "needle".into(), scope: SearchIn::Project });
        settle(&mut app, &|a| a.session.state().results().is_some_and(|r| r.done()));
        done_ms.push(started.elapsed().as_secs_f64() * 1000.0);
        settle(&mut app, &|a| a.session.state().results().is_some_and(|r| r.preview().is_some()));
        preview_ms.push(started.elapsed().as_secs_f64() * 1000.0);
        terminal.draw(|f| app.draw(f)).unwrap();
        search_ms.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    let hits = app.session.state().results().unwrap().hits().len();
    assert_eq!(hits, expected);
    // Step through the list: each Down moves the selection and repaints the panel and its preview.
    app.focus = Focus::Results;
    let mut frame_ms = Vec::new();
    for _ in 0..400 {
        let started = Instant::now();
        app.handle(Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)));
        terminal.draw(|f| app.draw(f)).unwrap();
        frame_ms.push(started.elapsed().as_secs_f64() * 1000.0);
        // The preview of the new selection is read off the UI thread; let it land as the app would.
        while let Ok(c) = app.session.completions().try_recv() { app.apply(c); }
    }
    let rss_kib = std::fs::read_to_string("/proc/self/status").ok()
        .and_then(|s| s.lines().find(|l| l.starts_with("VmRSS:")).and_then(|l| l.split_whitespace().nth(1)?.parse::<u64>().ok()));
    if std::env::var_os("LENZ_PERF_JSON").is_some() {
        println!("{}", serde_json::json!({
            "schema": 1, "probe": "tui_results_panel", "files": expected / 5, "hits": hits,
            "search_complete_ms": search_ms, "search_median_ms": percentile(&mut search_ms.clone(), 0.5),
            "done_median_ms": percentile(&mut done_ms.clone(), 0.5), "preview_median_ms": percentile(&mut preview_ms.clone(), 0.5),
            "down_frame_median_ms": percentile(&mut frame_ms.clone(), 0.5), "down_frame_p95_ms": percentile(&mut frame_ms.clone(), 0.95),
            "rss_kib": rss_kib,
        }));
    }
}
