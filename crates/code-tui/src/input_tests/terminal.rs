use super::*;
use code_core::terminal;

#[test]
fn terminal_rename_menu_replaces_the_name_and_cancel_preserves_source() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("keep.txt");
    std::fs::write(&path, b"keep\n").unwrap();
    let mut a = App::new(config(dir.path()), Some(path.clone()));
    settle(&mut a, |a| a.session.state().current_source().is_some());
    a.dispatch(Command::Terminal(terminal::Command::New));
    a.dispatch(Command::Terminal(terminal::Command::New));
    a.open_menu(Some(CommandGroup::Terminal), 0);
    a.modal.as_mut().unwrap().at = a.session.menu(Some(CommandGroup::Terminal)).iter().position(|row| row.target == MenuTarget::Run(CommandId::RenameTerminal)).unwrap();
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    assert!(matches!(a.modal.as_ref().map(|m| &m.kind), Some(Prompt::RenameTerminal { .. })));
    assert!(a.modal.as_ref().unwrap().selected, "the label starts selected");
    assert_eq!(a.modal.as_ref().unwrap().input, "Terminal 2");
    key(&mut a, KeyCode::Backspace, KeyModifiers::NONE);
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    assert!(a.modal.as_ref().unwrap().error.is_some());
    a.handle(Event::Paste("Claude — SQL".into()));
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(a.session.state().terminal().active().unwrap().label(), "Claude — SQL");
    a.run(CommandId::RenameTerminal);
    for (w,h) in [(0,0),(1,1),(18,6),(90,30)] { draw(&mut a,w,h); }
    a.handle(Event::Paste("cancelled".into()));
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    assert_eq!(a.session.state().terminal().active().unwrap().label(), "Claude — SQL");
    assert_eq!(std::fs::read(path).unwrap(), b"keep\n");
}

#[test]
fn terminal_empty_constructor_renders_and_terminal_focus_keeps_input_out_of_editor() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("keep.txt");
    std::fs::write(&path, b"keep\n").unwrap();
    let mut a = App::new(config(dir.path()), Some(path.clone()));
    settle(&mut a, |a| a.session.state().current_source().is_some());
    draw(&mut a, 90, 30);
    assert!(a.layout.terminal.area.is_empty());
    a.dispatch(Command::Terminal(terminal::Command::Show));
    draw(&mut a, 90, 30);
    assert!(!a.layout.terminal.area.is_empty());
    assert!(a.session.state().terminal().focused());
    for code in [KeyCode::Char('q'), KeyCode::Char('s'), KeyCode::Char('z'), KeyCode::Char('c'), KeyCode::Char('d')] {
        key(&mut a, code, KeyModifiers::CONTROL);
    }
    for code in [KeyCode::Enter, KeyCode::Tab, KeyCode::Esc, KeyCode::Char('x')] {
        key(&mut a, code, KeyModifiers::NONE);
    }
    a.handle(Event::Paste("do not edit".into()));
    assert!(!a.quit && !a.quit_all);
    let source = a.session.state().current_source().unwrap();
    assert_eq!(source.copy_range(0..source.len_bytes()).unwrap(), "keep\n");
    assert_eq!(std::fs::read(path).unwrap(), b"keep\n");
    key(&mut a, KeyCode::F(10), KeyModifiers::NONE);
    assert_eq!(a.modal.as_ref().map(|m| &m.kind), Some(&Prompt::Commands));
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    key(&mut a, KeyCode::Char('0'), KeyModifiers::CONTROL | KeyModifiers::ALT);
    assert!(!a.session.state().terminal().focused());
}

#[test]
fn terminal_small_layout_and_modals_are_safe() {
    let dir = tempfile::tempdir().unwrap();
    for dark in [true, false] {
        let mut cfg = config(dir.path()); cfg.dark = dark;
        let mut a = App::new(cfg, None);
        a.dispatch(Command::Terminal(terminal::Command::Show));
        for (w,h) in [(0,0), (1,1), (2,3), (12,6), (90,30)] {
            draw(&mut a, w, h);
            assert!(a.layout.terminal.area.bottom() <= h);
            assert!(a.layout.terminal.area.right() <= w);
        }
        a.prompt(Prompt::Open);
        a.handle(Event::Paste("modal-only".into()));
        assert_eq!(a.modal.as_ref().unwrap().input, "modal-only");
        key(&mut a, KeyCode::End, KeyModifiers::NONE);
        key(&mut a, KeyCode::Up, KeyModifiers::NONE);
        key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
        assert!(a.modal.is_none());
        assert!(!a.quit);
        let hide = a.layout.controls.iter().find(|(id, r)| *id == CommandId::ShowTerminal && a.layout.terminal.area.contains((r.x, r.y).into())).unwrap().1;
        mouse(&mut a, MouseButton::Left, hide.x, hide.y);
        draw(&mut a, 90, 30);
        assert!(!a.session.state().terminal().shown());
        assert!(a.layout.terminal.area.is_empty());
    }
}

fn screen_text(a: &App) -> String {
    a.session.state().terminal().active().and_then(|tab| tab.frame()).map(|frame| frame.cells.iter().map(|c| c.text.as_str()).collect()).unwrap_or_default()
}

#[cfg(target_os = "linux")]
#[test]
fn terminal_real_pty_tabs_input_and_stale_cell_hits() {
    let dir = tempfile::tempdir().unwrap();
    let mut a = App::new(config(dir.path()), None);
    a.dispatch(Command::Terminal(terminal::Command::New));
    settle(&mut a, |a| a.session.state().terminal().active().is_some_and(|t| t.running() && t.frame().is_some()));
    draw(&mut a, 100, 30);
    let first = a.session.state().terminal().active().unwrap().id();
    // Ordinary Enter reaches the PTY, while host paste is handled separately by the paste guard.
    for c in "printf 'terminal_%s' first".chars() { key(&mut a, KeyCode::Char(c), KeyModifiers::NONE); }
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    settle(&mut a, |a| screen_text(a).contains("terminal_first"));
    draw(&mut a, 100, 30);
    let screen = a.session.state().terminal().active().unwrap().frame().unwrap();
    let marker = "terminal_first";
    let start = screen.cells.windows(marker.len()).position(|cells| cells.iter().map(|c|c.text.as_str()).collect::<String>() == marker).unwrap();
    let (row,column) = (start / screen.columns, start % screen.columns);
    let cells = a.layout.terminal.cells;
    let (x,y) = (cells.x + column as u16,cells.y + row as u16);
    mouse(&mut a,MouseButton::Left,x,y);
    for kind in [MouseEventKind::Drag(MouseButton::Left),MouseEventKind::Up(MouseButton::Left)] {
        a.handle(Event::Mouse(MouseEvent {kind,column:x + marker.len() as u16 - 1,row:y,modifiers:KeyModifiers::NONE}));
    }
    key(&mut a,KeyCode::Char('c'),KeyModifiers::CONTROL | KeyModifiers::SHIFT);
    settle(&mut a,|a|a.copy.is_some());
    assert_eq!(a.copy.take().as_deref(),Some(marker));
    let stale_cells = a.layout.terminal.cells;
    let new = a.layout.controls.iter().find(|(id, _)| *id == CommandId::NewTerminal).unwrap().1;
    mouse(&mut a, MouseButton::Left, new.x, new.y);
    let second = a.session.state().terminal().active().unwrap().id();
    assert_ne!(first, second);
    // This coordinate belonged to the previous terminal; it must not select in the new one.
    mouse(&mut a, MouseButton::Left, stale_cells.x, stale_cells.y);
    assert_eq!(a.session.state().terminal().active().unwrap().id(), second);
    settle(&mut a, |a| a.session.state().terminal().active().is_some_and(|t| t.running() && t.frame().is_some()));
    draw(&mut a, 100, 30);
    assert_eq!(a.layout.terminal.active, Some(second));
    key(&mut a, KeyCode::PageUp, KeyModifiers::CONTROL | KeyModifiers::ALT);
    assert_eq!(a.session.state().terminal().active().unwrap().id(), first);
    draw(&mut a, 100, 30);
    let tab = a.layout.terminal.tabs.iter().find(|(id, _)| *id == second).unwrap().1;
    mouse(&mut a, MouseButton::Left, tab.x, tab.y);
    assert_eq!(a.session.state().terminal().active().unwrap().id(), second);
    let stop = a.layout.controls.iter().find(|(id, _)| *id == CommandId::StopTerminal).unwrap().1;
    mouse(&mut a, MouseButton::Left, stop.x, stop.y);
    let ticket = a.session.state().terminal().prompt().unwrap().ticket;
    assert!(matches!(a.modal.as_ref().map(|m| &m.kind), Some(Prompt::Details { kind: Details::Terminal(t), .. }) if *t == ticket));
    a.handle(Event::Paste("must not reach shell".into()));
    draw(&mut a, 30, 8);
    key(&mut a, KeyCode::End, KeyModifiers::NONE);
    let end = a.modal.as_ref().unwrap().top;
    assert!(end > 0);
    key(&mut a, KeyCode::Up, KeyModifiers::NONE);
    assert!(a.modal.as_ref().unwrap().top < end);
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    assert!(a.session.state().terminal().active().unwrap().running());
    assert!(a.modal.is_none());
    draw(&mut a, 100, 30);
    // The path prompt pastes quoted bytes without executing a shell command.
    for c in "printf '<%s>' ".chars() { key(&mut a, KeyCode::Char(c), KeyModifiers::NONE); }
    a.run(CommandId::PasteTerminalPath);
    assert_eq!(a.modal.as_ref().map(|m| &m.kind), Some(&Prompt::TerminalPath));
    let path = dir.path().join("quoted path's.txt");
    std::fs::write(&path, b"keep").unwrap();
    a.handle(Event::Paste(path.display().to_string()));
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    assert!(a.modal.is_none());
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    let output = format!("<{}>",path.display());
    settle(&mut a, |a| screen_text(a).contains(&output));
    assert_eq!(std::fs::read(&path).unwrap(), b"keep");
    // A tab's path menu keeps its captured file if background work activates another document.
    a.dispatch(Command::OpenRetained(path.clone()));
    settle(&mut a,|a|a.session.state().current_source().is_some());
    draw(&mut a,100,30);
    a.terminal_command(terminal::Command::Text("printf 'captured:<%s>' ".into()));
    let tab = a.layout.panes.iter().flat_map(|p|&p.tabs).find(|(id,_)|Some(*id)==a.session.state().active().map(|t|t.id())).unwrap().1;
    mouse(&mut a,MouseButton::Right,tab.x,tab.y);
    let other = dir.path().join("other.txt"); std::fs::write(&other,b"other").unwrap();
    a.dispatch(Command::OpenRetained(other.clone()));
    settle(&mut a,|a|a.session.state().active().is_some_and(|t|t.source()==other) && a.session.state().current_source().is_some());
    let at = a.choices().iter().position(|c|matches!(c.target,ChoiceTarget::Action(CommandId::PasteTerminalPath))).unwrap();
    a.modal.as_mut().unwrap().at=at;
    key(&mut a,KeyCode::Enter,KeyModifiers::NONE);
    key(&mut a,KeyCode::Enter,KeyModifiers::NONE);
    let captured = format!("captured:<{}>",path.display());
    settle(&mut a,|a|screen_text(a).contains(&captured));
    assert_eq!(std::fs::read(path).unwrap(),b"keep");
    assert_eq!(std::fs::read(other).unwrap(),b"other");
    for (w,h) in [(15,8),(2,3),(0,0),(100,30)] { draw(&mut a,w,h); }
    assert_eq!(a.session.state().terminal().tabs().len(),2);
}

#[cfg(target_os = "linux")]
#[test]
fn terminal_reported_keypad_key_uses_child_application_mode() {
    use crossterm::event::{KeyEventKind, KeyEventState};
    let dir = tempfile::tempdir().unwrap();
    let mut a = App::new(config(dir.path()), Some(dir.path().to_owned()));
    a.dispatch(Command::Terminal(terminal::Command::New));
    settle(&mut a, |a| a.session.state().terminal().active().is_some_and(|t|t.running() && t.frame().is_some()));
    draw(&mut a, 120, 36);
    a.handle(Event::Paste("stty raw -echo; printf '\\033='; printf 'keypad_%s' ready; od -An -tx1 -N3; printf '\\033>'; stty sane".into()));
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    settle(&mut a, |a| screen_text(a).contains("keypad_ready"));
    a.handle(Event::Key(KeyEvent::new_with_kind_and_state(KeyCode::Char('1'),KeyModifiers::NONE,KeyEventKind::Press,KeyEventState::KEYPAD)));
    settle(&mut a, |a| screen_text(a).contains("1b 4f 71"));
    assert!(!a.quit);
}

#[test]
fn terminal_find_key_paste_render_and_dismiss_keep_shell_and_source_safe() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("keep.txt"); std::fs::write(&path,b"keep\n").unwrap();
    let mut a = App::new(config(dir.path()),Some(path.clone()));
    settle(&mut a,|a|a.session.state().current_source().is_some());
    a.terminal_command(terminal::Command::New);
    settle(&mut a,|a|a.session.state().terminal().active().is_some_and(|t|t.running() && t.frame().is_some()));
    draw(&mut a,100,30);
    a.handle(Event::Paste("stty -echo; printf 'needle.*é界\\r\\n'; printf 'last\\r\\n'".into()));
    key(&mut a,KeyCode::Enter,KeyModifiers::NONE);
    settle(&mut a,|a|screen_text(a).contains("needle.*é界"));
    key(&mut a,KeyCode::Char('f'),KeyModifiers::CONTROL); draw(&mut a,100,30);
    assert_eq!(a.session.state().terminal().find(),Some(""));
    a.handle(Event::Paste("needle.*é界".into()));
    key(&mut a,KeyCode::Enter,KeyModifiers::NONE);
    settle(&mut a,|a|a.session.state().terminal().find_status() == "Match");
    assert!(draw_screen(&mut a,100,30).contains("Find:"));
    assert!(a.session.state().terminal().active().unwrap().frame().unwrap().cells.iter().any(|c|c.search_match));
    key(&mut a,KeyCode::F(10),KeyModifiers::NONE);
    a.handle(Event::Paste("modal-only".into()));
    key(&mut a,KeyCode::Esc,KeyModifiers::NONE);
    assert_eq!(a.session.state().terminal().find(),Some("needle.*é界"));
    key(&mut a,KeyCode::Enter,KeyModifiers::SHIFT);
    key(&mut a,KeyCode::Char('a'),KeyModifiers::CONTROL);
    a.handle(Event::Paste("printf bad > find-must-not-execute".into()));
    key(&mut a,KeyCode::Enter,KeyModifiers::NONE);
    settle(&mut a,|a|a.session.state().terminal().find_status() == "No matches");
    for (w,h) in [(18,6),(1,1),(0,0),(100,30)] { draw(&mut a,w,h); }
    assert!(!dir.path().join("find-must-not-execute").exists());
    key(&mut a,KeyCode::Esc,KeyModifiers::NONE);
    assert!(a.session.state().terminal().find().is_none() && a.session.state().terminal().focused());
    a.handle(Event::Paste("printf restored > find-shell-restored".into()));
    key(&mut a,KeyCode::Enter,KeyModifiers::NONE);
    settle(&mut a,|_|std::fs::read(dir.path().join("find-shell-restored")).is_ok_and(|b|b == b"restored"));
    key(&mut a,KeyCode::Char('f'),KeyModifiers::CONTROL);
    key(&mut a,KeyCode::Char('x'),KeyModifiers::NONE);
    key(&mut a,KeyCode::Char('n'),KeyModifiers::CONTROL | KeyModifiers::ALT);
    assert!(a.session.state().terminal().find().is_none());
    let source = a.session.state().current_source().unwrap();
    assert_eq!(source.copy_range(0..source.len_bytes()).unwrap(),"keep\n");
    assert_eq!(std::fs::read(path).unwrap(),b"keep\n");
}

#[cfg(target_os = "linux")]
#[test]
fn terminal_editor_drag_released_over_cells_ends_in_editor_and_stopped_tab_is_not_resized() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("keep.txt");
    std::fs::write(&path, b"keep\nmore\n").unwrap();
    let mut a = App::new(config(dir.path()), Some(path.clone()));
    settle(&mut a, |a| a.session.state().current_source().is_some());
    a.dispatch(Command::Terminal(terminal::Command::New));
    settle(&mut a, |a| a.session.state().terminal().active().is_some_and(|t| t.running() && t.frame().is_some()));
    draw(&mut a, 100, 30);
    let text = a.layout.panes[0].text;
    let cells = a.layout.terminal.cells;
    mouse(&mut a, MouseButton::Left, text.x, text.y);
    assert!(a.drag.is_some());
    for kind in [MouseEventKind::Drag(MouseButton::Left), MouseEventKind::Up(MouseButton::Left)] {
        a.handle(Event::Mouse(MouseEvent { kind, column: cells.x + 2, row: cells.y + 1, modifiers: KeyModifiers::NONE }));
    }
    assert!(a.drag.is_none(), "the editor receives the release of its own drag");
    assert!(a.terminal_drag.is_none());
    // Stop keeps the last screen; a later size change must not report the missing process.
    a.dispatch(Command::Terminal(terminal::Command::Stop));
    let ticket = a.session.state().terminal().prompt().unwrap().ticket;
    a.dispatch(Command::Terminal(terminal::Command::Confirm { ticket, accept: true }));
    settle(&mut a, |a| a.session.state().terminal().active().is_some_and(|t| t.status() == "Stopped"));
    for (w, h) in [(80, 24), (120, 40)] { draw(&mut a, w, h); }
    assert_eq!(a.session.state().terminal().active().unwrap().error(), None);
    assert_eq!(std::fs::read(path).unwrap(), b"keep\nmore\n");
}

#[cfg(target_os = "linux")]
#[test]
fn terminal_close_and_stop_ask_their_own_question_and_a_newer_one_replaces_it() {
    let dir = tempfile::tempdir().unwrap();
    let mut a = App::new(config(dir.path()), None);
    a.dispatch(Command::Terminal(terminal::Command::New));
    settle(&mut a, |a| a.session.state().terminal().active().is_some_and(|t| t.running()));
    draw(&mut a, 100, 30);
    let dialog = |a: &App| match a.modal.as_ref().map(|m| &m.kind) {
        Some(Prompt::Details { title, kind: Details::Terminal(ticket), .. }) => (title.to_string(), a.details_buttons().iter().map(|b| b.label.to_string()).collect::<Vec<_>>(), *ticket),
        other => panic!("terminal confirmation expected, not {other:?}"),
    };
    // Closing a running tab asks about closing it, not stopping it.
    a.run(CommandId::CloseTerminal);
    let (title, labels, first) = dialog(&a);
    assert_eq!((title.as_str(), labels), ("Close terminal tab?", vec!["Close tab".to_string(), "Cancel".to_string()]));
    // A newer question replaces it entirely, ticket included.
    a.terminal_command(terminal::Command::Stop);
    let (title, labels, second) = dialog(&a);
    assert_eq!((title.as_str(), labels), ("Stop terminal?", vec!["Stop".to_string(), "Cancel".to_string()]));
    assert!(second > first);
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    assert!(a.modal.is_none() && a.session.state().terminal().active().unwrap().running());
    a.run(CommandId::CloseTerminal);
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    assert!(a.session.state().terminal().tabs().is_empty() && a.modal.is_none());
}

#[cfg(target_os = "linux")]
#[test]
fn terminal_keeps_find_action_and_its_wheel_scrolls_history_during_find() {
    let dir = tempfile::tempdir().unwrap();
    let mut a = App::new(config(dir.path()), None);
    a.dispatch(Command::Terminal(terminal::Command::New));
    settle(&mut a, |a| a.session.state().terminal().active().is_some_and(|t| t.running() && t.frame().is_some()));
    draw(&mut a, 100, 30);
    // The terminal keeps the keys, but Find Action's chord still reaches Lenz, as in the desktop.
    key(&mut a, KeyCode::Char('p'), KeyModifiers::CONTROL | KeyModifiers::SHIFT);
    assert!(matches!(a.modal.as_ref().map(|m| &m.kind), Some(Prompt::Navigation)), "{:?}", a.modal.as_ref().map(|m| &m.kind));
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    a.terminal_command(terminal::Command::Focus(true));
    for c in "stty -echo; seq -f 'LINE_%03g' 1 200\n".chars() { key(&mut a, if c == '\n' { KeyCode::Enter } else { KeyCode::Char(c) }, KeyModifiers::NONE); }
    settle(&mut a, |a| screen_text(a).contains("LINE_200"));
    draw(&mut a, 100, 30);
    // During Find the wheel scrolls the retained history instead of reaching the program.
    key(&mut a, KeyCode::Char('f'), KeyModifiers::CONTROL);
    assert!(a.session.state().terminal().find().is_some());
    let cells = a.layout.terminal.cells;
    for _ in 0..10 {
        a.handle(Event::Mouse(MouseEvent { kind: MouseEventKind::ScrollUp, column: cells.x + 2, row: cells.y + 2, modifiers: KeyModifiers::NONE }));
    }
    settle(&mut a, |a| !screen_text(a).contains("LINE_200"));
    assert!(a.session.state().terminal().find().is_some(), "Find stays open");
}
