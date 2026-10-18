use super::*;
use crossterm::event::{KeyEvent, KeyModifiers};
use lenz_content::theme::{ThemeOverrides, ThemePaths};
use std::{
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

fn config(dir: &Path) -> Config {
    Config {
        themes: ThemePaths::under(&dir.join("config"), &dir.join("state"), "lenz-reader"),
        recent: dir.join("state/recent.json"),
        settings: dir.join("config/settings.json"),
        providers: dir.join("config/providers.json"),
        cache: dir.join("cache"),
        idle: Duration::ZERO,
        clock: Arc::new(Instant::now),
        overrides: ThemeOverrides::default(),
        dark: true,
    }
}
fn app(dir: &Path) -> App {
    App::new(config(dir), Some(dir.to_owned()))
}

fn key(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
    app.handle(Event::Key(KeyEvent::new(code, modifiers)));
}
fn settle(app: &mut App, done: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done(app) {
        assert!(Instant::now() < deadline, "background work timed out");
        if let Ok(c) = app.session.completions().try_recv() {
            app.apply(c);
        } else {
            std::thread::yield_now();
        }
    }
}
#[test]
fn right_click_uses_the_shared_context_actions_and_left_click_selects_text() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sample.txt");
    std::fs::write(&path, "hello world\nsecond\n").unwrap();
    let mut a = app(dir.path());
    a.dispatch(Command::Open(path.clone()));
    settle(&mut a, |a| a.session.state().document().is_some());
    draw(&mut a, 100, 26);
    let p = &a.layout.panes[0];
    let (x, y) = (p.text.x, p.text.y);
    click(&mut a, x + 2, y);
    assert!(a.modal.is_none());
    assert_eq!(a.session.state().selection().unwrap().head, 2);
    a.dispatch(Command::Select(Selection { anchor: 0, head: 5 }));
    a.handle(Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Right),
        column: x + 2,
        row: y,
        modifiers: KeyModifiers::NONE,
    }));
    let actions: Vec<_> = a
        .choices()
        .iter()
        .filter_map(|c| {
            if let ChoiceTarget::Action(id) = c.target {
                Some(id)
            } else {
                None
            }
        })
        .collect();
    let expected: Vec<_> = a
        .session
        .context_menu()
        .iter()
        .filter_map(|e| {
            if let reader_core::session::MenuEntry::Command(id) = e {
                Some(*id)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(actions, expected);
    assert!(!actions.contains(&CommandId::OpenFolder));
    assert_eq!(a.session.state().selected_text(), Some("hello"));
    draw(&mut a, 40, 12);
    let before = a.session.state().selection();
    click(&mut a, 0, 11);
    assert!(a.modal.is_none());
    assert_eq!(a.session.state().selection(), before);
    assert_eq!(
        std::fs::read_to_string(path).unwrap(),
        "hello world\nsecond\n"
    );
}

#[test]
fn top_bar_back_control_navigates_instead_of_opening_the_palette() {
    let dir = tempfile::tempdir().unwrap();
    let one = dir.path().join("one.txt");
    let two = dir.path().join("two.txt");
    std::fs::write(&one, "one\n").unwrap();
    std::fs::write(&two, "two\n").unwrap();
    let mut a = app(dir.path());
    a.dispatch(Command::OpenRetained(one.clone()));
    settle(&mut a, |a| a.session.state().document().is_some());
    a.dispatch(Command::OpenRetained(two));
    settle(&mut a, |a| {
        a.session
            .state()
            .active()
            .is_some_and(|t| t.source() == dir.path().join("two.txt"))
            && a.session.state().loading().is_none()
    });
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 26)).unwrap();
    terminal.draw(|f| a.draw(f)).unwrap();
    let top: String = (0..100)
        .map(|x| terminal.backend().buffer()[(x, 0)].symbol())
        .collect();
    let byte = top.find('←').expect("Back is visible beside the tabs");
    let x = top[..byte].chars().count() as u16;
    click(&mut a, x, 0);
    settle(&mut a, |a| {
        a.session
            .state()
            .active()
            .is_some_and(|t| t.source() == one)
            && a.session.state().loading().is_none()
    });
    assert!(a.modal.is_none());
}

#[test]
fn context_menu_follows_the_clicked_split_and_preview_and_scrolls_in_tiny_frames() {
    let dir = tempfile::tempdir().unwrap();
    let text = dir.path().join("a.txt");
    let markdown = dir.path().join("b.md");
    std::fs::write(&text, "α界 name\n").unwrap();
    std::fs::write(&markdown, "# Heading\n\nBody\n").unwrap();
    let mut a = app(dir.path());
    a.dispatch(Command::OpenRetained(text.clone()));
    settle(&mut a, |a| a.session.state().document().is_some());
    a.run(CommandId::SplitRight);
    a.dispatch(Command::OpenRetained(markdown));
    settle(&mut a, |a| {
        a.session
            .state()
            .active()
            .is_some_and(|t| t.source().ends_with("b.md"))
            && a.session.state().document().is_some()
    });
    a.run(CommandId::MarkdownSource);
    settle(&mut a, |a| {
        matches!(a.session.state().rendering(), Some(Rendering::Preview(_)))
    });
    draw(&mut a, 120, 30);
    let panes = a
        .layout
        .panes
        .iter()
        .map(|p| (p.pane, p.text, p.preview))
        .collect::<Vec<_>>();
    for (pane, bounds, preview) in panes {
        a.handle(Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Right),
            column: bounds.x + 1,
            row: bounds.y,
            modifiers: KeyModifiers::NONE,
        }));
        assert_eq!(a.session.state().focused_pane(), pane);
        assert_eq!(
            a.choices()
                .iter()
                .any(|c| matches!(c.target, ChoiceTarget::Action(CommandId::Definition))),
            !preview
        );
        for (w, h) in [(24, 8), (1, 1), (120, 30)] {
            draw(&mut a, w, h);
            key(&mut a, KeyCode::End, KeyModifiers::NONE);
            key(&mut a, KeyCode::Up, KeyModifiers::NONE);
            draw(&mut a, w, h);
            assert!(a.modal.as_ref().unwrap().at < a.choices().len());
        }
        key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    }
    // A right-click in the explorer leaves the document and modal state alone.
    a.handle(Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Right),
        column: 0,
        row: 2,
        modifiers: KeyModifiers::NONE,
    }));
    assert!(a.modal.is_none());
    assert_eq!(std::fs::read_to_string(text).unwrap(), "α界 name\n");
}

#[test]
fn modal_input_is_isolated_and_reader_never_writes_sources() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sample.txt");
    std::fs::write(&path, "hello\nworld\n").unwrap();
    let mut a = app(dir.path());
    a.dispatch(Command::Open(path.clone()));
    settle(&mut a, |a| a.session.state().document().is_some());
    a.dispatch(Command::Select(Selection { anchor: 0, head: 5 }));
    a.run(CommandId::GoToLine);
    key(&mut a, KeyCode::Char('a'), KeyModifiers::CONTROL);
    key(&mut a, KeyCode::Down, KeyModifiers::NONE);
    assert_eq!(
        a.session.state().selection(),
        Some(Selection { anchor: 0, head: 5 })
    );
    a.handle(Event::Paste("2:2".into()));
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    assert!(a.modal.is_none());
    assert_eq!(a.session.state().selection().unwrap().head, 7);
    assert_eq!(std::fs::read_to_string(path).unwrap(), "hello\nworld\n");
}
#[test]
fn palette_exposes_entire_core_catalog_and_filters() {
    let dir = tempfile::tempdir().unwrap();
    let mut a = app(dir.path());
    key(&mut a, KeyCode::F(10), KeyModifiers::NONE);
    assert_eq!(a.choices().len(), CommandId::ALL.len());
    a.handle(Event::Paste("split".into()));
    assert_eq!(a.choices().len(), 3);
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    assert!(a.modal.is_none());
    key(&mut a, KeyCode::F(10), KeyModifiers::NONE);
    a.handle(Event::Paste("language servers:".into()));
    let choices = a.choices();
    assert!(!choices.is_empty());
    assert!(choices.iter().all(|c| matches!(c.target, ChoiceTarget::Action(id) if id.group() == reader_core::actions::CommandGroup::LanguageServers)));
}

#[test]
fn theme_chooser_previews_before_enter_and_escape_restores_without_saving() {
    let dir = tempfile::tempdir().unwrap();
    let selection_file = config(dir.path()).themes.selection;
    let mut a = app(dir.path());
    let original = a.session.state().palette().clone();
    let selected = a.session.theme_id();
    a.run(CommandId::ChooseTheme);
    draw(&mut a, 100, 26);
    let (index, preview) = a
        .session
        .theme_entries()
        .iter()
        .enumerate()
        .find(|(_, entry)| entry.palette.bg != original.bg)
        .map(|(index, entry)| (index + 1, entry.palette.clone()))
        .unwrap();
    key(&mut a, KeyCode::Home, KeyModifiers::NONE);
    for _ in 0..index {
        key(&mut a, KeyCode::Down, KeyModifiers::NONE);
    }
    assert_eq!(a.session.state().palette().bg, preview.bg);
    assert_eq!(a.session.theme_id(), selected);
    assert!(!selection_file.exists());
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    assert!(a.modal.is_none());
    assert_eq!(a.session.state().palette().bg, original.bg);
    assert_eq!(a.session.theme_id(), selected);
    assert!(!selection_file.exists());
}

#[test]
fn theme_chooser_current_choice_navigation_and_enter_or_click_persist() {
    for mouse in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let selection_file = config(dir.path()).themes.selection;
        std::fs::create_dir_all(selection_file.parent().unwrap()).unwrap();
        std::fs::write(
            &selection_file,
            r#"{"version":1,"theme":"light","keep":true}"#,
        )
        .unwrap();
        let mut a = app(dir.path());
        a.run(CommandId::ChooseTheme);
        assert_eq!(a.highlighted_theme().as_deref(), Some("light"));
        key(&mut a, KeyCode::Home, KeyModifiers::NONE);
        assert_eq!(a.modal.as_ref().unwrap().at, 0);
        key(&mut a, KeyCode::PageDown, KeyModifiers::NONE);
        assert_eq!(a.modal.as_ref().unwrap().at, 8);
        key(&mut a, KeyCode::PageUp, KeyModifiers::NONE);
        assert_eq!(a.modal.as_ref().unwrap().at, 0);
        key(&mut a, KeyCode::End, KeyModifiers::NONE);
        assert_eq!(a.modal.as_ref().unwrap().at, a.choices().len() - 1);
        key(&mut a, KeyCode::Up, KeyModifiers::NONE);
        let chosen = a.highlighted_theme().unwrap();
        let palette = a.session.state().palette().clone();
        draw(&mut a, 100, 26);
        if mouse {
            let selected = a.modal.as_ref().unwrap().at;
            let row = a
                .layout
                .modal_items
                .iter()
                .position(|i| *i == selected)
                .unwrap();
            let (x, y) = (a.layout.modal_rows.x, a.layout.modal_rows.y + row as u16);
            click(&mut a, x, y);
        } else {
            key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
        }
        assert!(a.modal.is_none());
        assert_eq!(a.session.theme_id(), chosen);
        assert_eq!(a.session.state().palette().bg, palette.bg);
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let saved: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&selection_file).unwrap()).unwrap();
            if saved["theme"] == chosen {
                assert_eq!(saved["keep"], true);
                break;
            }
            assert!(Instant::now() < deadline, "theme selection was not saved");
            std::thread::yield_now();
        }
        a.run(CommandId::ChooseTheme);
        assert_eq!(a.highlighted_theme().as_deref(), Some(chosen.as_str()));
        key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    }
}

#[test]
fn theme_chooser_wheel_and_header_keep_preview_and_modal_blocks_source_input() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sample.txt");
    let source = "hello\r\nworld\r\n";
    std::fs::write(&path, source).unwrap();
    let mut a = app(dir.path());
    a.dispatch(Command::Open(path.clone()));
    settle(&mut a, |a| a.session.state().document().is_some());
    a.dispatch(Command::Select(Selection { anchor: 0, head: 5 }));
    let selection = a.session.state().selection();
    let original = a.session.state().palette().clone();
    a.run(CommandId::ChooseTheme);
    key(&mut a, KeyCode::Down, KeyModifiers::NONE);
    key(&mut a, KeyCode::Down, KeyModifiers::NONE);
    let preview = a.session.state().palette().clone();
    assert_ne!(preview.bg, original.bg);
    let at = a.modal.as_ref().unwrap().at;
    draw(&mut a, 100, 26);
    let (x, y) = (a.layout.modal_rows.x, a.layout.modal_rows.y);
    click(&mut a, x, y);
    assert!(
        a.modal.is_some(),
        "help/current source are not theme choices"
    );
    for (w, h) in [(100, 26), (24, 10), (8, 5), (1, 1), (0, 0), (100, 26)] {
        draw(&mut a, w, h);
        for kind in [MouseEventKind::ScrollDown, MouseEventKind::ScrollUp] {
            a.handle(Event::Mouse(MouseEvent {
                kind,
                column: x,
                row: y,
                modifiers: KeyModifiers::NONE,
            }));
        }
        a.handle(Event::Paste("ignored filter".into()));
        key(&mut a, KeyCode::Char('x'), KeyModifiers::NONE);
        for code in [
            KeyCode::Char('s'),
            KeyCode::Char('z'),
            KeyCode::Char('o'),
            KeyCode::Char('p'),
        ] {
            key(&mut a, code, KeyModifiers::CONTROL);
        }
        key(&mut a, KeyCode::F(5), KeyModifiers::NONE);
        assert_eq!(a.modal.as_ref().unwrap().at, at);
        assert!(a.modal.as_ref().unwrap().input.is_empty());
        assert_eq!(a.session.state().palette().bg, preview.bg);
        assert_eq!(a.session.state().selection(), selection);
    }
    click(&mut a, 0, 0);
    assert!(a.modal.is_none());
    assert_eq!(a.session.state().palette().bg, original.bg);
    assert_eq!(std::fs::read_to_string(path).unwrap(), source);
}

#[test]
fn theme_chooser_refresh_retains_highlighted_id_when_registry_rows_move() {
    use lenz_content::theme::{ThemeEntry, ThemeUpdate};
    let dir = tempfile::tempdir().unwrap();
    let mut a = app(dir.path());
    key(&mut a, KeyCode::F(10), KeyModifiers::NONE);
    a.handle(Event::Paste("Change theme".into()));
    assert_eq!(a.choices().len(), 1);
    assert_eq!(a.choices()[0].label, "Application: Change theme");
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    let entry = |id: &str| ThemeEntry {
        id: id.into(),
        label: id.into(),
        palette: a.session.state().palette().clone(),
    };
    let one = entry("user:one");
    let two = entry("user:two");
    a.apply(Completion::Theme(Box::new(ThemeUpdate::Registry {
        request: 1,
        entries: vec![one.clone(), two.clone()],
        notices: Vec::new(),
    })));
    key(&mut a, KeyCode::End, KeyModifiers::NONE);
    assert_eq!(a.highlighted_theme().as_deref(), Some("user:two"));
    a.apply(Completion::Theme(Box::new(ThemeUpdate::Registry {
        request: 1,
        entries: vec![two, one],
        notices: Vec::new(),
    })));
    assert_eq!(a.highlighted_theme().as_deref(), Some("user:two"));
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
}

#[test]
fn server_status_reports_missing_setup_and_offers_restart_or_enable_without_touching_source() {
    use reader_core::semantics::Status;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("main.rs");
    let source = "fn main() {}\n";
    std::fs::write(&path, source).unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname='status'\nversion='0.1.0'\n",
    )
    .unwrap();
    let cfg = config(dir.path());
    std::fs::create_dir_all(cfg.providers.parent().unwrap()).unwrap();
    std::fs::write(
        &cfg.providers,
        r#"{"providers":{"rust-analyzer":{"executable":"/nonexistent/lenz-status-test"}}}"#,
    )
    .unwrap();
    let mut a = App::new(cfg, Some(dir.path().into()));
    settle(&mut a, |a| a.session.state().project().is_some());
    a.dispatch(Command::Open(path.clone()));
    settle(&mut a, |a| {
        a.session
            .state()
            .provider()
            .is_some_and(|v| matches!(v.status, Status::Missing(_)))
    });
    a.dispatch(Command::Select(Selection { anchor: 0, head: 2 }));
    let selected = a.session.state().selection();
    draw(&mut a, 100, 26);
    let control = a
        .layout
        .controls
        .iter()
        .find(|(id, _)| *id == CommandId::LanguageServer)
        .unwrap()
        .1;
    click(&mut a, control.x, control.y);
    assert_eq!(a.modal.as_ref().unwrap().kind, Prompt::LanguageServer);
    draw(&mut a, 100, 26);
    let details = a
        .choices()
        .iter()
        .map(|c| c.label.as_str())
        .collect::<Vec<_>>()
        .join("");
    assert!(
        details.contains("was not found")
            && details.contains("rustup component add rust-analyzer")
            && details.contains("providers.json"),
        "{details}"
    );
    key(&mut a, KeyCode::Char('a'), KeyModifiers::CONTROL);
    key(&mut a, KeyCode::F(5), KeyModifiers::NONE);
    assert_eq!(a.session.state().selection(), selected);
    for (w, h) in [(26, 8), (1, 1), (80, 24)] {
        draw(&mut a, w, h);
        key(&mut a, KeyCode::End, KeyModifiers::NONE);
        key(&mut a, KeyCode::Up, KeyModifiers::NONE);
        draw(&mut a, w, h);
        assert!(a.modal.as_ref().unwrap().at < a.choices().len());
    }
    key(&mut a, KeyCode::Char('c'), KeyModifiers::CONTROL);
    let copied = a
        .copy
        .take()
        .expect("Ctrl+C copies the LSP error from the status dialog");
    assert!(
        copied.contains("was not found")
            && copied.contains("rustup component add rust-analyzer")
            && copied.contains("providers.json"),
        "{copied}"
    );
    assert!(!copied.contains("\nRestart Language Server\n"));
    assert_eq!(a.session.state().selection(), selected);
    assert_eq!(a.modal.as_ref().unwrap().kind, Prompt::LanguageServer);
    key(&mut a, KeyCode::Home, KeyModifiers::NONE);
    draw(&mut a, 80, 24);
    let copy = a
        .choices()
        .iter()
        .position(|c| matches!(c.target, ChoiceTarget::Action(CommandId::CopyServerDetails)))
        .unwrap();
    let row = a
        .layout
        .modal_items
        .iter()
        .position(|i| *i == copy)
        .unwrap();
    let (x, y) = (a.layout.modal_rows.x, a.layout.modal_rows.y + row as u16);
    click(&mut a, x, y);
    assert!(a.modal.is_none());
    assert_eq!(a.copy.take().as_deref(), Some(copied.as_str()));
    key(&mut a, KeyCode::F(2), KeyModifiers::NONE);
    draw(&mut a, 80, 24);
    let activate = |a: &mut App, id| {
        a.modal.as_mut().unwrap().at = a
            .choices()
            .iter()
            .position(|c| matches!(c.target, ChoiceTarget::Action(action) if action == id))
            .unwrap();
        key(a, KeyCode::Enter, KeyModifiers::NONE);
        assert!(a.modal.is_none());
    };
    activate(&mut a, CommandId::RestartServer);
    settle(&mut a, |a| {
        a.session
            .state()
            .provider()
            .is_some_and(|v| matches!(v.status, Status::Missing(_)))
    });
    key(&mut a, KeyCode::F(2), KeyModifiers::NONE);
    draw(&mut a, 80, 24);
    activate(&mut a, CommandId::DisableServer);
    assert_eq!(a.session.state().language_server_label(), "LSP off");
    key(&mut a, KeyCode::F(2), KeyModifiers::NONE);
    draw(&mut a, 80, 24);
    assert!(a.choices().iter().any(|c| matches!(
        c.target,
        ChoiceTarget::Action(CommandId::EnableServer)
    ) && c.disabled.is_none()));
    activate(&mut a, CommandId::EnableServer);
    settle(&mut a, |a| {
        a.session
            .state()
            .provider()
            .is_some_and(|v| matches!(v.status, Status::Missing(_)))
    });
    assert_eq!(std::fs::read_to_string(path).unwrap(), source);
    assert_eq!(a.session.state().selection(), selected);
}
#[test]
fn finder_and_find_use_real_core_workers() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notes.txt");
    std::fs::write(&path, "needle\nneedle\n").unwrap();
    let mut a = app(dir.path());
    settle(&mut a, |a| {
        a.session.state().project().is_some_and(|p| p.files() > 0)
    });
    a.run(CommandId::FindFile);
    a.handle(Event::Paste("notes.txt:2".into()));
    settle(&mut a, |a| {
        a.session
            .state()
            .navigation()
            .is_some_and(|s| !s.pending && !s.rows.is_empty())
    });
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    settle(&mut a, |a| a.session.state().document().is_some());
    assert_eq!(a.session.state().active().unwrap().source(), path);
    assert_eq!(a.session.state().selection().unwrap().head, 7);
    a.run(CommandId::FindInFile);
    a.handle(Event::Paste("needle".into()));
    settle(&mut a, |a| {
        a.session
            .state()
            .active()
            .and_then(|t| t.find())
            .is_some_and(|f| f.matches().len() == 2)
    });
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    assert!(a.modal.is_none());
}
#[test]
fn empty_real_constructor_and_tiny_frames_are_safe() {
    let dir = tempfile::tempdir().unwrap();
    let mut a = app(dir.path());
    for command in [
        CommandId::Reload,
        CommandId::Close,
        CommandId::Copy,
        CommandId::GoToLine,
    ] {
        a.run(command);
    }
    a.modal = None;
    let backend = ratatui::backend::TestBackend::new(1, 1);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();
    terminal.draw(|f| a.draw(f)).unwrap();
    key(&mut a, KeyCode::F(1), KeyModifiers::NONE);
    key(&mut a, KeyCode::End, KeyModifiers::NONE);
    key(&mut a, KeyCode::Up, KeyModifiers::NONE);
    terminal.draw(|f| a.draw(f)).unwrap();
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    assert!(a.modal.is_none());
}

fn draw(app: &mut App, width: u16, height: u16) {
    let mut terminal =
        ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
    terminal.draw(|f| app.draw(f)).unwrap();
}
fn click(app: &mut App, x: u16, y: u16) {
    app.handle(Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: x,
        row: y,
        modifiers: KeyModifiers::NONE,
    }));
}
#[test]
fn mouse_selection_keeps_utf8_bytes_and_modal_blocks_underlying_clicks() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("unicode.txt");
    std::fs::write(&path, "α界 z\nsecond\n").unwrap();
    let mut a = app(dir.path());
    a.dispatch(Command::Open(path.clone()));
    settle(&mut a, |a| a.session.state().document().is_some());
    draw(&mut a, 80, 20);
    let (x, y) = {
        let p = &a.layout.panes[0];
        (p.text.x, p.text.y)
    };
    click(&mut a, x, y);
    a.handle(Event::Mouse(MouseEvent {
        kind: MouseEventKind::Drag(MouseButton::Left),
        column: x + 3,
        row: y,
        modifiers: KeyModifiers::NONE,
    }));
    assert_eq!(a.session.state().selected_text(), Some("α界"));
    key(&mut a, KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert_eq!(a.copy.take().as_deref(), Some("α界"));
    let before = a.session.state().selection();
    key(&mut a, KeyCode::F(10), KeyModifiers::NONE);
    draw(&mut a, 80, 20);
    click(&mut a, x, y);
    assert_eq!(a.session.state().selection(), before);
    assert_eq!(std::fs::read_to_string(path).unwrap(), "α界 z\nsecond\n");
}
#[test]
fn split_tab_and_bookmark_shortcuts_reach_shared_state() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("one.txt");
    let second = dir.path().join("two.txt");
    std::fs::write(&first, "one\ntwo\nthree\n").unwrap();
    std::fs::write(&second, "other\n").unwrap();
    let mut a = app(dir.path());
    a.dispatch(Command::OpenRetained(first.clone()));
    settle(&mut a, |a| a.session.state().document().is_some());
    key(&mut a, KeyCode::Char('\\'), KeyModifiers::CONTROL);
    assert_eq!(
        a.session.state().split(),
        Some(reader_core::session::Split::Columns)
    );
    assert_eq!(a.session.state().focused_pane(), 1);
    a.dispatch(Command::OpenRetained(second));
    settle(&mut a, |a| {
        a.session
            .state()
            .active()
            .is_some_and(|t| t.source().ends_with("two.txt") && t.loaded())
    });
    key(&mut a, KeyCode::Char('1'), KeyModifiers::CONTROL);
    assert_eq!(a.session.state().active().unwrap().source(), first);
    key(&mut a, KeyCode::F(11), KeyModifiers::CONTROL);
    assert_eq!(a.modal.as_ref().unwrap().kind, Prompt::Mnemonic);
    key(&mut a, KeyCode::Char('a'), KeyModifiers::NONE);
    assert!(a.modal.is_none());
    assert_eq!(
        a.session.state().places().bookmarks()[0].mnemonic,
        Some('A')
    );
    a.dispatch(Command::GoToLine(
        lenz_content::goto::parse_line_column("3").unwrap(),
    ));
    a.dispatch(Command::JumpMnemonic('A'));
    assert_eq!(a.session.state().selection().unwrap().head, 0);
    key(&mut a, KeyCode::F(11), KeyModifiers::SHIFT);
    assert_eq!(a.choices().len(), 1);
    key(&mut a, KeyCode::Delete, KeyModifiers::NONE);
    assert!(a.session.state().places().bookmarks().is_empty());
}
#[test]
fn project_search_results_preview_then_open_through_input() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("file.txt");
    std::fs::write(&path, "first\nneedle\nlast\n").unwrap();
    let mut a = app(dir.path());
    settle(&mut a, |a| {
        a.session
            .state()
            .project()
            .is_some_and(|p| matches!(p.coverage(), reader_core::project::Coverage::Complete(_)))
    });
    key(
        &mut a,
        KeyCode::Char('f'),
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    );
    a.handle(Event::Paste("needle".into()));
    key(&mut a, KeyCode::Tab, KeyModifiers::NONE);
    a.handle(Event::Paste("*.txt".into()));
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    settle(&mut a, |a| {
        a.session.state().results().is_some_and(|r| r.done())
    });
    assert_eq!(a.session.state().results().unwrap().hits().len(), 1);
    assert!(a.session.state().active().is_none());
    draw(&mut a, 100, 30);
    key(&mut a, KeyCode::End, KeyModifiers::NONE);
    settle(&mut a, |a| {
        a.session
            .state()
            .results()
            .is_some_and(|r| r.preview().is_some())
    });
    assert!(a.session.state().active().is_none());
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    settle(&mut a, |a| a.session.state().document().is_some());
    assert_eq!(a.session.state().active().unwrap().source(), path);
    assert_eq!(a.session.state().selection().unwrap().range(), 6..12);
}
#[test]
fn path_validation_and_empty_independent_view_preserve_open_document() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("one.txt");
    std::fs::write(&path, "one").unwrap();
    let mut a = app(dir.path());
    a.dispatch(Command::Open(path.clone()));
    settle(&mut a, |a| a.session.state().document().is_some());
    a.run(CommandId::OpenFolder);
    a.handle(Event::Paste("one.txt".into()));
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    assert!(a.modal.as_ref().unwrap().error.is_some());
    assert_eq!(a.session.state().active().unwrap().source(), path);
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    a.run(CommandId::NewWindow);
    assert!(a.new_view);
    assert_eq!(a.session.state().active().unwrap().source(), path);
    let empty = App::new(config(dir.path()), None);
    assert!(empty.session.state().active().is_none());
    key(&mut a, KeyCode::F(1), KeyModifiers::NONE);
    key(&mut a, KeyCode::Char('q'), KeyModifiers::CONTROL);
    assert!(a.quit);
}

#[test]
fn markdown_preview_selects_and_copies_rendered_text_without_source_selection() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notes.md");
    std::fs::write(&path, "# Heading\n\nSome **bold** text.\n").unwrap();
    let mut a = app(dir.path());
    a.dispatch(Command::Open(path.clone()));
    settle(&mut a, |a| a.session.state().document().is_some());
    a.run(CommandId::MarkdownSource);
    settle(&mut a, |a| {
        matches!(a.session.state().rendering(), Some(Rendering::Preview(_)))
    });
    draw(&mut a, 100, 30);
    let rendered = match a.session.state().rendering() {
        Some(Rendering::Preview(p)) => p.doc.text.clone(),
        _ => unreachable!(),
    };
    let source_selection = a.session.state().selection();
    key(&mut a, KeyCode::Char('a'), KeyModifiers::CONTROL);
    key(&mut a, KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert_eq!(a.copy.take().as_deref(), Some(rendered.as_str()));
    assert!(!rendered.contains("**"));
    assert_eq!(a.session.state().selection(), source_selection);
    let (start, end) = {
        let p = a
            .layout
            .panes
            .iter()
            .find(|p| p.pane == a.session.state().focused_pane())
            .unwrap();
        assert!(p.preview);
        let first = p.hits.first().unwrap();
        let third = p.hits.iter().filter(|h| h.y == first.y).nth(2).unwrap();
        ((first.x, first.y), (third.x, third.y))
    };
    click(&mut a, start.0, start.1);
    a.handle(Event::Mouse(MouseEvent {
        kind: MouseEventKind::Drag(MouseButton::Left),
        column: end.0,
        row: end.1,
        modifiers: KeyModifiers::NONE,
    }));
    key(&mut a, KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert_eq!(a.copy.take().as_deref(), Some("He"));
    a.run(CommandId::MarkdownSource);
    assert!(a.preview_selection.is_none());
    assert_eq!(
        std::fs::read_to_string(path).unwrap(),
        "# Heading\n\nSome **bold** text.\n"
    );
}

#[test]
fn terminal_shortcuts_wrap_toggle_preview_reorder_tabs_and_reverse_focus() {
    let dir = tempfile::tempdir().unwrap();
    let one = dir.path().join("one.txt");
    let two = dir.path().join("two.txt");
    std::fs::write(&one, "one\n").unwrap();
    std::fs::write(&two, "two\n").unwrap();
    let mut a = app(dir.path());
    a.dispatch(Command::OpenRetained(one));
    settle(&mut a, |a| a.session.state().document().is_some());
    a.dispatch(Command::OpenRetained(two));
    settle(&mut a, |a| {
        a.session
            .state()
            .active()
            .is_some_and(|t| t.loaded() && t.source().ends_with("two.txt"))
    });
    let id = a.session.state().active().unwrap().id();
    key(
        &mut a,
        KeyCode::Left,
        KeyModifiers::ALT | KeyModifiers::SHIFT,
    );
    assert_eq!(a.session.state().pane_tabs(0).next().unwrap().id(), id);
    let wrap = a.session.state().soft_wrap();
    key(&mut a, KeyCode::Char('z'), KeyModifiers::ALT);
    assert_eq!(a.session.state().soft_wrap(), !wrap);
    key(&mut a, KeyCode::Tab, KeyModifiers::NONE);
    assert_eq!(a.focus, Focus::Tree);
    key(&mut a, KeyCode::BackTab, KeyModifiers::SHIFT);
    assert_eq!(a.focus, Focus::Text);
}
#[test]
fn long_single_line_markdown_preview_scrolls_within_block_and_clamps() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("long.md");
    std::fs::write(&path, "word ".repeat(2_000)).unwrap();
    let mut a = app(dir.path());
    a.dispatch(Command::Open(path));
    settle(&mut a, |a| a.session.state().document().is_some());
    key(
        &mut a,
        KeyCode::Char('v'),
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    );
    settle(&mut a, |a| {
        matches!(a.session.state().rendering(), Some(Rendering::Preview(_)))
    });
    draw(&mut a, 60, 15);
    let first = a.layout.panes[0].hits.first().unwrap().byte;
    key(&mut a, KeyCode::PageDown, KeyModifiers::NONE);
    draw(&mut a, 60, 15);
    assert!(a.layout.panes[0].hits.first().unwrap().byte > first);
    key(&mut a, KeyCode::End, KeyModifiers::NONE);
    draw(&mut a, 60, 15);
    let end = a.layout.panes[0].preview_top;
    assert!(end > 0);
    key(&mut a, KeyCode::Up, KeyModifiers::NONE);
    draw(&mut a, 60, 15);
    assert_eq!(a.layout.panes[0].preview_top, end - 1);
    draw(&mut a, 25, 8);
    assert!(a.layout.panes[0].preview_top < a.layout.panes[0].preview_total);
    key(&mut a, KeyCode::Home, KeyModifiers::NONE);
    draw(&mut a, 25, 8);
    assert_eq!(a.layout.panes[0].preview_top, 0);
}

#[test]
fn tree_selection_follows_path_when_streaming_inserts_an_earlier_row() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("z.txt"), "z").unwrap();
    let mut a = app(dir.path());
    settle(&mut a, |a| {
        a.session
            .state()
            .project()
            .is_some_and(|p| matches!(p.coverage(), reader_core::project::Coverage::Complete(_)))
    });
    let p = a.session.state().project().unwrap();
    a.tree_at = p.rows().iter().position(|r| &*r.path == "z.txt").unwrap();
    let generation = p.generation();
    a.apply(Completion::Scanned {
        generation,
        batch: reader_core::project::Batch {
            paths: vec!["a.txt".into()],
            done: true,
            skipped: 0,
            error: None,
        },
    });
    assert_eq!(
        &*a.session.state().project().unwrap().rows()[a.tree_at].path,
        "z.txt"
    );
}
#[test]
fn hover_is_debounced_cancelled_and_uses_real_core_documentation_request() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"hover-fixture\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    let path = root.join("main.rs");
    let source = "fn main() { helper(); }\nfn helper() {}\n";
    std::fs::write(&path, source).unwrap();
    let profile = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_owned();
    let mut build =
        std::process::Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    build
        .args(["build", "-q", "-p", "reader-core", "--example", "fake_lsp"])
        .current_dir(env!("CARGO_MANIFEST_DIR"));
    if profile.ends_with("release") {
        build.arg("--release");
    }
    assert!(build.status().unwrap().success());
    let fake = profile.join(format!("examples/fake_lsp{}", std::env::consts::EXE_SUFFIX));
    let log = dir.path().join("hover.jsonl");
    let mut cfg = config(dir.path());
    cfg.idle = reader_core::semantics::IDLE;
    std::fs::create_dir_all(cfg.providers.parent().unwrap()).unwrap();
    std::fs::write(&cfg.providers, serde_json::json!({ "providers": { "rust-analyzer": { "executable": fake, "env": { "FAKE_LSP_LOG": log, "FAKE_LSP_NO_DECOR": "1" } } } }).to_string()).unwrap();
    let mut a = App::new(cfg, Some(root));
    a.dispatch(Command::Open(path.clone()));
    settle(&mut a, |a| {
        a.session
            .state()
            .provider()
            .is_some_and(|p| p.status == reader_core::semantics::Status::Ready)
            && a.session.state().active().unwrap().highlights().is_some()
    });
    draw(&mut a, 100, 30);
    let at = source.find("helper").unwrap();
    let (x, y) = {
        let h = a.layout.panes[0]
            .hits
            .iter()
            .find(|h| h.line == 0 && h.byte == at)
            .unwrap();
        (h.x, h.y)
    };
    let moved = Event::Mouse(MouseEvent {
        kind: MouseEventKind::Moved,
        column: x,
        row: y,
        modifiers: KeyModifiers::NONE,
    });
    a.handle(moved.clone());
    let due = a.next_deadline().unwrap();
    assert!(!a.tick(due - Duration::from_millis(1)));
    a.handle(moved.clone());
    assert_eq!(a.next_deadline(), Some(due));
    let selection = a.session.state().selection();
    assert!(a.tick(due));
    settle(&mut a, |a| a.session.state().documentation().is_some());
    assert_eq!(
        a.session.state().documentation().unwrap().trigger,
        reader_core::session::DocTrigger::Pointer
    );
    assert!(matches!(a.modal.as_ref().unwrap().kind, Prompt::Details(_)));
    assert_eq!(a.session.state().selection(), selection);
    let messages = std::fs::read_to_string(&log).unwrap();
    assert_eq!(
        messages
            .lines()
            .filter(
                |line| serde_json::from_str::<serde_json::Value>(line).unwrap()["method"]
                    == "textDocument/hover"
            )
            .count(),
        1
    );
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    assert!(a.session.state().documentation().is_none());
    a.run(CommandId::DocumentationOnHover);
    a.handle(moved.clone());
    assert!(a.next_deadline().is_none());
    a.run(CommandId::DocumentationOnHover);
    a.handle(moved);
    assert!(a.next_deadline().is_some());
    key(&mut a, KeyCode::F(10), KeyModifiers::NONE);
    assert!(a.next_deadline().is_none());
    assert_eq!(std::fs::read_to_string(path).unwrap(), source);
}

#[test]
fn shifted_selection_head_stays_in_real_terminal_viewport() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("lines.txt");
    let source = (0..100)
        .map(|line| format!("line {line}\n"))
        .collect::<String>();
    std::fs::write(&path, &source).unwrap();
    let mut a = app(dir.path());
    a.dispatch(Command::Open(path.clone()));
    settle(&mut a, |a| a.session.state().document().is_some());
    draw(&mut a, 80, 12);
    for _ in 0..30 {
        key(&mut a, KeyCode::Down, KeyModifiers::SHIFT);
        draw(&mut a, 80, 12);
    }
    let head = a.session.state().selection().unwrap().head;
    let line = a.session.state().document().unwrap().line_of(head);
    assert_eq!(line, 30);
    assert!(a.layout.panes[0].rows.iter().any(|(_, l, _)| *l == line));
    assert_eq!(std::fs::read_to_string(path).unwrap(), source);
}

#[test]
fn a_drag_started_on_an_old_document_version_cannot_select_reloaded_text() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("changing.txt");
    std::fs::write(&path, "original content\n").unwrap();
    let mut a = app(dir.path());
    a.dispatch(Command::Open(path.clone()));
    settle(&mut a, |a| a.session.state().document().is_some());
    draw(&mut a, 80, 20);
    let (x, y) = {
        let h = &a.layout.panes[0].hits[0];
        (h.x, h.y)
    };
    click(&mut a, x, y);
    let version = a.session.state().document().unwrap().version();
    std::fs::write(&path, "replaced\n").unwrap();
    a.run(CommandId::Reload);
    settle(&mut a, |a| {
        a.session
            .state()
            .document()
            .is_some_and(|d| d.version() != version)
    });
    draw(&mut a, 80, 20);
    let selection = a.session.state().selection();
    a.handle(Event::Mouse(MouseEvent {
        kind: MouseEventKind::Drag(MouseButton::Left),
        column: x + 4,
        row: y,
        modifiers: KeyModifiers::NONE,
    }));
    assert_eq!(a.session.state().selection(), selection);
    assert_eq!(std::fs::read_to_string(path).unwrap(), "replaced\n");
}

#[test]
#[cfg(unix)]
fn server_runtime_dialog_validates_saves_and_preserves_the_document() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let source = "class Main {}\n";
    let path = dir.path().join("Main.java");
    std::fs::write(&path, source).unwrap();
    let java = dir.path().join("java with spaces");
    std::fs::write(&java, "#!/bin/sh\ntest \"$1\" = -version\n").unwrap();
    std::fs::set_permissions(&java, std::fs::Permissions::from_mode(0o755)).unwrap();
    let cfg = config(dir.path());
    let settings = cfg.providers.clone();
    std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
    let original = r#"{"providers":{"jdtls":{"executable":"/nonexistent/jdtls"}},"keep":true}"#;
    std::fs::write(&settings, original).unwrap();
    let mut a = App::new(cfg, Some(dir.path().into()));
    a.dispatch(Command::Open(path.clone()));
    settle(&mut a, |a| a.session.state().document().is_some());
    key(&mut a, KeyCode::Char('a'), KeyModifiers::CONTROL);
    let selected = a.session.state().selection();
    key(&mut a, KeyCode::F(2), KeyModifiers::NONE);
    let at = a
        .choices()
        .iter()
        .position(|c| matches!(c.target, ChoiceTarget::Action(CommandId::ServerRuntime)))
        .unwrap();
    for _ in 0..at {
        key(&mut a, KeyCode::Down, KeyModifiers::NONE);
    }
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(a.modal.as_ref().unwrap().kind, Prompt::ServerRuntime);
    a.handle(Event::Paste("/nonexistent/java".into()));
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    settle(&mut a, |a| {
        a.session
            .state()
            .server_runtime()
            .unwrap()
            .message
            .contains("not changed")
    });
    assert_eq!(std::fs::read_to_string(&settings).unwrap(), original);
    for (w, h) in [(26, 8), (1, 1), (100, 26)] {
        draw(&mut a, w, h);
        key(&mut a, KeyCode::End, KeyModifiers::NONE);
        key(&mut a, KeyCode::Up, KeyModifiers::NONE);
        draw(&mut a, w, h);
        assert!(a.modal.as_ref().unwrap().at < a.choices().len());
    }
    for _ in 0.."/nonexistent/java".len() {
        key(&mut a, KeyCode::Backspace, KeyModifiers::NONE);
    }
    a.handle(Event::Paste(java.to_str().unwrap().into()));
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    settle(&mut a, |a| {
        a.session
            .state()
            .server_runtime()
            .unwrap()
            .message
            .contains("saved.")
    });
    assert_eq!(
        a.session
            .state()
            .server_runtime()
            .unwrap()
            .configured
            .as_ref(),
        Some(&java)
    );
    key(&mut a, KeyCode::F(5), KeyModifiers::NONE);
    assert_eq!(a.session.state().selection(), selected);
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    assert!(a.modal.is_none());
    assert!(a.session.state().server_runtime().is_none());
    let saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&settings).unwrap()).unwrap();
    assert_eq!(saved["keep"], true);
    assert_eq!(
        saved["providers"]["jdtls"]["args"],
        serde_json::json!([format!("--java-executable={}", java.display())])
    );
    assert_eq!(std::fs::read_to_string(path).unwrap(), source);
}

fn navigation_ready(a: &mut App) {
    settle(a, |a| {
        a.session.state().navigation().is_some_and(|s| !s.pending)
    });
}

#[test]
fn navigation_shortcuts_cycle_scopes_and_keep_modal_input_isolated() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sample.txt");
    let source = "hello\nworld\n";
    std::fs::write(&path, source).unwrap();
    let mut a = app(dir.path());
    a.dispatch(Command::Open(path.clone()));
    settle(&mut a, |a| a.session.state().document().is_some());
    a.dispatch(Command::Select(Selection { anchor: 0, head: 5 }));
    let selection = a.session.state().selection();
    let ctrl_shift = KeyModifiers::CONTROL | KeyModifiers::SHIFT;
    for (code, modifiers, scope) in [
        ('p', KeyModifiers::CONTROL, Scope::Files),
        ('N', ctrl_shift, Scope::Files),
        ('N', ctrl_shift | KeyModifiers::ALT, Scope::Symbols),
        ('A', ctrl_shift, Scope::Actions),
        ('P', ctrl_shift, Scope::Actions),
        ('k', KeyModifiers::CONTROL, Scope::All),
    ] {
        key(&mut a, KeyCode::Char(code), modifiers);
        assert_eq!(a.modal.as_ref().unwrap().kind, Prompt::Navigation);
        assert_eq!(a.session.state().navigation().unwrap().scope, scope);
        key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
        assert!(a.session.state().navigation().is_none());
    }
    key(&mut a, KeyCode::Char('k'), KeyModifiers::CONTROL);
    a.handle(Event::Paste("α界\nquery".into()));
    assert_eq!(a.modal.as_ref().unwrap().input, "α界query");
    key(&mut a, KeyCode::Tab, KeyModifiers::NONE);
    assert_eq!(a.session.state().navigation().unwrap().scope, Scope::Files);
    key(&mut a, KeyCode::BackTab, KeyModifiers::SHIFT);
    assert_eq!(a.session.state().navigation().unwrap().scope, Scope::All);
    assert_eq!(a.session.state().navigation().unwrap().query, "α界query");
    key(&mut a, KeyCode::Char('s'), KeyModifiers::CONTROL);
    key(&mut a, KeyCode::Backspace, KeyModifiers::NONE);
    assert_eq!(a.modal.as_ref().unwrap().input, "α界quer");
    assert_eq!(a.session.state().selection(), selection);
    assert_eq!(std::fs::read_to_string(path).unwrap(), source);
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
}

#[test]
fn navigation_opens_syntactic_symbols_and_toggles_actions_through_events() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sample.rs");
    let source = "// heading\nfn uniquely_named_symbol() {}\n";
    std::fs::write(&path, source).unwrap();
    std::fs::create_dir_all(dir.path().join("config")).unwrap();
    std::fs::write(
        dir.path().join("config/providers.json"),
        serde_json::json!({"providers":{"rust-analyzer":{"disabled":[dir.path()]}}}).to_string(),
    )
    .unwrap();
    let mut a = app(dir.path());
    settle(&mut a, |a| {
        a.session
            .state()
            .project()
            .is_some_and(|p| matches!(p.coverage(), reader_core::project::Coverage::Complete(_)))
    });
    a.run(CommandId::FindSymbol);
    a.handle(Event::Paste("uniquely_named_symbol".into()));
    settle(&mut a, |a| {
        a.session.state().navigation().is_some_and(|s| {
            !s.pending
                && s.rows
                    .iter()
                    .any(|r| r.kind == reader_core::navigation::Kind::Symbol)
        })
    });
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    settle(&mut a, |a| a.session.state().document().is_some());
    assert_eq!(a.session.state().active().unwrap().source(), path);
    assert_eq!(
        a.session.state().selection().unwrap().head,
        source.find("fn uniquely_named_symbol").unwrap()
    );
    assert_eq!(a.session.state().language_server_label(), "LSP off");
    let before = a.session.state().soft_wrap();
    a.run(CommandId::FindAction);
    a.handle(Event::Paste("soft wrap".into()));
    navigation_ready(&mut a);
    let index = a
        .session
        .state()
        .navigation()
        .unwrap()
        .rows
        .iter()
        .position(|r| r.label == CommandId::SoftWrap.label())
        .unwrap();
    a.modal.as_mut().unwrap().at = index;
    draw(&mut a, 100, 26);
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    assert!(a.modal.is_none());
    assert!(a.session.state().navigation().is_none());
    assert_ne!(a.session.state().soft_wrap(), before);
    assert_eq!(std::fs::read_to_string(path).unwrap(), source);
}

#[test]
fn navigation_mouse_scopes_and_rows_reject_stale_displayed_revisions() {
    let dir = tempfile::tempdir().unwrap();
    let mut a = app(dir.path());
    a.run(CommandId::FindAction);
    a.handle(Event::Paste("soft wrap".into()));
    navigation_ready(&mut a);
    draw(&mut a, 100, 26);
    let old_revision = a.layout.navigation_revision.unwrap();
    let (x, y) = (a.layout.modal_rows.x, a.layout.modal_rows.y);
    let before = a.session.state().soft_wrap();
    a.handle(Event::Paste("xxxxxxxx".into()));
    navigation_ready(&mut a);
    assert_ne!(
        a.session.state().navigation().unwrap().revision,
        old_revision
    );
    click(&mut a, x, y);
    assert!(a.modal.is_some());
    assert_eq!(a.session.state().soft_wrap(), before);
    a.accept_navigation(a.session.state().navigation().unwrap().revision, usize::MAX);
    assert!(a.modal.is_some());
    key(&mut a, KeyCode::Char('u'), KeyModifiers::CONTROL);
    navigation_ready(&mut a);
    draw(&mut a, 100, 26);
    let (_, rect) = a
        .layout
        .navigation_scopes
        .iter()
        .find(|(s, _)| *s == Scope::Files)
        .unwrap();
    let (x, y) = (rect.x, rect.y);
    click(&mut a, x, y);
    assert_eq!(a.session.state().navigation().unwrap().scope, Scope::Files);
    assert!(a.modal.as_ref().unwrap().input.is_empty());
    a.navigation_scope(Scope::Actions);
    a.handle(Event::Paste("soft wrap".into()));
    navigation_ready(&mut a);
    draw(&mut a, 100, 26);
    let (x, y) = (a.layout.modal_rows.x, a.layout.modal_rows.y);
    click(&mut a, x, y);
    assert!(a.modal.is_none());
    assert_ne!(a.session.state().soft_wrap(), before);
}

fn shift_event(a: &mut App, kind: KeyEventKind) -> bool {
    a.handle(Event::Key(KeyEvent::new_with_kind(
        KeyCode::Modifier(ModifierKeyCode::LeftShift),
        if kind == KeyEventKind::Release {
            KeyModifiers::NONE
        } else {
            KeyModifiers::SHIFT
        },
        kind,
    )))
}

#[test]
fn navigation_double_shift_requires_modifier_reports_and_uninterrupted_taps() {
    let dir = tempfile::tempdir().unwrap();
    let mut a = app(dir.path());
    assert!(!shift_event(&mut a, KeyEventKind::Press));
    assert!(!shift_event(&mut a, KeyEventKind::Release));
    assert!(!shift_event(&mut a, KeyEventKind::Press));
    assert!(!shift_event(&mut a, KeyEventKind::Repeat));
    assert!(shift_event(&mut a, KeyEventKind::Release));
    assert_eq!(a.session.state().navigation().unwrap().scope, Scope::All);
    assert!(!a.handle(Event::Key(KeyEvent::new(
        KeyCode::Modifier(ModifierKeyCode::LeftControl),
        KeyModifiers::CONTROL,
    ))));
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    shift_event(&mut a, KeyEventKind::Press);
    key(&mut a, KeyCode::Char('X'), KeyModifiers::SHIFT);
    shift_event(&mut a, KeyEventKind::Release);
    shift_event(&mut a, KeyEventKind::Press);
    shift_event(&mut a, KeyEventKind::Release);
    assert!(a.modal.is_none(), "shifted text cannot count as a tap");
    a.handle(Event::FocusLost);
    shift_event(&mut a, KeyEventKind::Press);
    shift_event(&mut a, KeyEventKind::Release);
    assert!(a.modal.is_none(), "focus changes clear the previous tap");
    a.handle(Event::FocusLost);
    for _ in 0..2 {
        key(&mut a, KeyCode::Char('X'), KeyModifiers::SHIFT);
    }
    assert!(a.modal.is_none(), "uppercase text is never a double Shift");
    a.handle(Event::Key(KeyEvent::new_with_kind(
        KeyCode::Char('k'),
        KeyModifiers::CONTROL,
        KeyEventKind::Repeat,
    )));
    assert!(a.modal.is_none(), "search openers reject repeat events");
    a.handle(Event::Key(KeyEvent::new_with_kind(
        KeyCode::Char('k'),
        KeyModifiers::CONTROL,
        KeyEventKind::Release,
    )));
    assert!(a.modal.is_none(), "ordinary commands reject release events");
}

#[test]
fn navigation_enhanced_keyboard_repeats_move_caret_and_modal_selection() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("repeated.txt");
    std::fs::write(&path, "line\n".repeat(100)).unwrap();
    let mut a = app(dir.path());
    a.dispatch(Command::Open(path));
    settle(&mut a, |a| a.session.state().document().is_some());
    draw(&mut a, 100, 26);
    let repeat = |a: &mut App, code| {
        a.handle(Event::Key(KeyEvent::new_with_kind(
            code,
            KeyModifiers::NONE,
            KeyEventKind::Repeat,
        )));
    };
    repeat(&mut a, KeyCode::Down);
    assert_eq!(
        a.session.state().selection(),
        Some(Selection { anchor: 5, head: 5 })
    );
    assert_eq!(a.session.state().active().unwrap().top_line(), 0);
    a.run(CommandId::FindAction);
    navigation_ready(&mut a);
    draw(&mut a, 100, 26);
    key(&mut a, KeyCode::Down, KeyModifiers::NONE);
    repeat(&mut a, KeyCode::Down);
    assert_eq!(a.modal.as_ref().unwrap().at, 2);
    repeat(&mut a, KeyCode::Char('s'));
    assert_eq!(a.modal.as_ref().unwrap().input, "s");
    assert_eq!(
        a.session.state().selection(),
        Some(Selection { anchor: 5, head: 5 })
    );
    assert_eq!(a.session.state().active().unwrap().top_line(), 0);
}

#[test]
fn navigation_scope_shortcuts_preserve_query_inside_modal_and_plain_ctrl_a_clears() {
    let dir = tempfile::tempdir().unwrap();
    let mut a = app(dir.path());
    a.run(CommandId::SearchEverywhere);
    a.handle(Event::Paste("α界 typed query".into()));
    let ctrl_shift = KeyModifiers::CONTROL | KeyModifiers::SHIFT;
    for (code, modifiers, scope) in [
        ('A', ctrl_shift, Scope::Actions),
        ('p', KeyModifiers::CONTROL, Scope::Files),
        ('e', KeyModifiers::CONTROL, Scope::Recent),
        ('N', ctrl_shift | KeyModifiers::ALT, Scope::Symbols),
        ('P', ctrl_shift, Scope::Actions),
        ('N', ctrl_shift, Scope::Files),
        ('k', KeyModifiers::CONTROL, Scope::All),
    ] {
        a.modal.as_mut().unwrap().at = 5;
        a.modal.as_mut().unwrap().top = 4;
        key(&mut a, KeyCode::Char(code), modifiers);
        let modal = a.modal.as_ref().unwrap();
        assert_eq!(modal.kind, Prompt::Navigation);
        assert_eq!(modal.input, "α界 typed query");
        assert_eq!((modal.at, modal.top), (0, 0));
        let search = a.session.state().navigation().unwrap();
        assert_eq!(search.scope, scope);
        assert_eq!(search.query, "α界 typed query");
    }
    key(&mut a, KeyCode::Char('a'), KeyModifiers::CONTROL);
    assert!(a.modal.as_ref().unwrap().input.is_empty());
    assert!(a.session.state().navigation().unwrap().query.is_empty());
    assert_eq!(a.session.state().navigation().unwrap().scope, Scope::All);
}

#[test]
fn popup_wheel_moves_visible_rows_without_changing_enter_target_or_document() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("source.txt");
    std::fs::write(&path, "hello\nworld\n").unwrap();
    let mut a = app(dir.path());
    a.dispatch(Command::Open(path.clone()));
    settle(&mut a, |a| a.session.state().document().is_some());
    a.dispatch(Command::Select(Selection { anchor: 0, head: 5 }));
    let selection = a.session.state().selection();
    a.prompt(Prompt::Commands);
    draw(&mut a, 40, 10);
    let selected = a.modal.as_ref().unwrap().at;
    let first_target = a.choices()[selected].label.clone();
    for _ in 0..4 {
        a.handle(Event::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        }));
        draw(&mut a, 40, 10);
    }
    assert_eq!(a.modal.as_ref().unwrap().at, selected);
    assert!(a.modal.as_ref().unwrap().top > 0);
    assert!(!a.layout.modal_items.contains(&selected));
    assert_eq!(
        a.choices()[a.modal.as_ref().unwrap().at].label,
        first_target
    );
    assert_eq!(a.session.state().selection(), selection);
    assert_eq!(a.session.state().active().unwrap().top_line(), 0);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello\nworld\n");
    key(&mut a, KeyCode::Up, KeyModifiers::NONE);
    draw(&mut a, 40, 10);
    assert_eq!(a.modal.as_ref().unwrap().top, 0);
}

#[test]
fn plain_arrow_keys_move_utf8_caret_and_keep_both_axes_visible() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("caret.txt");
    let source = format!("é界z{}\n{}", "x".repeat(120), "é界z\n".repeat(60));
    std::fs::write(&path, &source).unwrap();
    let mut a = app(dir.path());
    a.dispatch(Command::Open(path.clone()));
    settle(&mut a, |a| a.session.state().document().is_some());
    draw(&mut a, 40, 12);
    for (code, head) in [
        (KeyCode::Right, 2),
        (KeyCode::Right, 5),
        (KeyCode::Left, 2),
        (KeyCode::Down, 129),
        (KeyCode::Up, 2),
    ] {
        key(&mut a, code, KeyModifiers::NONE);
        assert_eq!(
            a.session.state().selection(),
            Some(Selection { anchor: head, head }),
            "{code:?} lines {:?} {:?}",
            a.session.state().document().unwrap().line_range(0),
            a.session.state().document().unwrap().line_range(1)
        );
    }
    for _ in 0..50 {
        a.handle(Event::Key(KeyEvent::new_with_kind(
            KeyCode::Right,
            KeyModifiers::NONE,
            KeyEventKind::Repeat,
        )));
    }
    draw(&mut a, 40, 12);
    assert!(a.session.state().column() > 0);
    let head = a.session.state().selection().unwrap().head;
    assert!(a.layout.panes[0].hits.iter().any(|h| h.byte == head));
    for _ in 0..30 {
        key(&mut a, KeyCode::Down, KeyModifiers::NONE);
        draw(&mut a, 40, 12);
    }
    let selection = a.session.state().selection().unwrap();
    assert_eq!(selection.anchor, selection.head);
    let line = a
        .session
        .state()
        .document()
        .unwrap()
        .line_of(selection.head);
    assert_eq!(line, 30);
    assert!(a.layout.panes[0].rows.iter().any(|(_, l, _)| *l == line));
    key(&mut a, KeyCode::Right, KeyModifiers::SHIFT);
    assert_ne!(
        a.session.state().selection().unwrap().anchor,
        a.session.state().selection().unwrap().head
    );
    assert_eq!(std::fs::read_to_string(path).unwrap(), source);
}

#[test]
fn ctrl_e_opens_the_same_searchable_file_popup_as_find_files() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("recent.txt");
    std::fs::write(&path, "recent\n").unwrap();
    let mut a = app(dir.path());
    a.dispatch(Command::Open(path));
    settle(&mut a, |a| a.session.state().document().is_some());
    key(&mut a, KeyCode::Char('e'), KeyModifiers::CONTROL);
    assert_eq!(a.modal.as_ref().unwrap().kind, Prompt::Navigation);
    navigation_ready(&mut a);
    draw(&mut a, 80, 20);
    assert!(a.layout.navigation_revision.is_some());
}

#[test]
fn navigation_wheel_keeps_hidden_selection_for_enter() {
    let dir = tempfile::tempdir().unwrap();
    let mut a = app(dir.path());
    a.run(CommandId::FindAction);
    navigation_ready(&mut a);
    let index = a
        .session
        .state()
        .navigation()
        .unwrap()
        .rows
        .iter()
        .position(|r| r.label == CommandId::SoftWrap.label())
        .unwrap();
    a.modal.as_mut().unwrap().at = index;
    draw(&mut a, 40, 10);
    let before = a.session.state().soft_wrap();
    for _ in 0..100 {
        a.handle(Event::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        }));
        draw(&mut a, 40, 10);
    }
    assert_eq!(a.modal.as_ref().unwrap().at, index);
    assert_eq!(a.modal.as_ref().unwrap().top, 0);
    assert!(!a.layout.modal_items.contains(&index));
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    assert!(a.modal.is_none());
    assert_ne!(a.session.state().soft_wrap(), before);
}

#[test]
fn help_and_wrapped_details_wheel_scroll_without_selection_and_clamp_on_resize() {
    let dir = tempfile::tempdir().unwrap();
    let mut a = app(dir.path());
    for prompt in [
        Prompt::Help,
        Prompt::Details("long wrapped detail ".repeat(200)),
    ] {
        a.prompt(prompt);
        draw(&mut a, 24, 10);
        let selected = a.modal.as_ref().unwrap().at;
        for _ in 0..12 {
            a.handle(Event::Mouse(MouseEvent {
                kind: MouseEventKind::ScrollDown,
                column: 0,
                row: 0,
                modifiers: KeyModifiers::NONE,
            }));
            draw(&mut a, 24, 10);
        }
        assert_eq!(a.modal.as_ref().unwrap().at, selected);
        assert_eq!(a.modal.as_ref().unwrap().top, 12);
        key(&mut a, KeyCode::End, KeyModifiers::NONE);
        draw(&mut a, 24, 10);
        let end = a.modal.as_ref().unwrap().top;
        key(&mut a, KeyCode::Up, KeyModifiers::NONE);
        draw(&mut a, 24, 10);
        assert_eq!(a.modal.as_ref().unwrap().top, end - 1);
        draw(&mut a, 100, 26);
        assert!(
            a.modal.as_ref().unwrap().top
                <= a.layout
                    .modal_content_rows
                    .saturating_sub(a.layout.modal_rows.height as usize)
        );
        draw(&mut a, 1, 1);
        key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
        assert!(a.modal.is_none());
    }
}

#[test]
fn plain_arrows_follow_visual_wrap_rows_and_skip_collapsed_source_lines() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fold.rs");
    let source = "fn main() {\n  let x = 1;\n  let y = 2;\n}\ntrailing\n";
    std::fs::write(&path, source).unwrap();
    let mut a = app(dir.path());
    a.dispatch(Command::Open(path.clone()));
    settle(&mut a, |a| {
        a.session
            .state()
            .active()
            .is_some_and(|t| t.folds().is_some())
    });
    a.dispatch(Command::ToggleFold(0));
    draw(&mut a, 70, 12);
    key(&mut a, KeyCode::Down, KeyModifiers::NONE);
    let selection = a.session.state().selection().unwrap();
    assert_eq!(selection.anchor, selection.head);
    assert_eq!(
        a.session
            .state()
            .document()
            .unwrap()
            .line_of(selection.head),
        4
    );
    key(&mut a, KeyCode::Up, KeyModifiers::NONE);
    assert_eq!(
        a.session.state().selection(),
        Some(Selection { anchor: 0, head: 0 })
    );

    let wrapped = dir.path().join("wrapped.txt");
    std::fs::write(&wrapped, "abcdef ".repeat(30)).unwrap();
    a.dispatch(Command::Open(wrapped.clone()));
    settle(&mut a, |a| {
        a.session
            .state()
            .active()
            .is_some_and(|t| t.loaded() && t.source() == wrapped)
    });
    a.run(CommandId::SoftWrap);
    draw(&mut a, 30, 10);
    let second_row = a.layout.panes[0].rows[1].1;
    assert_eq!(second_row, 0);
    key(&mut a, KeyCode::Down, KeyModifiers::NONE);
    let selection = a.session.state().selection().unwrap();
    assert!(selection.head > 0 && selection.head < 210);
    assert_eq!(selection.anchor, selection.head);
    assert_eq!(
        a.session
            .state()
            .document()
            .unwrap()
            .line_of(selection.head),
        0
    );
    key(&mut a, KeyCode::Up, KeyModifiers::NONE);
    assert_eq!(
        a.session.state().selection(),
        Some(Selection { anchor: 0, head: 0 })
    );
    assert_eq!(std::fs::read_to_string(path).unwrap(), source);
}

#[test]
fn recent_popup_fuzzy_filters_forgets_and_opens_scrolled_paths_at_saved_caret() {
    let dir = tempfile::tempdir().unwrap();
    let mut a = app(dir.path());
    let mut paths = Vec::new();
    for i in 0..10 {
        let path = dir.path().join(format!("folder-{i}/same_name.txt"));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "first\nsecond\nthird\n").unwrap();
        a.dispatch(Command::OpenRetained(path.clone()));
        settle(&mut a, |a| {
            a.session
                .state()
                .active()
                .is_some_and(|t| t.loaded() && t.source() == path)
        });
        a.dispatch(Command::Select(Selection { anchor: 8, head: 8 }));
        paths.push(path);
    }
    key(&mut a, KeyCode::Char('e'), KeyModifiers::CONTROL);
    navigation_ready(&mut a);
    assert_eq!(a.session.state().navigation().unwrap().scope, Scope::Recent);
    assert!(
        a.session.state().navigation().unwrap().rows[0]
            .detail
            .contains("folder-8")
    );
    key(&mut a, KeyCode::Tab, KeyModifiers::NONE);
    assert_eq!(a.session.state().navigation().unwrap().scope, Scope::Recent);
    draw(&mut a, 40, 10);
    for _ in 0..4 {
        a.handle(Event::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        }));
        draw(&mut a, 40, 10);
    }
    assert_eq!(a.modal.as_ref().unwrap().at, 0);
    let visible = a.layout.modal_items[0];
    assert!(visible > 0);
    assert_eq!(visible, 4);
    assert!(
        a.session.state().navigation().unwrap().rows[visible]
            .detail
            .contains("folder-5")
    );
    let target = paths[5].clone();
    let (x, y) = (a.layout.modal_rows.x, a.layout.modal_rows.y);
    click(&mut a, x, y);
    settle(&mut a, |a| {
        a.session
            .state()
            .active()
            .is_some_and(|t| t.loaded() && t.source() == target)
    });
    assert_eq!(
        a.session.state().selection(),
        Some(Selection { anchor: 8, head: 8 })
    );
    key(&mut a, KeyCode::Char('e'), KeyModifiers::CONTROL);
    a.handle(Event::Paste("fdr3/smnm".into()));
    navigation_ready(&mut a);
    assert_eq!(a.session.state().navigation().unwrap().rows.len(), 1);
    assert!(
        a.session.state().navigation().unwrap().rows[0]
            .detail
            .contains("folder-3")
    );
    key(&mut a, KeyCode::Delete, KeyModifiers::NONE);
    assert_eq!(
        a.session.state().navigation().unwrap().rows.len(),
        1,
        "Delete must reject a stale displayed revision"
    );
    draw(&mut a, 40, 10);
    key(&mut a, KeyCode::Delete, KeyModifiers::NONE);
    navigation_ready(&mut a);
    assert!(a.session.state().navigation().unwrap().rows.is_empty());
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    for path in paths {
        assert_eq!(
            std::fs::read_to_string(path).unwrap(),
            "first\nsecond\nthird\n"
        );
    }
}

#[test]
fn chrome_status_click_opens_copyable_snapshot_and_keeps_document_input_modal() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("status.txt");
    let bytes = "hello 界e\u{301}\r\nsecond\r\n";
    std::fs::write(&path, bytes).unwrap();
    let mut a = app(dir.path());
    a.dispatch(Command::Open(path.clone()));
    settle(&mut a, |a| a.session.state().document().is_some());
    a.notice = Some("Adapter warning 界e\u{301} ".repeat(20) + "\nsecond diagnostic line");
    draw(&mut a, 100, 26);
    let before = a.session.state().selection();
    click(&mut a, 30, 25);
    let snapshot = match &a.modal.as_ref().expect("status click opens details").kind {
        Prompt::Details(body) => body.clone(),
        other => panic!("unexpected prompt: {other:?}"),
    };
    assert!(snapshot.contains(a.notice.as_deref().unwrap()));
    a.notice = Some("new background notice".into());
    key(&mut a, KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert_eq!(a.copy.as_deref(), Some(snapshot.as_str()));
    for (w, h) in [(28, 10), (8, 5), (1, 1), (0, 0), (100, 26)] {
        draw(&mut a, w, h);
        key(&mut a, KeyCode::End, KeyModifiers::NONE);
        draw(&mut a, w, h);
        key(&mut a, KeyCode::Up, KeyModifiers::NONE);
        key(&mut a, KeyCode::Right, KeyModifiers::SHIFT);
        key(&mut a, KeyCode::Char('s'), KeyModifiers::CONTROL);
        a.handle(Event::Paste("should not edit".into()));
    }
    assert_eq!(a.session.state().selection(), before);
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    assert!(a.modal.is_none());
    assert_eq!(std::fs::read_to_string(path).unwrap(), bytes);
}

#[test]
fn chrome_menu_button_opens_shared_commands_without_moving_caret() {
    let dir = tempfile::tempdir().unwrap();
    let mut a = app(dir.path());
    draw(&mut a, 100, 26);
    click(&mut a, 2, 0);
    assert!(matches!(a.modal.as_ref().map(|m| &m.kind), Some(Prompt::Commands)));
}

#[test]
fn chrome_toolbar_controls_and_footer_commands_run_catalog_actions() {
    let dir = tempfile::tempdir().unwrap();
    let mut a = app(dir.path());
    settle(&mut a, |a| a.session.state().project().is_some());
    for (id, expected) in [
        (CommandId::Open, Prompt::Open),
        (CommandId::FindFile, Prompt::Navigation),
        (CommandId::SearchProject, Prompt::Search),
    ] {
        draw(&mut a, 120, 26);
        let (_, rect) = a.layout.controls.iter().find(|(control, _)| *control == id).unwrap();
        let (x, y) = (rect.x, rect.y);
        click(&mut a, x, y);
        assert_eq!(a.modal.as_ref().unwrap().kind, expected);
        key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    }
    draw(&mut a, 120, 26);
    let (_, rect) = a.layout.controls.iter().find(|(id, _)| *id == CommandId::ToggleExplorer).unwrap();
    let (x, y) = (rect.x, rect.y);
    let visible = a.tree_shown;
    click(&mut a, x, y);
    assert_eq!(a.tree_shown, !visible);
    draw(&mut a, 120, 26);
    let rect = *a.layout.command_menus.last().unwrap();
    click(&mut a, rect.x, rect.y);
    assert_eq!(a.modal.as_ref().unwrap().kind, Prompt::Commands);
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    key(&mut a, KeyCode::F(2), KeyModifiers::SHIFT);
    assert!(matches!(a.modal.as_ref().unwrap().kind, Prompt::Details(_)));
    draw(&mut a, 120, 26);
    // A toolbar click dismisses the dialog, without opening its underlying command.
    click(&mut a, 2, 0);
    assert!(a.modal.is_none());
}
