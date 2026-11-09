use super::*;
#[test]
fn project_replace_prompt_and_preview_route_real_keys_and_paste_without_editing_underneath() {
    let dir=tempfile::tempdir().unwrap();let root=dir.path().join("project");std::fs::create_dir(&root).unwrap();
    let file=root.join("a.txt");std::fs::write(&file,"λ foo\r\nfoo\n").unwrap();
    let mut a=App::with_text_input(config(&dir.path().join("private-state")),Some(root),true);
    settle(&mut a,|a|a.session.state().project().is_some_and(|p|!matches!(p.coverage(),code_core::project::Coverage::Scanning(_))));
    a.session.dispatch(Command::Search {query:"foo".into(),scope:code_core::session::SearchIn::Project});
    settle(&mut a,|a|a.session.state().results().is_some_and(|r|r.done()));
    a.session.dispatch(Command::PreviewProjectReplace("cancelled".into()));
    key(&mut a,KeyCode::Esc,KeyModifiers::NONE);
    assert!(!a.session.workspace_busy());assert!(a.session.workspace_preview().is_none());
    a.run(CommandId::ProjectReplace);
    a.handle(Event::Paste("bar".into()));
    key(&mut a,KeyCode::Home,KeyModifiers::NONE);key(&mut a,KeyCode::Char('X'),KeyModifiers::NONE);
    assert_eq!(a.modal.as_ref().unwrap().input,"Xbar");
    key(&mut a,KeyCode::Delete,KeyModifiers::NONE);
    assert_eq!(a.modal.as_ref().unwrap().input,"Xar");
    key(&mut a,KeyCode::Char('a'),KeyModifiers::CONTROL);a.handle(Event::Paste("bar".into()));
    key(&mut a,KeyCode::Enter,KeyModifiers::NONE);
    settle(&mut a,|a|a.session.workspace_preview().is_some());
    for (width,height) in [(80,24),(24,8),(8,3)] { draw(&mut a,width,height); }
    key(&mut a,KeyCode::End,KeyModifiers::NONE);key(&mut a,KeyCode::Char('n'),KeyModifiers::NONE);
    assert_eq!(a.modal.as_ref().unwrap().top,0);
    key(&mut a,KeyCode::Char(' '),KeyModifiers::NONE);
    assert!(!a.session.workspace_preview().unwrap().files()[0].1);
    key(&mut a,KeyCode::Char('a'),KeyModifiers::NONE);
    assert!(a.session.state().tabs().is_empty());
    key(&mut a,KeyCode::Char(' '),KeyModifiers::NONE);
    key(&mut a,KeyCode::Char('a'),KeyModifiers::NONE);
    settle(&mut a,|a|!a.session.workspace_busy());
    assert_eq!(std::fs::read(&file).unwrap(),"λ foo\r\nfoo\n".as_bytes());
    let source=a.session.state().current_source().unwrap();assert_eq!(source.copy_range(0..source.len_bytes()).unwrap(),"λ bar\r\nbar\n");
    assert!(a.modal.is_none());key(&mut a,KeyCode::Char('z'),KeyModifiers::CONTROL);
    let source=a.session.state().current_source().unwrap();assert_eq!(source.copy_range(0..source.len_bytes()).unwrap(),"λ foo\r\nfoo\n");
}
