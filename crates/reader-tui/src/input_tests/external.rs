use super::*;

fn menu(app: &mut App, command: CommandId) {
    key(app, KeyCode::F(10), KeyModifiers::NONE);
    let row = app.choices().iter().position(|row| matches!(&row.target, ChoiceTarget::Action(id) if *id == command)).unwrap();
    for _ in 0..row { key(app, KeyCode::Down, KeyModifiers::NONE); }
    key(app, KeyCode::Enter, KeyModifiers::NONE);
}

#[test]
fn dirty_external_choices_use_real_terminal_menus_input_and_save_receipts() {
    for dark in [true, false] {
        let dir = tempfile::tempdir().unwrap(); let path = dir.path().join("external.txt");
        std::fs::write(&path,b"saved\r\n").unwrap();
        let mut cfg = config(dir.path()); cfg.dark = dark;
        let mut app = App::new(cfg, Some(dir.path().to_owned()));
        app.dispatch(Command::OpenRetained(path.clone()));
        settle(&mut app, |a| a.session.state().current_source().is_some());
        app.enable_draft_input_for_test(); draw(&mut app,100,18);
        key(&mut app,KeyCode::Char('a'),KeyModifiers::CONTROL);
        app.handle(Event::Paste("draft λ\r\n".into()));
        let source = app.session.state().current_source().unwrap().key();
        menu(&mut app,CommandId::OpenInEditor);
        assert!(app.session.external_prompt().is_some(), "the terminal menu must prepare the shared decision");
        let screen = draw_screen(&mut app,100,18);
        for label in ["Save and Open", "Open Saved Version", "Cancel"] { assert!(screen.contains(label), "{screen}"); }
        app.handle(Event::Paste("must not paste".into()));
        key(&mut app,KeyCode::Char('s'),KeyModifiers::CONTROL);
        key(&mut app,KeyCode::Char('z'),KeyModifiers::CONTROL);
        key(&mut app,KeyCode::Char('x'),KeyModifiers::NONE);
        for (w,h) in [(20,5),(1,1),(100,18)] { draw(&mut app,w,h); key(&mut app,KeyCode::End,KeyModifiers::NONE); key(&mut app,KeyCode::Up,KeyModifiers::NONE); }
        assert_eq!(app.session.state().current_source().unwrap().key(),source);
        assert!(app.launched.is_empty());
        key(&mut app,KeyCode::Esc,KeyModifiers::NONE);
        assert!(app.session.external_prompt().is_none() && app.modal.is_none());
        assert_eq!(std::fs::read(&path).unwrap(),b"saved\r\n");

        menu(&mut app,CommandId::OpenWithDefaultApp);
        key(&mut app,KeyCode::Char('o'),KeyModifiers::NONE);
        assert_eq!(app.launched.len(),1);
        assert_eq!(app.launched[0].args,[path.to_string_lossy().into_owned()]);
        assert!(app.session.state().view(app.session.state().active().unwrap().id()).unwrap().dirty());
        assert_eq!(std::fs::read(&path).unwrap(),b"saved\r\n");
        menu(&mut app,CommandId::OpenWithDefaultApp);
        key(&mut app,KeyCode::Char('s'),KeyModifiers::NONE);
        settle(&mut app,|a|a.launched.len()==2);
        assert!(app.session.external_prompt().is_none() && app.modal.is_none());
        assert_eq!(std::fs::read(&path).unwrap(),"draft λ\r\n".as_bytes());
        key(&mut app,KeyCode::Char('z'),KeyModifiers::CONTROL);
        assert!(app.session.state().view(app.session.state().active().unwrap().id()).unwrap().dirty());
        assert_eq!(std::fs::read(path).unwrap(),"draft λ\r\n".as_bytes());
    }
}
