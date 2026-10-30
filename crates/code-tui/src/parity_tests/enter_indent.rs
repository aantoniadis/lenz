use super::*;

#[gpui::test]
fn enter_preserves_each_carets_indentation_and_undo_in_both_adapters(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    for (name, original, indented) in [
        ("Main.java", "    alpha\r\n    beta\r\n", "    alpha\r\n    \r\n    beta\r\n    \r\n"),
        ("plain.txt", "\t alpha\n\t beta\n", "\t alpha\n\t \n\t beta\n\t \n"),
    ] {
        let path = dir.path().join(name);
        std::fs::write(&path, original).unwrap();
        let gui = cx.new(|cx| ReaderView::new(Session::new(config(&dir.path().join(format!("gui-{name}")))), Some(path.clone()), cx));
        let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| gui.clone())).unwrap();
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        let focus = gui.read_with(&visual, |view, _| view.focus_handle());
        visual.update(|window, cx| focus.focus(window, cx));
        let mut tui = App::new(config(&dir.path().join(format!("tui-{name}"))), Some(path.clone()));
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 24)).unwrap();
        let draw = |visual: &mut VisualTestContext, tui: &mut App, terminal: &mut ratatui::Terminal<ratatui::backend::TestBackend>| {
            visual.update(|window, cx| { let _ = window.draw(cx); });
            terminal.draw(|frame| tui.draw(frame)).unwrap();
        };
        let deadline = Instant::now() + Duration::from_secs(5);
        while gui.read_with(&visual, |view, _| view.session().state().current_source().is_none()) || tui.session.state().current_source().is_none() {
            gui.update(&mut visual, |view, cx| view.poll(cx));
            while let Ok(done) = tui.session.completions().try_recv() { tui.apply(done); }
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        gui.update(&mut visual, |view, _| view.enable_draft_input_for_test());
        tui.enable_draft_input_for_test();
        draw(&mut visual, &mut tui, &mut terminal);
        let key = |tui: &mut App, code, modifiers| { tui.handle(Event::Key(KeyEvent::new(code, modifiers))); };
        visual.simulate_keystrokes("ctrl-home end ctrl-alt-down enter");
        for (code, modifiers) in [(KeyCode::Home, KeyModifiers::CONTROL), (KeyCode::End, KeyModifiers::NONE),
            (KeyCode::Down, KeyModifiers::CONTROL | KeyModifiers::ALT), (KeyCode::Enter, KeyModifiers::NONE)] {
            key(&mut tui, code, modifiers);
        }
        let check = |visual: &VisualTestContext, tui: &App, expected: &str| {
            let state = snapshot(tui.session.state());
            assert_eq!(state.text.as_deref(), Some(expected), "{name}");
            assert_eq!(state, gui.read_with(visual, |view, _| snapshot(view.session().state())));
        };
        for (width, height) in [(80, 24), (24, 8)] {
            terminal.backend_mut().resize(width, height);
            visual.simulate_resize(gpui::size(gpui::px(f32::from(width) * 9.0), gpui::px(f32::from(height) * 20.0)));
            draw(&mut visual, &mut tui, &mut terminal);
            check(&visual, &tui, indented);
        }
        assert_eq!(snapshot(tui.session.state()).selections.len(), 2);
        visual.simulate_keystrokes("ctrl-z");
        key(&mut tui, KeyCode::Char('z'), KeyModifiers::CONTROL);
        check(&visual, &tui, original);
        visual.simulate_keystrokes("ctrl-shift-z");
        key(&mut tui, KeyCode::Char('z'), KeyModifiers::CONTROL | KeyModifiers::SHIFT);
        check(&visual, &tui, indented);
        assert_eq!(std::fs::read_to_string(path).unwrap(), original, "Enter and Undo never write the file");
    }
}
