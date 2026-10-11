use super::*;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use lenz::theme::{ThemeOverrides, ThemePaths, ThemeSelection, ThemeService};
use ratatui::{Terminal, backend::TestBackend};
fn key(s: &mut Screens, code: KeyCode) {
    s.handle(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
}
fn fixture() -> (tempfile::TempDir, Screens) {
    let d = tempfile::Builder::new()
        .prefix(".lenz-theme-")
        .tempdir_in(std::env::current_dir().unwrap())
        .unwrap();
    std::fs::write(d.path().join("left"), "old\n").unwrap();
    std::fs::write(d.path().join("right"), "new\n").unwrap();
    let req = cli::parse(
        &["--no-index".into(), "left".into(), "right".into()],
        d.path(),
    )
    .unwrap();
    let service = ThemeService::new(
        ThemePaths {
            config_home: d.path().join("config"),
            state_home: d.path().join("state"),
        },
        ThemeOverrides::default(),
        true,
    );
    let s = Screens::open_with_theme(req, d.path().into(), service);
    (d, s)
}
fn choose(s: &mut Screens, id: &str) {
    key(s, KeyCode::Char(':'));
    for c in "Change theme".chars() {
        key(s, KeyCode::Char(c));
    }
    key(s, KeyCode::Enter);
    let row = s
        .chooser
        .as_ref()
        .unwrap()
        .entries
        .iter()
        .position(|e| e.id == id)
        .unwrap();
    key(s, KeyCode::Home);
    for _ in 0..row {
        key(s, KeyCode::Down);
    }
    key(s, KeyCode::Enter);
}
#[test]
fn terminal_theme_commands_recolor_cached_toolbar_without_writing() {
    let (d, mut s) = fixture();
    let mut t = Terminal::new(TestBackend::new(100, 24)).unwrap();
    t.draw(|f| s.render(f)).unwrap();
    for e in lenz::theme::builtin_entries() {
        choose(&mut s, &e.id);
        t.draw(|f| s.render(f)).unwrap();
        assert_eq!(s.themes.snapshot().selection, ThemeSelection::Named(e.id));
        assert_eq!(
            t.backend().buffer()[(0, 0)].bg,
            chrome::color(e.palette.header_bg)
        );
        assert_eq!(std::fs::read(d.path().join("right")).unwrap(), b"new\n");
    }
}
#[test]
fn terminal_theme_modal_clamps_small_sizes_and_blocks_edits() {
    let (d, mut s) = fixture();
    let mut t = Terminal::new(TestBackend::new(40, 10)).unwrap();
    key(&mut s, KeyCode::Char(':'));
    for c in "Change theme".chars() {
        key(&mut s, KeyCode::Char(c));
    }
    key(&mut s, KeyCode::Enter);
    for (w, h) in [(40, 10), (1, 1), (10, 3), (80, 24)] {
        t.backend_mut().resize(w, h);
        t.resize(ratatui::layout::Rect::new(0, 0, w, h)).unwrap();
        key(&mut s, KeyCode::End);
        key(&mut s, KeyCode::Up);
        s.handle(Event::Key(KeyEvent::new(
            KeyCode::Char('s'),
            KeyModifiers::CONTROL,
        )));
        s.handle(Event::Key(KeyEvent::new(
            KeyCode::Right,
            KeyModifiers::ALT | KeyModifiers::SHIFT,
        )));
        t.draw(|f| s.render(f)).unwrap();
        assert!(s.chooser.is_some());
    }
    key(&mut s, KeyCode::Esc);
    assert!(s.chooser.is_none());
    assert_eq!(std::fs::read(d.path().join("right")).unwrap(), b"new\n");
}
#[test]
fn terminal_theme_chooser_preserves_global_quit_and_merge_confirmation() {
    for shortcut in ['c', 'q'] {
        let (d, mut s) = fixture();
        s.open_themes();
        s.handle(Event::Key(KeyEvent::new(
            KeyCode::Char(shortcut),
            KeyModifiers::CONTROL,
        )));
        s.poll();
        assert!(s.chooser.is_none(), "Ctrl-{shortcut} must dismiss chooser");
        assert_eq!(s.exit(), Some(0));
        assert_eq!(std::fs::read(d.path().join("right")).unwrap(), b"new\n");

        let (d, mut s) = fixture();
        for (name, bytes) in [
            ("base", "base\n"),
            ("ours", "ours\n"),
            ("theirs", "theirs\n"),
            ("result", "ours\n"),
        ] {
            std::fs::write(d.path().join(name), bytes).unwrap();
        }
        let req = cli::parse(
            &["merge", "base", "ours", "theirs", "result"].map(str::to_owned),
            d.path(),
        )
        .unwrap();
        s.views[0] = Screen::Merge {
            controller: Box::new(merge::Merger::open(req, d.path().into())),
            parent: None,
        };
        let until = Instant::now() + Duration::from_secs(3);
        while s.busy() {
            s.poll();
            assert!(Instant::now() < until);
            std::thread::sleep(Duration::from_millis(1));
        }
        key(&mut s, KeyCode::Char('n'));
        key(&mut s, KeyCode::Char('2'));
        s.open_themes();
        s.handle(Event::Key(KeyEvent::new(
            KeyCode::Char(shortcut),
            KeyModifiers::CONTROL,
        )));
        s.poll();
        assert!(s.chooser.is_none());
        assert_eq!(
            s.exit(),
            None,
            "unsaved merge requires existing confirmation"
        );
        let Screen::Merge { controller, .. } = &s.views[0] else {
            panic!()
        };
        let Some(lenz::conflict_session::FileView::Merge(m)) = &controller.workspace.file else {
            panic!()
        };
        assert!(m.dirty);
        assert_eq!(m.ask, Some(lenz::merge_session::Ask::Close));
        let mut t = Terminal::new(TestBackend::new(120, 24)).unwrap();
        t.draw(|f| s.render(f)).unwrap();
        assert!(
            t.backend()
                .buffer()
                .content
                .iter()
                .map(|c| c.symbol())
                .collect::<String>()
                .contains("Unsaved Result")
        );
        key(&mut s, KeyCode::Esc);
        assert_eq!(s.exit(), None);
        assert_eq!(std::fs::read(d.path().join("result")).unwrap(), b"ours\n");
    }
}
#[test]
fn terminal_theme_toolbar_keyboard_and_mouse_use_shared_selection() {
    let (d, mut s) = fixture();
    let mut t = Terminal::new(TestBackend::new(200, 24)).unwrap();
    t.draw(|f| s.render(f)).unwrap();
    // the palette: Change theme
    key(&mut s, KeyCode::Char(':'));for c in "change theme".chars(){key(&mut s,KeyCode::Char(c));}key(&mut s,KeyCode::Enter);
    assert!(s.chooser.is_some());
    key(&mut s, KeyCode::Esc);
    t.draw(|f| s.render(f)).unwrap();
    let locate=|buf:&ratatui::buffer::Buffer,needle:&str|{(0..buf.area.height).find_map(|y|(0..buf.area.width).find_map(|x|{let line=(x..buf.area.width).map(|x|buf[(x,y)].symbol()).collect::<String>();line.starts_with(needle).then_some((x,y))})).unwrap()};
    let at=locate(t.backend().buffer(),"☰");
    s.handle(Event::Mouse(event::MouseEvent {kind:event::MouseEventKind::Down(event::MouseButton::Left),column:at.0,row:at.1,modifiers:KeyModifiers::NONE}));
    t.draw(|f|s.render(f)).unwrap();let at=locate(t.backend().buffer(),"Change theme");
    s.handle(Event::Mouse(event::MouseEvent {kind:event::MouseEventKind::Down(event::MouseButton::Left),column:at.0,row:at.1,modifiers:KeyModifiers::NONE}));
    assert!(s.chooser.is_some());
    key(&mut s, KeyCode::End);
    key(&mut s, KeyCode::Enter);
    assert_eq!(
        s.themes.snapshot().selection,
        ThemeSelection::Named("gruvbox".into())
    );
    assert_eq!(std::fs::read(d.path().join("right")).unwrap(), b"new\n");
}
#[cfg(target_os = "linux")]
const COLORS: &str = "mode = \"dark\"\nbackground = \"#1e1e2e\"\nforeground = \"#cdd6f4\"\naccent = \"#89b4fa\"\nred = \"#f38ba8\"\ngreen = \"#a6e3a1\"\nblue = \"#89b4fa\"\nyellow = \"#f9e2af\"\nmagenta = \"#cba6f7\"\ncyan = \"#94e2d5\"\n";
#[cfg(target_os = "linux")]
#[test]
fn terminal_theme_idle_updates_and_source_changes_need_no_input() {
    let (d, mut s) = fixture();
    let current = d.path().join("state/omarchy/current");
    std::fs::create_dir_all(current.join("theme")).unwrap();
    std::fs::write(current.join("theme/colors.toml"), COLORS).unwrap();
    std::fs::write(current.join("theme.name"), "first").unwrap();
    let rx = s.themes.updates();
    let await_source = |s: &mut Screens, source: &str| {
        let until = Instant::now() + Duration::from_secs(3);
        while s.themes.snapshot().source != source {
            let update = futures_lite::future::block_on(futures_lite::future::race(
                async { rx.recv().await.unwrap() },
                async {
                    async_io::Timer::at(until).await;
                    panic!("no idle theme delivery")
                },
            ));
            s.theme_update(update);
        }
    };
    await_source(&mut s, "first");
    let revision = s.themes.snapshot().revision;
    std::fs::write(current.join("theme.name"), "renamed").unwrap();
    await_source(&mut s, "renamed");
    assert_eq!(s.themes.snapshot().revision, revision);
    let until = Instant::now() + Duration::from_secs(3);
    while s.busy() {
        s.poll();
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(1));
    }
    s.poll();
    for _ in 0..5 {
        assert!(!s.poll(), "idle must not request redraws");
    }
    let mut t = Terminal::new(TestBackend::new(80, 24)).unwrap();
    t.draw(|f| s.render(f)).unwrap();
    assert_eq!(
        t.backend().buffer()[(0, 0)].bg,
        chrome::color(s.themes.snapshot().palette.header_bg)
    );
    assert_eq!(std::fs::read(d.path().join("right")).unwrap(), b"new\n");
}

#[test]
fn terminal_theme_presets_preserve_apply_undo_and_inactive_views() {
    let (d, mut s) = fixture();
    let until = Instant::now() + Duration::from_secs(3);
    while s.busy() {
        s.poll();
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(1));
    }
    let Screen::Diff(a) = &mut s.views[0] else {
        panic!()
    };
    let _before_doc = a.session.document_arc().unwrap();
    key(&mut s, KeyCode::Char('n'));
    s.handle(Event::Key(KeyEvent::new(
        KeyCode::Right,
        KeyModifiers::ALT | KeyModifiers::SHIFT,
    )));
    s.poll();s.handle(Event::Key(KeyEvent::new(KeyCode::Char('s'),KeyModifiers::CONTROL)));
    let Screen::Diff(a) = &s.views[0] else {
        panic!()
    };
    assert_eq!(
        std::fs::read(d.path().join("right")).unwrap(),
        b"old\n",
        "change={:?}, notice={:?}, disabled={:?}",
        a.session.view().current_change(),
        a.session.notice(),
        a.session
            .edit_disabled(lenz::edit_session::EditMenuCommand::ApplyRight)
    );
    let Screen::Diff(a) = &s.views[0] else {
        panic!()
    };
    let doc = a.session.document_arc().unwrap();
    for e in lenz::theme::builtin_entries() {
        choose(&mut s, &e.id);
        let Screen::Diff(a) = &s.views[0] else {
            panic!()
        };
        assert!(std::sync::Arc::ptr_eq(
            &doc,
            &a.session.document_arc().unwrap()
        ));
    }
    let req = cli::parse(
        &["--no-index".into(), "left".into(), "right".into()],
        d.path(),
    )
    .unwrap();
    s.views
        .push(Screen::Diff(Box::new(app::App::open(req, d.path().into()))));
    key(&mut s, KeyCode::F(8));
    let mut t = Terminal::new(TestBackend::new(80, 24)).unwrap();
    t.draw(|f| s.render(f)).unwrap();
    assert_eq!(
        t.backend().buffer()[(0, 0)].bg,
        chrome::color(s.themes.snapshot().palette.header_bg)
    );
    key(&mut s, KeyCode::F(8));
    s.handle(Event::Key(KeyEvent::new(
        KeyCode::Char('z'),
        KeyModifiers::CONTROL,
    )));
    s.handle(Event::Key(KeyEvent::new(KeyCode::Char('s'),KeyModifiers::CONTROL)));
    assert_eq!(std::fs::read(d.path().join("right")).unwrap(), b"new\n");
    choose(&mut s, "catppuccin-latte");
    std::fs::write(d.path().join("right"), "external\n").unwrap();
    key(&mut s, KeyCode::F(5));
    let until = Instant::now() + Duration::from_secs(3);
    loop {
        s.poll();
        if let Screen::Diff(a) = &s.views[0]
            && a.session
                .document_arc()
                .is_some_and(|doc| doc.text(lenz::view::Side::Right) == "external\n")
        {
            break;
        }
        assert!(Instant::now() < until, "terminal reload did not finish");
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(
        std::fs::read(d.path().join("right")).unwrap(),
        b"external\n"
    );
    assert_eq!(s.themes.snapshot().palette.bg, 0xeff1f5);
}

#[test]
fn terminal_theme_merge_choices_survive_changes_and_questions_block_toolbar() {
    let (d, mut s) = fixture();
    for (name, bytes) in [
        ("base", b"base\n".as_slice()),
        ("ours", b"ours\n"),
        ("theirs", b"theirs\n"),
        ("result", b"ours\n"),
    ] {
        std::fs::write(d.path().join(name), bytes).unwrap();
    }
    let req = cli::parse(
        &[
            "merge".into(),
            "base".into(),
            "ours".into(),
            "theirs".into(),
            "result".into(),
        ],
        d.path(),
    )
    .unwrap();
    s.views[0] = Screen::Merge {
        controller: Box::new(merge::Merger::open(req, d.path().into())),
        parent: None,
    };
    let until = Instant::now() + Duration::from_secs(3);
    while s.busy() {
        s.poll();
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(1));
    }
    let mut t = Terminal::new(TestBackend::new(160, 30)).unwrap();
    t.draw(|f| s.render(f)).unwrap();
    key(&mut s, KeyCode::Char('n'));
    key(&mut s, KeyCode::Char('2'));
    key(&mut s, KeyCode::Char(':'));
    for c in "change theme".chars() { key(&mut s, KeyCode::Char(c)); }
    key(&mut s, KeyCode::Enter);
    assert!(s.chooser.is_some());
    key(&mut s, KeyCode::End);
    key(&mut s, KeyCode::Enter);
    let Screen::Merge { controller, .. } = &mut s.views[0] else {
        panic!()
    };
    let Some(lenz::conflict_session::FileView::Merge(m)) = &mut controller.workspace.file else {
        panic!()
    };
    assert!(
        m.dirty,
        "current={:?}, ask={:?}, notice={:?}",
        m.current, m.ask, controller.workspace.notice
    );
    m.ask = Some(lenz::merge_session::Ask::ChangedOnDisk);
    key(&mut s, KeyCode::Char(':'));
    for _ in 0..100 {
        key(&mut s, KeyCode::Down);
    }
    key(&mut s, KeyCode::Up);
    key(&mut s, KeyCode::Enter);
    assert!(s.chooser.is_none());
    assert_eq!(std::fs::read(d.path().join("result")).unwrap(), b"ours\n");
}

#[test]
fn terminal_theme_buffers_cover_all_presets_and_narrow_layouts() {
    let (_d, mut s) = fixture();
    for e in lenz::theme::builtin_entries() {
        choose(&mut s, &e.id);
        for (w, h) in [(1, 1), (12, 4), (40, 12), (120, 36)] {
            let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
            t.draw(|f| s.render(f)).unwrap();
            assert_eq!(
                t.backend().buffer()[(0, 0)].bg,
                chrome::color(e.palette.header_bg)
            );
            if let Some(path) = std::env::var_os("LENZ_THEME_ARTIFACTS") {
                let path = std::path::PathBuf::from(path);
                std::fs::create_dir_all(&path).unwrap();
                let buf = t.backend().buffer();
                let rows = (0..h)
                    .map(|y| (0..w).map(|x| buf[(x, y)].symbol()).collect::<String>())
                    .collect::<Vec<_>>()
                    .join("\n");
                let colors = buf
                    .content
                    .iter()
                    .map(|c| format!("{:?}/{:?}", c.fg, c.bg))
                    .collect::<Vec<_>>()
                    .join(" ");
                std::fs::write(
                    path.join(format!("{}-{w}x{h}.txt", e.id)),
                    format!("{rows}\n\ncell foreground/background:\n{colors}\n"),
                )
                .unwrap();
            }
        }
    }
}

#[test]
#[ignore = "60-second theme chooser, resize and cache stress"]
fn terminal_theme_sustained_render_stress() {
    let (d, mut s) = fixture();
    let req = cli::parse(
        &["--no-index".into(), "left".into(), "right".into()],
        d.path(),
    )
    .unwrap();
    s.views
        .push(Screen::Diff(Box::new(app::App::open(req, d.path().into()))));
    let mut t = Terminal::new(TestBackend::new(120, 36)).unwrap();
    let entries = lenz::theme::builtin_entries();
    let start = Instant::now();
    let seconds = std::env::var("LENZ_THEME_STRESS_SECONDS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(60);
    let mut cycles = 0;
    let mut timings = Vec::new();
    let mut memory = Vec::new();
    let mut last = 0;
    while start.elapsed() < Duration::from_secs(seconds) {
        let e = &entries[cycles % entries.len()];
        let at = Instant::now();
        s.themes
            .execute(lenz::theme::ThemeCommand::Select(ThemeSelection::Named(
                e.id.clone(),
            )));
        s.open_themes();
        key(&mut s, KeyCode::End);
        key(&mut s, KeyCode::Up);
        t.draw(|f| s.render(f)).unwrap();
        key(&mut s, KeyCode::Esc);
        key(&mut s, KeyCode::F(8));
        let (w, h) = if cycles % 2 == 0 { (40, 12) } else { (120, 36) };
        t.backend_mut().resize(w, h);
        t.resize(ratatui::layout::Rect::new(0, 0, w, h)).unwrap();
        s.poll();
        t.draw(|f| s.render(f)).unwrap();
        assert_eq!(
            t.backend().buffer()[(0, 0)].bg,
            chrome::color(e.palette.header_bg)
        );
        if timings.len() < 10000 {
            timings.push(at.elapsed().as_secs_f64() * 1000.0);
        }
        let elapsed = start.elapsed().as_secs();
        if elapsed > last {
            last = elapsed;
            if let Some(rss) = perf::rss_mib() {
                memory.push(rss);
            }
        }
        cycles += 1;
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(cycles > 0);
    assert_eq!(std::fs::read(d.path().join("right")).unwrap(), b"new\n");
    perf::record("tui/themes/select-chooser-resize-render", "ms", &timings);
    perf::record("tui/themes/resident-timeline", "MiB", &memory);
    perf::record(
        "tui/themes/stress-duration",
        "s",
        &[start.elapsed().as_secs_f64()],
    );
    perf::record("tui/themes/cycles", "count", &[cycles as f64]);
}
#[test]
fn terminal_theme_picker_previews_as_you_move() {
    let (_d, mut s) = fixture();
    let mut t = Terminal::new(TestBackend::new(120, 24)).unwrap();
    let shown = |s: &mut Screens, t: &mut Terminal<TestBackend>| { t.draw(|f| s.render(f)).unwrap(); match &s.views[s.active] { Screen::Diff(a) => a.palette.clone(), _ => unreachable!() } };
    let original = shown(&mut s, &mut t);
    s.open_themes();
    for _ in 0..200 { while let Ok(update) = s.themes.updates().try_recv() { s.themes.apply(update); } if s.themes.entries().len() > 1 { break; } std::thread::sleep(std::time::Duration::from_millis(5)); }
    if let Some(c) = &mut s.chooser { c.refresh(&s.themes); }
    let mut now = original.clone();
    for _ in 0..40 { key(&mut s, KeyCode::Down); now = shown(&mut s, &mut t); if *now != *original { break; } }
    assert!(*now != *original, "moving through the picker shows each theme");
    key(&mut s, KeyCode::Esc);
    assert_eq!(*shown(&mut s, &mut t), *original, "Esc puts the theme back");
}
#[test]
fn terminal_theme_picker_closes_on_a_click_outside() {
    let (_d, mut s) = fixture();
    let mut t = Terminal::new(TestBackend::new(120, 30)).unwrap();
    s.open_themes();
    t.draw(|f| s.render(f)).unwrap();
    s.handle(Event::Mouse(event::MouseEvent { kind: event::MouseEventKind::Down(event::MouseButton::Left), column: 0, row: 29, modifiers: KeyModifiers::NONE }));
    assert!(s.chooser.is_none(), "a click outside closes the picker");
}
