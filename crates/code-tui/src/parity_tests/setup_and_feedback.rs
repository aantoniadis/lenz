use super::*;
use std::path::PathBuf;

struct Pair {
    gui: gpui::Entity<ReaderView>,
    visual: VisualTestContext,
    tui: App,
    terminal: ratatui::Terminal<ratatui::backend::TestBackend>,
    roots: [PathBuf; 2],
    _dir: tempfile::TempDir,
}
impl Pair {
    fn new(cx: &mut TestAppContext, dark: bool, name: &str, text: &str, provider: Option<(&str, serde_json::Value)>) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let make = |side: &str| {
            let root = dir.path().join(side).join("project"); std::fs::create_dir_all(&root).unwrap();
            std::fs::write(root.join(name), text).unwrap(); std::fs::write(root.join("other.txt"), "other\n").unwrap();
            let mut cfg = config(&dir.path().join(side).join("private")); cfg.dark = dark;
            if let Some((id, value)) = &provider {
                let mut settings: serde_json::Value = serde_json::from_slice(&std::fs::read(&cfg.providers).unwrap()).unwrap();
                settings["providers"][*id] = value.clone(); std::fs::write(&cfg.providers, settings.to_string()).unwrap();
            }
            (cfg, root)
        };
        let ((gc, gr), (tc, tr)) = (make("gui"), make("tui"));
        let mut session = Session::with_text_input(gc, true); session.dispatch(Command::OpenRoot(gr.clone()));
        let gui = cx.new(|cx| ReaderView::new(session, Some(gr.join(name)), cx));
        let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| gui.clone())).unwrap();
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        let focus = gui.read_with(&visual, |v, _| v.focus_handle()); visual.update(|w, cx| focus.focus(w, cx));
        visual.simulate_resize(gpui::size(gpui::px(1000.), gpui::px(600.)));
        let mut tui = App::new(tc, Some(tr.clone())); tui.enable_draft_input_for_test(); tui.dispatch(Command::OpenRetained(tr.join(name))); tui.focus = crate::app::Focus::Text;
        Self { _dir: dir, gui, visual, tui, terminal: ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 32)).unwrap(), roots: [gr, tr] }
    }
    fn wait(&mut self, done: impl Fn(&Session) -> bool) {
        let until = Instant::now() + Duration::from_secs(10);
        loop {
            self.gui.update(&mut self.visual, |v, cx| v.poll(cx));
            while let Ok(completion) = self.tui.session.completions().try_recv() { self.tui.apply(completion); }
            self.visual.update(|w, cx| { let _ = w.draw(cx); }); self.terminal.draw(|f| self.tui.draw(f)).unwrap();
            if self.gui.read_with(&self.visual, |v, _| done(v.session())) && done(&self.tui.session) { break; }
            assert!(Instant::now() < until, "GUI {:?}, TUI {:?}", self.gui.read_with(&self.visual, |v, _| v.session().state().notices()), self.tui.session.state().notices());
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    fn command(&mut self, make: impl Fn(&Session, &Path) -> Command) {
        self.gui.update(&mut self.visual, |v, cx| v.dispatch_for_test(make(v.session(), &self.roots[0]), cx));
        self.tui.dispatch(make(&self.tui.session, &self.roots[1]));
    }
    fn key(&mut self, gui: &str, code: KeyCode, modifiers: KeyModifiers) {
        self.visual.simulate_keystrokes(gui); self.tui.handle(Event::Key(KeyEvent::new(code, modifiers)));
    }
    fn paste(&mut self, text: &str) {
        self.visual.update(|_, cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string(text.into())));
        self.visual.simulate_keystrokes("ctrl-v"); self.tui.handle(Event::Paste(text.into()));
    }
}

#[gpui::test]
fn edited_error_feedback_returns_after_split_tab_switch_in_both_adapters(cx: &mut TestAppContext) {
    for dark in [false, true] {
        let original = "fn main() {}\n"; let broken = "fn main( {\n";
        let mut pair = Pair::new(cx, dark, "main.rs", original, None);
        pair.wait(|s| s.state().highlights().is_some());
        pair.key("ctrl-a", KeyCode::Char('a'), KeyModifiers::CONTROL); pair.paste(broken);
        pair.key("ctrl-home", KeyCode::Home, KeyModifiers::CONTROL);
        pair.wait(|s| s.state().highlights().is_some() && s.state().active().and_then(|t| s.state().diagnostics(t.id())).is_some_and(|d| !d.items.is_empty()));
        pair.command(|_, _| Command::Split(code_core::session::Split::Columns));
        pair.wait(|s| s.state().highlights().is_some());
        pair.command(|_, root| Command::OpenRetained(root.join("other.txt")));
        pair.wait(|s| s.state().current_source().is_some_and(|d| d.source().ends_with("other.txt")));
        pair.command(|s, root| Command::ActivateTab(s.state().tabs().iter().find(|t| t.source() == root.join("main.rs") && t.pane() == s.state().focused_pane()).unwrap().id()));
        pair.wait(|s| s.state().highlights().is_some());
        let until = Instant::now() + Duration::from_secs(10);
        while pair.gui.read_with(&pair.visual, |v, _| v.problem_marks()).is_empty() {
            pair.wait(|_| true);
            assert!(Instant::now() < until, "returned source must paint its diagnostics");
            std::thread::sleep(Duration::from_millis(2));
        }
        let palette = **pair.tui.session.state().palette();
        assert!(!pair.gui.read_with(&pair.visual, |v, _| v.problem_marks()).is_empty(), "GUI {:?}", pair.gui.read_with(&pair.visual, |v, _| (v.visible_rows().iter().map(|r| r.text.clone()).collect::<Vec<_>>(), v.session().state().active().and_then(|t| v.session().state().diagnostics(t.id())).map(|d| d.items.clone()))));
        // visible_rows and problem_marks describe the active pane; inspect each view explicitly.
        for pane in [0, 1] {
            pair.command(|_, _| Command::FocusPane(pane));
            pair.wait(|s| s.state().highlights().is_some());
            let rows = pair.gui.read_with(&pair.visual, |v, _| v.visible_rows());
            assert_eq!(rows.iter().filter(|r| r.text.starts_with("fn main") && r.fg.iter().any(|(_, c)| *c == palette.syntax_color(lenz_content::highlight::Capture::Keyword))).count(), 1, "pane {pane}");
        }
        for pane in &pair.tui.layout.panes {
            let first = pane.hits.iter().find(|hit| hit.line == 0 && hit.byte == 0).unwrap();
            let state = pair.tui.session.state();
            let view = state.pane_tabs(pane.pane).find_map(|tab| state.view(tab.id())).unwrap();
            assert_eq!(view.style_at(0, 0).0, Some(lenz_content::highlight::Capture::Keyword));
            let cell = &pair.terminal.backend().buffer()[(first.x, first.y)];
            assert_eq!(cell.fg, crate::render::color(palette.error));
            assert!(cell.modifier.contains(ratatui::style::Modifier::UNDERLINED));
        }
        for (width, height) in [(40, 12), (12, 4), (120, 32)] {
            pair.terminal.backend_mut().resize(width, height); pair.terminal.draw(|f| pair.tui.draw(f)).unwrap();
            pair.visual.simulate_resize(gpui::size(gpui::px(f32::from(width) * 9.), gpui::px(f32::from(height) * 20.)));
            pair.visual.update(|w, cx| { let _ = w.draw(cx); });
        }
        for root in &pair.roots { assert_eq!(std::fs::read_to_string(root.join("main.rs")).unwrap(), original); }
    }
}

#[cfg(unix)]
#[gpui::test]
fn missing_server_setup_chooses_node_then_installs_from_actual_inputs_in_both_adapters(cx: &mut TestAppContext) {
    use std::os::unix::fs::PermissionsExt;
    let tools = tempfile::tempdir().unwrap();
    for (name, script) in [
        ("node", "#!/bin/sh\ncase \"$1\" in --version) echo v24.0.0;; --check) exit 0;; *) exec \"$@\";; esac\n"),
        ("npm", "#!/bin/sh\n/bin/mkdir -p \"$3/node_modules/bash-language-server/out\" \"$3/node_modules/.bin\"\nprintf '%s' '{\"name\":\"bash-language-server\",\"version\":\"5.8.1\"}' > \"$3/node_modules/bash-language-server/package.json\"\nprintf '%s' 'console.log(1)' > \"$3/node_modules/bash-language-server/out/cli.js\"\n/bin/ln -sf ../bash-language-server/out/cli.js \"$3/node_modules/.bin/bash-language-server\"\n/bin/chmod +x \"$3/node_modules/bash-language-server/out/cli.js\"\n"),
    ] { let path = tools.path().join(name); std::fs::write(&path, script).unwrap(); std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap(); }
    let mut pair = Pair::new(cx, true, "script.sh", "echo unchanged\n", Some(("bash-language-server", serde_json::json!({"env":{"PATH":"/nonexistent"}}))));
    pair.wait(|s| s.state().provider().is_some_and(|p| matches!(p.status, code_core::semantics::Status::Missing(_))));
    pair.key("f2", KeyCode::F(2), KeyModifiers::NONE);
    pair.key("enter", KeyCode::Enter, KeyModifiers::NONE);
    pair.wait(|s| s.state().server_install().is_some());
    assert!(pair.tui.session.state().server_install().unwrap().preview.disabled.is_some());
    pair.key("r", KeyCode::Char('r'), KeyModifiers::NONE);
    pair.wait(|s| s.state().server_runtime().is_some());
    pair.paste(tools.path().join("node").to_str().unwrap());
    pair.key("enter", KeyCode::Enter, KeyModifiers::NONE);
    pair.wait(|s| s.state().server_runtime().is_some_and(|r| r.message.contains("runtime saved")));
    pair.key("escape", KeyCode::Esc, KeyModifiers::NONE);
    pair.wait(|s| s.state().server_install().is_some_and(|m| m.preview.disabled.is_none()));
    pair.key("enter", KeyCode::Enter, KeyModifiers::NONE);
    pair.wait(|s| s.state().server_install().is_some_and(|m| m.finished));
    assert!(pair.tui.session.state().server_install().unwrap().message.contains("installed and validated"), "{} {}", pair.tui.session.state().server_install().unwrap().message, pair.tui.session.state().server_install().unwrap().output);
    assert!(pair.gui.read_with(&pair.visual, |v, _| v.session().state().server_install().unwrap().message.contains("installed and validated")));
    pair.key("escape", KeyCode::Esc, KeyModifiers::NONE);
    for root in &pair.roots { assert_eq!(std::fs::read_to_string(root.join("script.sh")).unwrap(), "echo unchanged\n"); }
}
