use super::*;

#[test]
fn xml_input_highlights_folds_and_shared_diagnostics_in_the_terminal() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("built-in.xml");
    let original = "<Root>\n  <ns:Child xmlns:ns='urn:test'/>\n</Root>";
    std::fs::write(&path, original).unwrap();
    let mut a = App::new(config(dir.path()), Some(path.clone()));
    a.enable_draft_input_for_test();
    settle(&mut a, |a| a.session.state().highlights().is_some() && a.session.state().active().is_some_and(|t| a.session.state().diagnostics(t.id()).is_some()));
    for theme in ["darcula", "github-light"] {
        a.dispatch(Command::Theme(lenz_content::theme::ThemeCommand::Select(lenz_content::theme::ThemeSelection::from_id(theme).unwrap())));
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 20)).unwrap();
        terminal.draw(|frame| a.draw(frame)).unwrap();
        let hit = a.layout.panes[0].hits.iter().find(|hit| hit.line == 0 && hit.byte == 1).unwrap();
        assert_eq!(terminal.backend().buffer()[(hit.x, hit.y)].fg, crate::render::color(a.session.state().palette().syntax_color(lenz_content::highlight::Capture::Tag)));
        assert!(a.session.state().diagnostics(a.session.state().active().unwrap().id()).unwrap().items.is_empty());
    }
    key(&mut a, KeyCode::F(10), KeyModifiers::NONE);
    let row = a.choices().iter().position(|row| matches!(row.target, ChoiceTarget::Action(CommandId::Collapse))).unwrap();
    for _ in 0..row { key(&mut a, KeyCode::Down, KeyModifiers::NONE); }
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    let screen = draw_screen(&mut a, 100, 20);
    assert!(!screen.contains("ns:Child"), "the XML child line folds away: {screen}");
    a.dispatch(Command::Expand);
    assert!(draw_screen(&mut a, 100, 20).contains("ns:Child"));
    a.dispatch(Command::Select(Selection { anchor: 6, head: 6 }));
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    let text = |a: &App| { let s = a.session.state().current_source().unwrap(); s.copy_range(0..s.len_bytes()).unwrap() };
    assert_eq!(text(&a), "<Root>\n    \n  <ns:Child xmlns:ns='urn:test'/>\n</Root>");
    key(&mut a, KeyCode::Char('z'), KeyModifiers::CONTROL);
    assert_eq!(text(&a), original);
    a.dispatch(Command::Select(Selection { anchor: 0, head: 0 }));
    key(&mut a, KeyCode::Char('/'), KeyModifiers::ALT);
    assert!(text(&a).starts_with("<!-- <Root> -->"));
    key(&mut a, KeyCode::Char('z'), KeyModifiers::CONTROL);
    assert_eq!(text(&a), original);
    for (w, h) in [(28, 6), (1, 1), (100, 20)] { draw(&mut a, w, h); }
    assert_eq!(std::fs::read_to_string(path).unwrap(), original);
}
