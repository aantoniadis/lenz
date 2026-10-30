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
        assert_eq!(app.launched[0].args.last(),Some(&path.to_string_lossy().into_owned()));
        assert!(!app.launched[0].args.iter().any(|arg|arg=="--goto" || arg.starts_with(&format!("{}:",path.display()))));
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
        menu(&mut app,CommandId::CompareWith);
        assert!(draw_screen(&mut app,100,18).contains("saved disk inputs"));
        key(&mut app,KeyCode::Esc,KeyModifiers::NONE);
    }
}

#[test]
fn external_save_failures_dismiss_the_terminal_decision_without_launching() {
    use lenz_content::write::readiness_fault::{at_path,Fault};
    use code_core::save::Status;
    for fault in [Fault::BeforeReplace,Fault::AfterReplace] {
        let dir=tempfile::tempdir().unwrap();let path=dir.path().join("external-fault.txt");
        std::fs::write(&path,b"saved").unwrap();let mut app=app(dir.path());
        app.dispatch(Command::OpenRetained(path.clone()));settle(&mut app,|a|a.session.state().current_source().is_some());
        app.enable_draft_input_for_test();draw(&mut app,100,18);
        key(&mut app,KeyCode::Char('a'),KeyModifiers::CONTROL);app.handle(Event::Paste("draft λ".into()));
        let tab=app.session.state().active().unwrap().id();let guard=at_path(&path,fault);
        menu(&mut app,CommandId::OpenWithDefaultApp);let ticket=app.session.external_prompt().unwrap().ticket;
        key(&mut app,KeyCode::Char('s'),KeyModifiers::NONE);
        settle(&mut app,|a|matches!(a.session.state().view(tab).unwrap().save_status(),Status::Rejected(_)|Status::Uncertain(_)));
        draw(&mut app,100,18);
        assert!(app.launched.is_empty() && app.modal.is_none() && app.session.external_prompt().is_none());
        assert!(app.session.state().view(tab).unwrap().dirty() && app.session.state().view(tab).unwrap().can_undo());
        app.dispatch(Command::ChooseExternal {ticket,choice:code_core::external::Choice::OpenSavedVersion});
        assert!(app.launched.is_empty());
        assert_eq!(std::fs::read(path).unwrap(),if fault==Fault::AfterReplace {"draft λ".as_bytes()}else{b"saved"});
        drop(guard);
    }
}

#[test]
fn external_launch_survives_terminal_draw_before_normal_completion_delivery() {
    for cancel in [false,true] {
        let dir=tempfile::tempdir().unwrap();let path=dir.path().join("delivery.txt");std::fs::write(&path,b"saved").unwrap();
        let mut app=app(dir.path());app.dispatch(Command::OpenRetained(path.clone()));settle(&mut app,|a|a.session.state().current_source().is_some());
        app.enable_draft_input_for_test();draw(&mut app,100,18);
        key(&mut app,KeyCode::Char('a'),KeyModifiers::CONTROL);app.handle(Event::Paste("saved draft λ".into()));
        menu(&mut app,CommandId::OpenWithDefaultApp);let ticket=app.session.external_prompt().unwrap().ticket;
        let _=app.session.dispatch(Command::ChooseExternal {ticket,choice:code_core::external::Choice::SaveAndOpen});
        let until=Instant::now()+Duration::from_secs(10);
        while matches!(app.session.state().view(app.session.state().active().unwrap().id()).unwrap().save_status(),code_core::save::Status::Saving|code_core::save::Status::Rechecking) {
            assert!(Instant::now()<until);draw(&mut app,100,18);std::thread::yield_now();
        }
        assert!(app.launched.is_empty());
        if cancel {key(&mut app,KeyCode::Esc,KeyModifiers::NONE);}
        else {assert!(app.apply(code_core::session::Completion::AnalysisWake),"delivery and modal removal require repaint");}
        assert_eq!(app.launched.len(),usize::from(!cancel),"draw cannot consume the launch");
        assert!(app.modal.is_none() && app.session.external_prompt().is_none());
        app.apply(code_core::session::Completion::AnalysisWake);draw(&mut app,100,18);
        assert_eq!(app.launched.len(),usize::from(!cancel));assert_eq!(std::fs::read(path).unwrap(),"saved draft λ".as_bytes());
    }
}
