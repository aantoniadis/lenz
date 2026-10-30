use std::{sync::Arc, time::{Duration, Instant}};
use code_core::{cache::Stamp, document::Document, session::{Command, Completion, Config, SearchIn, Session}};

fn wait(s: &mut Session, done: impl Fn(&Session) -> bool) {
    let until = Instant::now() + Duration::from_secs(3);
    while !done(s) {
        assert!(Instant::now() < until, "current-source workflow timed out");
        while let Ok(completion) = s.completions().try_recv() { s.dispatch(completion); }
        std::thread::yield_now();
    }
}

#[test]
fn an_immutable_result_preview_cannot_replace_a_loaded_current_source() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("source.txt");
    std::fs::write(&path, b"saved source\n").unwrap();
    let mut s = Session::new(Config {
        themes: lenz_content::theme::ThemePaths::under(&d.path().join("config"), &d.path().join("state"), "lenz-code"),
        recent: d.path().join("recent.json"), settings: d.path().join("settings.json"),
        providers: d.path().join("providers.json"), cache: d.path().join("cache"),
        idle: Duration::ZERO, clock: Arc::new(Instant::now), overrides: Default::default(), dark: true,
    });
    s.dispatch(Command::Open(path.clone()));
    wait(&mut s, |s| s.state().current_source().is_some());
    let current = s.state().current_source().unwrap().clone();
    s.dispatch(Command::Search { query: "saved".into(), scope: SearchIn::ActiveFile });
    wait(&mut s, |s| s.state().results().is_some_and(|r| r.done()));
    let id = s.state().results().unwrap().id();
    s.dispatch(Command::SelectResult(0));
    let preview = Arc::new(Document::decode(path.clone(), 100_000, b"saved result snapshot\n"));
    // Deliver the immutable filesystem preview result at its existing worker boundary.
    s.dispatch(Completion::Previewed { id, index: 0, doc: Ok((Stamp::of(&path).unwrap(), preview.clone())) });
    assert!(Arc::ptr_eq(s.state().current_source().unwrap(), &current), "a result/cache admission is not a current-source reload");
    assert!(Arc::ptr_eq(&s.state().results().unwrap().preview().unwrap().doc, &preview));
    s.dispatch(Command::SelectAll);
    assert_eq!(s.dispatch(Command::Copy).copy.as_deref(), Some("saved source\n"));
    assert_eq!(std::fs::read(path).unwrap(), b"saved source\n");
}
