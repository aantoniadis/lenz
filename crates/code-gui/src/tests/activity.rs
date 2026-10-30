use super::*;

#[gpui::test]
fn gui_activity_log_is_collapsed_scrollable_and_never_edits_the_document(cx: &mut TestAppContext) {
    let f = fixture(cx, Some(("keep.txt", b"keep\n")));
    settle(cx, &f, active_is("keep.txt"));
    f.view.update(cx, |r, cx| {
        r.enable_draft_input_for_test();
        for i in 0..80 { r.dispatch(Command::Notice(format!("Warning {i}: connection unavailable")), cx); }
    });
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(900.0), px(500.0))); draw(&mut v);
    assert!(f.view.read_with(&v, |r, _| !r.session.state().activity_log_shown()));
    let at = f.view.read_with(&v, |r, _| r.activity_button().unwrap().0.center());
    v.simulate_click(at, Modifiers::default()); draw(&mut v);
    assert!(f.view.read_with(&v, |r, _| r.session.state().activity_log_shown()));
    v.simulate_keystrokes("home up x delete ctrl-c");
    let copied = v.read(|cx| cx.read_from_clipboard().and_then(|item| item.text())).unwrap();
    assert!(copied.contains("Warning 0") && copied.contains("Warning 79"));
    assert!(f.view.read_with(&v, |r, _| r.log_scroll > 0));
    for theme in ["darcula", "github-light"] {
        f.view.update(&mut v, |r, cx| r.dispatch(Command::Theme(ThemeCommand::Select(ThemeSelection::from_id(theme).unwrap())), cx));
        for (w, h) in [(480.0, 300.0), (180.0, 140.0), (1.0, 1.0), (900.0, 500.0)] {
            v.simulate_resize(size(px(w), px(h))); draw(&mut v);
            f.view.update(&mut v, |r, _| { if let Some(l) = r.activity_layout() { assert!(l.bounds.bottom() <= px(h)); assert!(l.start <= l.rows.len()); if h >= 140.0 { assert!(l.visible >= 1); } } });
        }
    }
    let clear = f.view.update(&mut v, |r, _| r.activity_layout().unwrap().buttons.iter().find(|(_, id, _)| *id == CommandId::ClearActivityLog).unwrap().0.center());
    v.simulate_click(clear, Modifiers::default()); draw(&mut v);
    assert!(f.view.read_with(&v, |r, _| r.session.state().activity_log().is_empty()));
    v.simulate_keystrokes("escape");
    assert!(f.view.read_with(&v, |r, _| !r.session.state().activity_log_shown()));
    let text = f.view.read_with(&v, |r, _| { let s = r.session.state().current_source().unwrap(); s.copy_range(0..s.len_bytes()).unwrap() });
    assert_eq!(text, "keep\n");
    assert_eq!(std::fs::read(f.dir.path().join("keep.txt")).unwrap(), b"keep\n");
}
