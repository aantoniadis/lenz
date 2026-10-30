use crate::app::{App, Focus, Prompt, tests::{config, draw, draw_screen, settle}};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use code_core::session::{Command, CommandId};
use std::path::Path;

fn key(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
    app.handle(Event::Key(KeyEvent::new(code, modifiers)));
}

fn source(app: &App) -> String {
    let source = app.session.state().current_source().unwrap();
    source.copy_range(0..source.len_bytes()).unwrap()
}

fn row_menu(app: &mut App, name: &str) {
    draw(app, 100, 30);
    let row = app.session.state().tree_rows().position(|row| &*row.path == name).unwrap() - app.tree_top;
    let area = app.layout.tree;
    app.handle(Event::Mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Right), column: area.x + 8, row: area.y + row as u16, modifiers: KeyModifiers::NONE }));
    assert!(matches!(app.modal.as_ref().map(|m| &m.kind), Some(Prompt::Context { .. })));
}

fn context_command(app: &mut App, command: CommandId) {
    let index = app.choices().iter().position(|choice| choice.label == command.label()).unwrap();
    for _ in 0..index { key(app, KeyCode::Down, KeyModifiers::NONE); }
    key(app, KeyCode::Enter, KeyModifiers::NONE);
}

fn open_project(state: &Path, root: &Path) -> App {
    let mut app = App::new(config(state), Some(root.to_owned()));
    app.session.disable_desktop_file_clipboard_for_test();
    app.enable_draft_input_for_test();
    settle(&mut app, |a| a.session.state().project().is_some_and(|p| !p.refreshing() && p.rows().iter().any(|row| &*row.path == "src")));
    app.dispatch(Command::OpenRetained(root.join("src/a.txt")));
    app.dispatch(Command::RevealInTree("src/a.txt".into()));
    settle(&mut app, |a| a.session.state().current_source().is_some());
    app
}

#[test]
fn explorer_copy_text_uses_the_right_clicked_entry_not_the_editor() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/a.txt"), b"active\n").unwrap();
    std::fs::write(root.join("src/b.txt"), b"selected\n").unwrap();
    let mut app = open_project(&dir.path().join("state"), &root);
    for (command, expected) in [
        (CommandId::CopyExplorerName, "b.txt".to_string()),
        (CommandId::CopyExplorerFullPath, root.join("src/b.txt").display().to_string()),
        (CommandId::CopyExplorerRelativePath, "src/b.txt".to_string()),
    ] {
        row_menu(&mut app, "src/b.txt");
        let rows = app.choices();
        for id in [CommandId::CopyExplorerName, CommandId::CopyExplorerFullPath, CommandId::CopyExplorerRelativePath, CommandId::CopyExplorerEntry, CommandId::PasteExplorerEntry, CommandId::ExplorerHistory] {
            assert!(rows.iter().any(|row| row.label == id.label()), "{} missing", id.label());
        }
        assert_eq!(app.session.state().explorer().selected.as_deref(), Some("src/b.txt"));
        context_command(&mut app, command);
        assert_eq!(app.copy.take(), Some(expected));
        assert_eq!(app.session.state().active().unwrap().source(), root.join("src/a.txt"));
    }
    assert_eq!(source(&app), "active\n");
    assert_eq!(std::fs::read(root.join("src/b.txt")).unwrap(), b"selected\n");
}

#[test]
fn explorer_copy_paste_keys_keep_the_dirty_source_and_modal_ownership() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/a.txt"), b"active\r\n").unwrap();
    std::fs::write(root.join("src/b.txt"), b"other\r\n").unwrap();
    let mut app = open_project(&dir.path().join("state"), &root);
    draw(&mut app, 100, 30);
    let area = app.layout.panes[0].text;
    app.handle(Event::Mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: area.x + 1, row: area.y, modifiers: KeyModifiers::NONE }));
    key(&mut app, KeyCode::Home, KeyModifiers::CONTROL);
    key(&mut app, KeyCode::Char('x'), KeyModifiers::NONE);
    assert_eq!(source(&app), "xactive\r\n");
    row_menu(&mut app, "src/a.txt");
    key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    key(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert!(app.copy.is_none(), "tree Copy does not send a path as terminal text clipboard data");
    row_menu(&mut app, "src");
    key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    key(&mut app, KeyCode::Char('v'), KeyModifiers::CONTROL);
    assert!(matches!(app.modal.as_ref().map(|m| &m.kind), Some(Prompt::PasteEntry(request)) if request.source() == root.join("src/a.txt") && request.folder() == "src"));
    assert_eq!(app.modal.as_ref().unwrap().input, "a.txt");
    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    assert!(app.modal.as_ref().unwrap().error.as_deref().unwrap().contains("already exists"));
    key(&mut app, KeyCode::Char('s'), KeyModifiers::CONTROL);
    key(&mut app, KeyCode::Char('z'), KeyModifiers::CONTROL);
    key(&mut app, KeyCode::Char('x'), KeyModifiers::NONE);
    for (w, h) in [(1, 1), (12, 4), (100, 30)] {
        draw(&mut app, w, h);
        key(&mut app, KeyCode::End, KeyModifiers::NONE);
        key(&mut app, KeyCode::Up, KeyModifiers::NONE);
    }
    assert_eq!(source(&app), "xactive\r\n");
    assert_eq!(std::fs::read(root.join("src/a.txt")).unwrap(), b"active\r\n");
    key(&mut app, KeyCode::Char('a'), KeyModifiers::CONTROL);
    for c in "copied.txt".chars() { key(&mut app, KeyCode::Char(c), KeyModifiers::NONE); }
    assert_eq!(app.modal.as_ref().unwrap().input, "copied.txt", "the advertised Ctrl+A replaces the suggested name");
    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    settle(&mut app, |a| root.join("src/copied.txt").exists() && a.session.disabled(CommandId::CancelExplorerCopy).is_some());
    assert_eq!(std::fs::read(root.join("src/copied.txt")).unwrap(), b"active\r\n");
    assert_eq!(source(&app), "xactive\r\n");
    draw(&mut app, 100, 30);
    let area = app.layout.panes[0].text;
    app.handle(Event::Mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: area.x + 1, row: area.y, modifiers: KeyModifiers::NONE }));
    assert_eq!(app.focus, Focus::Text);
    key(&mut app, KeyCode::Char('a'), KeyModifiers::CONTROL);
    key(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert_eq!(app.copy.as_deref(), Some("xactive\r\n"));
    key(&mut app, KeyCode::Char('v'), KeyModifiers::CONTROL);
    assert!(app.modal.is_none(), "editor Ctrl+V never opens tree Paste");
    app.handle(Event::Paste("editor paste".into()));
    assert_eq!(source(&app), "editor paste");
    key(&mut app, KeyCode::Char('z'), KeyModifiers::CONTROL);
    assert_eq!(source(&app), "xactive\r\n");
    assert_eq!(std::fs::read(root.join("src/a.txt")).unwrap(), b"active\r\n");
}

#[test]
fn explorer_history_dispatches_the_selected_file_and_folder() {
    use lenz_content::git::testing::{repo, write, commit_all};
    let repository = repo();
    write(repository.path(), "src/a.txt", "active\n");
    write(repository.path(), "src/b.txt", "selected\n");
    commit_all(repository.path(), "files");
    let state = tempfile::tempdir().unwrap();
    std::fs::write(state.path().join("lenz-diff-tui"), "").unwrap();
    let mut app = open_project(state.path(), repository.path());
    app.exe = Some(state.path().join("lenz-code-tui"));
    row_menu(&mut app, "src/b.txt");
    settle(&mut app, |a| a.session.explorer_history_disabled().is_none());
    context_command(&mut app, CommandId::ExplorerHistory);
    let file = app.foreground.take().unwrap();
    row_menu(&mut app, "src");
    context_command(&mut app, CommandId::ExplorerHistory);
    let folder = app.foreground.take().unwrap();
    assert_eq!(file.args, ["log", "--follow", "src/b.txt"]);
    assert_eq!(folder.args, ["log", "--", ":(literal)src"]);
    assert_eq!(file.cwd.as_deref(), Some(repository.path()));
    assert!(file.program.ends_with("lenz-diff-tui") && folder.program.ends_with("lenz-diff-tui"));
    assert_eq!(source(&app), "active\n");
}

#[test]
fn explorer_clipboard_runtime_flushes_only_the_latest_ticket_including_inactive_views() {
    let dir = tempfile::tempdir().unwrap();
    let first = App::new(config(&dir.path().join("first")), None);
    let mut second = App::new(config(&dir.path().join("second")), None);
    second.session.share_clipboard_for_test(&first.session);
    let mut views = vec![first, second];
    // View 1 is active: an older pending copy in inactive view 0 must never overwrite it.
    views[0].dispatch(Command::CopyText("old".into()));
    views[1].dispatch(Command::CopyText("new".into()));
    let old = views[0].copy_ticket.unwrap();
    let latest = views[1].copy_ticket.unwrap();
    assert!(!views[0].session.clipboard_text_current(old));
    assert!(views[1].session.clipboard_text_current(latest));
    let mut output = Vec::new();
    crate::runtime::flush_clipboards(&mut views, &mut output).unwrap();
    assert_eq!(output, b"\x1b]52;c;bmV3\x07");
    assert!(views.iter().all(|view| view.copy.is_none() && view.copy_ticket.is_none()));

    // A completion in inactive view 0 publishes immediately; the later slot is now stale.
    views[1].dispatch(Command::CopyText("old".into()));
    views[0].dispatch(Command::CopyText("inactive".into()));
    output.clear();
    crate::runtime::flush_clipboards(&mut views, &mut output).unwrap();
    assert_eq!(output, b"\x1b]52;c;aW5hY3RpdmU=\x07");
    assert!(views.iter().all(|view| view.copy.is_none() && view.copy_ticket.is_none()));
    crate::runtime::flush_clipboards(&mut views, &mut output).unwrap();
    assert_eq!(output, b"\x1b]52;c;aW5hY3RpdmU=\x07", "acknowledged copies are not emitted again");
    assert!(views.iter().all(|view| view.session.state().current_source().is_none()));
}

#[test]
fn explorer_type_search_selects_without_opening_and_escape_restores() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/a.txt"), b"active\n").unwrap();
    std::fs::write(root.join("src/Beta.txt"), b"candidate\n").unwrap();
    let mut app = open_project(&dir.path().join("state"), &root);
    app.focus = Focus::Tree;
    app.tree_at = app.session.state().tree_rows().position(|r| &*r.path == "src/a.txt").unwrap();
    app.remember_explorer();
    key(&mut app, KeyCode::Char('b'), KeyModifiers::NONE);
    assert_eq!(&*app.session.state().tree_rows().nth(app.tree_at).unwrap().path, "src/Beta.txt");
    assert_eq!(app.session.state().active().unwrap().source(), root.join("src/a.txt"));
    key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert_eq!(&*app.session.state().tree_rows().nth(app.tree_at).unwrap().path, "src/a.txt");
    assert_eq!(app.focus, Focus::Tree);
}

#[test]
fn explorer_search_keys_cover_unicode_directories_no_match_and_modal_precedence() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    std::fs::create_dir_all(root.join("src/BetaDir")).unwrap();
    for (name, text) in [("src/a.txt", "active\r\n"), ("src/Éclair.txt", "unicode\n"), ("src/BetaDir/hidden.txt", "hidden\n")] {
        std::fs::write(root.join(name), text).unwrap();
    }
    let mut app = open_project(&dir.path().join("state"), &root);
    app.focus = Focus::Tree;
    let active = app.session.state().active().unwrap().id();
    key(&mut app, KeyCode::Char('é'), KeyModifiers::NONE);
    assert_eq!(app.session.state().explorer().selected.as_deref(), Some("src/Éclair.txt"));
    key(&mut app, KeyCode::Char('c'), KeyModifiers::NONE);
    key(&mut app, KeyCode::Backspace, KeyModifiers::NONE);
    assert_eq!(app.session.state().explorer_search().query(), "é");
    for theme in ["darcula", "github-light"] {
        app.dispatch(Command::Theme(lenz_content::theme::ThemeCommand::Select(lenz_content::theme::ThemeSelection::from_id(theme).unwrap())));
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let buffer = terminal.backend().buffer();
        let shown: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
        assert!(shown.contains("Search: é"), "{shown}");
        let palette = app.session.state().palette();
        let rgb = |n: u32| ratatui::style::Color::Rgb((n >> 16) as u8, (n >> 8) as u8, n as u8);
        assert!(buffer.content.iter().any(|cell| cell.symbol() == "É" && cell.bg == rgb(palette.accent)), "the matching filename text is highlighted");
        for (width, height) in [(100, 30), (30, 8), (1, 1)] { draw(&mut app, width, height); }
    }
    key(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert_eq!(app.session.state().explorer_search().query(), "é", "tree Copy keeps its shortcut");
    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    settle(&mut app, |a| a.session.state().current_source().is_some_and(|t| t.source() == root.join("src/Éclair.txt")));
    assert!(app.session.state().explorer_search().query().is_empty());
    for c in "betadir".chars() { key(&mut app, KeyCode::Char(c), KeyModifiers::NONE); }
    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    assert!(app.session.state().tree_rows().any(|r| &*r.path == "src/BetaDir" && r.dir == Some(true)));
    for c in "missing".chars() { key(&mut app, KeyCode::Char(c), KeyModifiers::NONE); }
    assert_eq!(app.session.state().explorer_search().matched(), 0);
    let before = app.session.state().active().unwrap().id();
    key(&mut app, KeyCode::F(10), KeyModifiers::NONE);
    assert!(app.modal.is_some());
    for c in "abc".chars() { key(&mut app, KeyCode::Char(c), KeyModifiers::NONE); }
    key(&mut app, KeyCode::Char('s'), KeyModifiers::CONTROL);
    key(&mut app, KeyCode::Char('z'), KeyModifiers::CONTROL);
    assert_eq!(app.session.state().explorer_search().query(), "missing");
    assert_eq!(source(&app), "unicode\n");
    key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(app.session.state().active().unwrap().id(), before);
    assert_ne!(before, active);
    assert_eq!(std::fs::read(root.join("src/a.txt")).unwrap(), b"active\r\n");
}

#[test]
fn clicked_folder_or_file_keeps_the_explorer_focused_for_keys_and_search() {
    let state = tempfile::tempdir().unwrap();
    let root = tempfile::tempdir().unwrap();
    for name in ["src/a.txt", "src/b.txt", "docs/c.txt"] {
        std::fs::create_dir_all(root.path().join(name).parent().unwrap()).unwrap();
        std::fs::write(root.path().join(name), name).unwrap();
    }
    let mut app = open_project(state.path(), root.path());
    let click = |app: &mut App, name: &str| {
        draw(app, 100, 30);
        let row = app.session.state().tree_rows().position(|row| &*row.path == name).unwrap() - app.tree_top;
        let area = app.layout.tree;
        app.focus = Focus::Text;
        app.handle(Event::Mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: area.x + 8, row: area.y + row as u16, modifiers: KeyModifiers::NONE }));
        app.handle(Event::Mouse(MouseEvent { kind: MouseEventKind::Up(MouseButton::Left), column: area.x + 8, row: area.y + row as u16, modifiers: KeyModifiers::NONE }));
    };
    let highlighted = |app: &App| app.session.state().tree_rows().nth(app.tree_at).map(|r| r.path.to_string());
    // A folder's name: the explorer keeps the keys; Down moves and typing searches it.
    click(&mut app, "docs");
    assert_eq!((app.focus, highlighted(&app).as_deref()), (Focus::Tree, Some("docs")));
    key(&mut app, KeyCode::Down, KeyModifiers::NONE);
    assert_ne!(highlighted(&app).as_deref(), Some("docs"));
    click(&mut app, "docs");
    key(&mut app, KeyCode::Char('d'), KeyModifiers::NONE);
    assert_eq!((app.focus, app.session.state().explorer_search().query()), (Focus::Tree, "d"));
    key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    // A file opens and the explorer keeps the keys, as in the desktop.
    click(&mut app, "src/b.txt");
    settle(&mut app, |a| a.session.state().current_source().is_some_and(|s| s.source().ends_with("b.txt")));
    assert_eq!((app.focus, highlighted(&app).as_deref()), (Focus::Tree, Some("src/b.txt")));
    key(&mut app, KeyCode::Up, KeyModifiers::NONE);
    assert_eq!(highlighted(&app).as_deref(), Some("src/a.txt"));
    key(&mut app, KeyCode::Char('c'), KeyModifiers::NONE);
    assert_eq!(app.session.state().explorer_search().query(), "c");
    assert_eq!(source(&app), "src/b.txt", "typing searches the explorer, not the document");
}

#[test]
fn explorer_right_edge_drags_its_width_and_the_layout_remembers_it() {
    let state = tempfile::tempdir().unwrap();
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("src")).unwrap();
    for name in ["src/a.txt", "src/b.txt"] { std::fs::write(root.path().join(name), name).unwrap(); }
    let mut app = open_project(state.path(), root.path());
    draw(&mut app, 100, 30);
    let tree = app.layout.tree;
    assert_eq!(tree.width, 25, "a quarter of the width by default");
    let event = |app: &mut App, kind, column| app.handle(Event::Mouse(MouseEvent { kind, column, row: tree.y + 3, modifiers: KeyModifiers::NONE }));
    // A click that does not move the edge keeps the width following the window.
    event(&mut app, MouseEventKind::Down(MouseButton::Left), tree.right() - 1);
    event(&mut app, MouseEventKind::Up(MouseButton::Left), tree.right() - 1);
    assert_eq!(app.session.state().layout().explorer_columns, None);
    event(&mut app, MouseEventKind::Down(MouseButton::Left), tree.right() - 1);
    event(&mut app, MouseEventKind::Drag(MouseButton::Left), 39);
    draw(&mut app, 100, 30);
    assert_eq!(app.layout.tree.width, 40, "the edge follows the pointer");
    event(&mut app, MouseEventKind::Up(MouseButton::Left), 39);
    assert_eq!(app.session.state().layout().explorer_columns, Some(40));
    // The text keeps at least 24 columns; a narrower window shrinks the explorer, not the text.
    event(&mut app, MouseEventKind::Down(MouseButton::Left), 39);
    event(&mut app, MouseEventKind::Drag(MouseButton::Left), 95);
    event(&mut app, MouseEventKind::Up(MouseButton::Left), 95);
    draw(&mut app, 100, 30);
    assert_eq!(app.layout.tree.width, 76);
    draw(&mut app, 60, 20);
    assert_eq!(app.layout.tree.width, 36);
    // A click inside the explorer still selects its rows.
    draw(&mut app, 100, 30);
    let row = app.session.state().tree_rows().position(|r| &*r.path == "src/b.txt").unwrap() - app.tree_top;
    app.handle(Event::Mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: app.layout.tree.x + 8, row: app.layout.tree.y + row as u16, modifiers: KeyModifiers::NONE }));
    assert_eq!(app.session.state().tree_rows().nth(app.tree_at).map(|r| r.path.to_string()).as_deref(), Some("src/b.txt"));
    assert_eq!(std::fs::read(root.path().join("src/a.txt")).unwrap(), b"src/a.txt");
}

#[test]
fn the_split_panes_boundary_drags_their_share_and_the_layout_remembers_it() {
    let state = tempfile::tempdir().unwrap();
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("src")).unwrap();
    for name in ["src/a.txt", "src/b.txt"] { std::fs::write(root.path().join(name), name).unwrap(); }
    let mut app = open_project(state.path(), root.path());
    app.dispatch(Command::Open(root.path().join("src/a.txt")));
    settle(&mut app, |a| a.session.state().current_source().is_some());
    app.dispatch(Command::Split(code_core::session::Split::Columns));
    draw(&mut app, 100, 30);
    let event = |app: &mut App, kind, column, row| app.handle(Event::Mouse(MouseEvent { kind, column, row, modifiers: KeyModifiers::NONE }));
    let widths = |app: &App| app.layout.panes.iter().map(|g| g.tab_bar.width).collect::<Vec<_>>();
    let reading = app.layout.reading;
    assert_eq!(widths(&app), [37, 37], "halves by default, with the divider column between them");
    let divider = app.layout.divider;
    assert_eq!((divider.x + 1, divider.width), (app.layout.panes[1].tab_bar.x, 1));
    let screen = draw_screen(&mut app, 100, 30);
    assert!(screen.contains('│'), "the divider is drawn");
    // The first pane's last column grabs the boundary; the second pane's gutter beside it keeps its clicks.
    let edge = app.layout.panes[1].tab_bar.x;
    event(&mut app, MouseEventKind::Down(MouseButton::Left), edge, reading.y + 5);
    assert!(app.split_drag.is_none());
    event(&mut app, MouseEventKind::Up(MouseButton::Left), edge, reading.y + 5);
    event(&mut app, MouseEventKind::Down(MouseButton::Left), edge - 1, reading.y + 5);
    event(&mut app, MouseEventKind::Drag(MouseButton::Left), reading.x + 49, reading.y + 5);
    draw(&mut app, 100, 30);
    assert_eq!(widths(&app), [49, 25], "the boundary follows the pointer");
    event(&mut app, MouseEventKind::Up(MouseButton::Left), reading.x + 49, reading.y + 5);
    assert_eq!(app.session.state().layout().split_permille, Some(666));
    // Each pane keeps at least 12 columns.
    event(&mut app, MouseEventKind::Down(MouseButton::Left), reading.x + 49, reading.y + 5);
    event(&mut app, MouseEventKind::Drag(MouseButton::Left), 99, reading.y + 5);
    event(&mut app, MouseEventKind::Up(MouseButton::Left), 99, reading.y + 5);
    draw(&mut app, 100, 30);
    assert_eq!(widths(&app)[1], 12);
    // Rows: the lower pane's tab bar, past its tabs, is the boundary; its tab still takes a click.
    app.dispatch(Command::Split(code_core::session::Split::Rows));
    app.dispatch(Command::SetLayout(code_core::session::Layout { split_permille: None, ..app.session.state().layout() }));
    draw(&mut app, 100, 30);
    let bar = app.layout.panes[1].tab_bar;
    let tab = app.layout.panes[1].tabs[0].1;
    event(&mut app, MouseEventKind::Down(MouseButton::Left), tab.x, bar.y);
    assert!(app.split_drag.is_none(), "a tab is not the boundary");
    event(&mut app, MouseEventKind::Up(MouseButton::Left), tab.x, bar.y);
    event(&mut app, MouseEventKind::Down(MouseButton::Left), bar.right() - 20, bar.y);
    event(&mut app, MouseEventKind::Drag(MouseButton::Left), bar.right() - 20, bar.y - 4);
    event(&mut app, MouseEventKind::Up(MouseButton::Left), bar.right() - 20, bar.y - 4);
    draw(&mut app, 100, 30);
    assert_eq!(app.layout.panes[1].tab_bar.y, bar.y - 4);
    assert!(app.session.state().layout().split_permille.is_some_and(|k| k < 500));
}

#[test]
fn the_results_heading_drags_the_panels_height_and_the_layout_remembers_it() {
    let state = tempfile::tempdir().unwrap();
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("src")).unwrap();
    std::fs::write(root.path().join("src/a.txt"), "needle\n").unwrap();
    let mut app = open_project(state.path(), root.path());
    app.dispatch(Command::Search { query: "needle".into(), scope: code_core::session::SearchIn::Project });
    settle(&mut app, |a| a.session.state().results().is_some_and(|r| r.done()));
    draw(&mut app, 100, 31);
    let event = |app: &mut App, kind, column, row| app.handle(Event::Mouse(MouseEvent { kind, column, row, modifiers: KeyModifiers::NONE }));
    let heading = app.layout.results.y - 1;
    let before = app.layout.results.height;
    // A plain click on the heading keeps the size.
    event(&mut app, MouseEventKind::Down(MouseButton::Left), 30, heading);
    event(&mut app, MouseEventKind::Up(MouseButton::Left), 30, heading);
    assert_eq!(app.session.state().layout().results_permille, None);
    event(&mut app, MouseEventKind::Down(MouseButton::Left), 30, heading);
    event(&mut app, MouseEventKind::Drag(MouseButton::Left), 30, heading - 6);
    draw(&mut app, 100, 31);
    assert_eq!(app.layout.results.height, before + 6, "the heading follows the pointer");
    event(&mut app, MouseEventKind::Up(MouseButton::Left), 30, heading - 6);
    assert!(app.session.state().layout().results_permille.is_some());
    // The text keeps three rows however far it is dragged.
    let heading = app.layout.results.y - 1;
    event(&mut app, MouseEventKind::Down(MouseButton::Left), 30, heading);
    event(&mut app, MouseEventKind::Drag(MouseButton::Left), 30, 0);
    event(&mut app, MouseEventKind::Up(MouseButton::Left), 30, 0);
    draw(&mut app, 100, 31);
    assert!(app.layout.reading.height >= 3);
}

#[test]
fn the_wheel_scrolls_the_explorer_without_moving_its_highlight() {
    let state = tempfile::tempdir().unwrap();
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("src")).unwrap();
    std::fs::write(root.path().join("src/a.txt"), "a").unwrap();
    for i in 0..60 { std::fs::write(root.path().join(format!("src/f{i:02}.txt")), "x").unwrap(); }
    let mut app = open_project(state.path(), root.path());
    draw(&mut app, 100, 20);
    let (at, top) = (app.tree_at, app.tree_top);
    let area = app.layout.tree;
    let wheel = |app: &mut App, kind| app.handle(Event::Mouse(MouseEvent { kind, column: area.x + 4, row: area.y + 3, modifiers: KeyModifiers::NONE }));
    for _ in 0..5 { wheel(&mut app, MouseEventKind::ScrollDown); }
    draw(&mut app, 100, 20);
    assert_eq!(app.tree_at, at, "the highlight stays");
    assert_eq!(app.tree_top, top + 15, "the view moves three rows a notch");
    wheel(&mut app, MouseEventKind::ScrollUp);
    draw(&mut app, 100, 20);
    assert_eq!((app.tree_at, app.tree_top), (at, top + 12));
    // A key moves the highlight, and the view follows it again.
    app.focus = Focus::Tree;
    key(&mut app, KeyCode::Down, KeyModifiers::NONE);
    draw(&mut app, 100, 20);
    assert_eq!(app.tree_at, at + 1);
    // The list below the title row shows it, step after step past the first screen.
    let visible = |app: &App| app.tree_top <= app.tree_at && app.tree_at < app.tree_top + app.layout.tree.height as usize - 1;
    assert!(visible(&app));
    for _ in 0..30 {
        key(&mut app, KeyCode::Down, KeyModifiers::NONE);
        draw(&mut app, 100, 20);
        assert!(visible(&app), "highlight {} below the list from {}", app.tree_at, app.tree_top);
    }
}
