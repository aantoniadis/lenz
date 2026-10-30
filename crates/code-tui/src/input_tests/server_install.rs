use super::*;

#[test]
fn server_install_menu_opens_preview_and_keeps_document_input_modal() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("script.sh");
    std::fs::write(&path, b"echo unchanged\n").unwrap();
    let cfg = config(dir.path());
    std::fs::create_dir_all(cfg.providers.parent().unwrap()).unwrap();
    std::fs::write(&cfg.providers, r#"{"providers":{"bash-language-server":{"env":{"PATH":"/nonexistent"}}}}"#).unwrap();
    let mut a = App::new(cfg, Some(path.clone()));
    settle(&mut a, |a| a.session.state().current_source().is_some());
    key(&mut a, KeyCode::F(2), KeyModifiers::NONE);
    let at = a.choices().iter().position(|c| matches!(c.target, ChoiceTarget::Action(CommandId::InstallServer))).unwrap();
    for _ in 0..at { key(&mut a, KeyCode::Down, KeyModifiers::NONE); }
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    assert!(matches!(a.modal.as_ref().map(|m| &m.kind), Some(Prompt::Details {title: "Install language server", ..})));
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(80,24)).unwrap();
    terminal.draw(|frame| a.draw(frame)).unwrap();
    let screen = terminal.backend().buffer().content.iter().map(|cell| cell.symbol()).collect::<String>();
    assert!(screen.contains("Start Installation") && screen.contains("bash-language-server"), "{screen}");
    key(&mut a, KeyCode::Char('s'), KeyModifiers::NONE);
    key(&mut a, KeyCode::Char('a'), KeyModifiers::CONTROL);
    key(&mut a, KeyCode::Char('s'), KeyModifiers::CONTROL);
    key(&mut a, KeyCode::F(5), KeyModifiers::NONE);
    assert!(!a.session.state().server_install().unwrap().running);
    for (w,h) in [(20,6),(1,1),(80,24)] { draw(&mut a,w,h); key(&mut a,KeyCode::End,KeyModifiers::NONE); key(&mut a,KeyCode::Up,KeyModifiers::NONE); draw(&mut a,w,h); }
    key(&mut a,KeyCode::Esc,KeyModifiers::NONE);
    assert!(a.modal.is_none() && a.session.state().server_install().is_none());
    assert_eq!(std::fs::read(path).unwrap(), b"echo unchanged\n");
}

#[cfg(unix)]
#[test]
fn server_install_start_progress_and_cancel_use_real_owned_processes() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("bin"); std::fs::create_dir(&bin).unwrap();
    for (name,script) in [("node","#!/bin/sh\nif [ \"$1\" = --version ]; then echo v24.0.0; else exec \"$@\"; fi\n"),("npm","#!/bin/sh\necho FIXTURE_INSTALL_PROGRESS\nexec /bin/sleep 30\n")] {
        let path=bin.join(name);std::fs::write(&path,script).unwrap();std::fs::set_permissions(path,std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let path=dir.path().join("script.sh");std::fs::write(&path,b"echo unchanged\n").unwrap();
    let cfg=config(dir.path());let providers=cfg.providers.clone();std::fs::create_dir_all(providers.parent().unwrap()).unwrap();
    let settings=serde_json::json!({"providers":{"bash-language-server":{"env":{"PATH":format!("{}:/usr/bin:/bin",bin.display())}}}});
    std::fs::write(&providers,settings.to_string()).unwrap();
    let mut a=App::new(cfg,Some(path.clone()));settle(&mut a,|a|a.session.state().current_source().is_some());
    key(&mut a,KeyCode::F(2),KeyModifiers::NONE);
    let at=a.choices().iter().position(|c|matches!(c.target,ChoiceTarget::Action(CommandId::InstallServer))).unwrap();
    for _ in 0..at {key(&mut a,KeyCode::Down,KeyModifiers::NONE);}
    key(&mut a,KeyCode::Enter,KeyModifiers::NONE);
    assert!(!a.session.state().server_install().unwrap().running);
    key(&mut a,KeyCode::Char('s'),KeyModifiers::NONE);
    assert!(a.session.state().server_install().unwrap().running);
    settle(&mut a,|a|a.session.state().server_install().unwrap().output.contains("FIXTURE_INSTALL_PROGRESS"));
    let mut terminal=ratatui::Terminal::new(ratatui::backend::TestBackend::new(100,26)).unwrap();terminal.draw(|f|a.draw(f)).unwrap();
    let screen=terminal.backend().buffer().content.iter().map(|c|c.symbol()).collect::<String>();
    assert!(screen.contains("Cancel Installation") && screen.contains("FIXTURE_INSTALL_PROGRESS"),"{screen}");
    key(&mut a,KeyCode::Char('d'),KeyModifiers::NONE);
    assert!(matches!(a.modal.as_ref().map(|m|&m.kind),Some(Prompt::Details {title:"Server details",..})));
    key(&mut a,KeyCode::Esc,KeyModifiers::NONE);
    assert!(matches!(a.modal.as_ref().map(|m|&m.kind),Some(Prompt::Details {kind:Details::Install,..})));
    key(&mut a,KeyCode::Char('s'),KeyModifiers::CONTROL);
    key(&mut a,KeyCode::Char('c'),KeyModifiers::NONE);
    settle(&mut a,|a|a.session.state().server_install().unwrap().finished);
    assert!(a.session.state().server_install().unwrap().message.contains("cancelled"));
    assert_eq!(serde_json::from_slice::<serde_json::Value>(&std::fs::read(providers).unwrap()).unwrap(),settings);
    key(&mut a,KeyCode::Esc,KeyModifiers::NONE);assert!(a.modal.is_none());
    assert_eq!(std::fs::read(path).unwrap(),b"echo unchanged\n");
}
