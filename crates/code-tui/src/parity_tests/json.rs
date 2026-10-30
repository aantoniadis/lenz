use super::*;
use lenz_content::highlight::Capture;

#[gpui::test]
fn json_format_highlighting_undo_and_save_match_in_both_adapters(cx: &mut TestAppContext) {
    for dark in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let original = "{\"key\":\"text\\n\",\"n\":1e+999,\"key\":true,\"a\":[]}\r\n";
        let formatted = "{\r\n  \"key\": \"text\\n\",\r\n  \"n\": 1e+999,\r\n  \"key\": true,\r\n  \"a\": []\r\n}\r\n";
        let make = |name: &str| {
            let root = dir.path().join(name).join("project");
            std::fs::create_dir_all(&root).unwrap();
            std::fs::write(root.join("data.json"), original).unwrap();
            std::fs::write(root.join(".editorconfig"), "root = true\n[*.json]\nindent_size = 2\nindent_style = space\n").unwrap();
            let mut settings = config(&dir.path().join(name).join("config"));
            settings.dark = dark;
            (settings, root)
        };
        let ((gc, groot), (tc, troot)) = (make("gui"), make("tui"));
        let file = |root: &Path| root.join("data.json");
        let mut core = Session::new(gc);
        core.dispatch(Command::OpenRoot(groot.clone()));
        let gui = cx.new(|cx| ReaderView::new(core, Some(file(&groot)), cx));
        let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| gui.clone())).unwrap();
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        let focus = gui.read_with(&visual, |view, _| view.focus_handle());
        visual.update(|window, cx| focus.focus(window, cx));
        let mut tui = App::new(tc, Some(troot.clone()));
        tui.session.dispatch(Command::Open(file(&troot)));
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
        let current = |state: &State| state.current_source().map(|d| d.copy_range(0..d.len_bytes()).unwrap());
        let settle = |visual: &mut VisualTestContext, tui: &mut App, terminal: &mut ratatui::Terminal<ratatui::backend::TestBackend>, done: &dyn Fn(&State) -> bool| {
            let until = Instant::now() + Duration::from_secs(10);
            loop {
                gui.update(visual, |view, cx| view.poll(cx));
                while let Ok(completion) = tui.session.completions().try_recv() { tui.apply(completion); }
                visual.update(|window, cx| { let _ = window.draw(cx); });
                terminal.draw(|frame| tui.draw(frame)).unwrap();
                if gui.read_with(visual, |view, _| done(view.session().state())) && done(tui.session.state()) { break; }
                assert!(Instant::now() < until, "GUI {:?}; TUI {:?}", gui.read_with(visual, |view, _| view.session().state().notices()), tui.session.state().notices());
                std::thread::sleep(Duration::from_millis(2));
            }
        };
        settle(&mut visual, &mut tui, &mut terminal, &|state| state.highlights().is_some());
        let rows = gui.read_with(&visual, |view, _| view.visible_rows());
        let palette = gui.read_with(&visual, |view, _| **view.session().state().palette());
        let buffer = terminal.backend().buffer().clone();
        let tpalette = **tui.session.state().palette();
        for (token, capture) in [("key", Capture::Property), ("text", Capture::String), ("\\n", Capture::Escape), ("1e+999", Capture::Number), ("true", Capture::ConstantBuiltin), ("{", Capture::Punctuation), (":", Capture::Punctuation)] {
            let row = rows.iter().find(|row| row.text.contains(token)).unwrap();
            let at = row.text.find(token).unwrap();
            assert!(row.fg.iter().any(|(range, color)| range.start <= at && at + token.len() <= range.end && *color == palette.syntax_color(capture)), "GUI {token} {capture:?}, dark={dark}: {:?} {:?}", row.text, row.fg);
            let at = original.find(token).unwrap();
            let hit = tui.layout.panes[0].hits.iter().find(|hit| hit.line == 0 && hit.byte == at).unwrap();
            assert_eq!(buffer[(hit.x, hit.y)].fg, crate::render::color(tpalette.syntax_color(capture)), "TUI {token} {capture:?}, dark={dark}");
        }
        gui.update(&mut visual, |view, _| view.enable_draft_input_for_test());
        tui.enable_draft_input_for_test();
        let key = |tui: &mut App, code, modifiers| { tui.handle(Event::Key(KeyEvent::new(code, modifiers))); };
        visual.simulate_keystrokes("ctrl-alt-l");
        key(&mut tui, KeyCode::Char('l'), KeyModifiers::CONTROL | KeyModifiers::ALT);
        settle(&mut visual, &mut tui, &mut terminal, &|state| current(state).as_deref() == Some(formatted));
        for root in [&groot, &troot] { assert_eq!(std::fs::read_to_string(file(root)).unwrap(), original); }
        visual.simulate_keystrokes("ctrl-z");
        key(&mut tui, KeyCode::Char('z'), KeyModifiers::CONTROL);
        assert_eq!(current(tui.session.state()).as_deref(), Some(original));
        assert_eq!(gui.read_with(&visual, |view, _| current(view.session().state())).as_deref(), Some(original));
        visual.simulate_keystrokes("ctrl-shift-z");
        key(&mut tui, KeyCode::Char('z'), KeyModifiers::CONTROL | KeyModifiers::SHIFT);
        for (width, height) in [(100, 30), (24, 8)] {
            terminal.backend_mut().resize(width, height);
            visual.simulate_resize(gpui::size(gpui::px(f32::from(width) * 9.0), gpui::px(f32::from(height) * 20.0)));
            settle(&mut visual, &mut tui, &mut terminal, &|state| current(state).as_deref() == Some(formatted));
        }
        visual.simulate_keystrokes("ctrl-s");
        key(&mut tui, KeyCode::Char('s'), KeyModifiers::CONTROL);
        settle(&mut visual, &mut tui, &mut terminal, &|_| [&groot, &troot].iter().all(|root| std::fs::read_to_string(file(root)).is_ok_and(|text| text == formatted)));
    }
}
