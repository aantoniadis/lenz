use super::*;

#[gpui::test]
fn gui_tab_wheel_scrolls_only_the_hovered_strip_without_switching_files(cx: &mut TestAppContext) {
    let f = fixture(cx, Some(("original.txt", b"one\ntwo\nthree\n")));
    settle(cx, &f, active_is("original.txt"));
    f.view.update(cx, |r, cx| r.dispatch(Command::Pin(r.session.state().active().unwrap().id()), cx));
    for i in 0..12 {
        let name = format!("long-file-name-{i:02}.txt"); let path = f.dir.path().join(&name);
        std::fs::write(&path, b"other\n").unwrap();
        f.view.update(cx, |r, cx| r.dispatch(Command::OpenRetained(path), cx));
        settle(cx, &f, |r| r.session.state().active().is_some_and(|t| t.title() == name) && r.session.state().loading().is_none());
    }
    f.view.update(cx, |r, cx| r.dispatch(Command::Split(code_core::session::Split::Columns), cx));
    settle(cx, &f, |r| r.session.state().shown(1).is_some_and(|t| t.loaded()));
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(900.0), px(500.0))); draw(&mut v);
    let (position, shown, offsets) = f.view.read_with(&v, |r, _| {
        let g = r.panes.iter().find(|g| g.pane == 0).unwrap();
        (point(px((g.bar.0 + g.bar.1) / 2.0), px(g.bar.2 + line_h() / 2.0)), [r.session.state().shown(0).unwrap().id(), r.session.state().shown(1).unwrap().id()], r.tab_scroll)
    });
    assert!(offsets[0] > 0.0);
    v.simulate_event(ScrollEvent { position, delta: ScrollDelta::Lines(point(0.0, 1.0)), ..Default::default() }); draw(&mut v);
    let after = f.view.read_with(&v, |r, _| r.tab_scroll);
    assert!(after[0] < offsets[0]); assert_eq!(after[1], offsets[1]);
    for _ in 0..3 { v.simulate_event(ScrollEvent { position, delta: ScrollDelta::Pixels(point(px(-2.0), px(0.0))), ..Default::default() }); }
    draw(&mut v);
    assert_eq!(f.view.read_with(&v, |r, _| r.tab_scroll[0]), after[0] + 6.0, "small horizontal trackpad input is retained");
    assert!(f.view.read_with(&v, |r, _| r.session.state().focused_pane() == 1 && [r.session.state().shown(0).unwrap().id(), r.session.state().shown(1).unwrap().id()] == shown));
    assert!(f.view.read_with(&v, |r, _| r.session.state().tabs().iter().all(|t| t.top_line() == 0 && t.column() == 0)));
    v.simulate_keystrokes("f10");
    let before = f.view.read_with(&v, |r, _| r.tab_scroll);
    v.simulate_event(ScrollEvent { position, delta: ScrollDelta::Lines(point(0.0, -1.0)), ..Default::default() });
    assert_eq!(f.view.read_with(&v, |r, _| r.tab_scroll), before, "the modal owns wheel input");
    v.simulate_keystrokes("escape");
    let first = f.view.read_with(&v, |r, _| r.session.state().pane_tabs(0).next().unwrap().id());
    f.view.update(&mut v, |r, cx| r.dispatch(Command::ActivateTab(first), cx));
    drop(v);
    settle(cx, &f, |r| r.session.state().presentation_shown(0).is_some_and(|t| t.id() == first));
    let mut v = visual(cx, &f); draw(&mut v);
    assert_eq!(f.view.read_with(&v, |r, _| r.tab_scroll[0]), 0.0, "keyboard/programmatic activation reveals the selected tab");
    v.simulate_resize(size(px(6000.0), px(500.0))); draw(&mut v);
    assert_eq!(f.view.read_with(&v, |r, _| r.tab_scroll), [0.0; 2], "resizing clamps manual offsets");
}
