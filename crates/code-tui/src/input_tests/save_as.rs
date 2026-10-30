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
    app.dispatch(Command::NewDocument);
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

#[test]
fn scratch_commands_and_tab_context_menu_use_real_terminal_input() {
    let d = tempfile::tempdir().unwrap();
    let mut a = App::new(config(d.path()), Some(d.path().to_owned()));
    draw(&mut a, 100, 18);
    // The shared command menu opens the name prompt.
    key(&mut a, KeyCode::F(10), KeyModifiers::NONE);
    a.handle(Event::Paste("New Scratch File".into()));
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    assert!(matches!(a.modal.as_ref().map(|m| &m.kind), Some(Prompt::NewScratch)));
    a.handle(Event::Paste("personal.rs".into()));
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    settle(&mut a, |a| a.session.state().current_source().is_some());
    let scratch = a.session.state().active().unwrap().source().to_owned();
    assert!(scratch.exists() && !scratch.starts_with(d.path().join("project")));
    assert!(draw_screen(&mut a, 100, 18).contains("personal.rs"));
    a.run(CommandId::Close);
    key(&mut a, KeyCode::F(10), KeyModifiers::NONE);
    a.handle(Event::Paste("Scratch Files".into()));
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    assert!(draw_screen(&mut a, 100, 18).contains("personal.rs"));
    for (w, h) in [(1, 1), (20, 5), (100, 18)] { draw(&mut a, w, h); key(&mut a, KeyCode::End, KeyModifiers::NONE); key(&mut a, KeyCode::Up, KeyModifiers::NONE); }
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    settle(&mut a, |a| a.session.state().active().is_some_and(|t| t.source() == scratch));
    a.dispatch(Command::NewDocument);
    draw(&mut a, 100, 18);
    let first = a.layout.panes[0].tabs[0].1;
    mouse(&mut a, MouseButton::Right, first.x, first.y);
    assert_eq!(a.session.state().active().unwrap().source(), scratch);
    let screen = draw_screen(&mut a, 100, 18);
    assert!(screen.contains("Close Other Tabs") && screen.contains("Close Tabs to Right"), "{screen}");
    key(&mut a, KeyCode::Down, KeyModifiers::NONE);
    key(&mut a, KeyCode::Down, KeyModifiers::NONE);
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(a.session.state().tabs().len(), 1);
    assert_eq!(a.session.state().active().unwrap().source(), scratch);
}

#[test]
fn new_file_key_names_a_file_in_the_highlighted_project_folder() {
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir(d.path().join("src")).unwrap();
    std::fs::write(d.path().join("src/keep.txt"), b"keep\n").unwrap();
    let mut a = App::new(config(d.path()), Some(d.path().to_owned()));
    settle(&mut a, |a| a.session.state().project().is_some_and(|p| !p.refreshing() && p.rows().iter().any(|r| &*r.name == "src")));
    a.tree_at = a.session.state().tree_rows().position(|r| &*r.path == "src").unwrap();
    key(&mut a, KeyCode::Char('n'), KeyModifiers::CONTROL);
    assert!(matches!(a.modal.as_ref().map(|m| &m.kind), Some(Prompt::Create { kind: code_core::create::Kind::File, dir }) if dir == "src"));
    a.handle(Event::Paste("named.rs".into()));
    for (w, h) in [(1, 1), (20, 5), (100, 18)] { draw(&mut a, w, h); key(&mut a, KeyCode::End, KeyModifiers::NONE); key(&mut a, KeyCode::Up, KeyModifiers::NONE); }
    assert!(!d.path().join("src/named.rs").exists());
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    settle(&mut a, |a| a.session.state().active().is_some_and(|t| t.source() == d.path().join("src/named.rs")));
    assert_eq!(std::fs::read(d.path().join("src/named.rs")).unwrap(), b"");
    key(&mut a, KeyCode::Char('n'), KeyModifiers::CONTROL);
    a.handle(Event::Paste("cancel.txt".into()));
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    assert!(!d.path().join("src/cancel.txt").exists());
}

#[test]
fn save_as_prompt_and_replace_keep_the_originating_tab_when_focus_changes() {
    let d = tempfile::tempdir().unwrap();
    let target = d.path().join("origin.rs");
    std::fs::write(&target, b"keep\n").unwrap();
    let mut a = App::new(config(d.path()), None);
    a.enable_draft_input_for_test();
    key(&mut a, KeyCode::Char('n'), KeyModifiers::CONTROL);
    a.handle(Event::Paste("origin draft".into()));
    let origin = a.session.state().active().unwrap().id();
    key(&mut a, KeyCode::Char('s'), KeyModifiers::CONTROL);
    key(&mut a, KeyCode::Char('u'), KeyModifiers::CONTROL);
    a.handle(Event::Paste(target.display().to_string()));
    for (w, h) in [(1, 1), (20, 5), (100, 18)] { draw(&mut a, w, h); key(&mut a, KeyCode::End, KeyModifiers::NONE); key(&mut a, KeyCode::Up, KeyModifiers::NONE); }
    a.dispatch(Command::NewDocument);
    let other = a.session.state().active().unwrap().id();
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    assert!(a.session.save_as_prompt().is_some());
    key(&mut a, KeyCode::Char('r'), KeyModifiers::NONE);
    settle(&mut a, |a| a.session.state().tabs().iter().any(|t| t.id() == origin && t.source() == target));
    assert_eq!(std::fs::read(&target).unwrap(), b"origin draft");
    assert_eq!(a.session.state().active().unwrap().id(), other);
    assert!(a.session.state().active().unwrap().untitled());
    assert_eq!(text(&a), "");
}
