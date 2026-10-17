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
            .project()
            .and_then(|p| p.finder())
            .is_some_and(|f| f.ready && !f.hits.is_empty())
    });
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    settle(&mut a, |a| a.session.state().document().is_some());
    assert_eq!(a.session.state().active().unwrap().source(), path);
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
