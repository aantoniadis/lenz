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
fn tui_whitespace_keeps_repeated_conflict_choices_separate() {
    use lenz::conflict_session::FileView;
    let dir=tempfile::tempdir().unwrap();
    for (name,text) in [("base","base\nseparator\nbase\n"),("ours","ours\nseparator\nours\n"),("theirs","theirs\nseparator\ntheirs\n"),("result","")] {
        std::fs::write(dir.path().join(name),text).unwrap();
    }
    let req=cli::parse(&["merge","base","ours","theirs","result"].map(str::to_owned),dir.path()).unwrap();
    let mut app=crate::merge::Merger::open(req,dir.path().into());
    let deadline=Instant::now()+Duration::from_secs(10);
    while app.busy(){app.poll();assert!(Instant::now()<deadline);thread::sleep(Duration::from_millis(2));}
    let press=|app:&mut crate::merge::Merger,code,mods|assert!(app.handle(Event::Key(KeyEvent::new(code,mods))));
    press(&mut app,KeyCode::Char('1'),KeyModifiers::NONE);
    press(&mut app,KeyCode::Char('w'),KeyModifiers::NONE);
    assert!(matches!(&app.workspace.file,Some(FileView::Merge(m)) if m.m.unresolved()==1));
    press(&mut app,KeyCode::Char('n'),KeyModifiers::NONE);
    press(&mut app,KeyCode::Char('n'),KeyModifiers::NONE);
    press(&mut app,KeyCode::Char('2'),KeyModifiers::NONE);
    press(&mut app,KeyCode::Char('w'),KeyModifiers::NONE);
    press(&mut app,KeyCode::Char('s'),KeyModifiers::CONTROL);
    assert_eq!(std::fs::read(dir.path().join("result")).unwrap(),b"ours\nseparator\ntheirs\n");
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

/// A zip of `files` (path, content) made by `git archive`, which every test machine has.
fn git_zip(files: &[(&str, &str)]) -> Vec<u8> {
    let source = tempfile::tempdir().unwrap();
    git(source.path(), &["init", "-q"]);
    for (path, content) in files {
        let path = source.path().join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }
    git(source.path(), &["add", "-A"]);
    git(source.path(), &["commit", "-qm", "archive"]);
    let out = Command::new("git").args(["archive", "--format=zip", "HEAD"]).current_dir(source.path()).output().unwrap();
    assert!(out.status.success());
    out.stdout
}

#[test]
fn committed_zip_opens_as_a_folder_whose_files_show_their_changes_read_only() {
    let dir = repository();
    std::fs::write(dir.path().join("bundle.zip"), git_zip(&[("src/main.rs", "fn main() {\n    old();\n}\n"), ("gone.txt", "bye\n")])).unwrap();
    git(dir.path(), &["add", "bundle.zip"]);
    git(dir.path(), &["commit", "-qm", "Add bundle"]);
    let v2 = git_zip(&[("src/main.rs", "fn main() {\n    new_call();\n}\n"), ("added.txt", "hi\n")]);
    std::fs::write(dir.path().join("bundle.zip"), &v2).unwrap();
    git(dir.path(), &["commit", "-qam", "Change bundle"]);
    let mut app = open(dir.path(), &["HEAD~", "HEAD"]);
    // the archive starts collapsed, and with nothing else changed its summary covers the first file
    let buffer = draw(&mut app, 160, 30);
    assert!(screen(&buffer).contains("▸ bundle.zip · 3 files"), "{}", screen(&buffer));
    assert_eq!(app.session.folders.summary().map(|s| s.folder.as_str()), Some("bundle.zip"));
    // a click on the heading's arrow expands it: the archive's members, as folders
    let (x, y) = cell_of(&buffer, "▸ bundle.zip");
    left_click(&mut app, (x, y));
    let visible = screen(&draw(&mut app, 160, 30));
    for row in ["bundle.zip/src", "main.rs", "added.txt", "gone.txt"] {
        assert!(visible.contains(row), "{row} in {visible}");
    }
    // Esc leaves Files, where ] is search text; ] steps to the member's comparison, which shows both versions of the
    // file inside the archive
    key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    for _ in 0..app.session.comparisons().len() {
        if app.session.labels().1 == "bundle.zip/src/main.rs" { break; }
        key(&mut app, KeyCode::Char(']'), KeyModifiers::NONE);
        ready(&mut app);
    }
    assert_eq!(app.session.labels().1, "bundle.zip/src/main.rs");
    let visible = screen(&draw(&mut app, 160, 30));
    assert!(visible.contains("old();") && visible.contains("new_call();"), "{visible}");
    // Apply is refused with the reason, and nothing is written
    key(&mut app, KeyCode::Right, KeyModifiers::ALT | KeyModifiers::SHIFT);
    assert!(app.session.notice().is_some_and(|n| n.contains("inside an archive")), "{:?}", app.session.notice());
    assert_eq!(std::fs::read(dir.path().join("bundle.zip")).unwrap(), v2);
    // Show file history follows the archive git knows
    key(&mut app, KeyCode::F(10), KeyModifiers::NONE);
    text(&mut app, "show file history");
    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    ready(&mut app);
    let followed = app.repository.as_ref().and_then(|r| r.core.picker.file_paths.as_ref()).map(|(path, _)| path.clone());
    assert_eq!(followed.as_deref(), Some("bundle.zip"));
}

/// The screen cell where `needle` starts, from the last drawn buffer.
fn cell_of(buffer: &Buffer, needle: &str) -> (u16, u16) {
    let width = usize::from(buffer.area.width);
    for y in 0..buffer.area.height {
        let row: String = (0..buffer.area.width).map(|x| buffer[(x, y)].symbol().to_string()).collect();
        if let Some(at) = row.find(needle) {
            return (row[..at].chars().count().min(width) as u16, y);
        }
    }
    panic!("{needle} not on screen:\n{}", screen(buffer));
}

fn left_click(app: &mut App, (column, row): (u16, u16)) {
    use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    app.handle(Event::Mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column, row, modifiers: KeyModifiers::NONE }));
    app.poll();
    app.handle(Event::Mouse(MouseEvent { kind: MouseEventKind::Up(MouseButton::Left), column, row, modifiers: KeyModifiers::NONE }));
    app.poll();
}

#[test]
fn archive_folders_start_collapsed_and_show_a_summary_of_their_changed_files() {
    use lenz::folders::{FolderAction, Stat};
    let dir = repository();
    std::fs::write(dir.path().join("bundle.zip"), git_zip(&[("src/main.rs", "fn main() {\n    old();\n}\n"), ("gone.txt", "bye\n")])).unwrap();
    git(dir.path(), &["add", "bundle.zip"]);
    git(dir.path(), &["commit", "-qm", "Add bundle"]);
    std::fs::write(dir.path().join("bundle.zip"), git_zip(&[("src/main.rs", "fn main() {\n    new_call();\n    more();\n}\n"), ("added.txt", "hi\n")])).unwrap();
    std::fs::write(dir.path().join("file.rs"), "const VALUE: &str = \"third\";\n").unwrap();
    git(dir.path(), &["commit", "-qam", "Change bundle"]);
    let mut app = open(dir.path(), &["HEAD~", "HEAD"]);
    // the archive is one collapsed heading with its file count; the plain file is shown
    let visible = screen(&draw(&mut app, 160, 30));
    assert!(visible.contains("▸ bundle.zip · 3 files") && !visible.contains("added.txt"), "{visible}");
    assert_eq!(app.session.labels().1, "file.rs");
    // ] skips the hidden files
    key(&mut app, KeyCode::Char(']'), KeyModifiers::NONE);
    assert_eq!(app.session.labels().1, "file.rs");
    assert!(app.session.notice().is_some_and(|n| n.contains("No later visible file")), "{:?}", app.session.notice());
    // a click on the heading's name shows the summary, its counts filled in the background
    let buffer = draw(&mut app, 160, 30);
    let (x, y) = cell_of(&buffer, "bundle.zip · 3");
    left_click(&mut app, (x + 2, y));
    ready(&mut app);
    let summary = app.session.folders.summary().cloned().expect("the summary shows");
    assert_eq!(summary.folder, "bundle.zip");
    let main = summary.files.iter().copied().find(|&i| app.session.comparisons()[i].path.ends_with("src/main.rs")).unwrap();
    assert_eq!(app.session.folders.stat(main), Some(&Stat::Lines { added: 2, removed: 1 }));
    let visible = screen(&draw(&mut app, 160, 30));
    assert!(visible.contains("bundle.zip · 3 changed files") && visible.contains("src/main.rs") && visible.contains("+2 −1"), "{visible}");
    assert!(!visible.contains("third"), "the comparison behind the summary doesn't show through: {visible}");
    // a click on the summary's heading gives it the keyboard; down to main.rs, Enter opens it, and the archive
    // expands to show it
    let at = cell_of(&draw(&mut app, 160, 30), "3 changed files");
    left_click(&mut app, at);
    assert!(!app.navigator.focused);
    for _ in 0..summary.files.len() {
        if app.session.folders.selected_file() == Some(main) { break; }
        key(&mut app, KeyCode::Down, KeyModifiers::NONE);
    }
    assert_eq!(app.session.folders.selected_file(), Some(main));
    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    ready(&mut app);
    assert!(app.session.folders.summary().is_none());
    assert_eq!(app.session.labels().1, "bundle.zip/src/main.rs");
    assert!(screen(&draw(&mut app, 160, 30)).contains("▾ bundle.zip"));
    // Collapse all archives from the palette hides the file on screen, so its folder's summary takes its place
    app.run_action(lenz::catalog::Action::Folder(FolderAction::CollapseArchives));
    assert!(app.session.folders.collapsed("bundle.zip"));
    assert_eq!(app.session.folders.summary().map(|s| s.folder.as_str()), Some("bundle.zip/src"));
    // Esc goes back to the file, its folders expanded again
    key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert!(app.session.folders.summary().is_none() && !app.session.folders.collapsed("bundle.zip"));
    assert_eq!(app.session.labels().1, "bundle.zip/src/main.rs");
    // with nothing to collapse at the top level, the palette says why
    use lenz::catalog::Availability;
    assert!(app.disabled(lenz::catalog::Action::Folder(FolderAction::ExpandAll)).is_some_and(|why| why.contains("No folder is collapsed")));
}

#[test]
fn files_keys_walk_folder_headings_as_a_tree() {
    let dir = repository();
    std::fs::write(dir.path().join("a.zip"), git_zip(&[("x.txt", "1\n")])).unwrap();
    git(dir.path(), &["add", "a.zip"]);
    git(dir.path(), &["commit", "-qm", "Add a.zip"]);
    std::fs::write(dir.path().join("a.zip"), git_zip(&[("x.txt", "2\n"), ("y.txt", "y\n")])).unwrap();
    std::fs::write(dir.path().join("file.rs"), "changed\n").unwrap();
    git(dir.path(), &["commit", "-qam", "Change a.zip"]);
    let mut app = open(dir.path(), &["HEAD~", "HEAD"]);
    draw(&mut app, 160, 30);
    key(&mut app, KeyCode::Tab, KeyModifiers::NONE);
    assert!(app.navigator.focused);
    // Down from the file reaches the archive's heading: its summary shows
    key(&mut app, KeyCode::Down, KeyModifiers::NONE);
    assert_eq!(app.session.folders.summary().map(|s| s.folder.as_str()), Some("a.zip"));
    // Right expands it, Down walks into it and opens its first file
    key(&mut app, KeyCode::Right, KeyModifiers::NONE);
    draw(&mut app, 160, 30);
    assert!(!app.session.folders.collapsed("a.zip"));
    key(&mut app, KeyCode::Down, KeyModifiers::NONE);
    ready(&mut app);
    assert_eq!(app.session.labels().1, "a.zip/x.txt");
    assert!(app.session.folders.summary().is_none());
    // Left on a file goes to its folder's heading; Left there collapses it
    key(&mut app, KeyCode::Left, KeyModifiers::NONE);
    draw(&mut app, 160, 30);
    assert_eq!(app.session.folders.summary().map(|s| s.folder.as_str()), Some("a.zip"));
    key(&mut app, KeyCode::Left, KeyModifiers::NONE);
    assert!(app.session.folders.collapsed("a.zip"));
    let visible = screen(&draw(&mut app, 160, 30));
    assert!(visible.contains("▸ a.zip · 2 files"), "{visible}");
}

/// A repository whose last commit changes all 60 files inside `big.zip`, open on `HEAD~ HEAD` (on the zip's summary).
fn big_archive() -> (tempfile::TempDir, App) {
    let dir = repository();
    let files = |v: &str| (0..60).map(|i| (format!("dir/f{i:02}.txt"), format!("{v} {i}\n"))).collect::<Vec<_>>();
    fn pairs(f: &[(String, String)]) -> Vec<(&str, &str)> { f.iter().map(|(p, c)| (p.as_str(), c.as_str())).collect() }
    std::fs::write(dir.path().join("big.zip"), git_zip(&pairs(&files("old")))).unwrap();
    git(dir.path(), &["add", "big.zip"]);
    git(dir.path(), &["commit", "-qm", "Add big"]);
    std::fs::write(dir.path().join("big.zip"), git_zip(&pairs(&files("new")))).unwrap();
    git(dir.path(), &["commit", "-qam", "Change big"]);
    let app = open(dir.path(), &["HEAD~", "HEAD"]);
    (dir, app)
}

#[test]
fn a_long_summary_scrolls_with_its_keyboard_row_at_every_size_and_esc_leaves_it() {
    use crossterm::event::{MouseEvent, MouseEventKind};
    let (_dir, mut app) = big_archive();
    assert_eq!(app.session.folders.summary().map(|s| s.files.len()), Some(60));
    let buffer = draw(&mut app, 160, 20);
    let at = cell_of(&buffer, "60 changed files");
    left_click(&mut app, at);
    // End then Up: the keyboard row is always on screen
    key(&mut app, KeyCode::End, KeyModifiers::NONE);
    assert_eq!(app.session.folders.summary().map(|s| s.selected), Some(59));
    assert!(screen(&draw(&mut app, 160, 20)).contains("dir/f59.txt"));
    key(&mut app, KeyCode::Up, KeyModifiers::NONE);
    let visible = screen(&draw(&mut app, 160, 20));
    assert!(visible.contains("dir/f58.txt") && !visible.contains("dir/f00.txt"), "{visible}");
    // the wheel scrolls away from it, and the next key brings it back
    for _ in 0..30 { app.handle(Event::Mouse(MouseEvent { kind: MouseEventKind::ScrollUp, column: at.0, row: at.1 + 3, modifiers: KeyModifiers::NONE })); }
    let visible = screen(&draw(&mut app, 160, 20));
    assert!(visible.contains("dir/f00.txt") && !visible.contains("dir/f58.txt"), "{visible}");
    key(&mut app, KeyCode::Up, KeyModifiers::NONE);
    assert!(screen(&draw(&mut app, 160, 20)).contains("dir/f57.txt"));
    // tiny and empty sizes draw without panicking and keep the selection
    for (w, h) in [(20, 10), (8, 3), (1, 1), (0, 0), (32, 6), (160, 20)] {
        app.handle(Event::Resize(w, h));
        draw(&mut app, w, h);
        key(&mut app, KeyCode::End, KeyModifiers::NONE);
        key(&mut app, KeyCode::Up, KeyModifiers::NONE);
    }
    assert_eq!(app.session.folders.summary().map(|s| s.selected), Some(58));
    // Esc from the scrolled summary returns to the file behind it, its folders expanded
    key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert!(app.session.folders.summary().is_none() && !app.session.folders.collapsed("big.zip"));
    assert!(screen(&draw(&mut app, 160, 20)).contains("big.zip/dir/f00.txt"));
}

#[test]
fn dialogs_over_the_summary_take_its_keys() {
    let (_dir, mut app) = big_archive();
    let at = cell_of(&draw(&mut app, 160, 30), "60 changed files");
    left_click(&mut app, at);
    key(&mut app, KeyCode::Down, KeyModifiers::NONE);
    let selected = app.session.folders.summary().map(|s| s.selected);
    assert_eq!(selected, Some(1));
    // the palette and help consume the arrows and Enter; the summary keeps its row and stays open
    key(&mut app, KeyCode::F(10), KeyModifiers::NONE);
    assert!(app.command_palette.is_some());
    key(&mut app, KeyCode::Down, KeyModifiers::NONE);
    key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert!(app.command_palette.is_none());
    key(&mut app, KeyCode::Char('?'), KeyModifiers::NONE);
    key(&mut app, KeyCode::Down, KeyModifiers::NONE);
    key(&mut app, KeyCode::End, KeyModifiers::NONE);
    key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert_eq!(app.session.folders.summary().map(|s| s.selected), selected);
    assert_eq!(app.session.current(), 0, "no file opened behind the dialogs");
}

#[test]
fn a_summary_and_a_pr_details_tab_never_share_the_diff_area() {
    use lenz::{catalog::{Action, PrAction}, folders::FolderAction, pr_session::DetailsTab};
    let (_dir, mut app) = big_archive();
    let pr = lenz::pr::PullRequest { number: 9, title: "PR".into(), author: lenz::pr::Account { login: "author".into() }, state: lenz::pr::State::Open, is_draft: false,
        body: "PR_BODY".into(), base_ref_name: "main".into(), head_ref_name: "feature".into(), head_ref_oid: "head".into(), url: "https://github.com/o/r/pull/9".into(),
        additions: 1, deletions: 0, changed_files: 1, is_cross_repository: false, head_repository: None, head_repository_owner: None };
    let mut state = lenz::pr_session::PrState::new(pr, lenz::pr::Fetched { number: 9, merge_base: "base".into(), head: "head".into() });
    state.marks = None;
    app.repository.as_mut().expect("a repository view").core.pr = Some(state);
    assert!(app.session.folders.summary().is_some());
    // a details tab, however it opens, closes the summary
    app.run_action(Action::Pr(PrAction::Details(DetailsTab::Conversation)));
    draw(&mut app, 160, 30);
    assert!(app.pr_details_shown() && app.session.folders.summary().is_none());
    // and the summary closes the tab
    app.run_action(Action::Folder(FolderAction::Summary));
    assert!(!app.pr_details_shown());
    assert_eq!(app.session.folders.summary().map(|s| s.folder.as_str()), Some("big.zip/dir"));
    let visible = screen(&draw(&mut app, 160, 30));
    assert!(visible.contains("60 changed files") && !visible.contains("PR_BODY"), "{visible}");
}

#[test]
fn a_heading_in_each_group_shows_that_group_only() {
    let dir = repository();
    std::fs::write(dir.path().join("a.zip"), git_zip(&[("f.txt", "1\n")])).unwrap();
    git(dir.path(), &["add", "a.zip"]);
    git(dir.path(), &["commit", "-qm", "Add a.zip"]);
    std::fs::write(dir.path().join("a.zip"), git_zip(&[("f.txt", "2\n")])).unwrap();
    git(dir.path(), &["add", "a.zip"]);
    std::fs::write(dir.path().join("a.zip"), git_zip(&[("f.txt", "3\n"), ("g.txt", "g\n")])).unwrap();
    let mut app = open(dir.path(), &["log"]);
    // Local changes: Staged and Unstaged, each with the archive's heading
    app.run_action(lenz::catalog::Action::Repository(lenz::catalog::RepoAction::LocalChanges));
    ready(&mut app);
    let buffer = draw(&mut app, 160, 40);
    assert!(screen(&buffer).contains("▸ a.zip · 1 file") && screen(&buffer).contains("▸ a.zip · 2 files"), "{}", screen(&buffer));
    left_click(&mut app, cell_of(&buffer, "a.zip · 2 files"));
    let summary = app.session.folders.summary().cloned().expect("the summary shows");
    assert_eq!(summary.files.iter().map(|&i| app.session.comparisons()[i].path.as_str()).collect::<Vec<_>>(), ["a.zip/f.txt", "a.zip/g.txt"]);
    assert_eq!(summary.range, app.session.change_set().unwrap().groups[1].1);
}

#[test]
fn archive_grouped_summary_cursor_uses_selected_group() {
    let dir = repository();
    std::fs::write(dir.path().join("a.zip"), git_zip(&[("f.txt", "1\n")])).unwrap();
    git(dir.path(), &["add", "a.zip"]); git(dir.path(), &["commit", "-qm", "Add archive"]);
    std::fs::write(dir.path().join("a.zip"), git_zip(&[("f.txt", "2\n")])).unwrap();
    git(dir.path(), &["add", "a.zip"]);
    std::fs::write(dir.path().join("a.zip"), git_zip(&[("f.txt", "3\n"), ("g.txt", "g\n")])).unwrap();
    let mut app = open(dir.path(), &["log"]);
    app.run_action(lenz::catalog::Action::Repository(lenz::catalog::RepoAction::LocalChanges)); ready(&mut app);
    let b = draw(&mut app,160,40); left_click(&mut app,cell_of(&b,"a.zip · 2 files"));
    let expected=app.session.change_set().unwrap().groups[1].1.start;
    assert_eq!(app.session.folders.summary().unwrap().range.start,expected);
    let b = draw(&mut app, 160, 40);
    let staged = cell_of(&b, "a.zip · 1 file");
    let unstaged = cell_of(&b, "a.zip · 2 files");
    assert_ne!(b[staged].bg, b[unstaged].bg, "only the selected group heading is highlighted");
    key(&mut app,KeyCode::Right,KeyModifiers::NONE); draw(&mut app,160,40);
    key(&mut app,KeyCode::Down,KeyModifiers::NONE); ready(&mut app);
    assert_eq!(app.session.current(),expected,"Down should open first Unstaged member, not Staged");
}
#[test]
fn archive_first_summary_row_click_opens_it_while_files_focused() {
    let (_dir,mut app)=big_archive();
    let b=draw(&mut app,160,30);
    left_click(&mut app,cell_of(&b,"big.zip · 60 files"));
    assert!(app.navigator.focused);
    let b=draw(&mut app,160,30); let at=cell_of(&b,"M dir/f01.txt");
    left_click(&mut app,at);ready(&mut app);
    assert!(app.session.folders.summary().is_none(),"first row click must open the row without an extra focus click");
    assert!(app.session.labels().1.ends_with("f01.txt"));
}

#[test]
fn archive_plain_reload_key_reads_changed_tar_members() {
    let source = repository();
    let d = tempfile::tempdir().unwrap();
    let archive = |rev| {
        let out = Command::new("git").args(["archive", "--format=tar.gz", rev]).current_dir(source.path()).output().unwrap();
        assert!(out.status.success()); out.stdout
    };
    std::fs::write(d.path().join("left.tgz"), archive("HEAD~")).unwrap();
    std::fs::write(d.path().join("right.tgz"), archive("HEAD")).unwrap();
    let mut app = open(d.path(), &["--no-index", "left.tgz", "right.tgz"]);
    app.session.open_file(0); ready(&mut app);
    assert!(app.session.document().unwrap().text(Side::Right).contains("second needle"));
    std::fs::write(source.path().join("file.rs"), "third needle\n").unwrap();
    git(source.path(), &["commit", "-qam", "third"]);
    std::fs::write(d.path().join("right.tgz"), archive("HEAD")).unwrap();
    key(&mut app, KeyCode::F(5), KeyModifiers::NONE); ready(&mut app);
    assert_eq!(app.session.document().unwrap().text(Side::Right), "third needle\n");
}

#[test]
fn summary_reload_key_refreshes_counts() {
    use lenz::folders::Stat;
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("left.txt"), "old\n").unwrap(); std::fs::write(d.path().join("right.txt"), "new\n").unwrap();
    let mut app = open(d.path(), &["--no-index", "left.txt", "right.txt"]);
    app.session.show_folder_of(0); ready(&mut app);
    assert_eq!(app.session.folders.stat(0), Some(&Stat::Lines { added: 1, removed: 1 }));
    std::fs::write(d.path().join("right.txt"), "new\nmore\nlast\n").unwrap();
    key(&mut app, KeyCode::F(5), KeyModifiers::NONE); ready(&mut app);
    assert_eq!(app.session.folders.stat(0), Some(&Stat::Lines { added: 3, removed: 1 }));
}

#[test]
fn summary_narrow_counts_stay_inside_the_pane() {
    use ratatui::{layout::Rect, style::Style};
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("left.bin"), vec![0; 1000]).unwrap(); std::fs::write(d.path().join("right.bin"), vec![0; 1001]).unwrap();
    let mut app = open(d.path(), &["--no-index", "left.bin", "right.bin"]);
    app.session.show_folder_of(0); ready(&mut app);
    let set = app.session.change_set().unwrap();
    let mut pane = crate::summary::SummaryPane::default();
    let mut terminal = Terminal::new(TestBackend::new(50, 10)).unwrap();
    for (width, palette) in [8, 18, 32].into_iter().flat_map(|width| [&lenz::theme::DARCULA, &lenz::theme::LIGHT].map(|palette| (width, palette))) {
        let area = Rect::new(50 - width, 0, width, 10);
        terminal.draw(|frame| {
            for y in 0..10 { frame.buffer_mut().set_string(0, y, "X".repeat(50), Style::default()); }
            pane.draw(frame, area, &set, &app.session.folders, palette);
        }).unwrap();
        let buffer = terminal.backend().buffer();
        for y in 0..10 { for x in 0..area.x { assert_eq!(buffer[(x, y)].symbol(), "X", "outside summary at {x},{y}, width {width}"); } }
    }
}

#[test]
fn encoded_search_and_directory_symlinks_use_real_input() {
    let dir=tempfile::tempdir().unwrap();
    for side in ["left","right"] {
        let root=dir.path().join(side);std::fs::create_dir(&root).unwrap();
        std::os::unix::fs::symlink(".",root.join("directory-link")).unwrap();
        let value=if side=="left" {"old\n"} else {"needle\n"};
        let bytes=std::iter::once(0xfeff).chain(value.encode_utf16()).flat_map(u16::to_le_bytes).collect::<Vec<_>>();
        std::fs::write(root.join("text.txt"),bytes).unwrap();
    }
    let mut app=open(dir.path(),&["--no-index","left","right"]);
    assert_eq!(app.session.comparisons().len(),1);
    key(&mut app,KeyCode::Char('f'),KeyModifiers::CONTROL|KeyModifiers::SHIFT);
    text(&mut app,"needle");key(&mut app,KeyCode::Enter,KeyModifiers::NONE);ready(&mut app);
    assert_eq!(app.set_search.as_ref().unwrap().model.hits().len(),1);
    assert!(screen(&draw(&mut app,120,36)).contains("needle"));
}

#[test]
fn whitespace_output_change_requires_save_before_close() {
    use lenz::{conflict_session::FileView,merge_session::{Act,Ask},merge::Pick};
    let dir=tempfile::tempdir().unwrap();
    for (name,text) in [("base","a\n"),("ours"," a\n"),("theirs","a\nb\n"),("result","")] {
        std::fs::write(dir.path().join(name),text).unwrap();
    }
    let req=cli::parse(&["merge","base","ours","theirs","result"].map(str::to_owned),dir.path()).unwrap();
    let mut app=crate::merge::Merger::open(req,dir.path().into());
    let deadline=Instant::now()+Duration::from_secs(10);
    while app.busy(){app.poll();assert!(Instant::now()<deadline);thread::sleep(Duration::from_millis(2));}
    if let Some(FileView::Merge(m))=&mut app.workspace.file {m.execute(Act::AllFrom(Pick::Theirs));} else {panic!()}
    for (code,mods) in [(KeyCode::Char('s'),KeyModifiers::CONTROL),(KeyCode::Char('w'),KeyModifiers::NONE),(KeyCode::Esc,KeyModifiers::NONE)] {
        assert!(app.handle(Event::Key(KeyEvent::new(code,mods))));
    }
    assert!(matches!(&app.workspace.file,Some(FileView::Merge(m)) if m.dirty&&m.ask==Some(Ask::Close)));
    assert_eq!(std::fs::read(dir.path().join("result")).unwrap(),b"a\nb\n");
    assert!(app.handle(Event::Key(KeyEvent::new(KeyCode::Char('s'),KeyModifiers::CONTROL))));
    assert_eq!(std::fs::read(dir.path().join("result")).unwrap(),b" a\nb\n");
}

#[test]
fn double_and_triple_click_select_the_word_and_its_syntax_group() {
    let dir = tempfile::tempdir().unwrap();
    let src = "fn f() { g(\"hello world\", 1); }\n";
    std::fs::write(dir.path().join("left.rs"), "x\n").unwrap();
    std::fs::write(dir.path().join("right.rs"), src).unwrap();
    let mut app = open(dir.path(), &["left.rs", "right.rs"]);
    let (x, y) = cell_of(&draw(&mut app, 160, 30), "world");
    let at = (x + 2, y);
    let selected = |app: &App| {
        let lines = app.session.document().map_or(&[][..], |d| d.lines_on(lenz::view::Side::Right));
        app.selection.map(|s| s.text(lines)).unwrap_or_default()
    };
    left_click(&mut app, at);
    left_click(&mut app, at);
    assert_eq!(selected(&app), "world");
    use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    app.handle(Event::Mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: at.0, row: at.1, modifiers: KeyModifiers::NONE }));
    // The main loop sleeps until input unless busy: the pending group keeps it polling.
    assert!(app.busy() || selected(&app) != "world", "the group is awaited");
    let deadline = Instant::now() + Duration::from_secs(10);
    while selected(&app) == "world" {
        assert!(Instant::now() < deadline, "the group did not arrive");
        thread::sleep(Duration::from_millis(2));
        app.poll();
    }
    assert_eq!(selected(&app), "hello world");
    left_click(&mut app, at);
    assert_eq!(selected(&app), "", "a fourth click starts over");
    assert_eq!(std::fs::read_to_string(dir.path().join("right.rs")).unwrap(), src);
}

/// A repository window on `main` (`a.txt`, `m.txt`), branch `other` changing `a.txt`, tag `v1`; `m.txt` changed.
fn checkout_repository() -> (tempfile::TempDir, App) {
    let dir = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
    let root = dir.path();
    git(root, &["init", "-q", "-b", "main"]);
    git(root, &["config", "user.name", "t"]);
    git(root, &["config", "user.email", "t@t"]);
    std::fs::write(root.join("a.txt"), "main\n").unwrap();
    std::fs::write(root.join("m.txt"), "base\n").unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "main"]);
    git(root, &["tag", "v1"]);
    git(root, &["switch", "-q", "-c", "other"]);
    std::fs::write(root.join("a.txt"), "other\n").unwrap();
    git(root, &["commit", "-qam", "other"]);
    git(root, &["switch", "-q", "main"]);
    std::fs::write(root.join("m.txt"), "changed\n").unwrap();
    let app = open(root, &["log"]);
    (dir, app)
}
fn current_branch(root: &Path) -> String {
    git(root, &["branch", "--show-current"])
}
/// Where `needle` starts on screen.
fn find(buffer: &Buffer, needle: &str) -> (u16, u16) {
    let width = buffer.area.width as usize;
    let cells: Vec<&str> = buffer.content.iter().map(|c| c.symbol()).collect();
    for y in 0..buffer.area.height as usize {
        let line: String = cells[y * width..(y + 1) * width].concat();
        if let Some(at) = line.find(needle) {
            return (line[..at].chars().count() as u16, y as u16);
        }
    }
    panic!("{needle:?} not on screen:\n{}", screen(buffer));
}

#[test]
fn the_sidebar_menu_checks_out_a_branch_in_the_terminal() {
    use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    let (dir, mut app) = checkout_repository();
    let buffer = draw(&mut app, 160, 40);
    let (x, y) = find(&buffer, "⎇ other");
    app.handle(Event::Mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Right), column: x + 2, row: y, modifiers: KeyModifiers::NONE }));
    let menu = screen(&draw(&mut app, 160, 40));
    assert!(menu.contains("Checkout") && menu.contains("New Branch from Here…") && menu.contains("Compare with Current"), "{menu}");
    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    ready(&mut app);
    assert_eq!(current_branch(dir.path()), "other");
    assert_eq!(std::fs::read_to_string(dir.path().join("a.txt")).unwrap(), "other\n");
    assert_eq!(std::fs::read_to_string(dir.path().join("m.txt")).unwrap(), "changed\n", "the local change went along");
    assert_eq!(app.repository.as_ref().unwrap().core.picker.title_branch, "other");
}

#[test]
fn the_checkout_dialog_offers_smart_and_a_confirmed_force_in_the_terminal() {
    use crossterm::event::KeyEventKind;
    let (dir, mut app) = checkout_repository();
    std::fs::write(dir.path().join("a.txt"), "mine\n").unwrap();
    app.git_command(lenz::repository_session::RepositoryCommand::Checkout(lenz::branches::Target::Branch("other".into())));
    ready(&mut app);
    let shown = screen(&draw(&mut app, 120, 30));
    assert!(shown.contains("s Smart Checkout") && shown.contains("would overwrite your local changes"), "{shown}");
    key(&mut app, KeyCode::Char('f'), KeyModifiers::NONE);
    ready(&mut app);
    assert!(screen(&draw(&mut app, 120, 30)).contains("y Force Checkout"));
    // Enter, a held y and s do nothing at the confirmation.
    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    let mut held = KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE);
    held.kind = KeyEventKind::Repeat;
    app.handle(Event::Key(held));
    key(&mut app, KeyCode::Char('s'), KeyModifiers::NONE);
    ready(&mut app);
    assert_eq!(std::fs::read_to_string(dir.path().join("a.txt")).unwrap(), "mine\n");
    // Tiny terminals draw it; then y confirms.
    draw(&mut app, 20, 6);
    key(&mut app, KeyCode::Char('y'), KeyModifiers::NONE);
    ready(&mut app);
    assert_eq!(current_branch(dir.path()), "other");
    assert_eq!(std::fs::read_to_string(dir.path().join("a.txt")).unwrap(), "other\n");
    assert!(!app.repository.as_ref().unwrap().modal());
}

#[test]
fn new_branch_and_revision_prompts_refuse_in_place_in_the_terminal() {
    use lenz::catalog::{Action, RepoAction};
    let (dir, mut app) = checkout_repository();
    app.run_action(Action::Repository(RepoAction::NewBranch));
    text(&mut app, "other");
    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    assert!(screen(&draw(&mut app, 160, 40)).contains("A branch named \"other\" already exists"));
    app.handle(Event::Paste("-x".into()));
    key(&mut app, KeyCode::Tab, KeyModifiers::NONE);
    assert!(screen(&draw(&mut app, 160, 40)).contains("[ ] check out (Tab): other-x"));
    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    ready(&mut app);
    assert_eq!(current_branch(dir.path()), "main");
    assert_eq!(git(dir.path(), &["rev-parse", "other-x"]), git(dir.path(), &["rev-parse", "main"]));
    app.run_action(Action::Repository(RepoAction::CheckoutRevision));
    text(&mut app, "nope");
    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    assert!(screen(&draw(&mut app, 160, 40)).contains("No commit named \"nope\""));
    key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert!(!app.repository.as_ref().unwrap().modal());
    app.run_action(Action::Repository(RepoAction::CheckoutRevision));
    text(&mut app, "v1");
    key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    ready(&mut app);
    assert_eq!(current_branch(dir.path()), "");
}

#[test]
fn an_unsaved_draft_blocks_a_checkout_in_the_terminal() {
    let (dir, mut app) = checkout_repository();
    let index = app.session.comparisons().iter().position(|c| c.path == "m.txt").unwrap();
    app.session.open_file(index);
    ready(&mut app);
    app.edit_menu(lenz::edit_session::EditMenuCommand::Revert);
    ready(&mut app);
    assert!(app.session.has_dirty());
    app.git_command(lenz::repository_session::RepositoryCommand::Checkout(lenz::branches::Target::Branch("other".into())));
    ready(&mut app);
    assert_eq!(current_branch(dir.path()), "main");
    assert_eq!(std::fs::read_to_string(dir.path().join("m.txt")).unwrap(), "changed\n");
    assert!(app.session.has_dirty() && app.session.notice().is_some_and(|n| n.starts_with("Unsaved changes")));
}
