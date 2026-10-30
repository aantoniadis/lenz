use super::*;

#[test]
fn activity_log_is_collapsed_scrollable_and_never_edits_the_document() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("keep.txt");
    std::fs::write(&path, b"keep\n").unwrap();
    let mut a = App::new(config(dir.path()), Some(path.clone()));
    a.enable_draft_input_for_test();
    settle(&mut a, |a| a.session.state().current_source().is_some());
    for i in 0..80 { a.dispatch(Command::Notice(format!("Warning {i}: connection unavailable"))); }
    draw(&mut a, 100, 30);
    assert!(!a.session.state().activity_log_shown());
    let b = a.layout.controls.iter().find(|(id, _)| *id == CommandId::ToggleActivityLog).unwrap().1;
    mouse(&mut a, MouseButton::Left, b.x, b.y); draw(&mut a, 100, 30);
    assert!(a.session.state().activity_log_shown());
    for code in [KeyCode::Home, KeyCode::Up, KeyCode::Char('x'), KeyCode::Delete] { key(&mut a, code, KeyModifiers::NONE); }
    assert!(a.log_scroll > 0);
    a.handle(Event::Paste("must not reach the draft".into()));
    key(&mut a, KeyCode::Char('c'), KeyModifiers::CONTROL);
    let copied = a.copy.take().unwrap();
    assert!(copied.contains("Warning 0") && copied.contains("Warning 79"));
    for (w, h) in [(40, 12), (18, 8), (1, 1), (100, 30)] {
        draw(&mut a, w, h);
        assert!(a.layout.activity.bottom() <= h);
    }
    let b = a.layout.controls.iter().find(|(id, _)| *id == CommandId::ClearActivityLog).unwrap().1;
    mouse(&mut a, MouseButton::Left, b.x, b.y); draw(&mut a, 100, 30);
    assert!(a.session.state().activity_log().is_empty());
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    assert!(!a.session.state().activity_log_shown());
    let s = a.session.state().current_source().unwrap();
    assert_eq!(s.copy_range(0..s.len_bytes()).unwrap(), "keep\n");
    assert_eq!(std::fs::read(path).unwrap(), b"keep\n");
}
