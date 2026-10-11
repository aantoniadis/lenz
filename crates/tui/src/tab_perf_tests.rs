//! Independent-view lifecycle checks; all instrumentation is test-only.
use crate::{Screen, Screens, perf};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend};
use std::{
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

fn key(screens: &mut Screens, code: KeyCode) {
    assert!(screens.handle(Event::Key(KeyEvent::new(code, KeyModifiers::NONE))));
}

fn request(dir: &Path, left: &str, right: &str) -> lenz::cli::Request {
    lenz::cli::parse(&["--no-index".into(), left.into(), right.into()], dir).unwrap()
}

fn sample_rss() -> Option<f64> {
    #[cfg(target_os = "linux")]
    if let Ok(status) = std::fs::read_to_string("/proc/self/status")
        && let Some(kb) = status.lines().find_map(|line| {
            line.strip_prefix("VmRSS:")
                .and_then(|v| v.split_whitespace().next())
                .and_then(|v| v.parse::<f64>().ok())
        })
    {
        return Some(kb / 1024.0);
    }
    perf::rss_mib()
}

fn sample_peak(peak: &mut Option<f64>) {
    if let Some(rss) = sample_rss() {
        *peak = Some(peak.unwrap_or(rss).max(rss));
    }
}

fn settle(screens: &mut Screens, terminal: &mut Terminal<TestBackend>, peak: &mut Option<f64>) {
    let start = Instant::now();
    loop {
        if screens.poll() {
            terminal.draw(|frame| screens.render(frame)).unwrap();
        }
        sample_peak(peak);
        if !screens.busy() {
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "views did not settle"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    terminal.draw(|frame| screens.render(frame)).unwrap();
    sample_peak(peak);
}

#[test]
fn f8_activation_checks_an_inactive_merge_for_disk_changes() {
    let dir = tempfile::tempdir().unwrap();
    for (path, text) in [
        ("base", "base\n"),
        ("local", "left\n"),
        ("remote", "right\n"),
        ("result", "base\n"),
    ] {
        std::fs::write(dir.path().join(path), text).unwrap();
    }
    let req = lenz::cli::parse(
        &[
            "merge".into(),
            "base".into(),
            "local".into(),
            "remote".into(),
            "result".into(),
        ],
        dir.path(),
    )
    .unwrap();
    let mut screens = Screens::open(req, dir.path().into());
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    let mut peak = None;
    settle(&mut screens, &mut terminal, &mut peak);
    let Screen::Merge { controller, .. } = &mut screens.views[0] else {
        panic!("missing merge")
    };
    assert!(
        matches!(&controller.workspace.file, Some(lenz::conflict_session::FileView::Merge(m)) if m.ask.is_none())
    );
    controller.new_view = Some((request(dir.path(), "local", "remote"), dir.path().into()));
    settle(&mut screens, &mut terminal, &mut peak);
    assert_eq!(screens.active, 1);
    std::fs::write(dir.path().join("result"), "external replacement\n").unwrap();
    key(&mut screens, KeyCode::F(8));
    assert_eq!(screens.active, 0);
    let Screen::Merge { controller, .. } = &screens.views[0] else {
        panic!("missing merge")
    };
    assert!(
        matches!(&controller.workspace.file, Some(lenz::conflict_session::FileView::Merge(m)) if m.ask == Some(lenz::merge_session::Ask::ChangedOnDisk))
    );
}

#[test]
#[ignore = "release four4K independent views, F8 input/render, close/cancel and resident memory"]
fn terminal_perf_independent_views() {
    let dir = tempfile::tempdir().unwrap();
    // Sampled before the fixtures exist: libmalloc's large cache keeps a freed 64 MiB pixel buffer
    // charged to the process, which read as a 72 MiB baseline on macOS against 7 MiB on Linux.
    let initial_rss = sample_rss();
    // Use the existing image buffer's encoder, without a terminal image dependency.
    let mut pixels = lenz::image_session::Pixels::new(4096, 4096);
    for (path, rgba) in [
        ("old.png", [210, 32, 64, 255]),
        ("new.png", [40, 90, 230, 255]),
    ] {
        for pixel in pixels.pixels_mut() {
            pixel.0 = rgba;
        }
        pixels.save(dir.path().join(path)).unwrap();
    }
    drop(pixels);
    let mut peak = initial_rss;
    let mut screens = Screens::open(request(dir.path(), "old.png", "new.png"), dir.path().into());
    let mut terminal = Terminal::new(TestBackend::new(160, 50)).unwrap();
    let mut opens = Vec::new();
    let mut switches = Vec::new();
    let mut closes = Vec::new();
    let mut resident = Vec::new();
    let mut references = Vec::new();
    let started = Instant::now();
    settle(&mut screens, &mut terminal, &mut peak);
    opens.push(started.elapsed().as_secs_f64() * 1000.0);
    for count in 1..=4 {
        if count > 1 {
            let started = Instant::now();
            let Screen::Diff(app) = &mut screens.views[screens.active] else {
                panic!("missing diff")
            };
            app.new_view = Some((request(dir.path(), "old.png", "new.png"), dir.path().into()));
            settle(&mut screens, &mut terminal, &mut peak);
            opens.push(started.elapsed().as_secs_f64() * 1000.0);
        }
        assert_eq!(screens.views.len(), count);
        let Screen::Diff(app) = &screens.views[screens.active] else {
            panic!("missing diff")
        };
        let image = app.session.image_result().unwrap().unwrap();
        assert!(
            std::ptr::eq(image.as_ref(), app.visual.image().unwrap()),
            "Session and Visual must share one decode"
        );
        assert_eq!(image.diff.changed, 4096 * 4096);
        references.push((
            Arc::downgrade(&image),
            app.visual.cancellation_weak().unwrap(),
        ));
        drop(image);
        if let Some(rss) = sample_rss() {
            resident.push(rss);
        }
        if count >= 2 {
            for _ in 0..40 {
                let started = Instant::now();
                key(&mut screens, KeyCode::F(8));
                screens.poll();
                terminal.draw(|frame| screens.render(frame)).unwrap();
                switches.push(started.elapsed().as_secs_f64() * 1000.0);
            }
        }
        let text = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("Old: PNG"));
    }
    // Close real controllers in reverse order; an Arc clone in the test must not
    // hide whether the controller released its decode and cancellation owner.
    while screens.views.len() > 1 {
        while screens.active != screens.views.len() - 1 {
            key(&mut screens, KeyCode::F(8));
        }
        let index = screens.active;
        let started = Instant::now();
        key(&mut screens, KeyCode::Char('q'));
        screens.poll();
        settle(&mut screens, &mut terminal, &mut peak);
        closes.push(started.elapsed().as_secs_f64() * 1000.0);
        assert_eq!(
            references[index].0.strong_count(),
            0,
            "closed image retained"
        );
        assert_eq!(
            references[index].1.strong_count(),
            0,
            "closed visual job owner retained"
        );
    }
    let after_close = sample_rss();
    // Hold the reusable document worker so that these newly opened views are
    // definitely closed before their queued comparison loads can begin.
    let token = Arc::new(());
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let blocker = lenz::work::submit(Arc::downgrade(&token), move || {
        entered_tx.send(()).unwrap();
        release_rx.recv_timeout(Duration::from_secs(30)).unwrap();
    });
    entered_rx.recv_timeout(Duration::from_secs(30)).unwrap();
    let cancellation_started = Instant::now();
    for _ in 0..12 {
        let Screen::Diff(app) = &mut screens.views[0] else {
            panic!("missing diff")
        };
        app.new_view = Some((request(dir.path(), "old.png", "new.png"), dir.path().into()));
        screens.poll();
        assert_eq!(screens.views.len(), 2);
        assert!(screens.busy());
        key(&mut screens, KeyCode::Char('q'));
        screens.poll();
        assert_eq!(screens.views.len(), 1);
    }
    release_tx.send(()).unwrap();
    assert_eq!(
        blocker.recv_timeout(Duration::from_secs(30)).unwrap(),
        Some(())
    );
    let drained = lenz::work::submit(Arc::downgrade(&token), || ());
    assert_eq!(
        drained.recv_timeout(Duration::from_secs(30)).unwrap(),
        Some(())
    );
    settle(&mut screens, &mut terminal, &mut peak);
    let canceled_ms = cancellation_started.elapsed().as_secs_f64() * 1000.0;
    key(&mut screens, KeyCode::Char('q'));
    screens.poll();
    assert_eq!(screens.exit(), Some(0));
    drop(screens);
    assert_eq!(references[0].0.strong_count(), 0);
    assert_eq!(references[0].1.strong_count(), 0);
    std::thread::sleep(Duration::from_millis(100));
    let settled = sample_rss();
    perf::record("tui/tabs/open-4k-ready", "ms", &opens);
    perf::record("tui/tabs/f8-input-render", "ms", &switches);
    perf::record("tui/tabs/close-ready", "ms", &closes);
    perf::record("tui/tabs/queued-open-close12", "ms", &[canceled_ms]);
    perf::record("tui/tabs/resident-1-through4", "MiB", &resident);
    for (name, value) in [
        ("initial", initial_rss),
        ("sampled-peak", peak),
        ("one-view-after-close", after_close),
        ("all-views-dropped-settled", settled),
    ] {
        if let Some(value) = value {
            perf::record(&format!("tui/tabs/{name}"), "MiB", &[value]);
        }
    }
}
