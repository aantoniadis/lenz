use super::*;

fn text(app: &App) -> String { let s = app.session.state().current_source().unwrap(); s.copy_range(0..s.len_bytes()).unwrap() }

fn save_as_to(app: &mut App, path: &Path) {
    key(app, KeyCode::Char('S'), KeyModifiers::CONTROL | KeyModifiers::SHIFT);
    assert!(matches!(app.modal.as_ref().map(|m| &m.kind), Some(Prompt::SaveAs(_))));
    key(app, KeyCode::Char('u'), KeyModifiers::CONTROL);
    app.handle(Event::Paste(path.display().to_string()));
    key(app, KeyCode::Enter, KeyModifiers::NONE);
}

#[test]
fn new_file_and_save_as_use_terminal_keys_prompts_and_the_core_replace_decision() {
    let dir = tempfile::tempdir().unwrap();
    let existing = dir.path().join("existing.txt");
    std::fs::write(&existing, b"keep\n").unwrap();
    let mut app = App::new(config(dir.path()), Some(dir.path().to_owned()));
    app.enable_draft_input_for_test();
    draw(&mut app, 100, 18);
    key(&mut app, KeyCode::Char('n'), KeyModifiers::CONTROL);
    let tab = app.session.state().active().unwrap().id();
    assert!(app.session.state().active().unwrap().untitled());
    app.handle(Event::Paste("draft λ\n".into()));
    assert_eq!(text(&app), "draft λ\n");
    assert!(draw_screen(&mut app, 100, 18).contains("Untitled-1"));
    // Ctrl+S on an untitled document asks for a path, prefilled with the project folder.
    key(&mut app, KeyCode::Char('s'), KeyModifiers::CONTROL);
    let modal = app.modal.as_ref().unwrap();
    assert_eq!(modal.kind, Prompt::SaveAs(tab));
    assert_eq!(PathBuf::from(&modal.input), dir.path().join(""));
    key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert!(app.modal.is_none() && app.session.state().active().unwrap().untitled());
    // An existing target asks Replace/Cancel; the dialog owns input until dismissed.
    save_as_to(&mut app, &existing);
    assert!(app.session.save_as_prompt().is_some());
    let screen = draw_screen(&mut app, 100, 18);
    for label in ["already exists", "[Replace r]", "[Cancel Esc]"] { assert!(screen.contains(label), "{screen}"); }
    app.handle(Event::Paste("ignored".into()));
    key(&mut app, KeyCode::Char('z'), KeyModifiers::CONTROL);
    for (w, h) in [(20, 5), (1, 1), (100, 18)] { draw(&mut app, w, h); key(&mut app, KeyCode::End, KeyModifiers::NONE); key(&mut app, KeyCode::Up, KeyModifiers::NONE); }
    assert_eq!(text(&app), "draft λ\n");
    key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert!(app.modal.is_none() && app.session.save_as_prompt().is_none());
    assert_eq!(std::fs::read(&existing).unwrap(), b"keep\n");
    save_as_to(&mut app, &existing);
    key(&mut app, KeyCode::Char('r'), KeyModifiers::NONE);
    settle(&mut app, |a| !a.session.state().tab_dirty(tab));
    assert_eq!(std::fs::read(&existing).unwrap(), "draft λ\n".as_bytes());
    let t = app.session.state().active().unwrap();
    assert!(!t.untitled() && t.source() == existing && t.title() == "existing.txt");
    // A new relative path resolves against the project; history is kept.
    key(&mut app, KeyCode::Char('S'), KeyModifiers::CONTROL | KeyModifiers::SHIFT);
    assert_eq!(app.modal.as_ref().unwrap().input, existing.display().to_string());
    key(&mut app, KeyCode::Char('u'), KeyModifiers::CONTROL);
    app.handle(Event::Paste("fresh.txt".into()));
    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    let fresh = dir.path().join("fresh.txt");
    settle(&mut app, |a| a.session.state().active().unwrap().source() == fresh);
    assert_eq!(std::fs::read(&fresh).unwrap(), "draft λ\n".as_bytes());
    key(&mut app, KeyCode::Char('z'), KeyModifiers::CONTROL);
    assert_eq!(text(&app), "");
    assert_eq!(std::fs::read(&fresh).unwrap(), "draft λ\n".as_bytes());
}
