//! Recovered drafts through the terminal's real keys and render: the dialog owns input, and every choice is reachable.
use super::*;

fn draft(dir: &Path, path: &Path, text: &str) {
    let mut a = app(dir);
    a.dispatch(Command::OpenRetained(path.to_owned()));
    settle(&mut a, |a| a.session.state().current_source().is_some());
    a.enable_draft_input_for_test();
    a.dispatch(Command::SelectAll);
    a.handle(Event::Paste(text.into()));
    a.session.flush_recovery_for_test();
}

fn restarted(dir: &Path, path: &Path) -> App {
    let mut a = App::with_text_input(config(dir), Some(path.to_owned()), false);
    settle(&mut a, |a| a.session.state().current_source().is_some());
    a.enable_draft_input_for_test();
    a
}

fn rendered(a: &mut App, width: u16, height: u16) -> String {
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
    terminal.draw(|f| a.draw(f)).unwrap();
    let buffer = terminal.backend().buffer();
    (0..height).map(|y| (0..width).map(|x| buffer[(x, y)].symbol().to_string()).collect::<String>()).collect::<Vec<_>>().join("\n")
}

fn recovered(a: &App) -> bool { matches!(a.modal.as_ref().map(|m| &m.kind), Some(Prompt::Details { kind: Details::Recovered, .. })) }

fn active_text(a: &App) -> String {
    let source = a.session.state().current_source().unwrap();
    source.copy_range(0..source.len_bytes()).unwrap()
}

#[test]
fn a_recovered_draft_is_offered_restored_saved_and_no_longer_recovered() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("recover.txt");
    std::fs::write(&path, b"saved\n").unwrap();
    draft(dir.path(), &path, "recovered λ\n");
    assert_eq!(std::fs::read(&path).unwrap(), b"saved\n", "recovery never writes the original file");
    let mut a = restarted(dir.path(), &path);
    draw(&mut a, 80, 24);
    assert!(recovered(&a), "the dialog opens at start");
    // The dialog owns input: a printable key does not edit the document behind it.
    key(&mut a, KeyCode::Char('x'), KeyModifiers::NONE);
    assert_eq!(active_text(&a), "saved\n");
    assert!(recovered(&a));
    // Tiny terminals keep every choice visible and dismissable without touching the document.
    for (w, h) in [(1, 1), (24, 6)] {
        draw(&mut a, w, h);
        key(&mut a, KeyCode::End, KeyModifiers::NONE);
        key(&mut a, KeyCode::Up, KeyModifiers::NONE);
    }
    draw(&mut a, 80, 24);
    key(&mut a, KeyCode::Char('r'), KeyModifiers::NONE);
    draw(&mut a, 80, 24);
    assert!(a.modal.is_none());
    assert!(a.session.state().active().unwrap().recovered());
    let screen = rendered(&mut a, 80, 24);
    assert!(screen.contains("⟲ ● recover.txt"), "the tab says recovered beside its unsaved mark: {screen}");
    assert_eq!(active_text(&a), "recovered λ\n");
    assert_eq!(std::fs::read(&path).unwrap(), b"saved\n", "Restore does not write");
    key(&mut a, KeyCode::Char('s'), KeyModifiers::CONTROL);
    settle(&mut a, |a| !a.session.state().tab_dirty(a.session.state().active().unwrap().id()));
    assert_eq!(std::fs::read(&path).unwrap(), "recovered λ\n".as_bytes());
    assert!(!a.session.state().active().unwrap().recovered());
    a.session.flush_recovery_for_test();
    let mut next = restarted(dir.path(), &path);
    draw(&mut next, 80, 24);
    assert!(!recovered(&next) && next.session.recovered_drafts().is_empty(), "saved work has nothing to recover");
}

#[test]
fn later_keeps_the_draft_and_the_command_offers_it_again() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("later.txt");
    std::fs::write(&path, b"saved\n").unwrap();
    draft(dir.path(), &path, "kept draft\n");
    let mut a = restarted(dir.path(), &path);
    draw(&mut a, 80, 24);
    assert!(recovered(&a));
    // A preset changes semantic chords, not the dialog's answers: VS Code's Ctrl+W closes a tab, but the dialog
    // takes the key first and nothing behind it changes.
    a.dispatch(Command::SetKeymapPreset(reader_core::keymap::KeymapPreset::VsCode));
    draw(&mut a, 80, 24);
    key(&mut a, KeyCode::Char('w'), KeyModifiers::CONTROL);
    draw(&mut a, 80, 24);
    assert!(recovered(&a), "the dialog keeps the key under another preset");
    assert!(a.session.state().active().is_some(), "Ctrl+W did not close the tab behind the dialog");
    assert_eq!(active_text(&a), "saved\n");
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    draw(&mut a, 80, 24);
    assert!(!recovered(&a), "Later dismisses the dialog");
    assert_eq!(a.session.recovered_drafts().len(), 1);
    a.run(CommandId::RecoverDrafts);
    draw(&mut a, 80, 24);
    assert!(recovered(&a), "the File command offers it again");
    assert_eq!(a.session.disabled(CommandId::RecoverDrafts), None);
}

#[test]
fn discard_and_its_click_route_keep_discarded_work_from_returning() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("discard.txt");
    std::fs::write(&path, b"saved\n").unwrap();
    draft(dir.path(), &path, "discarded draft\n");
    let mut a = restarted(dir.path(), &path);
    draw(&mut a, 80, 24);
    assert!(recovered(&a));
    // The Discard Draft button, clicked as drawn.
    let button = a.layout.details_choices.get(1).copied().expect("three buttons are drawn");
    a.handle(Event::Mouse(crossterm::event::MouseEvent {
        kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
        column: button.x,
        row: button.y,
        modifiers: KeyModifiers::NONE,
    }));
    draw(&mut a, 80, 24);
    assert!(!recovered(&a) && a.session.recovered_drafts().is_empty());
    assert_eq!(a.session.disabled(CommandId::RecoverDrafts), Some("No recovered drafts are offered"));
    a.session.flush_recovery_for_test();
    assert_eq!(std::fs::read(&path).unwrap(), b"saved\n");
    let mut next = restarted(dir.path(), &path);
    draw(&mut next, 80, 24);
    assert!(!recovered(&next) && next.session.recovered_drafts().is_empty(), "a discarded draft stays discarded");
}
