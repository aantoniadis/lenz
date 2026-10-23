use std::{path::Path, sync::Arc, time::{Duration, Instant}};
use reader_core::{projection::Request, session::{Command, Config, Session}};

fn session(path: &Path) -> Session {
    Session::new(Config {
        themes: lenz_content::theme::ThemePaths::under(&path.join("config"), &path.join("state"), "lenz-reader"),
        recent: path.join("state/recent.json"), settings: path.join("state/settings.json"),
        providers: path.join("config/providers.json"), cache: path.join("cache"),
        idle: reader_core::semantics::IDLE, clock: Arc::new(Instant::now),
        overrides: Default::default(), dark: true,
    })
}

fn wait(session: &mut Session, done: impl Fn(&Session) -> bool) -> bool {
    let rx = session.completions();
    let until = Instant::now() + Duration::from_secs(2);
    while !done(session) && Instant::now() < until {
        while let Ok(completion) = rx.try_recv() { session.dispatch(completion); }
        std::thread::sleep(Duration::from_millis(1));
    }
    done(session)
}

#[test]
fn session_delivers_bounded_projection_batches_and_cancels_obsolete_requests() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("long.txt");
    std::fs::write(&path, "x".repeat(120_000)).unwrap();
    let mut s = session(d.path());
    s.dispatch(Command::Open(path.clone()));
    assert!(wait(&mut s, |s| s.state().active().is_some_and(|t| t.loaded())));
    let tab = s.state().active().unwrap().id();
    let cursor = s.state().view(tab).unwrap().display_start(0).unwrap();
    for request in 1..=100 {
        s.dispatch(Command::PrepareProjection(Request { pane: 0, tab, cursor: cursor.clone(), request, context: 77 }));
    }
    assert!(wait(&mut s, |s| s.projection(0).is_some_and(|b| b.request.request == 100)));
    let batch = s.projection(0).unwrap();
    assert_eq!(batch.request.context, 77);
    assert!(!batch.failed && !batch.complete);
    assert_eq!(batch.windows.len(), 16);
    assert!(batch.windows.iter().all(|w| w.work <= 4096));
    assert!(batch.next.source_byte() > 0 && batch.next.source_byte() < 120_000);
    let next = batch.next.clone();
    s.dispatch(Command::PrepareProjection(Request { pane: 0, tab, cursor: next, request: 101, context: 77 }));
    s.dispatch(Command::CancelProjection { pane: 0, request: 101 });
    assert!(s.projection(0).is_none());
    s.dispatch(Command::Close);
    let rx = s.completions();
    for _ in 0..100 {
        while let Ok(completion) = rx.try_recv() { s.dispatch(completion); }
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(s.projection(0).is_none());
    assert_eq!(std::fs::read(path).unwrap(), vec![b'x'; 120_000]);
}
