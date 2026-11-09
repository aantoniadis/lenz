use super::*;
#[gpui::test]
fn project_replace_preview_input_selection_cancel_apply_and_undo_match_in_both_adapters(cx: &mut TestAppContext) {
    for dark in [true,false] {
        let dir=tempfile::tempdir().unwrap();
        let make=|name:&str| {
            let root=dir.path().join(name).join("project");std::fs::create_dir_all(&root).unwrap();
            std::fs::write(root.join("a.txt"),"λ foo\r\nfoo\n").unwrap();
            let mut c=config(&dir.path().join(name).join("private-state"));c.dark=dark;(c,root)
        };
        let ((gc,gr),(tc,tr))=(make("gui"),make("tui"));
        let mut core=Session::with_text_input(gc,true);core.dispatch(Command::OpenRoot(gr.clone()));
        let gui=cx.new(|cx|ReaderView::new(core,Some(gr.join("a.txt")),cx));
        let window=cx.update(|cx|cx.open_window(Default::default(),|_,_|gui.clone())).unwrap();
        let mut visual=VisualTestContext::from_window(window.into(),cx);
        let focus=gui.read_with(&visual,|v,_|v.focus_handle());
        visual.update(|w,cx|focus.focus(w,cx));
        let mut tui=App::new(tc,Some(tr.clone()));
        tui.enable_draft_input_for_test();
        tui.session.dispatch(Command::OpenRetained(tr.join("a.txt")));
        let mut terminal=ratatui::Terminal::new(ratatui::backend::TestBackend::new(80,24)).unwrap();
        let settle=|visual:&mut VisualTestContext,tui:&mut App,terminal:&mut ratatui::Terminal<ratatui::backend::TestBackend>,done:&dyn Fn(&Session)->bool| {
            let until=Instant::now()+Duration::from_secs(10);
            loop {
                gui.update(visual,|v,cx|v.poll(cx));
                while let Ok(c)=tui.session.completions().try_recv(){tui.apply(c);}
                visual.update(|w,cx|{let _=w.draw(cx);});terminal.draw(|f|tui.draw(f)).unwrap();
                if gui.read_with(visual,|v,_|done(v.session())) && done(&tui.session){break;}
                assert!(Instant::now()<until,"GUI {:?}; TUI {:?}",gui.read_with(visual,|v,_|v.session().state().notices()),tui.session.state().notices());
                std::thread::sleep(Duration::from_millis(2));
            }
        };
        settle(&mut visual,&mut tui,&mut terminal,&|s|s.state().current_source().is_some()&&s.state().project().is_some_and(|p|!matches!(p.coverage(),code_core::project::Coverage::Scanning(_))));
        gui.update(&mut visual,|v,cx|v.dispatch_for_test(Command::Search {query:"foo".into(),scope:code_core::session::SearchIn::Project},cx));
        tui.session.dispatch(Command::Search {query:"foo".into(),scope:code_core::session::SearchIn::Project});
        settle(&mut visual,&mut tui,&mut terminal,&|s|s.state().results().is_some_and(|r|r.done()));
        gui.update(&mut visual,|v,cx|v.dispatch_for_test(Command::PreviewProjectReplace("cancelled".into()),cx));
        tui.session.dispatch(Command::PreviewProjectReplace("cancelled".into()));
        visual.simulate_keystrokes("escape");tui.handle(Event::Key(KeyEvent::new(KeyCode::Esc,KeyModifiers::NONE)));
        assert!(!tui.session.workspace_busy());assert!(gui.read_with(&visual,|v,_|!v.session().workspace_busy()));
        assert!(tui.session.workspace_preview().is_none());assert!(gui.read_with(&visual,|v,_|v.session().workspace_preview().is_none()));
        gui.update_in(&mut visual,|v,w,cx|v.run(CommandId::ProjectReplace,w,cx));tui.run(CommandId::ProjectReplace);
        visual.update(|_,cx|cx.write_to_clipboard(gpui::ClipboardItem::new_string("bar".into())));
        visual.simulate_keystrokes("ctrl-v home X delete");
        tui.handle(Event::Paste("bar".into()));
        for code in [KeyCode::Home,KeyCode::Char('X'),KeyCode::Delete]{tui.handle(Event::Key(KeyEvent::new(code,KeyModifiers::NONE)));}
        assert_eq!(tui.modal.as_ref().unwrap().input,"Xar");
        assert!(gui.read_with(&visual,|v,_|v.modal_rows().iter().any(|(row,_)|row.contains("Xar"))), "{:?}",gui.read_with(&visual,|v,_|(v.modal_rows(),v.session().state().notices().to_vec())));
        visual.simulate_keystrokes("ctrl-a ctrl-v enter");
        tui.handle(Event::Key(KeyEvent::new(KeyCode::Char('a'),KeyModifiers::CONTROL)));tui.handle(Event::Paste("bar".into()));tui.handle(Event::Key(KeyEvent::new(KeyCode::Enter,KeyModifiers::NONE)));
        settle(&mut visual,&mut tui,&mut terminal,&|s|s.workspace_preview().is_some());
        for (width,height) in [(80,24),(24,8)] {
            terminal.backend_mut().resize(width,height);visual.simulate_resize(gpui::size(gpui::px(width as f32*9.),gpui::px(height as f32*20.)));
            settle(&mut visual,&mut tui,&mut terminal,&|s|s.workspace_preview().is_some());
        }
        visual.simulate_keystrokes("end n");for code in [KeyCode::End,KeyCode::Char('n')] {tui.handle(Event::Key(KeyEvent::new(code,KeyModifiers::NONE)));}
        assert_eq!(tui.modal.as_ref().unwrap().top,0);
        visual.simulate_keystrokes("space a");for c in [' ','a']{tui.handle(Event::Key(KeyEvent::new(KeyCode::Char(c),KeyModifiers::NONE)));}
        assert!(!tui.session.workspace_preview().unwrap().files()[0].1);
        visual.simulate_keystrokes("space a");for c in [' ','a']{tui.handle(Event::Key(KeyEvent::new(KeyCode::Char(c),KeyModifiers::NONE)));}
        settle(&mut visual,&mut tui,&mut terminal,&|s|!s.workspace_busy()&&s.workspace_preview().is_none());
        let text=|s:&Session|s.state().current_source().map(|s|s.copy_range(0..s.len_bytes()).unwrap());
        assert_eq!(text(&tui.session).as_deref(),Some("λ bar\r\nbar\n"));assert_eq!(gui.read_with(&visual,|v,_|text(v.session())),text(&tui.session));
        visual.simulate_keystrokes("x");tui.handle(Event::Key(KeyEvent::new(KeyCode::Char('x'),KeyModifiers::NONE)));
        assert!(text(&tui.session).unwrap().contains('x'));assert_eq!(gui.read_with(&visual,|v,_|text(v.session())),text(&tui.session));
        visual.simulate_keystrokes("ctrl-z");tui.handle(Event::Key(KeyEvent::new(KeyCode::Char('z'),KeyModifiers::CONTROL)));
        assert_eq!(text(&tui.session).as_deref(),Some("λ bar\r\nbar\n"));
        visual.simulate_keystrokes("ctrl-z");tui.handle(Event::Key(KeyEvent::new(KeyCode::Char('z'),KeyModifiers::CONTROL)));
        assert_eq!(text(&tui.session).as_deref(),Some("λ foo\r\nfoo\n"));assert_eq!(gui.read_with(&visual,|v,_|text(v.session())),text(&tui.session));
        for root in [&gr,&tr]{assert_eq!(std::fs::read(root.join("a.txt")).unwrap(),"λ foo\r\nfoo\n".as_bytes());}
        // PR #50's scratch-kind picker and save-on-leave operate on the replaced draft too.
        gui.update_in(&mut visual, |v,w,cx| v.run(CommandId::SaveOnLeave,w,cx));
        tui.run(CommandId::SaveOnLeave);
        gui.update(&mut visual, |v,cx| v.dispatch_for_test(Command::Search {query:"foo".into(),scope:code_core::session::SearchIn::Project},cx));
        tui.dispatch(Command::Search {query:"foo".into(),scope:code_core::session::SearchIn::Project});
        settle(&mut visual,&mut tui,&mut terminal,&|s|s.state().results().is_some_and(|r|r.done()));
        gui.update(&mut visual, |v,cx| v.dispatch_for_test(Command::PreviewProjectReplace("saved".into()),cx));
        tui.dispatch(Command::PreviewProjectReplace("saved".into()));
        settle(&mut visual,&mut tui,&mut terminal,&|s|s.workspace_preview().is_some());
        visual.simulate_keystrokes("a");
        tui.handle(Event::Key(KeyEvent::new(KeyCode::Char('a'),KeyModifiers::NONE)));
        settle(&mut visual,&mut tui,&mut terminal,&|s|!s.workspace_busy()&&s.workspace_preview().is_none());
        gui.update_in(&mut visual, |v,w,cx| v.run(CommandId::NewScratch,w,cx));
        tui.run(CommandId::NewScratch);
        visual.simulate_keystrokes("r u b enter");
        for code in [KeyCode::Char('r'),KeyCode::Char('u'),KeyCode::Char('b'),KeyCode::Enter] {
            tui.handle(Event::Key(KeyEvent::new(code,KeyModifiers::NONE)));
        }
        settle(&mut visual,&mut tui,&mut terminal,&|s|s.state().current_source().is_some_and(|d|d.source().ends_with("scratch_1.rb")) && s.state().tabs().iter().filter(|t|t.source().ends_with("a.txt")).all(|t|!s.state().tab_dirty(t.id())));
        for root in [&gr,&tr] { assert_eq!(std::fs::read(root.join("a.txt")).unwrap(),"λ saved\r\nsaved\n".as_bytes()); }
        visual.simulate_keystrokes("ctrl-tab");
        tui.handle(Event::Key(KeyEvent::new(KeyCode::Tab,KeyModifiers::CONTROL)));
        settle(&mut visual,&mut tui,&mut terminal,&|s|s.state().current_source().is_some_and(|d|d.source().ends_with("a.txt")));
        visual.simulate_keystrokes("ctrl-z");
        tui.handle(Event::Key(KeyEvent::new(KeyCode::Char('z'),KeyModifiers::CONTROL)));
        assert_eq!(text(&tui.session).as_deref(),Some("λ foo\r\nfoo\n"));
        assert_eq!(gui.read_with(&visual,|v,_|text(v.session())),text(&tui.session));
        // Leave that Undo unsaved, then replace the inactive pinned draft from the scratch tab.
        gui.update_in(&mut visual, |v,w,cx| v.run(CommandId::SaveOnLeave,w,cx));
        tui.run(CommandId::SaveOnLeave);
        visual.simulate_keystrokes("ctrl-tab");
        tui.handle(Event::Key(KeyEvent::new(KeyCode::Tab,KeyModifiers::CONTROL)));
        settle(&mut visual,&mut tui,&mut terminal,&|s|s.state().current_source().is_some_and(|d|d.source().ends_with("scratch_1.rb")));
        gui.update(&mut visual, |v,cx| v.dispatch_for_test(Command::Search {query:"foo".into(),scope:code_core::session::SearchIn::Project},cx));
        tui.dispatch(Command::Search {query:"foo".into(),scope:code_core::session::SearchIn::Project});
        settle(&mut visual,&mut tui,&mut terminal,&|s|s.state().results().is_some_and(|r|r.done()));
        gui.update(&mut visual, |v,cx| v.dispatch_for_test(Command::PreviewProjectReplace("again".into()),cx));
        tui.dispatch(Command::PreviewProjectReplace("again".into()));
        settle(&mut visual,&mut tui,&mut terminal,&|s|s.workspace_preview().is_some());
        visual.simulate_keystrokes("a");
        tui.handle(Event::Key(KeyEvent::new(KeyCode::Char('a'),KeyModifiers::NONE)));
        settle(&mut visual,&mut tui,&mut terminal,&|s|!s.workspace_busy()&&s.workspace_preview().is_none());
        assert_eq!(text(&tui.session).as_deref(),Some("λ again\r\nagain\n"));
        assert_eq!(gui.read_with(&visual,|v,_|text(v.session())),text(&tui.session));
        for root in [&gr,&tr] { assert_eq!(std::fs::read(root.join("a.txt")).unwrap(),"λ saved\r\nsaved\n".as_bytes()); }
        visual.simulate_keystrokes("ctrl-z ctrl-s");
        for c in ['z','s'] { tui.handle(Event::Key(KeyEvent::new(KeyCode::Char(c),KeyModifiers::CONTROL))); }
        settle(&mut visual,&mut tui,&mut terminal,&|s|s.state().active().is_some_and(|t|!s.state().tab_dirty(t.id())));
        for root in [&gr,&tr] { assert_eq!(std::fs::read(root.join("a.txt")).unwrap(),"λ foo\r\nfoo\n".as_bytes()); }
    }
}

#[gpui::test]
fn symbol_rename_prompt_and_multi_file_preview_match_in_both_adapters(cx: &mut TestAppContext) {
    let dir=tempfile::tempdir().unwrap();
    let profile=std::env::current_exe().unwrap().parent().unwrap().parent().unwrap().to_owned();
    let mut build=std::process::Command::new(std::env::var("CARGO").unwrap_or_else(|_|"cargo".into()));
    build.args(["build","-q","-p","code-core","--example","fake_lsp"]);
    if profile.ends_with("release") { build.arg("--release"); }
    assert!(build.status().unwrap().success());
    let server=dir.path().join("server");
    std::fs::write(&server,format!("#!/bin/sh\nexport FAKE_LSP_RENAME=1\nexec '{}' \"$@\"\n",profile.join("examples/fake_lsp").display())).unwrap();
    std::fs::set_permissions(&server,std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let make=|name:&str| {
        let root=dir.path().join(name).join("project");std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("Cargo.toml"),"[package]\nname = \"rename_fixture\"\n").unwrap();
        std::fs::write(root.join("a.rs"),"fn target() {}\n").unwrap();std::fs::write(root.join("b.rs"),"fn caller() { target(); }\n").unwrap();
        let c=config(&dir.path().join(name).join("private-state"));
        let mut providers:serde_json::Value=serde_json::from_slice(&std::fs::read(&c.providers).unwrap()).unwrap();
        providers["providers"]["rust-analyzer"]["executable"]=serde_json::json!(server);
        std::fs::write(&c.providers,providers.to_string()).unwrap();(c,root)
    };
    let ((gc,gr),(tc,tr))=(make("gui"),make("tui"));
    let mut core=Session::with_text_input(gc,true);core.dispatch(Command::OpenRoot(gr.clone()));
    let gui=cx.new(|cx|ReaderView::new(core,Some(gr.join("a.rs")),cx));
    let window=cx.update(|cx|cx.open_window(Default::default(),|_,_|gui.clone())).unwrap();let mut visual=VisualTestContext::from_window(window.into(),cx);
    let focus=gui.read_with(&visual,|v,_|v.focus_handle());visual.update(|w,cx|focus.focus(w,cx));
    let mut tui=App::new(tc,Some(tr.clone()));tui.enable_draft_input_for_test();tui.session.dispatch(Command::OpenRetained(tr.join("a.rs")));
    let mut terminal=ratatui::Terminal::new(ratatui::backend::TestBackend::new(80,24)).unwrap();
    let settle=|visual:&mut VisualTestContext,tui:&mut App,terminal:&mut ratatui::Terminal<ratatui::backend::TestBackend>,done:&dyn Fn(&Session)->bool| {
        let until=Instant::now()+Duration::from_secs(15);
        loop {
            gui.update(visual,|v,cx|v.poll(cx));while let Ok(c)=tui.session.completions().try_recv(){tui.apply(c);}
            visual.update(|w,cx|{let _=w.draw(cx);});terminal.draw(|f|tui.draw(f)).unwrap();
            if gui.read_with(visual,|v,_|done(v.session())) && done(&tui.session){break;}
            assert!(Instant::now()<until,"GUI {:?}; TUI {:?}",gui.read_with(visual,|v,_|v.session().state().notices()),tui.session.state().notices());std::thread::sleep(Duration::from_millis(2));
        }
    };
    settle(&mut visual,&mut tui,&mut terminal,&|s|s.state().provider().is_some_and(|p|p.offers(code_core::semantics::Question::Rename)) && s.disabled(CommandId::RenameSymbol).is_none());
    let caret=code_core::session::Selection {anchor:4,head:4};
    gui.update(&mut visual,|v,cx|v.dispatch_for_test(Command::Select(caret),cx));tui.session.dispatch(Command::Select(caret));
    gui.update_in(&mut visual,|v,w,cx|v.run(CommandId::RenameSymbol,w,cx));tui.run(CommandId::RenameSymbol);
    visual.update(|_,cx|cx.write_to_clipboard(gpui::ClipboardItem::new_string("renamed".into())));visual.simulate_keystrokes("ctrl-v enter");
    tui.handle(Event::Paste("renamed".into()));tui.handle(Event::Key(KeyEvent::new(KeyCode::Enter,KeyModifiers::NONE)));
    settle(&mut visual,&mut tui,&mut terminal,&|s|s.workspace_preview().is_some());
    assert_eq!(tui.session.workspace_preview().unwrap().files().len(),2);
    visual.simulate_keystrokes("a");tui.handle(Event::Key(KeyEvent::new(KeyCode::Char('a'),KeyModifiers::NONE)));
    settle(&mut visual,&mut tui,&mut terminal,&|s|!s.workspace_busy()&&s.workspace_preview().is_none());
    let text=|s:&Session|s.state().current_source().map(|s|s.copy_range(0..s.len_bytes()).unwrap());
    assert_eq!(text(&tui.session).as_deref(),Some("fn renamed() {}\n"));assert_eq!(gui.read_with(&visual,|v,_|text(v.session())),text(&tui.session));
    for root in [&gr,&tr]{assert_eq!(std::fs::read(root.join("a.rs")).unwrap(),b"fn target() {}\n");assert_eq!(std::fs::read(root.join("b.rs")).unwrap(),b"fn caller() { target(); }\n");}
}
