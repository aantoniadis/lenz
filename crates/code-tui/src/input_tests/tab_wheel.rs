use super::*;

#[test]
fn tab_wheel_scrolls_the_hovered_strip_without_switching_files() {
    let dir = tempfile::tempdir().unwrap();
    let mut a = app(dir.path());
    for i in 0..12 {
        let file = format!("long-file-name-{i:02}.txt");
        let path = dir.path().join(&file); std::fs::write(&path, "text\n".repeat(100)).unwrap();
        a.dispatch(Command::OpenRetained(path));
        settle(&mut a, |a| a.session.state().current_source().is_some_and(|s| s.source().ends_with(&file)));
    }
    a.dispatch(Command::Split(code_core::session::Split::Columns));
    settle(&mut a, |a| a.session.state().shown(1).is_some_and(|t| t.loaded()));
    draw(&mut a, 120, 30);
    let shown = [a.session.state().shown(0).unwrap().id(), a.session.state().shown(1).unwrap().id()];
    let (_, rect) = a.layout.panes.iter().find(|p| p.pane == 0).unwrap().tabs[0];
    let before = a.tab_strips.get()[0].first; assert!(before > 0);
    a.handle(Event::Mouse(MouseEvent { kind: MouseEventKind::ScrollUp, column: rect.x, row: rect.y, modifiers: KeyModifiers::NONE }));
    draw(&mut a, 120, 30);
    assert_eq!(a.tab_strips.get()[0].first, before - 1);
    a.handle(Event::Mouse(MouseEvent { kind: MouseEventKind::ScrollRight, column: rect.x, row: rect.y, modifiers: KeyModifiers::NONE }));
    draw(&mut a, 120, 30);
    assert_eq!(a.tab_strips.get()[0].first, before);
    assert_eq!(a.session.state().focused_pane(), 1);
    assert_eq!([a.session.state().shown(0).unwrap().id(), a.session.state().shown(1).unwrap().id()], shown);
    assert!(a.session.state().tabs().iter().all(|t| t.top_line() == 0 && t.column() == 0));
    key(&mut a, KeyCode::F(10), KeyModifiers::NONE);
    a.handle(Event::Mouse(MouseEvent { kind: MouseEventKind::ScrollUp, column: rect.x, row: rect.y, modifiers: KeyModifiers::NONE }));
    assert_eq!(a.tab_strips.get()[0].first, before, "modal blocks tab scrolling");
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    let first = a.session.state().pane_tabs(0).next().unwrap().id(); a.dispatch(Command::ActivateTab(first));
    draw(&mut a, 120, 30); assert_eq!(a.tab_strips.get()[0].first, 0);
    draw(&mut a, 1600, 30); assert_eq!(a.tab_strips.get()[0].first, 0);
    for (w,h) in [(1,1),(0,0),(24,8),(120,30)] { draw(&mut a,w,h); }
}
