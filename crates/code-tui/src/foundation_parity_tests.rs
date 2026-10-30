//! One continuous workflow through production constructors and actual adapter inputs.
use crate::app::App;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use gpui::{ClipboardItem, Entity, TestAppContext, VisualTestContext, px, size};
use code_core::session::{CommandId, Config, Selection, State};
use code_gui::ReaderView;
use std::{
    path::Path,
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

type Terminal = ratatui::Terminal<ratatui::backend::TestBackend>;

fn config(base: &Path, dark: bool) -> Config {
    std::fs::create_dir_all(base).unwrap();
    let providers = base.join("providers.json");
    let ids: serde_json::Map<_, _> = code_core::semantics::provider::PROVIDERS
        .iter()
        .map(|p| {
            (
                p.id.to_owned(),
                serde_json::json!({"executable":format!("/nonexistent/{}",p.id)}),
            )
        })
        .collect();
    std::fs::write(&providers, serde_json::json!({"providers":ids}).to_string()).unwrap();
    Config {
        themes: lenz_content::theme::ThemePaths::under(
            &base.join("config"),
            &base.join("state"),
            "lenz-code",
        ),
        recent: base.join("recent.json"),
        settings: base.join("settings.json"),
        providers,
        cache: base.join("cache"),
        idle: code_core::semantics::IDLE,
        clock: Arc::new(Instant::now),
        overrides: Default::default(),
        dark,
    }
}

fn text(s: &State) -> String {
    let source = s.current_source().unwrap();
    source.copy_range(0..source.len_bytes()).unwrap()
}

#[derive(Debug, PartialEq, Eq)]
struct Observed {
    text: String,
    dirty: bool,
    undo: bool,
    redo: bool,
    positions: Vec<(Option<Selection>, usize, usize)>,
}
fn observe(s: &State) -> Observed {
    let view = s.view(s.active().unwrap().id()).unwrap();
    Observed {
        text: text(s),
        dirty: view.dirty(),
        undo: view.can_undo(),
        redo: view.can_redo(),
        positions: (0..=u8::from(s.split().is_some()))
            .map(|pane| {
                let t = s.shown(pane).unwrap();
                (t.selection(), t.top_line(), t.column())
            })
            .collect(),
    }
}

struct Replay {
    gui: Entity<ReaderView>,
    tui: App,
    terminal: Terminal,
}
impl Replay {
    fn new(cx: &mut TestAppContext, base: &Path, dark: bool) -> (Self, VisualTestContext) {
        let root = base.join("gui");
        let window = cx.update(|cx| {
            code_gui::open_acceptance_window_for_test(
                cx,
                Rc::new(move |_| config(&root, dark)),
                None,
            )
            .unwrap()
        });
        let gui = cx.update(|cx| window.root(cx).unwrap());
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        let focus = gui.read_with(&visual, |v, _| v.focus_handle());
        visual.update(|w, cx| focus.focus(w, cx));
        let mut this = Self {
            gui,
            tui: App::new(config(&base.join("tui"), dark), None),
            terminal: Terminal::new(ratatui::backend::TestBackend::new(100, 24)).unwrap(),
        };
        this.gui.update(&mut visual, |v, cx| {
            v.enable_draft_input_for_test();
            cx.notify();
        });
        this.tui.enable_draft_input_for_test();
        this.draw(&mut visual);
        (this, visual)
    }
    fn draw(&mut self, v: &mut VisualTestContext) {
        v.update(|w, cx| {
            let _ = w.draw(cx);
        });
        self.terminal.draw(|f| self.tui.draw(f)).unwrap();
    }
    fn key(
        &mut self,
        v: &mut VisualTestContext,
        gui: &str,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) {
        self.draw(v);
        v.simulate_keystrokes(gui);
        self.tui.handle(Event::Key(KeyEvent::new(code, modifiers)));
        self.draw(v);
    }
    fn paste(&mut self, v: &mut VisualTestContext, value: &str) {
        self.draw(v);
        v.write_to_clipboard(ClipboardItem::new_string(value.into()));
        v.simulate_keystrokes("ctrl-v");
        self.tui.handle(Event::Paste(value.into()));
        self.draw(v);
    }
    fn settle(&mut self, v: &mut VisualTestContext, ready: impl Fn(&State) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            v.run_until_parked();
            self.gui.update(v, |r, cx| r.poll(cx));
            while let Ok(done) = self.tui.session.completions().try_recv() {
                self.tui.apply(done);
            }
            self.draw(v);
            if self.gui.read_with(v, |r, _| ready(r.session().state()))
                && ready(self.tui.session.state())
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "whole-chain completion: GUI={} TUI={}",
                self.gui.read_with(v, |r, _| r.session().status_message()),
                self.tui.session.status_message()
            );
            std::thread::yield_now();
        }
    }
    fn open(
        &mut self,
        cx: &mut TestAppContext,
        v: &mut VisualTestContext,
        path: &Path,
        tui_path: &Path,
    ) {
        self.key(v, "ctrl-o", KeyCode::Char('o'), KeyModifiers::CONTROL);
        assert!(cx.did_prompt_for_paths());
        cx.simulate_path_prompt_response(|o| {
            assert!(o.files);
            Some(vec![path.to_owned()])
        });
        self.tui
            .handle(Event::Paste(tui_path.to_str().unwrap().into()));
        self.tui.handle(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        self.settle(v, |s| {
            s.current_source()
                .is_some_and(|d| d.source() == path || d.source() == tui_path)
        });
        assert_eq!(
            self.tui.session.state().current_source().unwrap().source(),
            tui_path
        );
        assert!(self.gui.read_with(
            v,
            |r, _| r.session().state().current_source().unwrap().source() == path
        ));
    }
    fn keys(&self, v: &VisualTestContext) -> [code_core::text::SnapshotKey; 2] {
        // Worker reads reserve versions independently; fence identity within each Session.
        [
            self.tui.session.state().current_source().unwrap().key(),
            self.gui.read_with(v, |r, _| {
                r.session().state().current_source().unwrap().key()
            }),
        ]
    }
    fn check(&self, v: &VisualTestContext, expected: &str) -> Observed {
        let t = observe(self.tui.session.state());
        assert_eq!(t.text, expected);
        assert_eq!(
            t,
            self.gui.read_with(v, |r, _| observe(r.session().state()))
        );
        for id in [CommandId::Undo, CommandId::Redo, CommandId::Save] {
            assert_eq!(
                self.tui.session.disabled(id),
                self.gui.read_with(v, |r, _| r.session().disabled(id)),
                "{id:?}"
            );
        }
        t
    }
    fn go_to(&mut self, v: &mut VisualTestContext, input: &str) {
        self.key(v, "ctrl-g", KeyCode::Char('g'), KeyModifiers::CONTROL);
        v.simulate_input(input);
        v.simulate_keystrokes("enter");
        self.tui.handle(Event::Paste(input.into()));
        self.tui.handle(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        self.draw(v);
    }
}

#[gpui::test]
fn gui_and_tui_foundation_whole_chain_keeps_saved_history_and_independent_views(
    cx: &mut TestAppContext,
) {
    let dir = tempfile::tempdir().unwrap();
    // Each adapter owns its file; its independent Session opens that same file.
    let gui_files = dir.path().join("gui-files");
    let tui_files = dir.path().join("tui-files");
    std::fs::create_dir(&gui_files).unwrap();
    std::fs::create_dir(&tui_files).unwrap();
    let path = gui_files.join("main.txt");
    let tui_path = tui_files.join("main.txt");
    let original = "α saved\r\none\nthree\r\n";
    for p in [&path, &tui_path] {
        std::fs::write(p, original).unwrap();
    }
    let (mut primary, mut v) = Replay::new(cx, &dir.path().join("primary"), true);
    let (mut independent, mut other) = Replay::new(cx, &dir.path().join("independent"), false);
    assert_ne!(
        primary.tui.session.state().palette().bg,
        independent.tui.session.state().palette().bg
    );
    // Actual empty constructors must render and accept Undo/Reload/dialog dismissal before opening.
    for (p, visual) in [(&mut primary, &mut v), (&mut independent, &mut other)] {
        p.key(visual, "ctrl-z", KeyCode::Char('z'), KeyModifiers::CONTROL);
        p.key(visual, "f5", KeyCode::F(5), KeyModifiers::NONE);
        p.key(visual, "f10", KeyCode::F(10), KeyModifiers::NONE);
        visual.simulate_resize(size(px(1.), px(1.)));
        p.terminal
            .resize(ratatui::layout::Rect::new(0, 0, 1, 1))
            .unwrap();
        p.draw(visual);
        p.key(visual, "end", KeyCode::End, KeyModifiers::NONE);
        p.key(visual, "up", KeyCode::Up, KeyModifiers::NONE);
        p.key(visual, "escape", KeyCode::Esc, KeyModifiers::NONE);
        assert!(p.tui.session.state().active().is_none());
        assert!(
            p.gui
                .read_with(visual, |r, _| r.session().state().active().is_none())
        );
        visual.simulate_resize(size(px(900.), px(400.)));
        p.terminal
            .resize(ratatui::layout::Rect::new(0, 0, 100, 24))
            .unwrap();
        p.draw(visual);
    }
    independent.open(cx, &mut other, &path, &tui_path);
    independent.key(
        &mut other,
        "ctrl-a",
        KeyCode::Char('a'),
        KeyModifiers::CONTROL,
    );
    independent.paste(&mut other, "independent λ\r\n");
    let held = independent.check(&other, "independent λ\r\n");
    let held_keys = independent.keys(&other);
    primary.open(cx, &mut v, &path, &tui_path);
    primary.key(&mut v, "ctrl-a", KeyCode::Char('a'), KeyModifiers::CONTROL);
    primary.key(&mut v, "λ", KeyCode::Char('λ'), KeyModifiers::NONE);
    primary.paste(&mut v, "😀\r\none\nthree\r\n");
    let pasted = "λ😀\r\none\nthree\r\n";
    assert!(primary.check(&v, pasted).dirty);
    for p in [&path, &tui_path] {
        assert_eq!(std::fs::read(p).unwrap(), original.as_bytes());
    }
    primary.key(
        &mut v,
        "ctrl-\\",
        KeyCode::Char('\\'),
        KeyModifiers::CONTROL,
    );
    primary.key(&mut v, "ctrl-1", KeyCode::Char('1'), KeyModifiers::CONTROL);
    primary.go_to(&mut v, "2:1");
    primary.paste(&mut v, "left ");
    let before_right = "λ😀\r\nleft one\nthree\r\n";
    primary.check(&v, before_right);
    primary.key(&mut v, "ctrl-2", KeyCode::Char('2'), KeyModifiers::CONTROL);
    primary.go_to(&mut v, "3:1");
    primary.paste(&mut v, "right ");
    let final_text = "λ😀\r\nleft one\nright three\r\n";
    let edited = primary.check(&v, final_text);
    assert_eq!(
        edited
            .positions
            .iter()
            .map(|p| p.0.unwrap().head)
            .collect::<Vec<_>>(),
        vec![13, 23]
    );
    let shared = |s: &State| {
        let a = s.view(s.shown(0).unwrap().id()).unwrap();
        let b = s.view(s.shown(1).unwrap().id()).unwrap();
        assert!(Arc::ptr_eq(a.snapshot(), b.snapshot()));
    };
    shared(primary.tui.session.state());
    primary
        .gui
        .read_with(&v, |r, _| shared(r.session().state()));
    primary.key(&mut v, "ctrl-z", KeyCode::Char('z'), KeyModifiers::CONTROL);
    primary.check(&v, before_right);
    primary.key(
        &mut v,
        "ctrl-shift-z",
        KeyCode::Char('z'),
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    );
    assert_eq!(primary.check(&v, final_text).positions, edited.positions);
    primary.key(&mut v, "ctrl-s", KeyCode::Char('s'), KeyModifiers::CONTROL);
    primary.settle(&mut v, |s| s.active().is_some_and(|t| !s.tab_dirty(t.id())));
    let saved = primary.check(&v, final_text);
    let saved_keys = primary.keys(&v);
    assert!(!saved.dirty && saved.undo && !saved.redo);
    assert_eq!(primary.tui.session.disabled(CommandId::Undo), None);
    assert_eq!(
        primary.tui.session.disabled(CommandId::Redo),
        Some("Nothing to redo")
    );
    assert_eq!(
        primary.tui.session.disabled(CommandId::Save),
        Some("No unsaved changes")
    );
    for p in [&path, &tui_path] {
        assert_eq!(std::fs::read(p).unwrap(), final_text.as_bytes());
    }
    // Modal precedence protects the saved snapshot/history in both appearances, including tiny geometry.
    primary.key(&mut v, "shift-f2", KeyCode::F(2), KeyModifiers::SHIFT);
    primary.paste(&mut v, "blocked");
    primary.key(&mut v, "ctrl-s", KeyCode::Char('s'), KeyModifiers::CONTROL);
    primary.key(&mut v, "escape", KeyCode::Esc, KeyModifiers::NONE);
    assert_eq!(primary.check(&v, final_text), saved);
    assert_eq!(primary.keys(&v), saved_keys);
    let clean = gui_files.join("clean.txt");
    let pressure = gui_files.join("pressure.txt");
    let tui_clean = tui_files.join("clean.txt");
    let tui_pressure = tui_files.join("pressure.txt");
    for p in [&clean, &tui_clean] {
        std::fs::write(p, vec![b'a'; 17 << 20]).unwrap();
    }
    for p in [&pressure, &tui_pressure] {
        std::fs::write(p, vec![b'b'; 17 << 20]).unwrap();
    }
    primary.open(cx, &mut v, &clean, &tui_clean);
    // Published marker baselines must obey the same inactive-content budget as documents.
    primary.settle(&mut v, |s| s.active().is_some_and(|tab| s.markers(tab.id()).is_some_and(|m| !m.pending() && m.baseline().is_some())));
    let tui_weak = Arc::downgrade(primary.tui.session.state().current_source().unwrap());
    let gui_weak = primary.gui.read_with(&v, |r, _| {
        Arc::downgrade(r.session().state().current_source().unwrap())
    });
    primary.open(cx, &mut v, &pressure, &tui_pressure);
    primary.open(cx, &mut v, &path, &tui_path);
    primary.settle(&mut v, |_| tui_weak.upgrade().is_none() && gui_weak.upgrade().is_none());
    assert!(
        tui_weak.upgrade().is_none() && gui_weak.upgrade().is_none(),
        "actual clean inactive-cache eviction occurred in both adapters"
    );
    assert_eq!(
        primary.check(&v, final_text),
        saved,
        "saved buffer/history/independent positions survive cache pressure"
    );
    assert_eq!(primary.keys(&v), saved_keys);
    primary.key(&mut v, "ctrl-z", KeyCode::Char('z'), KeyModifiers::CONTROL);
    let undone = primary.check(&v, before_right);
    assert!(undone.dirty && undone.undo && undone.redo);
    assert_eq!(undone.positions[0].0.unwrap().head, 13);
    for p in [&path, &tui_path] {
        assert_eq!(std::fs::read(p).unwrap(), final_text.as_bytes());
    }
    independent.settle(&mut other, |s| s.current_source().is_some());
    assert_eq!(independent.keys(&other), held_keys);
    assert_eq!(
        independent.check(&other, "independent λ\r\n"),
        held,
        "another Session never inherits edits/history/positions or the primary Save"
    );
}
