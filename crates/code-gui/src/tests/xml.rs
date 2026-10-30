use super::*;

#[gpui::test]
fn gui_xml_input_highlights_folds_and_shared_diagnostics(cx: &mut TestAppContext) {
    let original = "<Root>\n  <ns:Child xmlns:ns='urn:test'/>\n</Root>";
    let f = fixture(cx, Some(("built-in.xml", original.as_bytes())));
    settle(cx, &f, |r| r.session.state().highlights().is_some() && r.session.state().active().is_some_and(|t| r.session.state().diagnostics(t.id()).is_some()));
    let mut v = visual(cx, &f);
    f.view.update(&mut v, |r, _| r.enable_draft_input_for_test());
    for theme in ["darcula", "github-light"] {
        f.view.update(&mut v, |r, cx| r.dispatch(Command::Theme(lenz_content::theme::ThemeCommand::Select(lenz_content::theme::ThemeSelection::from_id(theme).unwrap())), cx));
        draw(&mut v);
        f.view.read_with(&v, |r, _| {
            let rows = r.visible_rows();
            let root = rows.iter().find(|row| row.number == 1).unwrap();
            let colour = root.fg.iter().find(|(range, _)| range.contains(&1)).map(|(_, colour)| *colour);
            assert_eq!(colour, Some(r.session.state().palette().syntax_color(lenz_content::highlight::Capture::Tag)));
            assert!(r.session.state().diagnostics(r.session.state().active().unwrap().id()).unwrap().items.is_empty());
        });
    }
    choose_menu_command(&mut v, &f, CommandId::Collapse);
    draw(&mut v);
    assert!(f.view.read_with(&v, |r, _| r.visible_rows().iter().all(|row| row.number != 2)), "the XML child line folds away");
    choose_menu_command(&mut v, &f, CommandId::Expand);
    draw(&mut v);
    assert!(f.view.read_with(&v, |r, _| r.visible_rows().iter().any(|row| row.number == 2)));
    f.view.update(&mut v, |r, cx| r.dispatch(Command::Select(Selection { anchor: 6, head: 6 }), cx));
    v.simulate_keystrokes("enter");
    let text = |v: &VisualTestContext| f.view.read_with(v, |r, _| { let s = r.session.state().current_source().unwrap(); s.copy_range(0..s.len_bytes()).unwrap() });
    assert_eq!(text(&v), "<Root>\n    \n  <ns:Child xmlns:ns='urn:test'/>\n</Root>");
    v.simulate_keystrokes("ctrl-z");
    assert_eq!(text(&v), original);
    f.view.update(&mut v, |r, cx| r.dispatch(Command::Select(Selection { anchor: 0, head: 0 }), cx));
    v.simulate_keystrokes("alt-/");
    assert!(text(&v).starts_with("<!-- <Root> -->"));
    v.simulate_keystrokes("ctrl-z");
    assert_eq!(text(&v), original);
    for (w, h) in [(180.0, 100.0), (1.0, 1.0), (800.0, 400.0)] { v.simulate_resize(size(px(w), px(h))); draw(&mut v); }
    assert_eq!(std::fs::read_to_string(f.dir.path().join("built-in.xml")).unwrap(), original);
}
