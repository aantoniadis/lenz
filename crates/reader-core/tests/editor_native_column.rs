use std::{path::Path, sync::Arc, time::{Duration, Instant}};
use reader_core::session::{Command, Config, Session};

fn session(path: &Path) -> Session {
    Session::new(Config {
        themes: lenz_content::theme::ThemePaths::under(&path.join("config"), &path.join("state"), "lenz-reader"),
        recent: path.join("recent.json"), settings: path.join("settings.json"),
        providers: path.join("providers.json"), cache: path.join("cache"),
        idle: Duration::ZERO, clock: Arc::new(Instant::now),
        overrides: Default::default(), dark: true,
    })
}

fn wait(s: &mut Session, done: impl Fn(&Session) -> bool) {
    let until = Instant::now() + Duration::from_secs(2);
    while !done(s) {
        assert!(Instant::now() < until, "session work timed out");
        while let Ok(completion) = s.completions().try_recv() { s.dispatch(completion); }
        std::thread::yield_now();
    }
}

#[test]
fn native_column_clamp_rejects_obsolete_source_pane_and_width_and_never_scrolls_right() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("source.txt");
    std::fs::write(&path, "x".repeat(1_000)).unwrap();
    let mut s = session(d.path());
    s.dispatch(Command::Open(path.clone()));
    wait(&mut s, |s| s.state().active().is_some_and(|tab| tab.loaded()));
    s.dispatch(Command::SetSoftWrap(false));
    s.dispatch(Command::SetTextSize { pane: 0, cols: 80, rows: 20 });
    s.dispatch(Command::ScrollColumn(500));
    let tab = s.state().active().unwrap().id();
    let viewport = s.native_viewport(0).unwrap();
    s.dispatch(Command::SetTextSize { pane: 0, cols: 79, rows: 20 });
    s.dispatch(Command::ClampColumn { viewport: viewport.clone(), max: 10 });
    assert_eq!(s.state().column(), 500);
    s.dispatch(Command::SetTextSize { pane: 0, cols: 80, rows: 20 });
    let viewport = s.native_viewport(0).unwrap();
    s.dispatch(Command::ClampColumn { viewport: viewport.clone(), max: 300 });
    assert_eq!(s.state().column(), 300);
    s.dispatch(Command::ScrollColumn(500));
    s.dispatch(Command::ClampColumn { viewport, max: 0 });
    assert_eq!(s.state().column(), 500, "old measured origin cannot undo a pan");
    let viewport = s.native_viewport(0).unwrap();
    s.dispatch(Command::SetNativeContext { pane: 0, context: 3 });
    s.dispatch(Command::ClampColumn { viewport, max: 0 });
    assert_eq!(s.state().column(), 500);
    let viewport = s.native_viewport(0).unwrap();
    let old_key = s.state().view(tab).unwrap().snapshot().key();
    std::fs::write(&path, "changed".repeat(1_000)).unwrap();
    s.dispatch(Command::Reload);
    wait(&mut s, |s| s.state().view(tab).is_some_and(|view| view.snapshot().key() != old_key));
    s.dispatch(Command::ScrollColumn(200));
    s.dispatch(Command::ClampColumn { viewport, max: 0 });
    assert_eq!(s.state().column(), 200);
    assert_eq!(std::fs::read_to_string(path).unwrap(), "changed".repeat(1_000));
}
