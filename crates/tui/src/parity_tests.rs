//! End-to-end terminal adapter scenarios through App's real event routing and TestBackend.
use crate::app::App;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use lenz::{changeset::Spec, cli, view::Side};
use ratatui::{Terminal, backend::TestBackend, buffer::Buffer};
#[path = "../../app/test_support/user_flows.rs"]
mod user_flows;
#[path = "followup_tests.rs"]
mod followup_tests;
use std::{
    path::Path,
    process::Command,
    thread,
    time::{Duration, Instant},
};

fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .args([
            "-c",
            "user.name=Parity Author",
            "-c",
            "user.email=parity@example.invalid",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().into()
}
fn repository() -> tempfile::TempDir {
    repository_at(tempfile::tempdir().unwrap())
}
fn repository_at(dir: tempfile::TempDir) -> tempfile::TempDir {
    git(dir.path(), &["init", "-q"]);
    std::fs::write(
        dir.path().join("file.rs"),
        "const VALUE: &str = \"first needle\";\n",
    )
    .unwrap();
    git(dir.path(), &["add", "file.rs"]);
    git(dir.path(), &["commit", "-qm", "First subject"]);
    std::fs::write(
        dir.path().join("file.rs"),
        "const VALUE: &str = \"second needle\";\n",
    )
    .unwrap();
    git(dir.path(), &["commit", "-qam", "Second subject"]);
    git(dir.path(), &["branch", "topic"]);
    dir
}
fn open(root: &Path, args: &[&str]) -> App {
    let args = args.iter().map(|arg| arg.to_string()).collect::<Vec<_>>();
    let mut app = App::open(cli::parse(&args, root).unwrap(), root.into());
    ready(&mut app);
    app
}
fn ready(app: &mut App) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        app.poll();
        if !app.busy() {
            break;
        }
        assert!(Instant::now() < deadline, "application did not settle");
        thread::sleep(Duration::from_millis(2));
    }
}
fn key(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
    assert!(
        app.handle(Event::Key(KeyEvent::new(code, modifiers))),
        "unhandled {code:?} {modifiers:?}"
    );
    app.poll();
}
fn text(app: &mut App, query: &str) {
    for character in query.chars() {
        key(app, KeyCode::Char(character), KeyModifiers::NONE);
    }
}
fn draw(app: &mut App, width: u16, height: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| app.render(frame)).unwrap();
    ready(app);
    terminal.draw(|frame| app.render(frame)).unwrap();
    terminal.backend().buffer().clone()
}
fn screen(buffer: &Buffer) -> String {
    buffer
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<Vec<_>>()
        .join("")
}
fn details_body(buffer:&Buffer)->String {
    (2..buffer.area.height.saturating_sub(3))
        .flat_map(|y|(2..buffer.area.width.saturating_sub(2)).map(move |x|buffer[(x,y)].symbol()))
        .collect::<String>().chars().filter(|c|!c.is_whitespace()).collect()
}

#[test]
fn tui_recovery_details_external_save_refusal_preserves_draft_and_disk() {
    let dir = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
    std::fs::write(dir.path().join("left"), "old\n").unwrap();
    std::fs::write(dir.path().join("right"), "new\n").unwrap();
    let mut app = open(dir.path(), &["--no-index", "left", "right"]);
    key(&mut app, KeyCode::Right, KeyModifiers::ALT | KeyModifiers::SHIFT);
    ready(&mut app);
    std::fs::write(dir.path().join("right"), "external\n").unwrap();
    key(&mut app, KeyCode::Char('s'), KeyModifiers::CONTROL);
    for entry in lenz::theme::builtin_entries() {
        app.palette = entry.palette;
        for width in [120, 48] {
            let status = screen(&draw(&mut app, width, 24));
            assert!(status.contains("F2 details"), "{width}: {status}");
            key(&mut app, KeyCode::F(2), KeyModifiers::NONE);
            let details_buffer = draw(&mut app, width, 24);
            assert_eq!(details_buffer[(2,2)].bg,crate::chrome::color(app.palette.status_bg));
            let details = screen(&details_buffer);
            let body=details_body(&details_buffer);
            assert!(body.contains("changedondisk"), "{details}");
            assert!(body.contains("Discardunsavedchangesreloadsthecurrentfile"), "{details}");
            key(&mut app, KeyCode::Char('s'), KeyModifiers::CONTROL);
            key(&mut app, KeyCode::Char('z'), KeyModifiers::CONTROL);
            assert_eq!(app.session.document().unwrap().text(Side::Right), "old\n");
            assert!(app.session.dirty());
            assert_eq!(std::fs::read(dir.path().join("right")).unwrap(), b"external\n");
            key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
        }
    }
}

#[test]
fn tui_recovery_details_scroll_resize_and_dismiss_keep_parent_and_draft() {
    let dir = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
    std::fs::write(dir.path().join("left"), "old\n").unwrap();
    std::fs::write(dir.path().join("right"), "new\n").unwrap();
    let mut app = open(dir.path(), &["--no-index", "left", "right"]);
    key(&mut app, KeyCode::Right, KeyModifiers::ALT | KeyModifiers::SHIFT);
    ready(&mut app);
    key(&mut app, KeyCode::Char('z'), KeyModifiers::CONTROL);
    key(&mut app, KeyCode::Char('A'), KeyModifiers::NONE);
    key(&mut app, KeyCode::F(2), KeyModifiers::NONE);
    assert!(screen(&draw(&mut app, 48, 24)).contains("already undone"));
    key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert!(screen(&draw(&mut app, 48, 24)).contains("Applied history"));
    key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    app.session.set_notice((0..30).map(|i| format!("recovery step {i:02}\n")).collect::<String>() + "LAST RECOVERY");
    draw(&mut app, 48, 8);
    key(&mut app, KeyCode::F(2), KeyModifiers::NONE);
    key(&mut app, KeyCode::End, KeyModifiers::NONE);
    key(&mut app, KeyCode::Up, KeyModifiers::NONE);
    let previous = screen(&draw(&mut app, 48, 8));
    assert!(previous.contains("recovery step 27"), "{previous}");
    assert!(!previous.contains("LAST RECOVERY"), "End must clamp before Up: {previous}");
    for (w,h) in [(0,0), (1,1), (2,2), (8,5), (48,8)] {
        app.handle(Event::Resize(w,h));
        draw(&mut app,w,h);
        key(&mut app, KeyCode::End, KeyModifiers::NONE);
    }
    assert!(screen(&draw(&mut app, 48,8)).contains("LAST RECOVERY"));
    key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert!(!app.modal_input());
    assert_eq!(app.session.document().unwrap().text(Side::Right), "new\n");
    assert_eq!(std::fs::read(dir.path().join("right")).unwrap(), b"new\n");
}

#[test]
fn tui_recovery_details_repository_notice_is_reachable_from_log() {
    let dir = repository();
    let mut app = open(dir.path(), &["log"]);
    app.repository.as_mut().unwrap().core.notice = Some("Repository refused this action. Choose a branch before comparing; recovery is available in Sources.".into());
    assert!(screen(&draw(&mut app,48,24)).contains("F2 details"));
    key(&mut app, KeyCode::F(2), KeyModifiers::NONE);
    assert!(screen(&draw(&mut app,48,24)).contains("Sources."));
    let position = app.repository.as_ref().unwrap().core.picker.anchor;
    key(&mut app, KeyCode::Down, KeyModifiers::NONE);
    assert_eq!(app.repository.as_ref().unwrap().core.picker.anchor, position);
    key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
}

#[test]
fn tui_recovery_details_overlapping_history_and_disabled_menu_keep_parent_selection() {
    let dir = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
    std::fs::write(dir.path().join("left"), "first\n").unwrap();
    std::fs::write(dir.path().join("right"), "old\n").unwrap();
    let mut app = open(dir.path(), &["--no-index", "left", "right"]);
    key(&mut app, KeyCode::Right, KeyModifiers::ALT | KeyModifiers::SHIFT);
    ready(&mut app);
    key(&mut app, KeyCode::Char('s'),KeyModifiers::CONTROL);
    ready(&mut app);
    std::fs::write(dir.path().join("left"), "second\n").unwrap();
    key(&mut app, KeyCode::F(5),KeyModifiers::NONE);
    ready(&mut app);
    key(&mut app, KeyCode::Right,KeyModifiers::ALT|KeyModifiers::SHIFT);
    ready(&mut app);
    key(&mut app, KeyCode::Char('A'),KeyModifiers::NONE);
    key(&mut app, KeyCode::Home,KeyModifiers::NONE);
    let selection=app.selected;
    key(&mut app, KeyCode::F(2),KeyModifiers::NONE);
    let details=screen(&draw(&mut app,48,24));
    assert!(details.contains("overlaps"),"{details}");
    assert!(details.contains("normal undo"),"{details}");
    key(&mut app, KeyCode::Char('z'),KeyModifiers::CONTROL);
    key(&mut app, KeyCode::Esc,KeyModifiers::NONE);
    assert_eq!(app.selected,selection);
    assert!(app.prompt==Some(crate::app::Prompt::Applied));
    key(&mut app, KeyCode::Esc,KeyModifiers::NONE);
    key(&mut app, KeyCode::F(10),KeyModifiers::NONE);
    text(&mut app,"Revert current hunk");
    key(&mut app, KeyCode::F(2),KeyModifiers::NONE);
    assert!(screen(&draw(&mut app,48,24)).contains("working-tree comparison"));
    key(&mut app, KeyCode::Esc,KeyModifiers::NONE);
    assert!(app.command_palette.is_some());
    assert_eq!(app.session.document().unwrap().text(Side::Right),"second\n");
    assert!(app.session.dirty());
    assert_eq!(std::fs::read(dir.path().join("right")).unwrap(),b"first\n");
}

#[test]
fn tui_recovery_details_does_not_open_parent_status_through_search_entry() {
    let dir=repository();let mut app=open(dir.path(),&["HEAD~1..HEAD"]);
    key(&mut app,KeyCode::Char('/'),KeyModifiers::NONE);
    text(&mut app,"needle");
    let input=app.input.clone();
    app.handle(Event::Key(KeyEvent::new(KeyCode::F(2),KeyModifiers::NONE)));
    assert!(app.message.is_none());
    assert!(app.prompt==Some(crate::app::Prompt::Search));
    assert_eq!(app.input,input);
}

#[test]
fn tui_user_flow_discard_shorter_external_file_renders_before_diff_completes() {
    let dir = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
    std::fs::write(dir.path().join("left.txt"), "old\n".repeat(20)).unwrap();
    std::fs::write(dir.path().join("right.txt"), "new\n".repeat(20)).unwrap();
    let mut app = open(dir.path(), &["--no-index", "left.txt", "right.txt"]);
    key(
        &mut app,
        KeyCode::Right,
        KeyModifiers::ALT | KeyModifiers::SHIFT,
    );
    ready(&mut app);
    std::fs::write(dir.path().join("right.txt"), "external\n").unwrap();
    key(&mut app, KeyCode::Char('s'), KeyModifiers::CONTROL);
    assert!(app.session.has_dirty());
    key(&mut app, KeyCode::F(10), KeyModifiers::NONE);
    text(&mut app, "Discard unsaved changes");
    app.handle(Event::Key(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::NONE,
    )));
    let frame = app.session.view().frame();
    assert!(
        frame
            .right
            .iter()
            .all(|r| !matches!(r.content, lenz::view::Content::Line(line) if line >= 2))
    );
    let mut terminal = Terminal::new(TestBackend::new(120, 36)).unwrap();
    terminal.draw(|f| app.render(f)).unwrap();
    ready(&mut app);
    assert_eq!(
        app.session.document().unwrap().text(Side::Right),
        "external\n"
    );
    assert_eq!(
        std::fs::read(dir.path().join("right.txt")).unwrap(),
        b"external\n"
    );
}

#[test]
fn tui_user_flow_merge_replay() {
    use lenz::conflict_session::FileView;
    let dir = tempfile::tempdir().unwrap();
    for (name, text) in [
        ("base", "needle base\n"),
        ("ours", "needle ours\n"),
        ("theirs", "needle theirs\n"),
        ("result", "needle ours\n"),
    ] {
        std::fs::write(dir.path().join(name), text).unwrap();
    }
    let request = cli::parse(
        &["merge", "base", "ours", "theirs", "result"].map(str::to_owned),
        dir.path(),
    )
    .unwrap();
    let mut app = crate::merge::Merger::open(request, dir.path().into());
    let deadline = Instant::now() + Duration::from_secs(15);
    while app.busy() {
        app.poll();
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(2));
    }
    let mut terminal = Terminal::new(TestBackend::new(120, 36)).unwrap();
    for _ in 0..3 {
        for &(choice, expected) in user_flows::MERGE_CHOICES {
            let disk = std::fs::read(dir.path().join("result")).unwrap();
            let code = match choice {
                "right" => KeyCode::Right,
                "left" => KeyCode::Left,
                "down" => KeyCode::Down,
                "backspace" => KeyCode::Backspace,
                _ => unreachable!(),
            };
            app.handle(Event::Key(KeyEvent::new(
                code,
                KeyModifiers::ALT | KeyModifiers::SHIFT,
            )));
            let Some(FileView::Merge(m)) = &app.workspace.file else {
                panic!("expected merge");
            };
            assert_eq!(m.text(), expected);
            assert_eq!(m.m.unresolved(), 0);
            assert!(m.dirty);
            assert_eq!(std::fs::read(dir.path().join("result")).unwrap(), disk);
            app.handle(Event::Key(KeyEvent::new(
                KeyCode::Char('z'),
                KeyModifiers::CONTROL,
            )));
            assert!(
                matches!(&app.workspace.file, Some(FileView::Merge(m)) if m.m.unresolved() == 1)
            );
            app.handle(Event::Key(KeyEvent::new(
                KeyCode::Char('z'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT,
            )));
            app.handle(Event::Key(KeyEvent::new(
                KeyCode::Char('s'),
                KeyModifiers::CONTROL,
            )));
            assert_eq!(
                std::fs::read(dir.path().join("result")).unwrap(),
                expected.as_bytes()
            );
            assert!(matches!(&app.workspace.file, Some(FileView::Merge(m)) if !m.dirty));
            app.handle(Event::Key(KeyEvent::new(
                KeyCode::Char('z'),
                KeyModifiers::CONTROL,
            )));
            assert!(
                matches!(&app.workspace.file, Some(FileView::Merge(m)) if m.m.unresolved() == 1)
            );
            assert_eq!(
                std::fs::read(dir.path().join("result")).unwrap(),
                expected.as_bytes()
            );
            terminal.draw(|f| app.render(f)).unwrap();
            assert!(screen(terminal.backend().buffer()).contains("Result"));
        }
    }
}

#[test]
fn tui_user_flow_replay() {
    use user_flows::{Snapshot, Step};
    for case in user_flows::CASES {
        let dir = tempfile::Builder::new()
            .prefix(".lenz-flow-")
            .tempdir_in(std::env::current_dir().unwrap())
            .unwrap();
        for (side, content) in [("left", case.left), ("right", case.right)] {
            std::fs::create_dir(dir.path().join(side)).unwrap();
            std::fs::write(dir.path().join(side).join("a.txt"), content).unwrap();
            std::fs::write(
                dir.path().join(side).join("b.txt"),
                format!("sentinel {side}\n"),
            )
            .unwrap();
        }
        let mut app = open(dir.path(), &["--no-index", "left", "right"]);
        draw(&mut app, 120, 36);
        user_flows::replay(case, |step| {
            match step {
                Step::Inspect => {}
                Step::ApplyRight | Step::ApplyLeft => {
                    for _ in 0..3 {
                        if app.session.view().current_change() == Some(0) {
                            break;
                        }
                        key(&mut app, KeyCode::Char('n'), KeyModifiers::NONE);
                    }
                    assert_eq!(
                        app.session.view().current_change(),
                        Some(0),
                        "{} {step:?}",
                        case.name
                    );
                    key(
                        &mut app,
                        if matches!(step, Step::ApplyRight) {
                            KeyCode::Right
                        } else {
                            KeyCode::Left
                        },
                        KeyModifiers::ALT | KeyModifiers::SHIFT,
                    );
                }
                Step::Undo => key(&mut app, KeyCode::Char('z'), KeyModifiers::CONTROL),
                Step::Redo => key(
                    &mut app,
                    KeyCode::Char('z'),
                    KeyModifiers::CONTROL | KeyModifiers::SHIFT,
                ),
                Step::Save | Step::RefusedSave => {
                    key(&mut app, KeyCode::Char('s'), KeyModifiers::CONTROL)
                }
                Step::Discard => {
                    key(&mut app, KeyCode::F(10), KeyModifiers::NONE);
                    text(&mut app, "Discard unsaved changes");
                    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
                }
                Step::Reload => key(&mut app, KeyCode::F(5), KeyModifiers::NONE),
                Step::SwitchRoundTrip => {
                    key(&mut app, KeyCode::Char(']'), KeyModifiers::NONE);
                    ready(&mut app);
                    assert_eq!(app.session.current(), 1);
                    assert_eq!(
                        app.session.document().unwrap().text(Side::Right),
                        "sentinel right\n"
                    );
                    key(&mut app, KeyCode::Char('['), KeyModifiers::NONE);
                }
                Step::External(content) => {
                    std::fs::write(dir.path().join("right/a.txt"), content).unwrap()
                }
            }
            ready(&mut app);
            let painted = screen(&draw(&mut app, 120, 36));
            if matches!(step, Step::RefusedSave) {
                assert!(
                    app.session
                        .notice()
                        .is_some_and(|s| s.contains("nothing written"))
                );
                assert!(painted.contains("file changed on disk"), "{painted}");
            }
            assert_eq!(app.session.current(), 0);
            Snapshot {
                documents: [Side::Left, Side::Right]
                    .map(|s| app.session.document().unwrap().text(s).to_owned()),
                disk: ["left/a.txt", "right/a.txt"]
                    .map(|p| std::fs::read(dir.path().join(p)).unwrap()),
                dirty: app.session.has_dirty(),
                last_applied_active: app.session.applied().last().map(|r| r.active),
            }
        });
        assert_eq!(
            std::fs::read(dir.path().join("right/b.txt")).unwrap(),
            b"sentinel right\n"
        );
    }
}

#[test]
fn tui_user_flow_modal_recovery() {
    let dir = tempfile::Builder::new()
        .prefix(".lenz-modal-flow-")
        .tempdir_in(std::env::current_dir().unwrap())
        .unwrap();
    std::fs::write(dir.path().join("left.txt"), "old\n").unwrap();
    std::fs::write(dir.path().join("right.txt"), "new\n").unwrap();
    let mut app = open(dir.path(), &["--no-index", "left.txt", "right.txt"]);
    draw(&mut app, 120, 36);
    key(
        &mut app,
        KeyCode::Right,
        KeyModifiers::ALT | KeyModifiers::SHIFT,
    );
    ready(&mut app);
    let before = (
        app.session.document().unwrap().text(Side::Right).to_owned(),
        app.session.options(),
        app.session.has_dirty(),
    );
    for _ in 0..3 {
        for [width, height] in [[120, 36], [48, 16], [1, 1], [0, 0]] {
            app.handle(Event::Resize(width, height));
            draw(&mut app, width, height);
            for modal in [
                "about", "help", "commands", "goto", "files", "find", "findset",
            ] {
                for (code, modifiers) in [
                    (KeyCode::Char('s'), KeyModifiers::CONTROL),
                    (KeyCode::Char('z'), KeyModifiers::CONTROL),
                    (
                        KeyCode::Char('z'),
                        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
                    ),
                    (KeyCode::Left, KeyModifiers::ALT | KeyModifiers::SHIFT),
                ] {
                    match modal {
                        "about" => {
                            key(&mut app, KeyCode::F(10), KeyModifiers::NONE);
                            text(&mut app, "About Lenz");
                            key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
                        }
                        "help" => key(&mut app, KeyCode::F(1), KeyModifiers::NONE),
                        "commands" => key(&mut app, KeyCode::F(10), KeyModifiers::NONE),
                        "goto" => key(&mut app, KeyCode::Char('g'), KeyModifiers::NONE),
                        "files" => key(&mut app, KeyCode::Char('p'), KeyModifiers::CONTROL),
                        "find" => key(&mut app, KeyCode::Char('f'), KeyModifiers::CONTROL),
                        "findset" => key(
                            &mut app,
                            KeyCode::Char('f'),
                            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
                        ),
                        _ => unreachable!(),
                    }
                    assert!(
                        app.about.is_some()
                            || app.prompt.is_some()
                            || app.command_palette.is_some()
                            || app.set_search.as_ref().is_some_and(|s| s.model.editing()),
                        "{modal} must open at {width}x{height}"
                    );
                    draw(&mut app, width, height);
                    // Unsupported shortcuts need not request a repaint; assert their
                    // actual state/disk effects rather than App's redraw hint.
                    app.handle(Event::Key(KeyEvent::new(code, modifiers)));
                    ready(&mut app);
                    assert_eq!(
                        (
                            app.session.document().unwrap().text(Side::Right).to_owned(),
                            app.session.options(),
                            app.session.has_dirty()
                        ),
                        before,
                        "{modal}: {code:?} at {width}x{height}"
                    );
                    assert_eq!(
                        std::fs::read(dir.path().join("right.txt")).unwrap(),
                        b"new\n"
                    );
                    key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
                    assert!(
                        app.about.is_none() && app.prompt.is_none() && app.command_palette.is_none() && app.set_search.is_none()
                    );
                    assert!(!app.quit);
                }
            }
        }
    }
    key(&mut app, KeyCode::Char('p'), KeyModifiers::CONTROL);
    text(&mut app, "does-not-exist");
    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    key(&mut app, KeyCode::Char('s'), KeyModifiers::CONTROL);
    ready(&mut app);
    assert_eq!(
        std::fs::read(dir.path().join("right.txt")).unwrap(),
        b"old\n"
    );
}

#[test]
fn clean_default_enters_history_and_real_repository_keys_filter_switch_and_leave_focus() {
    let dir = repository();
    let mut app = open(dir.path(), &[]);
    let initial = screen(&draw(&mut app, 120, 36));
    assert!(
        initial.contains("Second subject") && initial.contains("First subject"),
        "{initial}"
    );
    assert_eq!(
        app.session.document().unwrap().text(Side::Right),
        "const VALUE: &str = \"second needle\";\n"
    );
    key(&mut app,KeyCode::F(10),KeyModifiers::NONE);text(&mut app,"filter");key(&mut app,KeyCode::Enter,KeyModifiers::NONE);
    text(&mut app, "first");
    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    ready(&mut app);
    let filtered = screen(&draw(&mut app, 120, 36));
    assert!(
        filtered.contains("First subject") && !filtered.contains("Second subject"),
        "{filtered}"
    );
    key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    key(&mut app, KeyCode::F(6), KeyModifiers::NONE);
    key(&mut app, KeyCode::Char('r'), KeyModifiers::CONTROL);
    assert!(!app.repository.as_ref().unwrap().core.picker.shown);
    assert!(screen(&draw(&mut app, 120, 36)).contains("second needle"));
}

#[test]
fn staged_source_and_followed_file_selection_use_actual_application_inputs() {
    let dir = repository();
    std::fs::write(
        dir.path().join("file.rs"),
        "const VALUE: &str = \"staged needle\";\n",
    )
    .unwrap();
    git(dir.path(), &["add", "file.rs"]);
    let mut app = open(dir.path(), &["log"]);
    assert_eq!(app.repository.as_ref().unwrap().core.picker.anchor, 0);
    key(&mut app,KeyCode::F(10),KeyModifiers::NONE);text(&mut app,"staged");key(&mut app,KeyCode::Enter,KeyModifiers::NONE);
    ready(&mut app);
    assert!(matches!(app.session.spec(), Spec::Git { cached: true, .. }));
    assert!(screen(&draw(&mut app, 120, 36)).contains("staged needle"));
    key(&mut app,KeyCode::F(10),KeyModifiers::NONE);text(&mut app,"follow");key(&mut app,KeyCode::Enter,KeyModifiers::NONE);
    ready(&mut app);
    assert!(
        app.repository
            .as_ref()
            .unwrap()
            .core
            .picker
            .file_paths
            .is_some()
    );
    assert!(
        app.session
            .comparisons()
            .iter()
            .all(|comparison| comparison.path == "file.rs")
    );
    key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    key(&mut app, KeyCode::Char('q'), KeyModifiers::CONTROL);
    assert!(app.quit);
}

#[test]
fn repository_sidebar_mouse_cannot_undo_applied_changes_or_scroll_comparison() {
    let dir = repository_at(
        tempfile::Builder::new()
            .prefix(".lenz-sidebar-write-")
            .tempdir_in(std::env::current_dir().unwrap())
            .unwrap(),
    );
    let path = dir.path().join("file.rs");
    std::fs::write(&path, "const VALUE: &str = \"dirty needle\";\n").unwrap();
    let mut app = open(dir.path(), &["log"]);
    key(&mut app, KeyCode::F(6), KeyModifiers::NONE);
    key(
        &mut app,
        KeyCode::Right,
        KeyModifiers::ALT | KeyModifiers::SHIFT,
    );
    ready(&mut app);
    assert_eq!(
        app.session.applied().len(),
        1,
        "{}",
        screen(&draw(&mut app, 120, 30))
    );
    draw(&mut app, 120, 30);
    let written = std::fs::read(&path).unwrap();
    let before = app.session.view().frame();
    let row = app.diff_area.bottom() - 3;
    assert!(app.handle(Event::Mouse(crossterm::event::MouseEvent {
        kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
        column: 1,
        row,
        modifiers: KeyModifiers::NONE
    })));
    ready(&mut app);
    assert_eq!(std::fs::read(&path).unwrap(), written);
    assert!(app.session.applied()[0].active);
    assert!(app.handle(Event::Mouse(crossterm::event::MouseEvent {
        kind: crossterm::event::MouseEventKind::ScrollDown,
        column: 1,
        row,
        modifiers: KeyModifiers::NONE
    })));
    assert_eq!(app.session.view().frame(), before);
}

#[test]
fn change_set_search_from_app_opens_a_different_file_and_preserves_exact_match_side() {
    let dir = tempfile::tempdir().unwrap();
    for side in ["left", "right"] {
        std::fs::create_dir(dir.path().join(side)).unwrap();
    }
    std::fs::write(dir.path().join("left/a.rs"), "const A: i32 = 1;\n").unwrap();
    std::fs::write(dir.path().join("right/a.rs"), "const A: i32 = 2;\n").unwrap();
    std::fs::write(
        dir.path().join("left/b.rs"),
        "const B: &str = \"old needle\";\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("right/b.rs"),
        "const B: &str = \"new needle\";\n",
    )
    .unwrap();
    let mut app = open(dir.path(), &["--no-index", "left", "right"]);
    key(
        &mut app,
        KeyCode::Char('f'),
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    );
    text(&mut app, "needle");
    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    ready(&mut app);
    let results = screen(&draw(&mut app, 120, 36));
    assert!(
        results.contains("2 matches") && results.contains("b.rs"),
        "{results}"
    );
    key(&mut app, KeyCode::F(3), KeyModifiers::NONE);
    ready(&mut app);
    assert_eq!(app.session.current(), 1);
    assert_eq!(app.focus, Side::Left);
    let selected = app
        .selection
        .expect("search result creates a text selection");
    assert_eq!(selected.side, Side::Left);
    assert_eq!(selected.anchor.line, 0);
    assert_eq!((selected.anchor.col, selected.head.col), (21, 27));
    let hit_screen = draw(&mut app, 120, 36);
    assert!(screen(&hit_screen).contains("old needle"));
    assert!(
        hit_screen
            .content
            .iter()
            .any(|cell| cell.symbol() == "n" && cell.bg == crate::chrome::color(crate::chrome::palette().found_bg))
    );
    key(&mut app, KeyCode::F(3), KeyModifiers::NONE);
    ready(&mut app);
    assert_eq!(app.focus, Side::Right);
    key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert!(app.set_search.is_none());
}

#[test]
fn syntax_and_find_are_observable_together_as_styles_on_source_cells() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("left.rs"),
        "fn before() { let count = 1; }\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("right.rs"),
        "fn after() { let count = 2; }\n",
    )
    .unwrap();
    let mut app = open(dir.path(), &["--no-index", "left.rs", "right.rs"]);
    let syntax = draw(&mut app, 120, 30);
    assert!(syntax.content.iter().any(|cell| {
        cell.symbol() == "f"
            && cell.fg
                == crate::visual::syntax_style(crate::chrome::palette(),lenz::highlight::Capture::Keyword)
                    .fg
                    .unwrap()
    }));
    key(&mut app, KeyCode::Char('/'), KeyModifiers::NONE);
    text(&mut app, "count");
    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    let found = draw(&mut app, 120, 30);
    assert!(found.content.iter().any(|cell|cell.symbol()=="c"&&cell.bg==crate::chrome::color(app.palette.sel_bg)),"the selected match takes priority, as in the GUI");
    app.selection=None;
    let found = draw(&mut app, 120, 30);
    assert!(
        found
            .content
            .iter()
            .any(|cell| cell.symbol() == "c" && cell.bg == crate::chrome::color(crate::chrome::palette().found_bg))
    );
    assert!(screen(&found).contains("Find: count"));
}

#[test]
fn blame_toggle_and_commit_inspection_are_visible_through_application_routing() {
    let dir = repository();
    let mut app = open(dir.path(), &["HEAD~1", "HEAD"]);
    key(&mut app, KeyCode::Char('b'), KeyModifiers::NONE);
    ready(&mut app);
    let annotated = screen(&draw(&mut app, 150, 30));
    let sha = git(dir.path(), &["rev-parse", "HEAD"]);
    assert!(
        annotated.contains(&sha[..7]) && annotated.contains("Parity"),
        "expected commit {sha}: {annotated}"
    );
    // Inspect a committed line; the initial viewport anchor can be the trailing blank line.
    key(&mut app, KeyCode::Char('g'), KeyModifiers::NONE);
    text(&mut app, "1");
    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    key(&mut app, KeyCode::Char('B'), KeyModifiers::NONE);
    ready(&mut app);
    let details = screen(&draw(&mut app, 150, 30));
    assert!(
        details.contains("Second subject")
            && details.contains("file.rs")
            && details.contains("Parity Author"),
        "{details}"
    );
    key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert!(!app.visual.commit_visible());
}

#[test]
fn interrupted_change_set_search_and_rapid_file_switches_do_not_publish_stale_results() {
    let dir = tempfile::tempdir().unwrap();
    for side in ["left", "right"] {
        std::fs::create_dir(dir.path().join(side)).unwrap();
        for name in ["a.rs", "b.rs", "c.rs"] {
            std::fs::write(
                dir.path().join(side).join(name),
                format!("{side} {name} needle\n").repeat(1000),
            )
            .unwrap();
        }
    }
    let mut app = open(dir.path(), &["--no-index", "left", "right"]);
    key(
        &mut app,
        KeyCode::Char('f'),
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    );
    text(&mut app, "needle");
    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    for _ in 0..8 {
        key(&mut app, KeyCode::Char(']'), KeyModifiers::NONE);
        key(&mut app, KeyCode::Char('['), KeyModifiers::NONE);
    }
    ready(&mut app);
    assert_eq!(app.session.current(), 0);
    assert!(app.set_search.is_none());
    let visible = screen(&draw(&mut app, 120, 30));
    assert!(visible.contains("a.rs") && !visible.contains("Find needle"));
}

#[test]
fn unreadable_comparison_error_is_visible_and_quit_remains_responsive() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("right"), "text\n").unwrap();
    let mut app = App::open(
        cli::Request {
            launch: cli::Launch::Diff,
            spec: Spec::Plain {
                left: dir.path().join("missing"),
                right: dir.path().join("right"),
            },
            options: Default::default(),
            name: None,
            scroll: Default::default(),
            blame: false,
            line: None,
            markdown: Default::default(),
        },
        dir.path().into(),
    );
    ready(&mut app);
    assert!(app.session.failed());
    let visible = screen(&draw(&mut app, 100, 20));
    assert!(visible.contains("missing"), "{visible}");
    key(&mut app, KeyCode::Char('q'), KeyModifiers::NONE);
    assert!(app.quit);
}

#[test]
fn initial_repository_comparison_preserves_name_and_line(){
    let dir=repository();let content=(0..80).map(|i|format!("line {i}\n")).collect::<String>();std::fs::write(dir.path().join("file.rs"),&content).unwrap();git(dir.path(),&["commit","-qam","Long file"]);
    let mut app=open(dir.path(),&["log","--name","named.rs","--line","40"]);draw(&mut app,120,36);assert_eq!(app.session.labels(),("named.rs","named.rs"));assert!(app.session.view().anchor_line_on(Side::Right)>=25.0,"{}",app.session.view().anchor_line_on(Side::Right));
}

#[test]
fn launch_from_subdirectory_resolves_root_files_sources_follow_and_search() {
    let dir=repository();let subdir=dir.path().join("nested/deep");std::fs::create_dir_all(&subdir).unwrap();
    std::fs::write(dir.path().join("file.rs"),"const VALUE: &str = \"working needle\";\n").unwrap();
    let mut app=open(&subdir,&["log"]);
    assert_eq!(app.repository.as_ref().unwrap().core.picker.root,dir.path().canonicalize().unwrap());
    assert_eq!(app.repository.as_ref().unwrap().core.picker.sources.local,1);
    assert_eq!(app.session.comparisons()[0].path,"file.rs");
    assert!(screen(&draw(&mut app,120,36)).contains("working needle"));
    key(&mut app,KeyCode::F(10),KeyModifiers::NONE);text(&mut app,"follow");key(&mut app,KeyCode::Enter,KeyModifiers::NONE);ready(&mut app);
    assert!(app.repository.as_ref().unwrap().core.picker.file_paths.as_ref().is_some_and(|(path,commits)|path=="file.rs"&&commits.len()==2));
    key(&mut app,KeyCode::Char('f'),KeyModifiers::CONTROL|KeyModifiers::SHIFT);
    text(&mut app,"needle");key(&mut app,KeyCode::Enter,KeyModifiers::NONE);ready(&mut app);
    assert!(!app.set_search.as_ref().unwrap().model.hits().is_empty());
    assert!(screen(&draw(&mut app,120,36)).contains("file.rs"));
}

#[test]
fn comparison_popups_paint_above_visible_repository_panes(){
    let dir=repository();let mut app=open(dir.path(),&["log"]);draw(&mut app,120,36);key(&mut app,KeyCode::F(6),KeyModifiers::NONE);key(&mut app,KeyCode::F(6),KeyModifiers::NONE);key(&mut app,KeyCode::Char(':'),KeyModifiers::NONE);let painted=screen(&draw(&mut app,120,36));assert!(painted.contains("Commands:"),"{painted}");assert!(painted.contains("Next change"),"{painted}");
}

#[test]
fn refresh_keeps_applied_file_visible_after_git_omits_it_and_undo_writes_correct_path(){
    let dir=repository_at(tempfile::Builder::new().prefix(".lenz-refresh-").tempdir_in(std::env::current_dir().unwrap()).unwrap());git(dir.path(),&["config","core.autocrlf","false"]);std::fs::write(dir.path().join("z.txt"),"old\n").unwrap();git(dir.path(),&["add","z.txt"]);git(dir.path(),&["commit","-qm","Base for apply"]);std::fs::write(dir.path().join("z.txt"),"new\n").unwrap();let mut app=open(dir.path(),&[]);draw(&mut app,120,36);assert_eq!(app.session.comparisons()[app.session.current()].path,"z.txt");key(&mut app,KeyCode::Right,KeyModifiers::ALT|KeyModifiers::SHIFT);ready(&mut app);key(&mut app,KeyCode::Char('s'),KeyModifiers::CONTROL);ready(&mut app);assert_eq!(std::fs::read_to_string(dir.path().join("z.txt")).unwrap(),"old\n");let id=app.session.applied()[0].id;
    std::fs::write(dir.path().join("a.txt"),"new added file\n").unwrap();key(&mut app,KeyCode::F(5),KeyModifiers::NONE);ready(&mut app);let painted=screen(&draw(&mut app,120,36));assert!(painted.contains("Applied"),"{painted}");assert_eq!(app.session.comparisons()[app.session.current()].path,"z.txt");assert_eq!(app.session.applied()[0].id,id);assert!(app.session.comparisons().iter().any(|c|c.path=="a.txt"));key(&mut app,KeyCode::Char('z'),KeyModifiers::CONTROL);ready(&mut app);key(&mut app,KeyCode::Char('s'),KeyModifiers::CONTROL);ready(&mut app);assert_eq!(std::fs::read_to_string(dir.path().join("z.txt")).unwrap(),"new\n");assert_eq!(std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),"new added file\n");
}

#[test]
fn files_type_search_keeps_diff_until_confirmation_and_routes_modal_commands() {
    let dir=repository();
    std::fs::create_dir(dir.path().join("nested")).unwrap();
    for path in ["alpha.txt","nested/Beta.txt","nested/Beta-two.txt"]{std::fs::write(dir.path().join(path),"new content\n").unwrap();}
    for i in 0..12 {std::fs::write(dir.path().join(format!("z-{i:02}.txt")),"new content\n").unwrap();}
    let mut app=open(dir.path(),&[]);draw(&mut app,120,36);key(&mut app,KeyCode::F(6),KeyModifiers::NONE);assert!(app.navigator.focused);
    let original=app.session.current();let spec=app.session.spec().clone();let frame=app.session.view().frame();
    for ch in "qbgynjk:/?".chars(){key(&mut app,KeyCode::Char(ch),KeyModifiers::NONE);assert_eq!(app.session.current(),original);assert_eq!(app.session.spec(),&spec);assert_eq!(app.session.view().frame(),frame);}
    assert_eq!(app.navigator.search.query(),"qbgynjk:/?");assert!(app.prompt.is_none()&&!app.quit);
    key(&mut app,KeyCode::Esc,KeyModifiers::NONE);assert!(!app.navigator.search.active());assert_eq!(app.navigator.candidate(),original);
    key(&mut app,KeyCode::Char('f'),KeyModifiers::CONTROL);assert!(app.navigator.search.active());assert!(app.prompt.is_none());
    app.handle(Event::Paste("NESTED/bEtA".into()));ready(&mut app);let candidate=app.navigator.candidate();assert!(app.session.comparisons()[candidate].path.contains("Beta"));assert_eq!(app.session.current(),original);
    let buffer=draw(&mut app,120,36);assert!(buffer.content.iter().any(|cell|cell.symbol()=="B"&&cell.bg==crate::chrome::color(crate::chrome::palette().found_bg)));let painted=screen(&buffer);assert!(painted.contains("alpha.txt")&&painted.contains("nested/Beta.txt")&&painted.contains("Find: NESTED/bEtA"));
    key(&mut app,KeyCode::F(3),KeyModifiers::NONE);assert_ne!(app.navigator.candidate(),candidate);assert_eq!(app.session.current(),original);
    key(&mut app,KeyCode::F(3),KeyModifiers::SHIFT);assert_eq!(app.navigator.candidate(),candidate);
    let files_text=|buffer:&Buffer,area:ratatui::layout::Rect|{let mut text=String::new();for y in area.y..area.bottom(){for x in area.x..area.right(){text.push_str(buffer[(x,y)].symbol());}}text};
    draw(&mut app,120,20);key(&mut app,KeyCode::End,KeyModifiers::NONE);let buffer=draw(&mut app,120,20);let last=files_text(&buffer,app.navigator.area);assert!(last.contains("z-11.txt")&&!last.contains("alpha.txt"),"{last}");assert_eq!(app.navigator.candidate(),candidate);assert_eq!(app.session.current(),original);
    key(&mut app,KeyCode::Home,KeyModifiers::NONE);assert!({let buffer=draw(&mut app,120,20);files_text(&buffer,app.navigator.area)}.contains("alpha.txt"));key(&mut app,KeyCode::PageDown,KeyModifiers::NONE);assert!(!{let buffer=draw(&mut app,120,20);files_text(&buffer,app.navigator.area)}.contains("alpha.txt"));key(&mut app,KeyCode::PageUp,KeyModifiers::NONE);assert!({let buffer=draw(&mut app,120,20);files_text(&buffer,app.navigator.area)}.contains("alpha.txt"));assert_eq!(app.navigator.candidate(),candidate);assert_eq!(app.session.current(),original);draw(&mut app,120,36);
    key(&mut app,KeyCode::F(10),KeyModifiers::SHIFT);assert!(screen(&draw(&mut app,120,36)).contains("Copy file path"));assert_eq!(app.session.current(),original);key(&mut app,KeyCode::Esc,KeyModifiers::NONE);
    key(&mut app,KeyCode::Enter,KeyModifiers::NONE);ready(&mut app);assert_eq!(app.session.current(),candidate);assert_eq!(app.navigator.search.query(),"NESTED/bEtA");
    key(&mut app,KeyCode::Char('a'),KeyModifiers::CONTROL);assert_eq!(app.navigator.search.query(),"");
    key(&mut app,KeyCode::Esc,KeyModifiers::NONE);assert_eq!(app.navigator.candidate(),candidate);assert_eq!(app.session.current(),candidate);
    key(&mut app,KeyCode::Char('f'),KeyModifiers::CONTROL|KeyModifiers::SHIFT);assert!(app.set_search.is_some());assert!(!app.navigator.search.active());
}

#[test]
fn focused_file_and_commit_escape_preserves_completed_change_set_search() {
    let dir=repository();let mut app=open(dir.path(),&["log"]);draw(&mut app,120,36);
    key(&mut app,KeyCode::Char('f'),KeyModifiers::CONTROL|KeyModifiers::SHIFT);text(&mut app,"needle");key(&mut app,KeyCode::Enter,KeyModifiers::NONE);ready(&mut app);draw(&mut app,120,36);
    assert!(!app.set_search.as_ref().unwrap().model.editing());
    for _ in 0..8 {if !app.set_search.as_ref().unwrap().focused&&app.repository.as_ref().unwrap().core.picker.focus==lenz::repository_session::Pane::Log{break;}key(&mut app,KeyCode::Tab,KeyModifiers::NONE);}
    assert_eq!(app.repository.as_ref().unwrap().core.picker.focus,lenz::repository_session::Pane::Log);
    let original=app.session.spec().clone();let anchor=app.repository.as_ref().unwrap().core.picker.anchor;
    text(&mut app,"first");assert!(app.repository.as_ref().unwrap().core.picker.pane_search.active());key(&mut app,KeyCode::Esc,KeyModifiers::NONE);assert!(!app.repository.as_ref().unwrap().core.picker.pane_search.active());assert!(app.set_search.is_some());assert_eq!(app.session.spec(),&original);assert_eq!(app.repository.as_ref().unwrap().core.picker.anchor,anchor);
    draw(&mut app,160,48);key(&mut app,KeyCode::Tab,KeyModifiers::NONE);assert!(app.navigator.focused);text(&mut app,"file");assert!(app.navigator.search.active());key(&mut app,KeyCode::Esc,KeyModifiers::NONE);assert!(!app.navigator.search.active());assert!(app.set_search.is_some());assert_eq!(app.session.spec(),&original);
}

#[test]
fn pending_edits_survive_file_switch_and_refuse_repository_replacement_or_whole_file_discard() {
    use lenz::{edit_session::EditMenuCommand,file_actions::FileAction};
    let dir=repository_at(tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap());std::fs::write(dir.path().join("second.txt"),"second old\n").unwrap();git(dir.path(),&["add","second.txt"]);git(dir.path(),&["commit","-qm","Second file"]);
    std::fs::write(dir.path().join("file.rs"),"pending draft\n").unwrap();std::fs::write(dir.path().join("second.txt"),"second new\n").unwrap();
    let mut app=open(dir.path(),&[]);draw(&mut app,240,48);let index=app.session.comparisons().iter().position(|c|c.path=="file.rs").unwrap();app.session.open_file(index);ready(&mut app);
    let original_spec=app.session.spec().clone();let mut alias_set=(*app.session.change_set().unwrap()).clone();let mut alias=alias_set.comparisons[index].clone();alias.path="./file.rs".into();let alias_index=alias_set.comparisons.len();alias_set.comparisons.push(alias);app.session.replace_set(std::sync::Arc::new(alias_set),original_spec.clone(),true);ready(&mut app);
    let disk=std::fs::read(dir.path().join("file.rs")).unwrap();app.edit_menu(EditMenuCommand::Revert);ready(&mut app);assert!(app.session.dirty());assert_eq!(std::fs::read(dir.path().join("file.rs")).unwrap(),disk);
    assert!(!app.session.dirty_at(alias_index)&&app.session.file_has_draft(alias_index));assert!(app.file_disabled(FileAction::Discard,alias_index).unwrap().contains("unsaved"));app.file_action(FileAction::Discard,alias_index);assert!(app.session.file_actions().confirmation().is_none());assert!(app.session.undo_file(alias_index).is_err());assert_eq!(std::fs::read(dir.path().join("file.rs")).unwrap(),disk);
    let other=app.session.comparisons().iter().position(|c|c.path=="second.txt").unwrap();app.session.open_file(other);ready(&mut app);assert!(!app.session.dirty()&&app.session.has_dirty());assert!(app.file_disabled(FileAction::Discard,index).unwrap().contains("unsaved"));app.file_action(FileAction::Discard,index);assert!(app.session.file_actions().confirmation().is_none());assert_eq!(std::fs::read(dir.path().join("file.rs")).unwrap(),disk);
    app.repository.as_mut().unwrap().core.execute(lenz::repository_session::RepositoryCommand::SelectRow{row:1,extend:false,two:false});ready(&mut app);assert_eq!(app.session.spec(),&original_spec);assert!(app.session.has_dirty());assert_eq!(app.session.current(),index);assert_eq!(std::fs::read(dir.path().join("file.rs")).unwrap(),disk);
    app.edit_menu(EditMenuCommand::Discard);ready(&mut app);assert!(!app.session.has_dirty());assert_eq!(std::fs::read(dir.path().join("file.rs")).unwrap(),disk);
    app.file_action(FileAction::Discard,index);ready(&mut app);assert!(app.session.file_actions().confirmation().is_some());key(&mut app,KeyCode::Enter,KeyModifiers::NONE);ready(&mut app);assert_eq!(std::fs::read_to_string(dir.path().join("file.rs")).unwrap(),"const VALUE: &str = \"second needle\";\n");
}

#[test]
fn whole_file_discard_restores_deleted_file_and_undo_restores_deletion() {
    use lenz::file_actions::FileAction;
    let dir=repository_at(tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap());std::fs::remove_file(dir.path().join("file.rs")).unwrap();let mut app=open(dir.path(),&[]);draw(&mut app,240,40);let index=app.session.comparisons().iter().position(|c|c.path=="file.rs").unwrap();assert!(!dir.path().join("file.rs").exists());
    app.file_action(FileAction::Discard,index);ready(&mut app);assert!(app.session.file_actions().confirmation().is_some());key(&mut app,KeyCode::Enter,KeyModifiers::NONE);ready(&mut app);assert_eq!(std::fs::read_to_string(dir.path().join("file.rs")).unwrap(),"const VALUE: &str = \"second needle\";\n");
    let index=app.session.comparisons().iter().position(|c|c.path=="file.rs").unwrap();app.file_action(FileAction::UndoDiscard,index);ready(&mut app);assert!(!dir.path().join("file.rs").exists());
}

#[test]
fn commit_context_menu_compares_with_a_branch_and_opens_the_commit_in_a_new_view() {
    use lenz::repository_session::{Pane, RepositoryCommand, RepositorySession};
    let dir = repository();
    let head = git(dir.path(), &["rev-parse", "HEAD"]);
    let mut app = open(dir.path(), &["log"]);
    let row = app.repository.as_mut().map(|r| {
        r.core.execute(RepositoryCommand::Focus(Pane::Log));
        r.core.picker.anchor
    }).unwrap();
    let shared = RepositorySession::log_context_commands(row, "").len();
    let first_target = app.repository.as_ref().unwrap().core.branch_targets(row)[0].1.clone();
    // Shift-F10 ▸ Compare with Branch… (after the shared commands) ▸ the first branch
    key(&mut app, KeyCode::F(10), KeyModifiers::SHIFT);
    for _ in 0..shared {
        key(&mut app, KeyCode::Down, KeyModifiers::NONE);
    }
    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    ready(&mut app);
    let RepositoryCommand::CompareRevs(left, right) = first_target else { panic!() };
    assert_eq!(left, head);
    assert!(matches!(app.session.spec(), Spec::Git { revs, .. } if revs == &[left.clone(), right.clone()]));
    // Shift-F10 ▸ Open in New Window: the commit in an independent view
    let open_at = RepositorySession::log_context_commands(row, "").iter().position(|c| *c == RepositoryCommand::OpenCommit).unwrap();
    key(&mut app, KeyCode::F(10), KeyModifiers::SHIFT);
    for _ in 0..open_at {
        key(&mut app, KeyCode::Down, KeyModifiers::NONE);
    }
    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    let (request, _) = app.new_view.take().expect("a new view for the commit");
    assert!(matches!(&request.spec, Spec::Git { revs, .. } if revs.last() == Some(&head) && revs.len() == 2));
}
