//! Replay actual keys in both adapters and compare the resulting semantic state and source bytes.
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use gpui::{AppContext, TestAppContext, VisualTestContext};
use reader_core::session::{Command, CommandId, Config, Session, State};
use reader_gui::ReaderView;
use std::{
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

use crate::app::App;

fn config(base: &Path) -> Config {
    let providers = base.join("providers.json");
    std::fs::create_dir_all(base).unwrap();
    let providers_json: serde_json::Map<_, _> = reader_core::semantics::provider::PROVIDERS
        .iter()
        .map(|p| {
            (
                p.id.to_owned(),
                serde_json::json!({"executable": format!("/nonexistent/{}", p.id)}),
            )
        })
        .collect();
    std::fs::write(
        &providers,
        serde_json::json!({"providers": providers_json}).to_string(),
    )
    .unwrap();
    Config {
        themes: lenz_content::theme::ThemePaths::under(
            &base.join("config"),
            &base.join("state"),
            "lenz-reader",
        ),
        recent: base.join("state/recent.json"),
        settings: base.join("settings.json"),
        providers,
        cache: base.join("cache"),
        idle: reader_core::semantics::IDLE,
        clock: Arc::new(Instant::now),
        overrides: Default::default(),
        dark: true,
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Snapshot {
    tabs: Vec<(u8, String, bool)>,
    active: Option<String>,
    text: Option<String>,
    selection: Option<reader_core::session::Selection>,
    split: Option<reader_core::session::Split>,
    pane: u8,
    selected: Option<String>,
    find: Option<(String, usize, Option<usize>)>,
    wrap: bool,
}

fn snapshot(s: &State) -> Snapshot {
    Snapshot {
        tabs: s
            .tabs()
            .iter()
            .map(|t| (t.pane(), t.title(), t.retained()))
            .collect(),
        active: s.active().map(|t| t.title()),
        text: s.document().map(|d| d.text().to_string()),
        selection: s.selection(),
        selected: s.selected_text().map(str::to_owned),
        split: s.split(),
        pane: s.focused_pane(),
        wrap: s.soft_wrap(),
        find: s
            .active()
            .and_then(|t| t.find())
            .map(|f| (f.query().into(), f.matches().len(), f.current())),
    }
}

#[gpui::test]
fn markdown_header_buttons_target_their_own_pane_and_respect_modals(cx: &mut TestAppContext) {
    use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    use reader_core::session::{Rendering, Split};
    for dark in [true, false] {
        for split in [Split::Columns, Split::Rows] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("a-long-markdown-filename-to-clip-in-the-header.md");
            let bytes = "# Title\n\nSome **Markdown**.\n";
            std::fs::write(&path, bytes).unwrap();
            let mut gc = config(&dir.path().join("gui")); gc.dark = dark;
            let mut tc = config(&dir.path().join("tui")); tc.dark = dark;
            let gui = cx.new(|cx| ReaderView::new(Session::new(gc), Some(path.clone()), cx));
            let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| gui.clone())).unwrap();
            let mut visual = VisualTestContext::from_window(window.into(), cx);
            let focus = gui.read_with(&visual, |v, _| v.focus_handle());
            visual.update(|w, cx| focus.focus(w, cx));
            let mut tui = App::new(tc, Some(path.clone()));
            let settle = |visual: &mut VisualTestContext, tui: &mut App| {
                let deadline = Instant::now() + Duration::from_secs(5);
                loop {
                    gui.update(visual, |v, cx| v.poll(cx));
                    while let Ok(c) = tui.session.completions().try_recv() { tui.apply(c); }
                    let ready = |s: &State| s.document().is_some() && (0..2).all(|pane| !matches!(s.shown(pane).and_then(|t| t.rendering()), Some(Rendering::Pending)));
                    if gui.read_with(visual, |v, _| ready(v.session().state())) && ready(tui.session.state()) { break; }
                    assert!(Instant::now() < deadline);
                    std::thread::sleep(Duration::from_millis(2));
                }
            };
            let draw = |visual: &mut VisualTestContext, tui: &mut App, w: f32, h: f32, cols: u16, rows: u16| {
                visual.simulate_resize(gpui::size(gpui::px(w), gpui::px(h)));
                visual.update(|w, cx| { let _ = w.draw(cx); });
                let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(cols, rows)).unwrap();
                terminal.draw(|f| tui.draw(f)).unwrap();
                if tui.modal.is_none() {
                    for pane in &tui.layout.panes {
                        for (_, b) in &pane.controls {
                            let text: String = (b.x..b.right()).map(|x| terminal.backend().buffer()[(x, b.y)].symbol()).collect();
                            assert!(text.contains(tui.session.preview_control(pane.pane).unwrap().label));
                        }
                    }
                }
            };
            let click = |visual: &mut VisualTestContext, tui: &mut App, pane| {
                let point = gui.read_with(visual, |v, _| {
                    let g = *v.panes.iter().find(|g| g.pane == pane).unwrap();
                    let (button, _) = v.preview_button(g).unwrap();
                    assert!(button.left() >= gpui::px(g.bar.0) && button.right() <= gpui::px(g.bar.1));
                    button.center()
                });
                visual.simulate_click(point, gpui::Modifiers::default());
                let g = tui.layout.panes.iter().find(|p| p.pane == pane).unwrap();
                let (_, b) = g.controls.iter().find(|(id, _)| *id == CommandId::MarkdownSource).unwrap();
                assert!(g.tabs.iter().all(|(_, tab)| tab.right() <= b.x));
                tui.handle(Event::Mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: b.x + 1, row: b.y, modifiers: KeyModifiers::NONE }));
            };
            settle(&mut visual, &mut tui);
            gui.update_in(&mut visual, |v, w, cx| v.run(if split == Split::Columns { CommandId::SplitRight } else { CommandId::SplitDown }, w, cx));
            tui.dispatch(Command::Split(split));
            for pane in [0, 1, 0, 1] {
                // Force focus elsewhere: the clicked header must choose the target before toggling.
                visual.simulate_keystrokes(if pane == 0 { "ctrl-2" } else { "ctrl-1" });
                tui.dispatch(Command::FocusPane(1 - pane));
                draw(&mut visual, &mut tui, 900., 500., 100, 30);
                assert!(gui.read_with(&visual, |v, _| v.nav_buttons().iter().all(|b| b.2 != CommandId::MarkdownSource)));
                let before: Vec<_> = (0..2).map(|p| tui.session.state().shown(p).unwrap().markdown_source()).collect();
                click(&mut visual, &mut tui, pane);
                settle(&mut visual, &mut tui);
                for p in 0..2 {
                    let expected = if p == pane { !before[p as usize] } else { before[p as usize] };
                    assert_eq!(tui.session.state().shown(p).unwrap().markdown_source(), expected);
                    assert_eq!(gui.read_with(&visual, |v, _| v.session().state().shown(p).unwrap().markdown_source()), expected);
                }
                assert_eq!(tui.session.state().focused_pane(), pane);
                assert_eq!(gui.read_with(&visual, |v, _| v.session().state().focused_pane()), pane);
            }
            draw(&mut visual, &mut tui, 900., 500., 100, 30);
            visual.simulate_keystrokes("f2");
            tui.handle(Event::Key(KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE)));
            draw(&mut visual, &mut tui, 900., 500., 100, 30);
            click(&mut visual, &mut tui, 0); // dismisses the modal; never reaches the header underneath
            assert!((0..2).all(|p| tui.session.state().shown(p).unwrap().markdown_source()));
            assert!(gui.read_with(&visual, |v, _| (0..2).all(|p| v.session().state().shown(p).unwrap().markdown_source())));
            visual.simulate_keystrokes("escape");
            tui.handle(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
            for (w, h, cols, rows) in [(380., 180., 34, 12), (120., 50., 8, 4), (1., 1., 1, 1), (900., 500., 100, 30)] {
                draw(&mut visual, &mut tui, w, h, cols, rows);
                gui.read_with(&visual, |v, _| {
                    for g in &v.panes {
                        if let Some((b, _)) = v.preview_button(*g) { assert!(b.left() >= gpui::px(0.) && b.right() <= gpui::px(w)); }
                    }
                });
            }
            assert_eq!(std::fs::read_to_string(&path).unwrap(), bytes);
        }
    }
}

#[gpui::test]
fn gui_and_terminal_project_lsp_actions_save_and_reset_the_same_scope(cx: &mut TestAppContext) {
    for dark in [true, false] {
        let scratch = tempfile::tempdir().unwrap();
        let root = scratch.path().join("rails");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("Gemfile"), "").unwrap();
        let source = "class Application; end\n";
        let path = root.join("application.rb");
        std::fs::write(&path, source).unwrap();
        let mut gc = config(&scratch.path().join("gui"));
        let mut tc = config(&scratch.path().join("tui"));
        gc.dark = dark;
        tc.dark = dark;
        let gui_file = gc.providers.clone();
        let tui_file = tc.providers.clone();
        let gui = cx.new(|cx| ReaderView::new(Session::new(gc), Some(path.clone()), cx));
        let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| gui.clone())).unwrap();
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        let focus = gui.read_with(&visual, |v, _| v.focus_handle());
        visual.update(|w, cx| focus.focus(w, cx));
        let mut tui = App::new(tc, Some(path.clone()));
        let key = |app: &mut App, code| { app.handle(Event::Key(KeyEvent::new(code, KeyModifiers::NONE))); };
        let settle = |visual: &mut VisualTestContext, tui: &mut App, configured: bool, excluded: bool| {
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                gui.update(visual, |v, cx| v.poll(cx));
                while let Ok(c) = tui.session.completions().try_recv() { tui.apply(c); }
                let done = |s: &Session| s.state().provider().is_some_and(|v| matches!(v.status, reader_core::semantics::Status::Missing(_)))
                    && s.disabled(CommandId::ExcludeRubyTooling).is_none()
                    && s.disabled(CommandId::ResetProjectServerSettings).is_none() == configured
                    && s.actions().iter().find(|a| a.id == CommandId::ExcludeRubyTooling).and_then(|a| a.on) == Some(excluded);
                if gui.read_with(visual, |v, _| done(v.session())) && done(&tui.session) { break; }
                assert!(Instant::now() < deadline, "GUI {:?}; TUI {:?}", gui.read_with(visual, |v, _| v.session().state().notices()), tui.session.state().notices());
                std::thread::sleep(Duration::from_millis(2));
            }
            assert_eq!(gui.read_with(visual, |v, _| snapshot(v.session().state())), snapshot(tui.session.state()));
            assert_eq!(std::fs::read_to_string(&path).unwrap(), source);
        };
        settle(&mut visual, &mut tui, false, true);
        // Both adapters open the same frozen report from the first F2 action, then copy it after scrolling.
        visual.simulate_keystrokes("f2 enter");
        key(&mut tui, KeyCode::F(2)); key(&mut tui, KeyCode::Enter);
        for (width, height) in [(26, 8), (1, 1), (80, 24)] {
            let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
            terminal.draw(|f| tui.draw(f)).unwrap();
            key(&mut tui, KeyCode::End); key(&mut tui, KeyCode::Up);
        }
        for (width, height) in [(180., 90.), (1., 1.), (800., 400.)] {
            visual.simulate_resize(gpui::size(gpui::px(width), gpui::px(height)));
            visual.simulate_keystrokes("end up");
            visual.update(|w, cx| { let _ = w.draw(cx); });
        }
        visual.simulate_keystrokes("ctrl-c");
        tui.handle(Event::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)));
        let copied = visual.update(|_, cx| cx.read_from_clipboard().and_then(|c| c.text())).unwrap();
        let normalize = |s: &str| s.replace(&gui_file.display().to_string(), "providers.json").replace(&tui_file.display().to_string(), "providers.json");
        assert_eq!(normalize(&copied), normalize(tui.copy.as_deref().unwrap()));
        assert!(copied.contains("Ruby tooling gem exclusions: enabled") && copied.contains("Workspace:"));
        visual.simulate_keystrokes("escape"); key(&mut tui, KeyCode::Esc);
        assert!(tui.modal.is_none());
        settle(&mut visual, &mut tui, false, true);
        for (id, excluded) in [(CommandId::ExcludeRubyTooling, false), (CommandId::ExcludeRubyTooling, true), (CommandId::ResetProjectServerSettings, true)] {
            visual.simulate_keystrokes("f2 home");
            key(&mut tui, KeyCode::F(2));
            key(&mut tui, KeyCode::Home);
            for (width, height) in [(26, 8), (1, 1), (80, 24)] {
                let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
                terminal.draw(|f| tui.draw(f)).unwrap();
            }
            for (width, height) in [(180., 90.), (1., 1.), (800., 400.)] {
                visual.simulate_resize(gpui::size(gpui::px(width), gpui::px(height)));
                visual.update(|w, cx| { let _ = w.draw(cx); });
            }
            let at = tui.session.language_server_rows().iter().position(|(_, action)| *action == Some(id)).unwrap();
            for _ in 0..at { visual.simulate_keystrokes("down"); key(&mut tui, KeyCode::Down); }
            visual.simulate_keystrokes("enter");
            key(&mut tui, KeyCode::Enter);
            settle(&mut visual, &mut tui, id == CommandId::ExcludeRubyTooling, excluded);
            let g: serde_json::Value = serde_json::from_slice(&std::fs::read(&gui_file).unwrap()).unwrap();
            let t: serde_json::Value = serde_json::from_slice(&std::fs::read(&tui_file).unwrap()).unwrap();
            assert_eq!(g, t);
            let ruby = &g["providers"]["ruby-lsp"];
            assert!(ruby.get("settings").is_none(), "global settings were changed");
            if id == CommandId::ExcludeRubyTooling {
                assert_eq!(ruby["projects"][root.to_str().unwrap()]["indexing"]["excludedGems"].as_array().unwrap().contains(&serde_json::json!("parser")), excluded);
                let details = tui.session.language_server_details();
                assert!(details.contains("Project settings for") && details.contains("project overrides"));
                assert!(details.contains(if excluded { "Ruby tooling gem exclusions: enabled" } else { "Ruby tooling gem exclusions: not applied" }));
            } else { assert!(ruby.get("projects").is_none()); }
        }
    }
}

#[gpui::test]
fn gui_and_terminal_index_settings_and_rebuild_are_accessible_through_f2(cx: &mut TestAppContext) {
    for dark in [true, false] {
        let scratch = tempfile::tempdir().unwrap();
        let root = scratch.path().join("project"); std::fs::create_dir(&root).unwrap();
        let path = root.join("a.rs"); let source = "struct Alpha;\n"; std::fs::write(&path, source).unwrap();
        let mut gc = config(&scratch.path().join("gui")); let mut tc = config(&scratch.path().join("tui"));
        gc.dark = dark; tc.dark = dark;
        let gs = gc.settings.clone(); let ts = tc.settings.clone();
        let mut core = Session::new(gc); core.dispatch(Command::OpenRoot(root.clone()));
        let gui = cx.new(|cx| ReaderView::new(core, Some(path.clone()), cx));
        let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| gui.clone())).unwrap();
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        let focus = gui.read_with(&visual, |v, _| v.focus_handle()); visual.update(|w, cx| focus.focus(w, cx));
        let mut tui = App::new(tc, Some(root.clone()));
        tui.session.dispatch(Command::Open(path.clone()));
        let key = |app: &mut App, code| { app.handle(Event::Key(KeyEvent::new(code, KeyModifiers::NONE))); };
        for (id, enabled) in [(None, true), (Some(CommandId::PersistentIndex), false), (Some(CommandId::PersistentIndex), true), (Some(CommandId::RebuildIndex), true)] {
            if let Some(id) = id {
                visual.simulate_keystrokes("f2 home"); key(&mut tui, KeyCode::F(2)); key(&mut tui, KeyCode::Home);
                for (width, height) in [(26, 8), (1, 1), (80, 24)] {
                    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap(); terminal.draw(|f| tui.draw(f)).unwrap();
                }
                for (width, height) in [(180., 90.), (1., 1.), (800., 400.)] {
                    visual.simulate_resize(gpui::size(gpui::px(width), gpui::px(height))); visual.update(|w, cx| { let _ = w.draw(cx); });
                }
                let at = tui.session.language_server_rows().iter().position(|(_, action)| *action == Some(id)).unwrap();
                for _ in 0..at { visual.simulate_keystrokes("down"); key(&mut tui, KeyCode::Down); }
                visual.simulate_keystrokes("enter"); key(&mut tui, KeyCode::Enter);
            }
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                gui.update(&mut visual, |v, cx| v.poll(cx));
                while let Ok(c) = tui.session.completions().try_recv() { tui.apply(c); }
                let done = |s: &Session| s.state().index_coverage().indexed == 1 && s.state().index_coverage().complete()
                    && s.actions().iter().find(|a| a.id == CommandId::PersistentIndex).and_then(|a| a.on) == Some(enabled)
                    && s.disabled(CommandId::RebuildIndex).is_none()
                    && (id != Some(CommandId::RebuildIndex) || s.state().notices().iter().any(|n| n.contains("Saved declarations cleared")));
                if gui.read_with(&visual, |v, _| done(v.session())) && done(&tui.session) { break; }
                assert!(Instant::now() < deadline, "index setting/rebuild did not settle: {:?}", tui.session.state().notices());
                std::thread::sleep(Duration::from_millis(2));
            }
            assert_eq!(gui.read_with(&visual, |v, _| snapshot(v.session().state())), snapshot(tui.session.state()));
            assert_eq!(std::fs::read_to_string(&path).unwrap(), source);
            if id.is_some() {
                let deadline = Instant::now() + Duration::from_secs(5);
                loop {
                    let saved = |p: &std::path::PathBuf| std::fs::read(p).ok().and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok()).is_some_and(|v| v["persistent_index"] == enabled);
                    if saved(&gs) && saved(&ts) { break; }
                    assert!(Instant::now() < deadline); std::thread::sleep(Duration::from_millis(2));
                }
            }
        }
    }
}

#[gpui::test]
fn gui_and_terminal_keys_share_selection_find_split_reload_and_modal_outcomes(
    cx: &mut TestAppContext,
) {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path().join("project");
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("a.txt");
    let original = "héllo\r\nneedle here\r\nother needle\r\n";
    std::fs::write(&path, original).unwrap();
    let gui = cx.new(|cx| {
        ReaderView::new(
            Session::new(config(&scratch.path().join("gui"))),
            Some(path.clone()),
            cx,
        )
    });
    let window = cx
        .update(|cx| cx.open_window(Default::default(), |_, _| gui.clone()))
        .unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let focus = gui.read_with(&visual, |v, _| v.focus_handle());
    visual.update(|w, cx| focus.focus(w, cx));
    let mut tui = App::new(config(&scratch.path().join("tui")), Some(path.clone()));
    let key = |app: &mut App, code, modifiers| {
        app.handle(Event::Key(KeyEvent::new(code, modifiers)));
    };
    let settle = |visual: &mut VisualTestContext, tui: &mut App| {
        let until = Instant::now() + Duration::from_secs(10);
        loop {
            gui.update(visual, |v, cx| v.poll(cx));
            while let Ok(c) = tui.session.completions().try_recv() {
                tui.apply(c);
            }
            let ready = |s: &State| {
                s.loading().is_none()
                    && s.document().is_some()
                    && s.active()
                        .and_then(|t| t.find())
                        .is_none_or(|f| !f.pending())
            };
            if gui.read_with(visual, |v, _| ready(v.session().state()))
                && ready(tui.session.state())
            {
                break;
            }
            assert!(Instant::now() < until, "adapters did not settle");
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(
            gui.read_with(visual, |v, _| snapshot(v.session().state())),
            snapshot(tui.session.state())
        );
        assert_eq!(std::fs::read(&path).unwrap(), original.as_bytes());
    };
    settle(&mut visual, &mut tui);
    visual.simulate_keystrokes("ctrl-a");
    key(&mut tui, KeyCode::Char('a'), KeyModifiers::CONTROL);
    settle(&mut visual, &mut tui);
    assert_eq!(tui.session.state().selected_text(), Some(original));

    // Both tab bars expose the same navigation, and actual right-clicks expose the same context actions.
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
    terminal.draw(|f| tui.draw(f)).unwrap();
    visual.update(|window, cx| {
        let _ = window.draw(cx);
    });
    let gui_nav: Vec<_> = gui.read_with(&visual, |v, _| {
        v.nav_buttons().iter().map(|b| b.2).collect()
    });
    assert_eq!(
        tui.layout
            .controls
            .iter()
            .filter(|(_, rect)| rect.y == 0)
            .map(|(id, _)| *id)
            .collect::<Vec<_>>(),
        gui_nav
    );
    visual.simulate_event(gpui::MouseDownEvent {
        button: gpui::MouseButton::Right,
        position: gpui::point(gpui::px(60.0), gpui::px(70.0)),
        modifiers: gpui::Modifiers::default(),
        click_count: 1,
        first_mouse: false,
    });
    let text = tui.layout.panes[0].text;
    tui.handle(Event::Mouse(crossterm::event::MouseEvent {
        kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Right),
        column: text.x + 1,
        row: text.y,
        modifiers: KeyModifiers::NONE,
    }));
    let gui_rows = gui.read_with(&visual, |v, _| {
        v.modal_rows()
            .into_iter()
            .filter(|r| r.0 != "—")
            .collect::<Vec<_>>()
    });
    assert!(!gui_rows.is_empty());
    assert_eq!(
        tui.choices()
            .into_iter()
            .map(|c| (c.label, c.disabled))
            .collect::<Vec<_>>(),
        gui_rows
    );
    visual.update(|window, cx| {
        let _ = window.draw(cx);
    });
    let (bounds, row) = gui.read_with(&visual, |v, _| {
        (
            v.context_bounds().unwrap(),
            v.context_row_y("Copy").unwrap(),
        )
    });
    visual.simulate_click(
        gpui::point(bounds.origin.x + gpui::px(5.0), gpui::px(row + 1.0)),
        gpui::Modifiers::default(),
    );
    terminal.draw(|f| tui.draw(f)).unwrap();
    let copy = tui
        .choices()
        .iter()
        .position(|c| c.label == "Copy")
        .unwrap();
    let row = tui
        .layout
        .modal_items
        .iter()
        .position(|i| *i == copy)
        .unwrap();
    tui.handle(Event::Mouse(crossterm::event::MouseEvent {
        kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
        column: tui.layout.modal_rows.x + 1,
        row: tui.layout.modal_rows.y + row as u16,
        modifiers: KeyModifiers::NONE,
    }));
    assert_eq!(
        visual
            .read_from_clipboard()
            .and_then(|c| c.text())
            .as_deref(),
        Some(original)
    );
    assert_eq!(tui.copy.take().as_deref(), Some(original));
    assert!(tui.modal.is_none());
    settle(&mut visual, &mut tui);

    visual.simulate_keystrokes("f2 ctrl-c");
    key(&mut tui, KeyCode::F(2), KeyModifiers::NONE);
    key(&mut tui, KeyCode::Char('c'), KeyModifiers::CONTROL);
    let gui_details = visual.read_from_clipboard().and_then(|c| c.text()).unwrap();
    let tui_details = tui.copy.take().unwrap();
    assert!(gui_details.contains("No language server supports this document"));
    assert_eq!(
        gui_details.split("\n\nProvider settings:").next(),
        tui_details.split("\n\nProvider settings:").next()
    );
    visual.simulate_keystrokes("escape");
    key(&mut tui, KeyCode::Esc, KeyModifiers::NONE);
    settle(&mut visual, &mut tui);

    visual.simulate_keystrokes("ctrl-f");
    key(&mut tui, KeyCode::Char('f'), KeyModifiers::CONTROL);
    for c in "needle".chars() {
        visual.simulate_keystrokes(&c.to_string());
        key(&mut tui, KeyCode::Char(c), KeyModifiers::NONE);
    }
    settle(&mut visual, &mut tui);
    assert_eq!(
        tui.session
            .state()
            .active()
            .unwrap()
            .find()
            .unwrap()
            .matches()
            .len(),
        2
    );
    visual.simulate_keystrokes("f3");
    key(&mut tui, KeyCode::F(3), KeyModifiers::NONE);
    settle(&mut visual, &mut tui);
    // GUI's find box and terminal find prompt dismiss through the shared close command.
    visual.simulate_keystrokes("escape");
    key(&mut tui, KeyCode::Esc, KeyModifiers::NONE);
    settle(&mut visual, &mut tui);

    visual.simulate_keystrokes("f10");
    key(&mut tui, KeyCode::F(10), KeyModifiers::NONE);
    let before = snapshot(tui.session.state());
    visual.simulate_keystrokes("ctrl-a f5");
    key(&mut tui, KeyCode::Char('a'), KeyModifiers::CONTROL);
    key(&mut tui, KeyCode::F(5), KeyModifiers::NONE);
    assert_eq!(snapshot(tui.session.state()), before);
    visual.simulate_keystrokes("escape");
    key(&mut tui, KeyCode::Esc, KeyModifiers::NONE);
    settle(&mut visual, &mut tui);

    for id in [
        CommandId::SplitRight,
        CommandId::MoveTabToOtherPane,
        CommandId::Unsplit,
        CommandId::SoftWrap,
        CommandId::Reload,
        CommandId::OpenInEditor,
        CommandId::OpenWithDefaultApp,
    ] {
        // Reach the same action with the same keys through both adapters' application menus.
        visual.simulate_click(
            gpui::point(gpui::px(12.0), gpui::px(10.0)),
            gpui::Modifiers::default(),
        );
        terminal.draw(|f| tui.draw(f)).unwrap();
        let button = tui.layout.app_menu;
        tui.handle(Event::Mouse(crossterm::event::MouseEvent {
            kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
            column: button.x + 1,
            row: button.y,
            modifiers: KeyModifiers::NONE,
        }));
        let group = reader_core::actions::CommandGroup::ALL
            .iter()
            .position(|g| *g == id.group())
            .unwrap();
        visual.simulate_keystrokes(&format!("{}enter", "down ".repeat(group)));
        for _ in 0..group {
            key(&mut tui, KeyCode::Down, KeyModifiers::NONE);
        }
        key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
        let gui_rows = gui.read_with(&visual, |v, _| v.modal_rows());
        let tui_rows: Vec<_> = tui.choices().into_iter().map(|c| (c.label, c.disabled)).collect();
        assert_eq!(tui_rows, gui_rows, "{id:?}");
        let row = CommandId::ALL
            .iter()
            .filter(|c| c.group() == id.group())
            .position(|c| *c == id)
            .unwrap();
        visual.simulate_keystrokes(&format!("{}enter", "down ".repeat(row)));
        for _ in 0..row {
            key(&mut tui, KeyCode::Down, KeyModifiers::NONE);
        }
        key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
        settle(&mut visual, &mut tui);
    }
    // Both handed the same editor and default-app launches over (recorded, not started).
    assert_eq!(tui.launched.len(), 2);
    assert_eq!(gui.read_with(&visual, |v, _| v.launched.clone()), tui.launched);
    // A pending navigation can be cancelled in either adapter without changing the read-only document.
    tui.dispatch(Command::CancelSemantic);
    assert_eq!(std::fs::read(&path).unwrap(), original.as_bytes());
}

#[gpui::test]
#[cfg(unix)]
fn gui_and_tui_server_runtime_repairs_produce_the_same_settings(cx: &mut TestAppContext) {
    use std::os::unix::fs::PermissionsExt;
    for (name, source, runtime_name, flag, version, provider) in [
        (
            "Main.java",
            "class Main {}\n",
            "java",
            "-version",
            "",
            "jdtls",
        ),
        (
            "main.py",
            "def main(): pass\n",
            "node",
            "--version",
            "v26.8.1",
            "pyright",
        ),
    ] {
        let scratch = tempfile::tempdir().unwrap();
        let path = scratch.path().join(name);
        std::fs::write(&path, source).unwrap();
        let java = scratch
            .path()
            .join("runtime with spaces/bin")
            .join(runtime_name);
        std::fs::create_dir_all(java.parent().unwrap()).unwrap();
        std::fs::write(
            &java,
            format!("#!/bin/sh\ntest \"$1\" = {flag} || exit 2\nprintf '%s\\n' '{version}'\n"),
        )
        .unwrap();
        if provider == "pyright" {
            let server = java.parent().unwrap().join("pyright-langserver");
            std::fs::write(&server, "#!/bin/sh\nexit 1\n").unwrap();
            std::fs::set_permissions(&server, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        std::fs::set_permissions(&java, std::fs::Permissions::from_mode(0o755)).unwrap();
        let gui_config = config(&scratch.path().join("gui"));
        let gui_settings = gui_config.providers.clone();
        let gui = cx.new(|cx| ReaderView::new(Session::new(gui_config), Some(path.clone()), cx));
        let window = cx
            .update(|cx| cx.open_window(Default::default(), |_, _| gui.clone()))
            .unwrap();
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        let focus = gui.read_with(&visual, |v, _| v.focus_handle());
        visual.update(|w, cx| focus.focus(w, cx));
        let tui_config = config(&scratch.path().join("tui"));
        let tui_settings = tui_config.providers.clone();
        let mut tui = App::new(tui_config, Some(path.clone()));
        let key = |app: &mut App, code| {
            app.handle(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
        };
        let settle = |visual: &mut VisualTestContext, tui: &mut App, saved: bool| {
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                gui.update(visual, |v, cx| v.poll(cx));
                while let Ok(c) = tui.session.completions().try_recv() {
                    tui.apply(c);
                }
                let done = |s: &State| {
                    if saved {
                        s.server_runtime()
                            .is_some_and(|m| m.message.contains("saved."))
                    } else {
                        s.document().is_some()
                    }
                };
                if gui.read_with(visual, |v, _| done(v.session().state()))
                    && done(tui.session.state())
                {
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "{provider}: GUI {:?}; TUI {:?}",
                    gui.read_with(visual, |v, _| v
                        .session()
                        .state()
                        .server_runtime()
                        .map(|m| m.message.clone())),
                    tui.session.state().server_runtime().map(|m| &m.message)
                );
                std::thread::sleep(Duration::from_millis(2));
            }
            assert_eq!(
                gui.read_with(visual, |v, _| snapshot(v.session().state())),
                snapshot(tui.session.state())
            );
        };
        settle(&mut visual, &mut tui, false);
        visual.simulate_keystrokes("f2");
        key(&mut tui, KeyCode::F(2));
        let at = tui
            .session
            .language_server_rows()
            .iter()
            .position(|(_, action)| *action == Some(CommandId::ServerRuntime))
            .unwrap();
        for _ in 0..at {
            visual.simulate_keystrokes("down");
            key(&mut tui, KeyCode::Down);
        }
        visual.simulate_keystrokes("enter");
        key(&mut tui, KeyCode::Enter);
        visual.update(|_, cx| {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                java.to_str().unwrap().into(),
            ))
        });
        visual.simulate_keystrokes("ctrl-v enter");
        tui.handle(Event::Paste(java.to_str().unwrap().into()));
        key(&mut tui, KeyCode::Enter);
        settle(&mut visual, &mut tui, true);
        let read = |path| {
            serde_json::from_slice::<serde_json::Value>(&std::fs::read(path).unwrap()).unwrap()
        };
        let saved = read(gui_settings);
        assert_eq!(saved, read(tui_settings));
        if provider == "pyright" {
            assert_eq!(saved["providers"][provider]["node"], java.to_str().unwrap());
        }
        visual.simulate_keystrokes("escape");
        key(&mut tui, KeyCode::Esc);
        assert!(tui.session.state().server_runtime().is_none());
        assert!(gui.read_with(&visual, |v, _| {
            v.session().state().server_runtime().is_none()
        }));
        assert_eq!(std::fs::read_to_string(path).unwrap(), source);
    }
}

#[gpui::test]
fn workspace_reopens_from_tui_to_gui_and_back_with_real_input(cx: &mut TestAppContext) {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path().join("project");
    std::fs::create_dir_all(&root).unwrap();
    let source = (0..120)
        .map(|i| format!("line_{i:03} héllo workspace\n"))
        .collect::<String>();
    for name in ["a.txt", "b.txt"] {
        std::fs::write(root.join(name), &source).unwrap();
    }
    let base = scratch.path().join("preferences");
    let mut tui = App::new(config(&base), Some(root.clone()));
    let key = |app: &mut App, code, modifiers| {
        app.handle(Event::Key(KeyEvent::new(code, modifiers)));
    };
    let wait = |app: &mut App, done: &dyn Fn(&State) -> bool| {
        let end = Instant::now() + Duration::from_secs(10);
        while !done(app.session.state()) {
            assert!(
                Instant::now() < end,
                "workspace wait: {:?}, {:?}, {:?}",
                app.session.state().loading(),
                app.session.state().notices(),
                app.session.state().document().map(|d| d.source())
            );
            if let Ok(c) = app.session.completions().try_recv() {
                app.apply(c);
            } else {
                std::thread::yield_now();
            }
        }
    };
    wait(&mut tui, &|s| {
        s.project()
            .is_some_and(|p| matches!(p.coverage(), reader_core::project::Coverage::Complete(_)))
    });
    let open = |app: &mut App, name: &str| {
        key(app, KeyCode::Char('p'), KeyModifiers::CONTROL);
        app.handle(Event::Paste(name.into()));
        wait(app, &|s| {
            s.navigation()
                .is_some_and(|n| !n.pending && !n.rows.is_empty())
        });
        key(app, KeyCode::Enter, KeyModifiers::NONE);
        wait(app, &|s| {
            s.document().is_some_and(|d| d.source().ends_with(name))
        });
    };
    open(&mut tui, "a.txt");
    key(&mut tui, KeyCode::Char('g'), KeyModifiers::CONTROL);
    tui.handle(Event::Paste("42:3".into()));
    key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    key(&mut tui, KeyCode::Char('\\'), KeyModifiers::CONTROL);
    key(&mut tui, KeyCode::Char('2'), KeyModifiers::CONTROL);
    open(&mut tui, "b.txt");
    key(&mut tui, KeyCode::Char('g'), KeyModifiers::CONTROL);
    tui.handle(Event::Paste("65:4".into()));
    key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    key(&mut tui, KeyCode::Char('b'), KeyModifiers::CONTROL);
    let expected = snapshot(tui.session.state());
    assert!(!tui.tree_shown);
    drop(tui);
    let gui = cx.new(|cx| ReaderView::new(Session::new(config(&base)), Some(root.clone()), cx));
    let window = cx
        .update(|cx| cx.open_window(Default::default(), |_, _| gui.clone()))
        .unwrap();
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        cx.run_until_parked();
        gui.update(cx, |r, cx| r.poll(cx));
        if gui.read_with(cx, |r, _| {
            r.session()
                .state()
                .document()
                .is_some_and(|d| d.source().ends_with("b.txt"))
                && r.session()
                    .state()
                    .shown(0)
                    .is_some_and(|t| t.document().is_some())
        }) {
            break;
        }
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(
        gui.read_with(cx, |r, _| snapshot(r.session().state())),
        expected
    );
    assert!(!gui.read_with(cx, |r, _| r.session().state().explorer().visible));
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let focus = gui.read_with(&visual, |r, _| r.focus_handle());
    visual.update(|w, cx| focus.focus(w, cx));
    visual.simulate_keystrokes("ctrl-b ctrl-1 ctrl-g 7 enter");
    let changed = gui.read_with(&visual, |r, _| snapshot(r.session().state()));
    assert_eq!(changed.active.as_deref(), Some("a.txt"));
    assert!(gui.read_with(&visual, |r, _| r.session().state().explorer().visible));
    visual.update(|w, cx| {
        let _ = w.draw(cx);
    });
    visual.update(|w, _| w.remove_window());
    drop(visual);
    drop(gui);
    cx.run_until_parked();
    let saved_file = reader_core::places::file_for(&base.join("state/projects"), &root);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&saved_file).unwrap()).unwrap();
        if saved["workspace"]["focused"] == 0 && saved["workspace"]["explorer"]["visible"] == true {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "closing the GUI did not persist its workspace: {saved}"
        );
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(2));
    }
    let mut tui = App::new(config(&base), Some(root.clone()));
    wait(&mut tui, &|s| {
        s.document().is_some_and(|d| d.source().ends_with("a.txt"))
    });
    assert_eq!(snapshot(tui.session.state()), changed);
    assert!(tui.tree_shown);
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(26, 8)).unwrap();
    terminal.draw(|f| tui.draw(f)).unwrap();
    for name in ["a.txt", "b.txt"] {
        assert_eq!(std::fs::read_to_string(root.join(name)).unwrap(), source);
    }
}

#[gpui::test]
fn unified_name_search_replays_files_symbols_actions_caret_and_recents_in_both_adapters(
    cx: &mut TestAppContext,
) {
    use reader_core::navigation::{Kind, Scope};
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path().join("project");
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("wrap.rs");
    let source = "// fixture\npub fn wrap() {}\n";
    std::fs::write(&path, source).unwrap();
    let other = root.join("other.txt");
    std::fs::write(&other, "other\n").unwrap();
    let gui = cx.new(|cx| {
        ReaderView::new(
            Session::new(config(&scratch.path().join("gui"))),
            Some(root.clone()),
            cx,
        )
    });
    let window = cx
        .update(|cx| cx.open_window(Default::default(), |_, _| gui.clone()))
        .unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let focus = gui.read_with(&visual, |v, _| v.focus_handle());
    visual.update(|w, cx| focus.focus(w, cx));
    let mut tui = App::new(config(&scratch.path().join("tui")), Some(root.clone()));
    let key = |app: &mut App, code, modifiers| {
        app.handle(Event::Key(KeyEvent::new(code, modifiers)));
    };
    let ready = |s: &State| {
        s.project()
            .is_some_and(|p| matches!(p.coverage(), reader_core::project::Coverage::Complete(_)))
            && s.index_coverage().complete()
            && s.navigation().is_none_or(|n| !n.pending)
            && s.loading().is_none()
    };
    let settle = |visual: &mut VisualTestContext, tui: &mut App| {
        let until = Instant::now() + Duration::from_secs(10);
        loop {
            gui.update(visual, |r, cx| r.poll(cx));
            while let Ok(c) = tui.session.completions().try_recv() {
                tui.apply(c);
            }
            if gui.read_with(visual, |r, _| ready(r.session().state()))
                && ready(tui.session.state())
            {
                break;
            }
            assert!(Instant::now() < until, "navigation adapters did not settle");
            std::thread::yield_now();
        }
        visual.update(|w, cx| {
            let _ = w.draw(cx);
        });
        let rows = |s: &State| {
            s.navigation().map(|n| {
                (
                    n.scope,
                    n.query.clone(),
                    n.rows
                        .iter()
                        .map(|r| (r.kind, r.label.clone(), r.detail.clone(), r.disabled, r.on))
                        .collect::<Vec<_>>(),
                )
            })
        };
        assert_eq!(
            gui.read_with(visual, |r, _| rows(r.session().state())),
            rows(tui.session.state())
        );
        assert_eq!(
            gui.read_with(visual, |r, _| snapshot(r.session().state())),
            snapshot(tui.session.state())
        );
        assert_eq!(std::fs::read(&path).unwrap(), source.as_bytes());
        assert_eq!(std::fs::read(&other).unwrap(), b"other\n");
    };
    settle(&mut visual, &mut tui);
    visual.simulate_keystrokes("ctrl-k w r a p");
    key(&mut tui, KeyCode::Char('k'), KeyModifiers::CONTROL);
    tui.handle(Event::Paste("wrap".into()));
    settle(&mut visual, &mut tui);
    assert_eq!(tui.session.state().navigation().unwrap().scope, Scope::All);
    for kind in [Kind::File, Kind::Symbol, Kind::Action] {
        assert!(
            tui.session
                .state()
                .navigation()
                .unwrap()
                .rows
                .iter()
                .any(|r| r.kind == kind)
        );
    }
    for scope in [Scope::Files, Scope::Symbols, Scope::Actions, Scope::All] {
        visual.simulate_keystrokes("tab");
        key(&mut tui, KeyCode::Tab, KeyModifiers::NONE);
        settle(&mut visual, &mut tui);
        assert_eq!(tui.session.state().navigation().unwrap().scope, scope);
        assert_eq!(tui.session.state().navigation().unwrap().query, "wrap");
    }
    visual.simulate_keystrokes("escape ctrl-p w r a p");
    key(&mut tui, KeyCode::Esc, KeyModifiers::NONE);
    key(&mut tui, KeyCode::Char('p'), KeyModifiers::CONTROL);
    tui.handle(Event::Paste("wrap".into()));
    // Enter accepts only a completed row; wait for both scopes before opening it.
    settle(&mut visual, &mut tui);
    visual.simulate_keystrokes("enter");
    key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    settle(&mut visual, &mut tui);
    assert!(tui.session.state().document().is_some());
    visual.simulate_keystrokes("ctrl-alt-shift-n w r a p");
    key(
        &mut tui,
        KeyCode::Char('n'),
        KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SHIFT,
    );
    tui.handle(Event::Paste("wrap".into()));
    settle(&mut visual, &mut tui);
    visual.simulate_keystrokes("enter");
    key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    settle(&mut visual, &mut tui);
    assert_eq!(
        tui.session.state().selection().unwrap().head,
        "// fixture\n".len()
    );
    // Actual plain arrows collapse the caret in both adapters, including vertical movement.
    for (gui_key, code, head) in [
        ("right", KeyCode::Right, 12),
        ("left", KeyCode::Left, 11),
        ("up", KeyCode::Up, 0),
        ("down", KeyCode::Down, 11),
    ] {
        visual.simulate_keystrokes(gui_key);
        key(&mut tui, code, KeyModifiers::NONE);
        settle(&mut visual, &mut tui);
        assert_eq!(
            tui.session.state().selection(),
            Some(reader_core::session::Selection { anchor: head, head })
        );
    }
    visual.simulate_keystrokes("ctrl-shift-a s o f t space w r a p");
    key(
        &mut tui,
        KeyCode::Char('a'),
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    );
    tui.handle(Event::Paste("soft wrap".into()));
    settle(&mut visual, &mut tui);
    visual.simulate_keystrokes("enter");
    key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    settle(&mut visual, &mut tui);
    assert!(tui.session.state().soft_wrap());

    // Keep each tab so Recent Files must restore its existing caret instead of resetting it.
    visual.simulate_keystrokes("ctrl-shift-k right ctrl-p o t h e r");
    key(
        &mut tui,
        KeyCode::Char('k'),
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    );
    key(&mut tui, KeyCode::Right, KeyModifiers::NONE);
    key(&mut tui, KeyCode::Char('p'), KeyModifiers::CONTROL);
    tui.handle(Event::Paste("other".into()));
    settle(&mut visual, &mut tui);
    visual.simulate_keystrokes("enter");
    key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    settle(&mut visual, &mut tui);
    assert_eq!(tui.session.state().active().unwrap().source(), other);
    visual.simulate_keystrokes("ctrl-shift-k right right ctrl-e");
    key(
        &mut tui,
        KeyCode::Char('k'),
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    );
    key(&mut tui, KeyCode::Right, KeyModifiers::NONE);
    key(&mut tui, KeyCode::Right, KeyModifiers::NONE);
    key(&mut tui, KeyCode::Char('e'), KeyModifiers::CONTROL);
    settle(&mut visual, &mut tui);
    assert_eq!(
        tui.session.state().navigation().unwrap().scope,
        Scope::Recent
    );
    assert_eq!(
        tui.session.state().navigation().unwrap().rows[0].label,
        "wrap.rs"
    );
    let selection = tui.session.state().selection();
    let top = tui.session.state().top_line();
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(40, 10)).unwrap();
    terminal.draw(|f| tui.draw(f)).unwrap();
    visual.simulate_event(gpui::ScrollWheelEvent {
        position: gpui::point(gpui::px(400.0), gpui::px(200.0)),
        delta: gpui::ScrollDelta::Lines(gpui::point(0.0, -1.0)),
        ..Default::default()
    });
    tui.handle(Event::Mouse(crossterm::event::MouseEvent {
        kind: crossterm::event::MouseEventKind::ScrollDown,
        column: 0,
        row: 0,
        modifiers: KeyModifiers::NONE,
    }));
    settle(&mut visual, &mut tui);
    assert_eq!(tui.session.state().selection(), selection);
    assert_eq!(tui.session.state().top_line(), top);
    visual.simulate_keystrokes("enter");
    key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    settle(&mut visual, &mut tui);
    assert_eq!(tui.session.state().active().unwrap().source(), path);
    assert_eq!(
        tui.session.state().selection(),
        Some(reader_core::session::Selection {
            anchor: 12,
            head: 12
        })
    );

    visual.simulate_keystrokes("ctrl-e o h r");
    key(&mut tui, KeyCode::Char('e'), KeyModifiers::CONTROL);
    tui.handle(Event::Paste("ohr".into()));
    settle(&mut visual, &mut tui);
    assert_eq!(tui.session.state().navigation().unwrap().rows.len(), 1);
    assert_eq!(
        tui.session.state().navigation().unwrap().rows[0].label,
        "other.txt"
    );
    terminal.draw(|f| tui.draw(f)).unwrap();
    visual.simulate_keystrokes("delete");
    key(&mut tui, KeyCode::Delete, KeyModifiers::NONE);
    settle(&mut visual, &mut tui);
    assert!(tui.session.state().navigation().unwrap().rows.is_empty());
    visual.simulate_keystrokes("escape ctrl-e");
    key(&mut tui, KeyCode::Esc, KeyModifiers::NONE);
    key(&mut tui, KeyCode::Char('e'), KeyModifiers::CONTROL);
    settle(&mut visual, &mut tui);
    assert_eq!(tui.session.state().navigation().unwrap().rows.len(), 1);
    assert_eq!(
        tui.session.state().navigation().unwrap().rows[0].label,
        "wrap.rs"
    );
}

#[gpui::test]
fn dependency_definitions_reveal_browsable_libraries_in_both_explorers(cx: &mut TestAppContext) {
    use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    use reader_core::{project::Coverage, semantics::Status};
    let profile = std::env::current_exe().unwrap().parent().unwrap().parent().unwrap().to_owned();
    assert!(std::process::Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .args(["build", "-q", "--locked", "-p", "reader-core", "--example", "fake_lsp"])
        .current_dir(env!("CARGO_MANIFEST_DIR")).status().unwrap().success());
    let fake = profile.join("examples/fake_lsp");
    for dark in [true, false] {
        for folder in [true, false] {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().join("project");
            let dep = dir.path().join(".cargo/registry/src/index/dep-1.0");
            let main = root.join("main.rs");
            let target = dep.join("src/lib.rs");
            let sibling = dep.join("src/sibling.rs");
            let files = [
                (root.join("Cargo.toml"), "[package]\nname='test'\nversion='0.1.0'\n"),
                (main.clone(), "fn main() { from_dep(); }\n"),
                (target.clone(), "pub fn from_dep() {}\n"),
                (sibling.clone(), "pub fn sibling() {}\n"),
                (dep.join("unopened/nested.rs"), "pub fn nested() {}\n"),
            ];
            for (path, text) in &files { std::fs::create_dir_all(path.parent().unwrap()).unwrap(); std::fs::write(path, text).unwrap(); }
            let make = |name| {
                let mut c = config(&dir.path().join(name)); c.dark = dark;
                let mut settings: serde_json::Value = serde_json::from_slice(&std::fs::read(&c.providers).unwrap()).unwrap();
                settings["providers"]["rust-analyzer"] = serde_json::json!({"executable":fake,"env":{"FAKE_LSP_EXTRA":dep}});
                std::fs::write(&c.providers, settings.to_string()).unwrap(); c
            };
            let path = if folder { root.clone() } else { main.clone() };
            let gui = cx.new(|cx| ReaderView::new(Session::new(make("gui")), Some(path.clone()), cx));
            let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| gui.clone())).unwrap();
            let mut visual = VisualTestContext::from_window(window.into(), cx);
            let focus = gui.read_with(&visual, |v, _| v.focus_handle());
            visual.update(|w, cx| focus.focus(w, cx));
            let mut tui = App::new(make("tui"), Some(path));
            let settle = |visual: &mut VisualTestContext, tui: &mut App, ready: &dyn Fn(&State) -> bool| {
                let until = Instant::now() + Duration::from_secs(10);
                loop {
                    gui.update(visual, |v, cx| v.poll(cx));
                    while let Ok(c) = tui.session.completions().try_recv() { tui.apply(c); }
                    if gui.read_with(visual, |v, _| ready(v.session().state())) && ready(tui.session.state()) { break; }
                    assert!(Instant::now() < until, "{:?}", tui.session.state().notices());
                    std::thread::sleep(Duration::from_millis(2));
                }
            };
            if folder {
                settle(&mut visual, &mut tui, &|s| s.project().is_some_and(|p| matches!(p.coverage(), Coverage::Complete(_))));
                visual.simulate_keystrokes("ctrl-shift-e end enter");
                for (key, modifiers) in [(KeyCode::Char('e'), KeyModifiers::CONTROL | KeyModifiers::SHIFT), (KeyCode::End, KeyModifiers::NONE), (KeyCode::Enter, KeyModifiers::NONE)] { tui.handle(Event::Key(KeyEvent::new(key, modifiers))); }
            }
            settle(&mut visual, &mut tui, &|s| s.provider().is_some_and(|p| p.status == Status::Ready));
            visual.simulate_keystrokes("ctrl-home");
            tui.handle(Event::Key(KeyEvent::new(KeyCode::Home, KeyModifiers::CONTROL)));
            for _ in 0..13 { visual.simulate_keystrokes("right"); tui.handle(Event::Key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE))); }
            visual.simulate_keystrokes("f12");
            tui.handle(Event::Key(KeyEvent::new(KeyCode::F(12), KeyModifiers::NONE)));
            settle(&mut visual, &mut tui, &|s| s.document().is_some_and(|d| d.source() == target) && s.tree_rows().any(|r| r.name.as_ref() == "sibling.rs") && s.tree_rows().all(|r| !r.name.contains("loading")));
            let tree = |s: &State| s.tree_rows().map(|r| (r.depth, r.path.to_string(), r.name.to_string(), r.dir)).collect::<Vec<_>>();
            assert_eq!(gui.read_with(&visual, |v, _| tree(v.session().state())), tree(tui.session.state()));
            assert_eq!(tui.session.state().project().map(|p| p.root()), folder.then_some(root.as_path()));
            assert_eq!(tui.session.state().tree_rows().nth(tui.tree_at).unwrap().path.as_ref(), target.to_str().unwrap());
            assert_eq!(gui.read_with(&visual, |v, _| v.tree_sel.clone()).unwrap().as_ref(), target.to_str().unwrap());
            assert!(!tui.session.state().tree_rows().any(|r| r.name.as_ref() == "nested.rs"));
            let draw = |visual: &mut VisualTestContext, tui: &mut App, width, height, cols, rows| {
                visual.simulate_resize(gpui::size(gpui::px(width), gpui::px(height)));
                visual.update(|w, cx| { let _ = w.draw(cx); });
                let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(cols, rows)).unwrap();
                term.draw(|f| tui.draw(f)).unwrap();
                term.backend().buffer().content.iter().map(|c| c.symbol()).collect::<String>()
            };
            let rendered = draw(&mut visual, &mut tui, 900., 500., 100, 30);
            assert!(rendered.contains("Libraries") && rendered.contains("dep-1.0"));
            assert!(gui.read_with(&visual, |v, _| v.tree_lines(v.tree_at()).iter().any(|r| r.name == "Libraries")));
            let select = |visual: &mut VisualTestContext, tui: &mut App, path: &Path| {
                let at = tui.session.state().tree_rows().position(|r| r.path.as_ref() == path.to_str().unwrap()).unwrap();
                visual.simulate_keystrokes("ctrl-shift-e home");
                tui.handle(Event::Key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL | KeyModifiers::SHIFT)));
                tui.handle(Event::Key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE)));
                for _ in 0..at { visual.simulate_keystrokes("down"); tui.handle(Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))); }
            };
            select(&mut visual, &mut tui, &dep);
            visual.simulate_keystrokes("left right");
            for key in [KeyCode::Left, KeyCode::Right] { tui.handle(Event::Key(KeyEvent::new(key, KeyModifiers::NONE))); }
            settle(&mut visual, &mut tui, &|s| s.tree_rows().any(|r| r.name.as_ref() == "unopened"));
            select(&mut visual, &mut tui, &dep.join("unopened"));
            visual.simulate_keystrokes("right");
            tui.handle(Event::Key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE)));
            settle(&mut visual, &mut tui, &|s| s.tree_rows().any(|r| r.name.as_ref() == "nested.rs"));
            select(&mut visual, &mut tui, &dep.join("unopened/nested.rs"));
            visual.simulate_keystrokes("enter");
            tui.handle(Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)));
            settle(&mut visual, &mut tui, &|s| s.document().is_some_and(|d| d.source().ends_with("nested.rs")));
            assert_eq!(gui.read_with(&visual, |v, _| snapshot(v.session().state())), snapshot(tui.session.state()));
            // A menu owns input, including clicks on the explorer behind it.
            draw(&mut visual, &mut tui, 900., 500., 100, 30);
            visual.simulate_keystrokes("f2");
            tui.handle(Event::Key(KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE)));
            visual.simulate_click(gpui::point(gpui::px(30.), gpui::px(60.)), gpui::Modifiers::default());
            tui.handle(Event::Mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: tui.layout.tree.x + 1, row: tui.layout.tree.y, modifiers: KeyModifiers::NONE }));
            assert!(tui.session.state().document().unwrap().source().ends_with("nested.rs"));
            assert!(gui.read_with(&visual, |v, _| v.session().state().document().unwrap().source().ends_with("nested.rs")));
            visual.simulate_keystrokes("escape ctrl-shift-e end up");
            for (key, modifiers) in [(KeyCode::Esc, KeyModifiers::NONE), (KeyCode::Char('e'), KeyModifiers::CONTROL | KeyModifiers::SHIFT), (KeyCode::End, KeyModifiers::NONE), (KeyCode::Up, KeyModifiers::NONE)] { tui.handle(Event::Key(KeyEvent::new(key, modifiers))); }
            for (w, h, cols, rows) in [(280., 180., 35, 8), (70., 55., 8, 3), (900., 500., 100, 30)] { draw(&mut visual, &mut tui, w, h, cols, rows); }
            for (path, text) in &files { assert_eq!(std::fs::read_to_string(path).unwrap(), *text); }
        }
    }
}

#[gpui::test]
fn gui_and_terminal_double_and_triple_clicks_select_the_same_word_and_group(cx: &mut TestAppContext) {
    use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.rs");
    let src = "fn f() { g(\"hello world\", 1); }\n";
    std::fs::write(&path, src).unwrap();
    let gui = cx.new(|cx| ReaderView::new(Session::new(config(&dir.path().join("gui"))), Some(path.clone()), cx));
    let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| gui.clone())).unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let mut tui = App::new(config(&dir.path().join("tui")), Some(path.clone()));
    let settle = |visual: &mut VisualTestContext, tui: &mut App, done: &dyn Fn(&State) -> bool| {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            gui.update(visual, |v, cx| v.poll(cx));
            while let Ok(c) = tui.session.completions().try_recv() { tui.apply(c); }
            if gui.read_with(visual, |v, _| done(v.session().state())) && done(tui.session.state()) { break; }
            assert!(Instant::now() < deadline, "adapters did not settle");
            std::thread::sleep(Duration::from_millis(2));
        }
    };
    settle(&mut visual, &mut tui, &|s| s.document().is_some());
    visual.simulate_resize(gpui::size(gpui::px(800.0), gpui::px(400.0)));
    visual.update(|w, cx| { let _ = w.draw(cx); });
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 20)).unwrap();
    terminal.draw(|f| tui.draw(f)).unwrap();
    let at = src.find("world").unwrap() + 2;
    let (gx, gy) = gui.read_with(&visual, |v, _| v.text_point_of(at)).unwrap();
    let text = tui.layout.panes[0].text;
    let selected = |s: &State| s.selected_text().unwrap_or_default().to_string();
    for clicks in 1..=3 {
        visual.simulate_event(gpui::MouseDownEvent { button: gpui::MouseButton::Left, position: gpui::point(gpui::px(gx), gpui::px(gy)), modifiers: gpui::Modifiers::default(), click_count: clicks, first_mouse: false });
        visual.simulate_event(gpui::MouseUpEvent { button: gpui::MouseButton::Left, position: gpui::point(gpui::px(gx), gpui::px(gy)), modifiers: gpui::Modifiers::default(), click_count: clicks });
        tui.handle(Event::Mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: text.x + at as u16, row: text.y, modifiers: KeyModifiers::NONE }));
        let want = ["", "world", "hello world"][clicks - 1];
        settle(&mut visual, &mut tui, &|s| selected(s) == want);
        assert_eq!(gui.read_with(&visual, |v, _| snapshot(v.session().state())), snapshot(tui.session.state()), "{clicks} clicks");
    }
    assert_eq!(std::fs::read_to_string(&path).unwrap(), src);
}

#[gpui::test]
fn gui_and_terminal_blame_through_their_menus_show_the_same_commit(cx: &mut TestAppContext) {
    use lenz_content::git::testing::{commit_all, git, repo, write};
    let r = repo();
    write(r.path(), "a.txt", "one\ntwo\n");
    commit_all(r.path(), "First line\n\nThe body.");
    let sha = git(r.path(), &["rev-parse", "HEAD"]).trim().to_string();
    let path = r.path().join("a.txt");
    let state = tempfile::tempdir().unwrap();
    let gui = cx.new(|cx| ReaderView::new(Session::new(config(&state.path().join("gui"))), Some(path.clone()), cx));
    let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| gui.clone())).unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let focus = gui.read_with(&visual, |v, _| v.focus_handle());
    visual.update(|w, cx| focus.focus(w, cx));
    let mut tui = App::new(config(&state.path().join("tui")), Some(path.clone()));
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 24)).unwrap();
    let key = |app: &mut App, code, modifiers| { app.handle(Event::Key(KeyEvent::new(code, modifiers))); };
    let settle = |visual: &mut VisualTestContext, tui: &mut App, done: &dyn Fn(&Session) -> bool| {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            gui.update(visual, |v, cx| v.poll(cx));
            while let Ok(c) = tui.session.completions().try_recv() { tui.apply(c); }
            if gui.read_with(visual, |v, _| done(v.session())) && done(&tui.session) { break; }
            assert!(Instant::now() < deadline, "adapters did not settle: gui {} / tui {}", gui.read_with(visual, |v, _| { let s = v.session(); format!("{:?} {:?} {}", s.state().active().map(|t| (t.annotation_cols(), t.annotation(0).map(|a| a.label), t.document().map(|d| d.line_count()))), s.disabled(CommandId::AnnotateBlame), s.status_message()) }), { let s = &tui.session; format!("{:?} {:?} {}", s.state().active().map(|t| (t.annotation_cols(), t.annotation(0).map(|a| a.label), t.document().map(|d| d.line_count()))), s.disabled(CommandId::AnnotateBlame), s.status_message()) });
            std::thread::sleep(Duration::from_millis(2));
        }
    };
    settle(&mut visual, &mut tui, &|s| s.state().document().is_some());
    visual.update(|w, cx| { let _ = w.draw(cx); });
    terminal.draw(|f| tui.draw(f)).unwrap();
    // The same command from each adapter's application menu: its group, then its row.
    let menu = |visual: &mut VisualTestContext, tui: &mut App, terminal: &mut ratatui::Terminal<ratatui::backend::TestBackend>, id: CommandId| {
        visual.simulate_click(gpui::point(gpui::px(12.0), gpui::px(10.0)), gpui::Modifiers::default());
        terminal.draw(|f| tui.draw(f)).unwrap();
        let button = tui.layout.app_menu;
        tui.handle(Event::Mouse(crossterm::event::MouseEvent { kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left), column: button.x + 1, row: button.y, modifiers: KeyModifiers::NONE }));
        let group = reader_core::actions::CommandGroup::ALL.iter().position(|g| *g == id.group()).unwrap();
        let row = CommandId::ALL.iter().filter(|c| c.group() == id.group()).position(|c| *c == id).unwrap();
        visual.simulate_keystrokes(&format!("{}enter {}enter", "down ".repeat(group), "down ".repeat(row)));
        for n in [group, row] {
            for _ in 0..n { key(tui, KeyCode::Down, KeyModifiers::NONE); }
            key(tui, KeyCode::Enter, KeyModifiers::NONE);
        }
    };
    menu(&mut visual, &mut tui, &mut terminal, CommandId::AnnotateBlame);
    settle(&mut visual, &mut tui, &|s| s.state().active().is_some_and(|t| t.annotation(1).is_some()));
    let labels = |s: &Session| (0..2).map(|l| s.annotation(s.state().active().unwrap().id(), l).unwrap().label).collect::<Vec<_>>();
    assert_eq!(gui.read_with(&visual, |v, _| labels(v.session())), labels(&tui.session));
    // Show Commit… on the caret's line (the first): the same popup text, and Copy Revision copies the same id.
    visual.simulate_keystrokes("down up");
    for code in [KeyCode::Down, KeyCode::Up] { key(&mut tui, code, KeyModifiers::NONE); }
    assert_eq!(gui.read_with(&visual, |v, _| v.session().state().selection()), tui.session.state().selection());
    assert_eq!(tui.session.state().selection().map(|s| s.head), Some(0));
    menu(&mut visual, &mut tui, &mut terminal, CommandId::ShowCommit);
    settle(&mut visual, &mut tui, &|s| s.state().commit_popup().is_some_and(|c| c.details.is_some()));
    visual.update(|w, cx| { let _ = w.draw(cx); });
    terminal.draw(|f| tui.draw(f)).unwrap();
    let text = tui.session.commit_text().unwrap();
    assert!(text.starts_with(&format!("commit {sha}")), "{text}");
    assert_eq!(gui.read_with(&visual, |v, _| v.session().commit_text()), Some(text));
    visual.simulate_keystrokes("ctrl-c");
    key(&mut tui, KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert_eq!(visual.update(|_, cx| cx.read_from_clipboard().and_then(|c| c.text())), Some(sha.clone()));
    assert_eq!(tui.copy.take(), Some(sha.clone()));
    visual.simulate_keystrokes("escape");
    key(&mut tui, KeyCode::Esc, KeyModifiers::NONE);
    settle(&mut visual, &mut tui, &|s| s.state().commit_popup().is_none());
    // Show Diff: neither finds Lenz Diff beside the test binary, and both say so the same way.
    menu(&mut visual, &mut tui, &mut terminal, CommandId::ShowCommitDiff);
    let said = tui.session.status_message();
    assert!(said.starts_with("Show Diff: Lenz Diff is not beside Lenz Code"), "{said}");
    assert_eq!(gui.read_with(&visual, |v, _| v.session().status_message()), said.replace("lenz-diff-tui", "lenz-diff"));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "one\ntwo\n");
}

#[gpui::test]
fn gui_and_terminal_git_branches_list_the_same_rows_and_check_out_the_same_way(cx: &mut TestAppContext) {
    use lenz_content::git::testing::{commit_all, git, repo, write};
    // Two identical repositories: one per adapter (a checkout in one must not change the other's).
    let make = || {
        let r = repo();
        git(r.path(), &["config", "user.name", "t"]);
        git(r.path(), &["config", "user.email", "t@t"]);
        write(r.path(), "a.txt", "main\n");
        commit_all(r.path(), "main");
        git(r.path(), &["tag", "v1"]);
        git(r.path(), &["switch", "-q", "-c", "other"]);
        write(r.path(), "a.txt", "other\n");
        commit_all(r.path(), "other");
        git(r.path(), &["switch", "-q", "main"]);
        r
    };
    let (gr, tr) = (make(), make());
    let state = tempfile::tempdir().unwrap();
    let gui = cx.new(|cx| ReaderView::new(Session::new(config(&state.path().join("gui"))), Some(gr.path().to_path_buf()), cx));
    let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| gui.clone())).unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let focus = gui.read_with(&visual, |v, _| v.focus_handle());
    visual.update(|w, cx| focus.focus(w, cx));
    let mut tui = App::new(config(&state.path().join("tui")), Some(tr.path().to_path_buf()));
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 24)).unwrap();
    let key = |app: &mut App, code, modifiers| { app.handle(Event::Key(KeyEvent::new(code, modifiers))); };
    let settle = |visual: &mut VisualTestContext, tui: &mut App, done: &dyn Fn(&Session) -> bool| {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            gui.update(visual, |v, cx| v.poll(cx));
            while let Ok(c) = tui.session.completions().try_recv() { tui.apply(c); }
            if gui.read_with(visual, |v, _| done(v.session())) && done(&tui.session) { break; }
            assert!(Instant::now() < deadline, "adapters did not settle");
            std::thread::sleep(Duration::from_millis(2));
        }
    };
    settle(&mut visual, &mut tui, &|s| s.branch_label().is_some());
    visual.update(|w, cx| { let _ = w.draw(cx); });
    terminal.draw(|f| tui.draw(f)).unwrap();
    // Git Branches… from each application menu.
    visual.simulate_click(gpui::point(gpui::px(12.0), gpui::px(10.0)), gpui::Modifiers::default());
    let button = tui.layout.app_menu;
    tui.handle(Event::Mouse(crossterm::event::MouseEvent { kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left), column: button.x + 1, row: button.y, modifiers: KeyModifiers::NONE }));
    let group = reader_core::actions::CommandGroup::ALL.iter().position(|g| *g == reader_core::actions::CommandGroup::Git).unwrap();
    let row = CommandId::ALL.iter().filter(|c| c.group() == reader_core::actions::CommandGroup::Git).position(|c| *c == CommandId::GitBranches).unwrap();
    visual.simulate_keystrokes(&format!("{}enter {}enter", "down ".repeat(group), "down ".repeat(row)));
    for n in [group, row] {
        for _ in 0..n { key(&mut tui, KeyCode::Down, KeyModifiers::NONE); }
        key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    }
    settle(&mut visual, &mut tui, &|s| s.state().branches().is_some_and(|b| !b.loading));
    visual.update(|w, cx| { let _ = w.draw(cx); });
    terminal.draw(|f| tui.draw(f)).unwrap();
    let gui_rows: Vec<String> = gui.read_with(&visual, |v, _| v.modal_rows().into_iter().skip(2).map(|r| r.0).collect());
    let tui_rows: Vec<String> = tui.choices().into_iter().map(|c| c.label).collect();
    assert_eq!(gui_rows, tui_rows);
    assert!(tui_rows.iter().any(|r| r.contains("● main")) && tui_rows.iter().any(|r| r.ends_with("v1")));
    // The same keys check out `other` in both.
    visual.simulate_keystrokes("o t h e r enter");
    for c in "other".chars() { key(&mut tui, KeyCode::Char(c), KeyModifiers::NONE); }
    key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    settle(&mut visual, &mut tui, &|s| !s.state().checking_out() && s.branch_label().as_deref() == Some("⎇ other"));
    assert_eq!(std::fs::read(gr.path().join("a.txt")).unwrap(), std::fs::read(tr.path().join("a.txt")).unwrap());
    assert_eq!(std::fs::read_to_string(tr.path().join("a.txt")).unwrap(), "other\n");
}

#[gpui::test]
fn gui_and_terminal_git_comparisons_start_the_same_lenz_diff(cx: &mut TestAppContext) {
    use lenz_content::git::testing::{commit_all, git, repo, write};
    let r = repo();
    write(r.path(), "a.txt", "main\n");
    commit_all(r.path(), "main");
    git(r.path(), &["tag", "v1"]);
    write(r.path(), "a.txt", "edited\n");
    let bin = tempfile::tempdir().unwrap();
    for name in ["lenz-diff", "lenz-diff-tui"] {
        std::fs::write(bin.path().join(name), "").unwrap();
    }
    let state = tempfile::tempdir().unwrap();
    let gui = cx.new(|cx| ReaderView::new(Session::new(config(&state.path().join("gui"))), Some(r.path().to_path_buf()), cx));
    let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| gui.clone())).unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let focus = gui.read_with(&visual, |v, _| v.focus_handle());
    visual.update(|w, cx| focus.focus(w, cx));
    gui.update(&mut visual, |v, _| v.exe = Some(bin.path().join("lenz-code")));
    let mut tui = App::new(config(&state.path().join("tui")), Some(r.path().to_path_buf()));
    tui.exe = Some(bin.path().join("lenz-code"));
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 24)).unwrap();
    let key = |app: &mut App, code, modifiers| { app.handle(Event::Key(KeyEvent::new(code, modifiers))); };
    let settle = |visual: &mut VisualTestContext, tui: &mut App, done: &dyn Fn(&Session) -> bool| {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            gui.update(visual, |v, cx| v.poll(cx));
            while let Ok(c) = tui.session.completions().try_recv() { tui.apply(c); }
            if gui.read_with(visual, |v, _| done(v.session())) && done(&tui.session) { break; }
            assert!(Instant::now() < deadline, "adapters did not settle");
            std::thread::sleep(Duration::from_millis(2));
        }
    };
    settle(&mut visual, &mut tui, &|s| s.branch_label().is_some());
    visual.update(|w, cx| { let _ = w.draw(cx); });
    terminal.draw(|f| tui.draw(f)).unwrap();
    // Show Local Changes from each application menu.
    visual.simulate_click(gpui::point(gpui::px(12.0), gpui::px(10.0)), gpui::Modifiers::default());
    let button = tui.layout.app_menu;
    tui.handle(Event::Mouse(crossterm::event::MouseEvent { kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left), column: button.x + 1, row: button.y, modifiers: KeyModifiers::NONE }));
    let group = reader_core::actions::CommandGroup::ALL.iter().position(|g| *g == reader_core::actions::CommandGroup::Git).unwrap();
    let row = CommandId::ALL.iter().filter(|c| c.group() == reader_core::actions::CommandGroup::Git).position(|c| *c == CommandId::ShowLocalChanges).unwrap();
    visual.simulate_keystrokes(&format!("{}enter {}enter", "down ".repeat(group), "down ".repeat(row)));
    for n in [group, row] {
        for _ in 0..n { key(&mut tui, KeyCode::Down, KeyModifiers::NONE); }
        key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    }
    let gui_args = |visual: &mut VisualTestContext| gui.read_with(visual, |v, _| v.launched.last().map(|l| (l.args.clone(), l.cwd.clone())));
    let tui_args = |tui: &mut App| tui.foreground.take().map(|l| (l.args, l.cwd));
    let local = tui_args(&mut tui);
    assert_eq!(gui_args(&mut visual), local);
    assert_eq!(local.map(|l| l.0), Some(vec!["HEAD".to_string()]));
    // Compare with Working Tree from tag v1's row menu.
    visual.simulate_keystrokes("ctrl-shift-`");
    tui.run(CommandId::GitBranches);
    settle(&mut visual, &mut tui, &|s| s.state().branches().is_some_and(|b| !b.loading));
    visual.simulate_keystrokes("v 1 right down down enter");
    for c in "v1".chars() { key(&mut tui, KeyCode::Char(c), KeyModifiers::NONE); }
    key(&mut tui, KeyCode::Right, KeyModifiers::NONE);
    for _ in 0..2 { key(&mut tui, KeyCode::Down, KeyModifiers::NONE); }
    key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    let tag = tui_args(&mut tui);
    assert_eq!(gui_args(&mut visual), tag);
    assert_eq!(tag.map(|l| l.0), Some(vec!["refs/tags/v1".to_string()]));
    assert_eq!(std::fs::read_to_string(r.path().join("a.txt")).unwrap(), "edited\n");
}
