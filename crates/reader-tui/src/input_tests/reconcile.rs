//! Real terminal input for reconciling an external change with a dirty draft.
use super::*;

fn menu(app: &mut App, command: CommandId) {
    key(app, KeyCode::F(10), KeyModifiers::NONE);
    let row = app.choices().iter().position(|row| matches!(&row.target, ChoiceTarget::Action(id) if *id == command)).unwrap();
    for _ in 0..row { key(app, KeyCode::Down, KeyModifiers::NONE); }
    key(app, KeyCode::Enter, KeyModifiers::NONE);
}

/// A saved file with the draft `draft λ\r\n`, then the file changed on disk and the conflict observed.
fn conflict(dir: &Path, name: &str) -> (App, PathBuf) {
    let path = dir.join(name);
    std::fs::write(&path, b"saved\r\n").unwrap();
    let mut app = App::new(config(dir), Some(dir.to_owned()));
    app.dispatch(Command::OpenRetained(path.clone()));
    settle(&mut app, |a| a.session.state().current_source().is_some());
    app.enable_draft_input_for_test();
    draw(&mut app, 100, 18);
    key(&mut app, KeyCode::Char('a'), KeyModifiers::CONTROL);
    app.handle(Event::Paste("draft λ\r\n".into()));
    std::fs::write(&path, b"external replacement\r\n").unwrap();
    app.dispatch(Command::Recheck);
    settle(&mut app, |a| a.session.state().active().unwrap().stale_on_disk());
    (app, path)
}

fn open_dialog(app: &mut App) {
    menu(app, CommandId::ResolveExternalChange);
    settle(app, |a| a.session.reconcile_prompt().is_some_and(|prompt| prompt.disk.is_some()));
    draw(app, 100, 18);
}

#[test]
fn the_conflict_dialog_takes_real_terminal_input_and_taking_disk_is_undoable() {
    let dir = tempfile::tempdir().unwrap();
    let (mut app, path) = conflict(dir.path(), "take.txt");
    open_dialog(&mut app);
    let screen = draw_screen(&mut app, 120, 30);
    for label in ["Compare Disk, Draft and Baseline…", "Take Disk Version", "Save Draft over Disk", "Keep Draft"] {
        assert!(screen.contains(label), "{label} is missing from {screen}");
    }
    assert!(screen.contains("Confirmed baseline"), "{screen}");
    // The dialog owns input: nothing reaches the draft or the file.
    let source = app.session.state().current_source().unwrap().key();
    app.handle(Event::Paste("must not paste".into()));
    key(&mut app, KeyCode::Char('s'), KeyModifiers::CONTROL);
    key(&mut app, KeyCode::Char('z'), KeyModifiers::CONTROL);
    key(&mut app, KeyCode::Char('x'), KeyModifiers::NONE);
    for (w, h) in [(20, 5), (1, 1), (120, 30)] {
        draw(&mut app, w, h);
        key(&mut app, KeyCode::End, KeyModifiers::NONE);
        key(&mut app, KeyCode::Up, KeyModifiers::NONE);
    }
    assert_eq!(app.session.state().current_source().unwrap().key(), source);
    assert_eq!(std::fs::read(&path).unwrap(), b"external replacement\r\n");
    // D takes the disk version as one undoable edit; the file itself is not written.
    key(&mut app, KeyCode::Char('d'), KeyModifiers::NONE);
    settle(&mut app, |a| source_text(a.session.state().current_source().unwrap()) == "external replacement\r\n");
    let view = app.session.state().view(app.session.state().active().unwrap().id()).unwrap();
    assert!(!view.dirty() && view.can_undo());
    assert!(app.modal.is_none() && app.session.reconcile_prompt().is_none());
    assert_eq!(std::fs::read(&path).unwrap(), b"external replacement\r\n");
    key(&mut app, KeyCode::Char('z'), KeyModifiers::CONTROL);
    assert!(app.session.state().view(app.session.state().active().unwrap().id()).unwrap().dirty());
    assert_eq!(std::fs::read(&path).unwrap(), b"external replacement\r\n");
}

#[test]
fn the_terminal_compares_all_three_sides_and_keeps_the_draft_when_the_dialog_is_escaped() {
    let dir = tempfile::tempdir().unwrap();
    let (mut app, path) = conflict(dir.path(), "compare.txt");
    open_dialog(&mut app);
    key(&mut app, KeyCode::Char('c'), KeyModifiers::NONE);
    settle(&mut app, |a| a.session.state().saved_comparison().is_some());
    assert!(app.modal.is_none(), "the comparison takes the dialog's place");
    let comparison = app.session.state().saved_comparison().unwrap();
    assert_eq!(comparison.kind(), reader_core::session::ComparisonKind::Conflict);
    let (disk, current, baseline) = (comparison.saved_tab(), comparison.current_tab(), comparison.context_tab().unwrap());
    let screen = draw_screen(&mut app, 200, 40);
    for label in ["Disk (read-only)", "Current draft", "Confirmed baseline (read-only)"] {
        assert!(screen.contains(label), "{label} is missing from {screen}");
    }
    assert_eq!(source_text(app.session.state().view(disk).unwrap().snapshot()), "external replacement\r\n");
    assert_eq!(source_text(app.session.state().view(baseline).unwrap().snapshot()), "saved\r\n");
    assert_eq!(source_text(app.session.state().view(current).unwrap().snapshot()), "draft λ\r\n");
    // Closing the comparison brings the dialog back for the same draft.
    menu(&mut app, CommandId::CloseSavedComparison);
    settle(&mut app, |a| a.session.state().saved_comparison().is_none());
    draw(&mut app, 120, 30);
    assert!(app.session.reconcile_prompt().is_some(), "the dialog returns when the comparison closes");
    // Escape keeps the draft, and adopts nothing.
    key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert!(app.modal.is_none() && app.session.reconcile_prompt().is_none());
    assert_eq!(source_text(app.session.state().current_source().unwrap()), "draft λ\r\n");
    assert!(app.session.state().view(app.session.state().active().unwrap().id()).unwrap().dirty());
    assert_eq!(std::fs::read(&path).unwrap(), b"external replacement\r\n");
    // Save Draft over Disk writes after one more check.
    open_dialog(&mut app);
    key(&mut app, KeyCode::Char('s'), KeyModifiers::NONE);
    settle(&mut app, |a| {
        let view = a.session.state().view(a.session.state().active().unwrap().id()).unwrap();
        !view.dirty() && !matches!(view.save_status(), reader_core::save::Status::Saving | reader_core::save::Status::Rechecking)
    });
    assert_eq!(std::fs::read(&path).unwrap(), "draft λ\r\n".as_bytes());
    key(&mut app, KeyCode::Char('z'), KeyModifiers::CONTROL);
    assert!(app.session.state().view(app.session.state().active().unwrap().id()).unwrap().dirty());
    assert_eq!(std::fs::read(&path).unwrap(), "draft λ\r\n".as_bytes());
}

#[test]
fn an_uncertain_terminal_write_keeps_the_draft_and_reports_the_uncertainty() {
    use lenz_content::write::readiness_fault::{at_path, Fault};
    let dir = tempfile::tempdir().unwrap();
    let (mut app, path) = conflict(dir.path(), "uncertain.txt");
    let guard = at_path(&path, Fault::AfterReplace);
    open_dialog(&mut app);
    key(&mut app, KeyCode::Char('s'), KeyModifiers::NONE);
    settle(&mut app, |a| matches!(a.session.state().view(a.session.state().active().unwrap().id()).unwrap().save_status(), reader_core::save::Status::Uncertain(_)));
    drop(guard);
    let view = app.session.state().view(app.session.state().active().unwrap().id()).unwrap();
    assert!(view.dirty() && view.can_undo());
    let tab = app.session.state().active().unwrap().id();
    assert!(app.session.disabled(CommandId::Save).is_some(), "a decision is needed, not another write");
    draw(&mut app, 120, 30);
    let screen = draw_screen(&mut app, 120, 30);
    assert!(screen.contains("uncertain") || screen.contains("Recheck"), "{screen}");
    assert_eq!(std::fs::read(&path).unwrap(), "draft λ\r\n".as_bytes());
    app.dispatch(Command::RecheckSave { tab });
    settle(&mut app, |a| matches!(a.session.state().view(tab).unwrap().save_status(), reader_core::save::Status::Checked(_)));
    assert_eq!(std::fs::read(&path).unwrap(), "draft λ\r\n".as_bytes());
}

/// The draft's text, as a test fixture comparison.
fn source_text(source: &reader_core::text::TextSnapshot) -> String { source.copy_range(0..source.len_bytes()).unwrap() }
