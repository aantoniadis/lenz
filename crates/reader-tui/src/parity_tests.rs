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
    ] {
        // Reach the same action through the GUI's visible app menu and the terminal's command palette.
        visual.simulate_click(
            gpui::point(gpui::px(12.0), gpui::px(10.0)),
            gpui::Modifiers::default(),
        );
        let group = reader_core::actions::CommandGroup::ALL
            .iter()
            .position(|g| *g == id.group())
            .unwrap();
        visual.simulate_keystrokes(&format!("{}enter", "down ".repeat(group)));
        let row = CommandId::ALL
            .iter()
            .filter(|c| c.group() == id.group())
            .position(|c| *c == id)
            .unwrap();
        visual.simulate_keystrokes(&format!("{}enter", "down ".repeat(row)));
        key(&mut tui, KeyCode::F(10), KeyModifiers::NONE);
        tui.handle(Event::Paste(id.label().into()));
        let row = tui
            .choices()
            .iter()
            .position(
                |c| matches!(c.target, crate::app::ChoiceTarget::Action(target) if target == id),
            )
            .unwrap();
        for _ in 0..row {
            key(&mut tui, KeyCode::Down, KeyModifiers::NONE);
        }
        key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
        settle(&mut visual, &mut tui);
    }
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
