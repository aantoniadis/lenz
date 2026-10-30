use super::*;

#[gpui::test]
fn blank_lines_complete_only_on_ctrl_space_in_both_adapters(cx: &mut TestAppContext) {
    let profile = std::env::current_exe().unwrap().parent().unwrap().parent().unwrap().to_owned();
    assert!(std::process::Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .args(["build", "-q", "--locked", "-p", "code-core", "--example", "fake_lsp"])
        .current_dir(env!("CARGO_MANIFEST_DIR")).status().unwrap().success());
    let fake = profile.join("examples/fake_lsp");
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    std::fs::create_dir(&root).unwrap();
    let path = root.join("main.rs");
    let original = "\n    \nfn helper() {}\n";
    std::fs::write(root.join("Cargo.toml"), "[package]\nname='blank-completion'\nversion='0.1.0'\n").unwrap();
    std::fs::write(&path, original).unwrap();
    let make = |name| {
        let c = config(&dir.path().join(name));
        let mut settings: serde_json::Value = serde_json::from_slice(&std::fs::read(&c.providers).unwrap()).unwrap();
        settings["providers"]["rust-analyzer"] = serde_json::json!({"executable":fake,"env":{
            "FAKE_LSP_NO_DECOR":"1", "FAKE_LSP_WHITESPACE_TRIGGERS":"1"
        }});
        std::fs::write(&c.providers, settings.to_string()).unwrap();
        c
    };
    let gui = cx.new(|cx| ReaderView::new(Session::new(make("gui")), Some(path.clone()), cx));
    let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| gui.clone())).unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let focus = gui.read_with(&visual, |view, _| view.focus_handle());
    visual.update(|window, cx| focus.focus(window, cx));
    let mut tui = App::new(make("tui"), Some(root.clone()));
    tui.session.dispatch(Command::Open(path.clone()));
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
    let poll = |visual: &mut VisualTestContext, tui: &mut App, terminal: &mut ratatui::Terminal<ratatui::backend::TestBackend>| {
        gui.update(visual, |view, cx| view.poll(cx));
        while let Ok(done) = tui.session.completions().try_recv() { tui.apply(done); }
        visual.update(|window, cx| { let _ = window.draw(cx); });
        terminal.draw(|frame| tui.draw(frame)).unwrap();
    };
    let wait = |visual: &mut VisualTestContext, tui: &mut App, terminal: &mut ratatui::Terminal<ratatui::backend::TestBackend>, done: &dyn Fn(&State) -> bool| {
        let until = Instant::now() + Duration::from_secs(10);
        loop {
            poll(visual, tui, terminal);
            if gui.read_with(visual, |view, _| done(view.session().state())) && done(tui.session.state()) { break; }
            assert!(Instant::now() < until, "GUI {:?}; TUI {:?}", gui.read_with(visual, |view, _| view.session().state().notices()), tui.session.state().notices());
            std::thread::yield_now();
        }
    };
    wait(&mut visual, &mut tui, &mut terminal, &|state| {
        state.provider().is_some_and(|p| p.status == code_core::semantics::Status::Ready)
            && state.active().is_some_and(|tab| state.view(tab.id()).is_some_and(|v| v.highlights().is_some()))
    });
    gui.update(&mut visual, |view, _| view.enable_draft_input_for_test());
    tui.enable_draft_input_for_test();
    let key = |tui: &mut App, code, modifiers| { tui.handle(Event::Key(KeyEvent::new(code, modifiers))); };
    let same = |visual: &VisualTestContext, tui: &App, expected: &str| {
        let state = snapshot(tui.session.state());
        assert_eq!(state.text.as_deref(), Some(expected));
        assert_eq!(state, gui.read_with(visual, |view, _| snapshot(view.session().state())));
    };
    let no_popup = |visual: &mut VisualTestContext, tui: &mut App, terminal: &mut ratatui::Terminal<ratatui::backend::TestBackend>| {
        // Drain replies too: an immediate check could miss an unwanted asynchronous popup.
        let until = Instant::now() + Duration::from_millis(300);
        loop {
            poll(visual, tui, terminal);
            assert!(tui.session.state().completion().is_none());
            assert!(gui.read_with(visual, |view, _| view.session().state().completion().is_none()));
            assert_eq!(tui.layout.completion.width, 0);
            assert!(gui.read_with(visual, |view, _| view.completion_bounds().is_none()));
            if Instant::now() >= until { break; }
            std::thread::yield_now();
        }
    };
    let manual = |visual: &mut VisualTestContext, tui: &mut App, terminal: &mut ratatui::Terminal<ratatui::backend::TestBackend>| {
        visual.simulate_keystrokes("ctrl-space");
        key(tui, KeyCode::Char(' '), KeyModifiers::CONTROL);
        wait(visual, tui, terminal, &|state| state.completion().is_some());
        let labels = |state: &State| state.completion().unwrap().rows().map(|r| r.label.clone()).collect::<Vec<_>>();
        assert_eq!(labels(tui.session.state()), gui.read_with(visual, |view, _| labels(view.session().state())));
        assert!(labels(tui.session.state()).iter().any(|label| label == "helper"));
        assert!(tui.layout.completion.width > 0);
        assert!(gui.read_with(visual, |view, _| view.completion_bounds().is_some()));
        visual.simulate_keystrokes("escape");
        key(tui, KeyCode::Esc, KeyModifiers::NONE);
        no_popup(visual, tui, terminal);
    };

    visual.simulate_keystrokes("ctrl-home enter");
    key(&mut tui, KeyCode::Home, KeyModifiers::CONTROL);
    key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    no_popup(&mut visual, &mut tui, &mut terminal);
    same(&visual, &tui, "\n\n    \nfn helper() {}\n");
    manual(&mut visual, &mut tui, &mut terminal);

    visual.simulate_keystrokes("down end space");
    for code in [KeyCode::Down, KeyCode::End, KeyCode::Char(' ')] {
        key(&mut tui, code, KeyModifiers::NONE);
    }
    no_popup(&mut visual, &mut tui, &mut terminal);
    same(&visual, &tui, "\n\n     \nfn helper() {}\n");
    visual.simulate_keystrokes("enter");
    key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    no_popup(&mut visual, &mut tui, &mut terminal);
    let indented = "\n\n     \n     \nfn helper() {}\n";
    same(&visual, &tui, indented);
    manual(&mut visual, &mut tui, &mut terminal);

    visual.simulate_keystrokes("h");
    key(&mut tui, KeyCode::Char('h'), KeyModifiers::NONE);
    wait(&mut visual, &mut tui, &mut terminal, &|state| state.completion().is_some());
    visual.simulate_keystrokes("backspace");
    key(&mut tui, KeyCode::Backspace, KeyModifiers::NONE);
    no_popup(&mut visual, &mut tui, &mut terminal);
    same(&visual, &tui, indented);

    visual.simulate_keystrokes("h");
    key(&mut tui, KeyCode::Char('h'), KeyModifiers::NONE);
    wait(&mut visual, &mut tui, &mut terminal, &|state| state.completion().is_some());
    visual.simulate_keystrokes("enter .");
    key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    key(&mut tui, KeyCode::Char('.'), KeyModifiers::NONE);
    wait(&mut visual, &mut tui, &mut terminal, &|state| state.completion().is_some());
    same(&visual, &tui, "\n\n     \n     helper.\nfn helper() {}\n");
    assert_eq!(std::fs::read_to_string(path).unwrap(), original, "completion input never saves the file");
}
