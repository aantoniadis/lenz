use super::*;
use code_core::session::{Selection, VimMode};

struct Pair {
    _dir: tempfile::TempDir,
    paths: [std::path::PathBuf; 2],
    gui: gpui::Entity<ReaderView>,
    visual: VisualTestContext,
    tui: App,
    terminal: ratatui::Terminal<ratatui::backend::TestBackend>,
}

impl Pair {
    fn new(cx: &mut TestAppContext, source: &str, dark: bool) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let paths = [dir.path().join("gui.txt"), dir.path().join("tui.txt")];
        for path in &paths { std::fs::write(path, source).unwrap(); }
        let mut gc = config(&dir.path().join("gui-config")); gc.dark = dark;
        let mut tc = config(&dir.path().join("tui-config")); tc.dark = dark;
        let gui = cx.new(|cx| ReaderView::new(Session::new(gc), Some(paths[0].clone()), cx));
        let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| gui.clone())).unwrap();
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        let focus = gui.read_with(&visual, |view, _| view.focus_handle());
        visual.update(|window, cx| focus.focus(window, cx));
        let tui = App::new(tc, Some(paths[1].clone()));
        let terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 26)).unwrap();
        let mut p = Self { _dir: dir, paths, gui, visual, tui, terminal };
        p.settle(|s| s.current_source().is_some());
        p.gui.update(&mut p.visual, |view, _| view.enable_draft_input_for_test());
        p.tui.enable_draft_input_for_test();
        p.command(Command::Select(Selection { anchor: 0, head: 0 }));
        p.command(Command::SetVimEnabled(true));
        p.draw(); p
    }
    fn command(&mut self, command: Command) {
        self.gui.update(&mut self.visual, |view, cx| view.dispatch_for_test(command.clone(), cx));
        self.tui.dispatch(command);
    }
    fn draw(&mut self) {
        self.visual.update(|window, cx| { let _ = window.draw(cx); });
        self.terminal.draw(|frame| self.tui.draw(frame)).unwrap();
    }
    fn settle(&mut self, done: impl Fn(&State) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            self.gui.update(&mut self.visual, |view, cx| view.poll(cx));
            while let Ok(done) = self.tui.session.completions().try_recv() { self.tui.apply(done); }
            self.draw();
            if done(self.tui.session.state()) && self.gui.read_with(&self.visual, |view, _| done(view.session().state())) { break; }
            assert!(Instant::now() < deadline, "GUI {}; TUI {}", self.gui.read_with(&self.visual, |view, _| view.status()), self.tui.session.status_details());
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    fn key(&mut self, gui: &str, code: KeyCode, modifiers: KeyModifiers) {
        self.visual.simulate_keystrokes(gui);
        self.tui.handle(Event::Key(KeyEvent::new(code, modifiers)));
    }
    fn chars(&mut self, keys: &str) {
        for c in keys.chars() {
            let gui = if c.is_ascii_uppercase() { format!("shift-{}", c.to_ascii_lowercase()) } else { c.to_string() };
            self.key(&gui, KeyCode::Char(c), if c.is_ascii_uppercase() { KeyModifiers::SHIFT } else { KeyModifiers::NONE });
        }
    }
    fn escape(&mut self) { self.key("escape", KeyCode::Esc, KeyModifiers::NONE); }
    fn enter(&mut self) { self.key("enter", KeyCode::Enter, KeyModifiers::NONE); }
    fn check(&self, expected: &str, mode: VimMode) {
        let state = self.tui.session.state();
        let actual = snapshot(state);
        assert_eq!(actual.text.as_deref(), Some(expected));
        let gui = self.gui.read_with(&self.visual, |view, _| snapshot(view.session().state()));
        // The fixtures use independent files so a save in one adapter cannot influence the other.
        assert_eq!((&actual.text, actual.selection, &actual.selections), (&gui.text, gui.selection, &gui.selections));
        assert_eq!(state.vim_mode(), mode);
        assert_eq!(self.gui.read_with(&self.visual, |view, _| view.session().state().vim_mode()), mode);
    }
    fn check_status(&mut self, mode: &str) {
        self.draw();
        assert!(self.gui.read_with(&self.visual, |view, _| view.status_position().1).contains(mode), "GUI must show {mode}");
        assert!(self.terminal.backend().buffer().content.iter().map(|c| c.symbol()).collect::<String>().contains(mode), "TUI must show {mode}");
    }
}

#[gpui::test]
fn vim_actual_keys_unicode_counts_visual_undo_and_modal_ime_match(cx: &mut TestAppContext) {
    let original = "α one\r\nβ two\r\nγ three";
    for dark in [false, true] {
        let mut p = Pair::new(cx, original, dark);
        p.chars("qjk"); p.check(original, VimMode::Normal);
        let entity = p.gui.clone();
        let mut input = p.visual.update(|window, cx| gpui::PlatformInputHandler::new(window.to_async(cx), Box::new(gpui::ElementInputHandler::new(gpui::Bounds::default(), entity))));
        input.replace_and_mark_text_in_range(None, "blocked preedit", None);
        assert!(input.marked_text_range().is_none(), "Normal mode must not show a preedit");
        input.replace_text_in_range(None, "blocked commit"); input.unmark_text();
        p.check(original, VimMode::Normal);
        p.chars("ggiλ"); p.check("λα one\r\nβ two\r\nγ three", VimMode::Insert);
        p.check_status("INSERT");
        p.escape(); p.check("λα one\r\nβ two\r\nγ three", VimMode::Normal);
        p.chars("u2dd"); p.check("γ three", VimMode::Normal);
        p.chars("P"); p.check(original, VimMode::Normal);
        p.chars("ggv"); p.check(original, VimMode::Visual);
        p.check_status("VISUAL");
        p.chars("ld"); p.check("one\r\nβ two\r\nγ three", VimMode::Normal);
        p.chars("u"); p.check(original, VimMode::Normal);
        p.key("ctrl-r", KeyCode::Char('r'), KeyModifiers::CONTROL);
        p.check("one\r\nβ two\r\nγ three", VimMode::Normal);
        p.chars("u");
        p.key("f10", KeyCode::F(10), KeyModifiers::NONE);
        assert!(p.tui.modal.is_some());
        p.chars("2ddiλ");
        input.replace_text_in_range(None, "modal commit");
        p.escape(); p.check(original, VimMode::Normal);
        p.check_status("NORMAL");
        for path in &p.paths { assert_eq!(std::fs::read_to_string(path).unwrap(), original); }
    }
}

#[gpui::test]
fn vim_colon_save_close_and_dirty_close_prompt_match(cx: &mut TestAppContext) {
    let mut p = Pair::new(cx, "one\r\ntwo", true);
    p.chars("iλ"); p.escape(); p.chars(":w"); p.enter();
    p.settle(|s| s.active().is_some_and(|t| !s.tab_dirty(t.id())));
    let saved = "λone\r\ntwo";
    for path in &p.paths { assert_eq!(std::fs::read_to_string(path).unwrap(), saved); }
    p.chars("i界"); p.escape(); p.chars(":q"); p.enter();
    assert!(p.tui.session.close_prompt().is_some());
    assert!(p.gui.read_with(&p.visual, |view, _| view.session().close_prompt().is_some()));
    p.escape(); p.check("界λone\r\ntwo", VimMode::Normal);
    for path in &p.paths { assert_eq!(std::fs::read_to_string(path).unwrap(), saved); }
    p.chars(":wq"); p.enter(); p.settle(|s| s.active().is_none());
    for path in &p.paths { assert_eq!(std::fs::read_to_string(path).unwrap(), "界λone\r\ntwo"); }
}

#[gpui::test]
fn vim_relative_gutters_keep_current_absolute_and_wrapped_rows_blank(cx: &mut TestAppContext) {
    let original = format!("alpha\nbeta\ngamma {}\ndelta\nepsilon", "word ".repeat(14));
    for dark in [false, true] {
        let mut p = Pair::new(cx, &original, dark);
        let at = original.find("gamma").unwrap();
        p.command(Command::Select(Selection { anchor: at, head: at }));
        p.command(Command::SetRelativeLineNumbers(true));
        for (width, wrapped) in [(90, false), (30, true)] {
            p.terminal.backend_mut().resize(width, 24);
            p.visual.simulate_resize(gpui::size(gpui::px(f32::from(width) * 9.0), gpui::px(480.0)));
            p.command(Command::SetSoftWrap(wrapped));
            p.settle(|_| true);
            // Native wrap layout can arrive after the first frame.
            if wrapped {
                let deadline = Instant::now() + Duration::from_secs(5);
                while !p.tui.layout.panes[0].rows.iter().any(|(_, _, continuation)| *continuation > 0)
                    || !p.gui.read_with(&p.visual, |view, _| view.visible_rows().iter().any(|r| r.number == 0)) {
                    p.settle(|_| true); assert!(Instant::now() < deadline, "wrap rows must be visible");
                }
            }
            let geometry = &p.tui.layout.panes[0];
            let buffer = p.terminal.backend().buffer();
            for &(y, line, continuation) in &geometry.rows {
                let gutter = (geometry.gutter_x..geometry.text.x).map(|x| buffer[(x, y)].symbol()).collect::<String>();
                let digits: String = gutter.chars().filter(char::is_ascii_digit).collect();
                let expected = if continuation > 0 { String::new() } else { [2, 1, 3, 1, 2][line].to_string() };
                assert_eq!(digits, expected, "line={line} continuation={continuation}: {gutter:?}");
            }
            let rows = p.gui.read_with(&p.visual, |view, _| view.visible_rows());
            for (needle, expected) in [("alpha", 2), ("beta", 1), ("gamma", 3)] {
                if let Some(row) = rows.iter().find(|r| r.text.starts_with(needle)) { assert_eq!(row.number, expected); }
            }
            p.check(&original, VimMode::Normal);
        }
    }
}
