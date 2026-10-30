use super::*;

#[test]
fn project_syntax_palette_checks_unopened_files_and_keeps_coverage_readable_when_narrow() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project"); std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("a.rs"), b"fn broken( {\n").unwrap();
    std::fs::write(root.join("bad.xml"), b"<Root></root>\n").unwrap();
    let mut a = app(dir.path()); a.dispatch(Command::OpenRoot(root.clone()));
    settle(&mut a, |a| a.session.state().project().is_some_and(|p| p.root() == root && matches!(p.coverage(), code_core::project::Coverage::Complete(_))));
    assert!(a.session.state().tabs().is_empty() && a.session.state().results().is_none());
    key(&mut a, KeyCode::F(10), KeyModifiers::NONE);
    let row = a.choices().iter().position(|row| matches!(&row.target, ChoiceTarget::Action(CommandId::CheckProjectSyntax))).unwrap();
    for _ in 0..row { key(&mut a, KeyCode::Down, KeyModifiers::NONE); }
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    settle(&mut a, |a| a.session.state().results().is_some_and(|r| r.done()));
    assert_eq!(a.focus, Focus::Results);
    let results = a.session.state().results().unwrap();
    assert!(results.hits().iter().any(|h| h.path.as_ref() == "a.rs"));
    assert!(results.hits().iter().any(|h| h.path.as_ref() == "bad.xml"));
    assert!(a.session.state().tabs().is_empty());
    assert!(draw_screen(&mut a, 100, 22).contains('!'));
    draw(&mut a, 28, 6);
    key(&mut a, KeyCode::F(10), KeyModifiers::NONE);
    let details = a.choices().iter().position(|row| matches!(&row.target, ChoiceTarget::Action(CommandId::StatusDetails))).unwrap();
    for _ in 0..details { key(&mut a, KeyCode::Down, KeyModifiers::NONE); }
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    assert!(matches!(a.modal.as_ref().map(|m| &m.kind), Some(Prompt::Details { body, .. }) if body.contains("2 checked") && body.contains("1024 paths")), "{:?}; {}", a.modal.as_ref().map(|m| &m.kind), a.session.status_details());
    draw(&mut a, 28, 6); draw(&mut a, 1, 1);
    key(&mut a, KeyCode::End, KeyModifiers::NONE); key(&mut a, KeyCode::Up, KeyModifiers::NONE);
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    assert!(a.modal.is_none());
    assert_eq!(std::fs::read(root.join("a.rs")).unwrap(), b"fn broken( {\n");
}
