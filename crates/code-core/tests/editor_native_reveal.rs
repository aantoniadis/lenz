use std::{sync::Arc, time::{Duration, Instant}};
use code_core::session::{Command, Config, Session};

#[test]
fn native_reveal_feedback_is_versioned_context_bound_and_does_not_replay_after_user_pan() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("source.txt");
    std::fs::write(&path, "x".repeat(20_000) + " END" + &" ".repeat(100)).unwrap();
    let mut s = Session::new(Config {
        themes: lenz_content::theme::ThemePaths::under(&d.path().join("config"), &d.path().join("state"), "lenz-code"),
        recent: d.path().join("recent.json"), settings: d.path().join("settings.json"), providers: d.path().join("providers.json"),
        cache: d.path().join("cache"), idle: Duration::ZERO, clock: Arc::new(Instant::now), overrides: Default::default(), dark: true,
    });
    s.dispatch(Command::Open(path));
    let until = Instant::now() + Duration::from_secs(2);
    while !s.state().active().is_some_and(|tab| tab.loaded()) {
        assert!(Instant::now() < until);
        while let Ok(done) = s.completions().try_recv() { s.dispatch(done); }
    }
    s.dispatch(Command::SetSoftWrap(false));
    s.dispatch(Command::SetTextSize { pane: 0, cols: 40, rows: 10 });
    s.dispatch(Command::SetNativeContext { pane: 0, context: 7 });
    s.dispatch(Command::GoToLine(lenz_content::goto::LineColumn { line: 1, column: Some(20_002) }));
    let first = s.native_reveal(0).unwrap().clone();
    assert_eq!(first.source, 20_001);
    assert_eq!(s.state().column(), 0, "logical columns must not guess native origins");
    s.dispatch(Command::SetNativeContext { pane: 0, context: 8 });
    let second = s.native_reveal(0).unwrap().clone();
    assert!(second.request > first.request);
    s.dispatch(Command::MeasuredReveal { goal: first, cols: 40, bounds: 20_001..20_002 });
    assert_eq!(s.state().column(), 0);
    s.dispatch(Command::SetTextSize { pane: 0, cols: 30, rows: 10 });
    let third = s.native_reveal(0).unwrap().clone();
    assert!(third.request > second.request);
    s.dispatch(Command::MeasuredReveal { goal: second, cols: 40, bounds: 20_001..20_002 });
    assert_eq!(s.state().column(), 0);
    s.dispatch(Command::MeasuredReveal { goal: third.clone(), cols: 30, bounds: 20_001..20_002 });
    assert_eq!(s.state().column(), 19_975);
    assert!(s.native_reveal(0).is_none());
    s.dispatch(Command::ScrollColumn(0));
    s.dispatch(Command::MeasuredReveal { goal: third, cols: 30, bounds: 20_001..20_002 });
    assert_eq!(s.state().column(), 0);
    s.dispatch(Command::GoToLine(lenz_content::goto::LineColumn { line: 1, column: Some(20_002) }));
    let superseded = s.native_reveal(0).unwrap().clone();
    s.dispatch(Command::ScrollColumn(0));
    s.dispatch(Command::MeasuredReveal { goal: superseded, cols: 30, bounds: 20_001..20_002 });
    assert_eq!(s.state().column(), 0, "even a no-op user pan supersedes pending reveal");
    s.dispatch(Command::SetSoftWrap(true));
    s.dispatch(Command::SetTextSize { pane: 0, cols: 40, rows: 10 });
    s.dispatch(Command::GoToLine(lenz_content::goto::LineColumn { line: 1, column: Some(20_002) }));
    s.dispatch(Command::SetTextSize { pane: 0, cols: 20, rows: 10 });
    let until = Instant::now() + Duration::from_secs(2);
    while s.native_reveal(0).is_none() {
        assert!(Instant::now() < until, "resized cold logical seek must resume");
        while let Ok(done) = s.completions().try_recv() { s.dispatch(done); }
        std::thread::yield_now();
    }
    let tab = s.state().active().unwrap().id();
    let view = s.state().view(tab).unwrap();
    let code_core::layout::Query::Ready(Some(position)) = view.layout_position(20_001, Some(20)) else { panic!("completed caret row") };
    assert!(view.layout_rows(10, Some(20)).rows.iter().any(|row| row.index == position.row));

}
