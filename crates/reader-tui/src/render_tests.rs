use super::*;

#[test]
fn terminal_cells_preserve_source_bytes_through_tabs_wide_and_combining_text() {
    let shown = reader_core::shown::shown_line("\té界e\u{301}", &[]);
    let cells = source_cells(&shown, 0..shown.text.len(), 0, 20);
    assert_eq!(
        cells
            .iter()
            .map(|c| (c.x, c.width, c.byte))
            .collect::<Vec<_>>(),
        vec![
            (0, 1, 0),
            (1, 1, 0),
            (2, 1, 0),
            (3, 1, 0),
            (4, 1, 1),
            (5, 2, 3),
            (7, 1, 6)
        ]
    );
}

#[test]
fn horizontal_clipping_never_draws_half_a_wide_character() {
    let shown = reader_core::shown::shown_line("a界b", &[]);
    let cells = source_cells(&shown, 0..shown.text.len(), 2, 2);
    assert_eq!(
        cells.iter().map(|c| (c.x, c.text)).collect::<Vec<_>>(),
        vec![(1, "b")]
    );
}

fn app(dir: &std::path::Path, dark: bool) -> App {
    use lenz_content::theme::{ThemeOverrides, ThemePaths};
    let providers = dir.join("providers.json");
    let settings: serde_json::Map<String, serde_json::Value> =
        reader_core::semantics::provider::PROVIDERS
            .iter()
            .map(|p| {
                (
                    p.id.into(),
                    serde_json::json!({"executable": format!("/nonexistent/{}", p.id)}),
                )
            })
            .collect();
    std::fs::write(
        &providers,
        serde_json::json!({"providers":settings}).to_string(),
    )
    .unwrap();
    App::new(
        reader_core::session::Config {
            themes: ThemePaths::under(&dir.join("config"), &dir.join("state"), "lenz-reader"),
            recent: dir.join("recent.json"),
            settings: dir.join("settings.json"),
            providers,
            cache: dir.join("cache"),
            idle: std::time::Duration::ZERO,
            clock: std::sync::Arc::new(std::time::Instant::now),
            overrides: ThemeOverrides::default(),
            dark,
        },
        None,
    )
}

fn draw(app: &mut App, width: u16, height: u16) -> Buffer {
    let mut terminal =
        ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    terminal.backend().buffer().clone()
}

fn text(buffer: &Buffer) -> String {
    buffer.content.iter().map(|cell| cell.symbol()).collect()
}

fn settle(app: &mut App, done: impl Fn(&App) -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !done(app) {
        assert!(
            std::time::Instant::now() < deadline,
            "reader did not settle"
        );
        if let Ok(c) = app.session.completions().try_recv() {
            app.apply(c);
        } else {
            std::thread::yield_now();
        }
    }
}

fn open(app: &mut App, dir: &std::path::Path, name: &str, bytes: &[u8]) {
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    app.dispatch(Command::Open(path));
    settle(app, |a| a.session.state().current_source().is_some());
}

#[test]
fn indexing_percentage_is_visible_in_the_footer_and_status_dialog() {
    let profile = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_owned();
    let mut build =
        std::process::Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    build.args(["build", "-q", "-p", "reader-core", "--example", "fake_lsp"]);
    if profile.ends_with("release") {
        build.arg("--release");
    }
    assert!(build.status().unwrap().success());
    let fake = profile.join(format!("examples/fake_lsp{}", std::env::consts::EXE_SUFFIX));
    for dark in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        std::fs::write(
            base.join("providers.json"),
            serde_json::json!({"providers":{"rust-analyzer":{"executable":fake}}}).to_string(),
        )
        .unwrap();
        let mut app = App::new(
            reader_core::session::Config {
                themes: lenz_content::theme::ThemePaths::under(
                    &base.join("config"),
                    &base.join("state"),
                    "lenz-reader",
                ),
                recent: base.join("recent.json"),
                settings: base.join("settings.json"),
                providers: base.join("providers.json"),
                cache: base.join("cache"),
                idle: reader_core::semantics::IDLE,
                clock: std::sync::Arc::new(std::time::Instant::now),
                overrides: Default::default(),
                dark,
            },
            None,
        );
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname='progress'\nversion='0.1.0'\n",
        )
        .unwrap();
        open(&mut app, dir.path(), "main.rs", b"fn main() {}\n");
        settle(&mut app, |a| {
            a.session
                .state()
                .provider()
                .is_some_and(|p| p.status == reader_core::semantics::Status::Ready)
        });
        app.apply(reader_core::session::Completion::Semantic(reader_core::semantics::Event::Message {
                session: 1,
                message: serde_json::json!({"jsonrpc":"2.0", "method":"$/progress", "params":{"token":"ruby-index", "value":{"kind":"begin", "title":"Indexing Ruby files", "percentage":42}}}),
            }));
        for (width, height) in [(100, 30), (60, 15)] {
            assert!(text(&draw(&mut app, width, height)).contains("42%"));
            app.handle(crossterm::event::Event::Key(
                crossterm::event::KeyEvent::new(
                    crossterm::event::KeyCode::F(2),
                    crossterm::event::KeyModifiers::NONE,
                ),
            ));
            app.handle(crossterm::event::Event::Key(crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Enter, crossterm::event::KeyModifiers::NONE,
            )));
            assert!(text(&draw(&mut app, width, height)).contains("42%"));
            app.handle(crossterm::event::Event::Key(
                crossterm::event::KeyEvent::new(
                    crossterm::event::KeyCode::Esc,
                    crossterm::event::KeyModifiers::NONE,
                ),
            ));
        }
        // An error it logs: marked first in the footer (kept when narrow), its words in the details.
        app.apply(reader_core::session::Completion::Semantic(reader_core::semantics::Event::Message {
                session: 1,
                message: serde_json::json!({"jsonrpc":"2.0", "method":"window/logMessage", "params":{"type":1, "message":"Error processing initialized: (Gem::ConflictError)"}}),
            }));
        for (width, height) in [(100, 30), (60, 15)] {
            assert!(text(&draw(&mut app, width, height)).contains("LSP error"));
            app.handle(crossterm::event::Event::Key(crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::F(2), crossterm::event::KeyModifiers::NONE,
            )));
            app.handle(crossterm::event::Event::Key(crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Enter, crossterm::event::KeyModifiers::NONE,
            )));
            let shown = text(&draw(&mut app, width, height));
            assert!(shown.contains("Server error: Error processing"), "{shown}");
            app.handle(crossterm::event::Event::Key(crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Esc, crossterm::event::KeyModifiers::NONE,
            )));
        }
        assert_eq!(
            std::fs::read(dir.path().join("main.rs")).unwrap(),
            b"fn main() {}\n"
        );
    }
}

#[test]
fn nerd_reader_icons_preserve_tab_mouse_targets_and_source_bytes() {
    use crossterm::event::{Event, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    for dark in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path(), dark);
        open(&mut app, dir.path(), "sample.rs", b"fn main() {}\n");
        app.dispatch(Command::OpenRoot(dir.path().to_owned()));
        settle(&mut app, |a| {
            a.session
                .state()
                .project()
                .is_some_and(|p| p.rows().iter().any(|r| r.name.as_ref() == "sample.rs"))
        });
        app.icons_override = Some(true);
        let buffer = draw(&mut app, 100, 24);
        assert!(text(&buffer).contains("\u{e7a8} sample.rs"));
        let glyphs: Vec<_> = buffer
            .content
            .iter()
            .filter(|c| c.symbol() == "\u{e7a8}")
            .collect();
        assert!(
            glyphs.len() >= 2,
            "Rust icons appear in both explorer and tab"
        );
        assert!(
            glyphs
                .iter()
                .all(|c| c.fg != color(app.session.state().palette().fg)),
            "file icons keep their own color"
        );
        assert!(text(&buffer).contains("Menu"));
        assert!(text(&buffer).contains("F10 Commands"));
        let (id, tab) = app.layout.panes[0].tabs[0];
        app.handle(Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: tab.x + 1,
            row: tab.y,
            modifiers: KeyModifiers::NONE,
        }));
        assert_eq!(app.session.state().active().unwrap().id(), id);
        for (w, h) in [(48, 12), (18, 5), (1, 1), (0, 0)] {
            draw(&mut app, w, h);
        }
        app.icons_override = Some(false);
        let buffer = draw(&mut app, 100, 24);
        assert!(!text(&buffer).contains('\u{e7a8}'));
        assert!(text(&buffer).contains("sample.rs"));
        // Without LENZ_ICONS the File Icons setting decides: on by default.
        app.icons_override = None;
        assert!(text(&draw(&mut app, 100, 24)).contains("\u{e7a8} sample.rs"));
        // The View menu's File Icons turns them off (remembered), and on again; it replaces LENZ_ICONS.
        app.run(CommandId::FileIcons);
        assert!(!text(&draw(&mut app, 100, 24)).contains('\u{e7a8}'));
        assert!(!app.session.state().file_icons());
        app.icons_override = Some(false);
        app.run(CommandId::FileIcons);
        assert_eq!((app.icons_override, app.session.state().file_icons()), (None, true));
        assert!(text(&draw(&mut app, 100, 24)).contains("\u{e7a8} sample.rs"));
        assert_eq!(
            std::fs::read(dir.path().join("sample.rs")).unwrap(),
            b"fn main() {}\n"
        );
    }
}

#[test]
fn focused_reader_tab_has_a_visible_accent_in_light_and_dark() {
    for dark in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path(), dark);
        open(&mut app, dir.path(), "sample.rs", b"fn main() {}\n");
        let buffer = draw(&mut app, 90, 20);
        let tab = app.layout.panes[0].tabs[0].1;
        let p = app.session.state().palette();
        assert_eq!(buffer[(tab.x + 1, tab.y)].fg, color(p.accent));
        assert!(
            buffer[(tab.x + 1, tab.y)]
                .modifier
                .contains(Modifier::UNDERLINED)
        );
        app.focus = Focus::Tree;
        let buffer = draw(&mut app, 90, 20);
        assert!(
            !buffer[(tab.x + 1, tab.y)]
                .modifier
                .contains(Modifier::UNDERLINED)
        );
    }
}

#[test]
fn actual_empty_constructor_renders_dark_light_narrow_and_zero_and_closes_scrolled_help() {
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
    for dark in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path(), dark);
        for (width, height) in [(100, 30), (18, 7), (1, 1), (0, 0)] {
            draw(&mut app, width, height);
        }
        app.handle(Event::Key(KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE)));
        assert!(matches!(app.modal.as_ref().unwrap().kind, Prompt::Help));
        let help = text(&draw(&mut app, 100, 40));
        assert!(help.contains("Ctrl+S") && help.contains("Ctrl+Z") && help.contains("bracketed paste"));
        draw(&mut app, 24, 9);
        app.handle(Event::Key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE)));
        draw(&mut app, 24, 9);
        let end = app.modal.as_ref().unwrap().top;
        app.handle(Event::Key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)));
        draw(&mut app, 24, 9);
        assert!(app.modal.as_ref().unwrap().top <= end);
        app.handle(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
        assert!(app.modal.is_none());
        assert!(app.session.state().tabs().is_empty());
    }
}

#[test]
fn theme_chooser_shows_current_source_help_and_wrapped_choices_in_both_appearances() {
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
    for dark in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let mut a = app(dir.path(), dark);
        a.run(CommandId::ChooseTheme);
        let current = a.session.theme_source();
        let buffer = draw(&mut a, 100, 26);
        let shown = text(&buffer);
        for expected in [
            "Theme",
            "↑/↓ choose · Enter select · Esc close",
            "Follow system",
            &format!("Current: {current}"),
        ] {
            assert!(shown.contains(expected), "missing {expected}: {shown}");
        }
        assert_eq!(&a.layout.modal_items[..2], &[usize::MAX, usize::MAX]);
        assert_eq!(a.layout.modal.width, 70);
        for (width, height) in [(28, 18), (12, 8), (4, 4), (1, 1), (0, 0), (28, 18)] {
            draw(&mut a, width, height);
            a.handle(Event::Key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE)));
            draw(&mut a, width, height);
            let end = a.modal.as_ref().unwrap().at;
            a.handle(Event::Key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)));
            draw(&mut a, width, height);
            assert_eq!(a.modal.as_ref().unwrap().at, end - 1);
            assert_eq!(a.session.theme_source(), current);
            if a.layout.modal_rows.height > 0 {
                assert!(
                    a.layout.modal_items.contains(&(end - 1)),
                    "selected wrapped choice is visible"
                );
            }
        }
        a.handle(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
        assert!(a.modal.is_none());
    }
}

#[test]
fn source_renders_find_selection_unicode_and_independent_splits() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app(dir.path(), true);
    open(
        &mut app,
        dir.path(),
        "sample.rs",
        "fn main() {\n\tlet é界 = 3;\n}\n".as_bytes(),
    );
    settle(&mut app, |a| a.session.state().highlights().is_some());
    app.dispatch(Command::Find("é界".into()));
    settle(&mut app, |a| {
        a.session
            .state()
            .active()
            .and_then(|t| t.find())
            .is_some_and(|f| !f.pending())
    });
    let buffer = draw(&mut app, 90, 20);
    assert!(text(&buffer).contains("é界"));
    assert!(
        buffer
            .content
            .iter()
            .any(|c| c.bg == color(app.session.state().palette().sel_bg))
    );
    let pane = &app.layout.panes[0];
    let hit = pane
        .hits
        .iter()
        .find(|h| h.line == 1 && h.byte == 7)
        .unwrap();
    assert_eq!(hit.width, 2);
    let tab = app.session.state().active().unwrap();
    let location = (
        tab.source().to_owned(),
        tab.top_line(),
        tab.column(),
        tab.selection(),
    );
    app.dispatch(Command::Split(Split::Columns));
    draw(&mut app, 90, 20);
    assert_eq!(app.layout.panes.len(), 2);
    app.dispatch(Command::Scroll(2));
    let tab = app.session.state().shown(0).unwrap();
    assert_eq!(
        (
            tab.source().to_owned(),
            tab.top_line(),
            tab.column(),
            tab.selection()
        ),
        location
    );
    app.dispatch(Command::Split(Split::Rows));
    draw(&mut app, 22, 8);
    assert_eq!(app.layout.panes.len(), 2);
}

#[test]
fn source_folds_and_sticky_rows_keep_source_hits() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app(dir.path(), false);
    open(
        &mut app,
        dir.path(),
        "fold.rs",
        b"fn main() {\n  let x = 1;\n  let y = 2;\n}\n",
    );
    settle(&mut app, |a| {
        a.session
            .state()
            .active()
            .is_some_and(|t| a.session.state().view(t.id()).is_some_and(|view| view.folds().is_some()))
    });
    app.dispatch(Command::ToggleFold(0));
    let buffer = draw(&mut app, 70, 16);
    assert!(text(&buffer).contains('▸'));
    assert!(
        !app.layout.panes[0]
            .rows
            .iter()
            .any(|(_, line, _)| *line == 1)
    );
    app.dispatch(Command::ExpandAll);
    app.dispatch(Command::SetStickyScroll(true));
    settle(&mut app, |a| {
        a.session
            .state()
            .active()
            .is_some_and(|t| a.session.state().view(t.id()).is_some_and(|view| view.declarations().is_some()))
    });
    app.dispatch(Command::Scroll(1));
    draw(&mut app, 70, 16);
    assert!(
        app.layout.panes[0]
            .rows
            .iter()
            .any(|(_, line, _)| *line == 0)
    );
}

#[test]
fn raster_image_and_markdown_preview_use_shared_decoded_content() {
    let dir = tempfile::tempdir().unwrap();
    let image_path = dir.path().join("tiny.png");
    image::RgbaImage::from_pixel(4, 4, image::Rgba([255, 0, 0, 255]))
        .save(&image_path)
        .unwrap();
    let mut app = app(dir.path(), true);
    app.dispatch(Command::Open(image_path));
    settle(&mut app, |a| a.session.state().current_source().is_some());
    assert!(text(&draw(&mut app, 50, 15)).contains('▀'));
    app.dispatch(Command::Close);
    open(&mut app, dir.path(), "tiny.svg", br##"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><rect width="8" height="8" fill="#ff0000"/></svg>"##);
    assert!(text(&draw(&mut app, 50, 15)).contains('▀'));
    app.dispatch(Command::Close);
    open(
        &mut app,
        dir.path(),
        "notes.md",
        b"# Heading\n\nSome **strong** and `code`.\n\n![tiny](tiny.png)\n",
    );
    app.dispatch(Command::ToggleMarkdownSource);
    settle(
        &mut app,
        |a| matches!(a.session.state().rendering(), Some(Rendering::Preview(preview)) if preview.images.is_some()),
    );
    let buffer = draw(&mut app, 50, 15);
    assert!(text(&buffer).contains("Heading"));
    assert!(
        buffer
            .content
            .iter()
            .any(|c| c.symbol() == "H" && c.modifier.contains(Modifier::BOLD))
    );
    assert!(text(&buffer).contains("Some strong and code."));
    assert!(
        text(&buffer).contains('▀'),
        "local Markdown image uses decoded core pixels"
    );
    assert!(app.layout.panes[0].preview);
    let hit = app.layout.panes[0]
        .hits
        .iter()
        .find(|h| h.byte == 0)
        .unwrap();
    assert_eq!(buffer[(hit.x, hit.y)].symbol(), "H");
    let tab = app.session.state().active().unwrap();
    app.preview_selection = Some((
        tab.id(),
        crate::app::PreviewSource::new(&app.session.state().view(tab.id()).unwrap()),
        reader_core::session::Selection { anchor: 0, head: 7 },
    ));
    let selected = draw(&mut app, 50, 15);
    assert!(
        selected
            .content
            .iter()
            .any(|c| c.symbol() == "H" && c.bg == color(app.session.state().palette().sel_bg))
    );
}

#[test]
fn many_long_tab_titles_keep_the_active_tab_visible() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app(dir.path(), true);
    for n in 0..9 {
        let path = dir
            .path()
            .join(format!("very-long-source-file-title-{n}.txt"));
        std::fs::write(&path, n.to_string()).unwrap();
        app.dispatch(Command::OpenRetained(path));
        settle(&mut app, |a| a.session.state().loading().is_none());
    }
    draw(&mut app, 35, 9);
    let id = app.session.state().active().unwrap().id();
    assert!(
        app.layout.panes[0]
            .tabs
            .iter()
            .any(|(tab, rect)| *tab == id && rect.width > 0)
    );
}

#[test]
fn markdown_rows_are_cached_scroll_to_code_tail_and_resize_at_the_same_text() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app(dir.path(), true);
    let code = (0..300)
        .map(|i| format!("let value_{i} = {i};\n"))
        .collect::<String>();
    let source = format!(
        "# Header\n\n{}\n\n```rust\n{code}```\n\nTAIL_SENTINEL\n",
        "a paragraph word ".repeat(300)
    );
    open(&mut app, dir.path(), "long.md", source.as_bytes());
    app.dispatch(Command::ToggleMarkdownSource);
    settle(&mut app, |a| {
        matches!(a.session.state().rendering(), Some(Rendering::Preview(_)))
    });
    draw(&mut app, 72, 18);
    let cached = app.layout.panes[0].markdown.clone().unwrap();
    app.dispatch(Command::ScrollRows(40));
    draw(&mut app, 72, 18);
    assert!(std::sync::Arc::ptr_eq(
        &cached,
        app.layout.panes[0].markdown.as_ref().unwrap()
    ));
    let before = app.layout.panes[0].hits.first().unwrap().byte;
    draw(&mut app, 34, 14);
    let after = app.layout.panes[0].hits.first().unwrap().byte;
    assert!(
        after <= before && before - after < 40,
        "resize retained rendered text anchor: {before} -> {after}"
    );
    app.dispatch(Command::Scroll(usize::MAX));
    let tail = draw(&mut app, 34, 14);
    assert!(text(&tail).contains("TAIL_SENTINEL"));
    assert!(
        app.session.state().top_line() > 250,
        "rendered scrolling reports its source line to core"
    );
}

#[test]
fn tall_local_markdown_images_keep_layout_metadata_bounded_on_resize() {
    use std::sync::Arc;
    let source: Arc<str> = "![tall](tall.png)\n".into();
    let doc = lenz_content::markdown::parse(
        source.clone(),
        lenz_content::markdown::Limits::DEFAULT,
        &mut 1000,
        &|| true,
    )
    .unwrap();
    let preview = Arc::new(reader_core::markdown::Preview {
        doc: Arc::new(doc),
        syntax: Arc::default(),
        images: Some(std::collections::HashMap::from([(
            0,
            Ok(Arc::new(
                lenz_content::image::Pixels::from_raw(1, 8192, vec![255; 8192 * 4]).unwrap(),
            )),
        )])),
    });
    let source_doc =
        reader_core::document::Document::decode("tall.md".into(), 1, source.as_bytes());
    let source_snapshot = reader_core::text::TextSnapshot::from_document(reader_core::text::BufferId(1), &source_doc);
    for width in [160, 18, 1000, 1, 160] {
        let layout = MarkdownLayout::new(preview.clone(), width, &source_snapshot);
        assert!(
            layout.rows.len() <= 21,
            "image layout rows: {}",
            layout.rows.len()
        );
        assert_eq!(layout.source_lines.len(), layout.rows.len());
    }
}

#[test]
fn navigation_popup_shows_scopes_types_details_and_toggle_state_in_both_appearances() {
    for dark in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let mut a = app(dir.path(), dark);
        a.run(CommandId::FindAction);
        a.handle(crossterm::event::Event::Paste("soft wrap".into()));
        settle(&mut a, |a| {
            a.session.state().navigation().is_some_and(|s| !s.pending)
        });
        let buffer = draw(&mut a, 100, 26);
        let shown = text(&buffer);
        for expected in [
            "Search Everywhere",
            "All",
            "Files",
            "Symbols",
            "Actions",
            "Soft Wrap",
            "Action",
        ] {
            assert!(shown.contains(expected), "missing {expected}: {shown}");
        }
        assert!(shown.contains('✓') || shown.contains('○'));
        let revision = a.session.state().navigation().unwrap().revision;
        assert_eq!(a.layout.navigation_revision, Some(revision));
        assert!(!a.layout.modal_items.is_empty());
        for (width, height) in [(42, 12), (20, 8), (4, 4), (1, 1), (0, 0), (100, 26)] {
            a.handle(crossterm::event::Event::Resize(width, height));
            draw(&mut a, width, height);
        }
    }
}

#[test]
fn navigation_empty_popup_scroll_and_resize_are_clamped() {
    let dir = tempfile::tempdir().unwrap();
    let mut a = app(dir.path(), true);
    a.run(CommandId::FindAction);
    settle(&mut a, |a| {
        a.session.state().navigation().is_some_and(|s| !s.pending)
    });
    let key = |a: &mut App, code| {
        a.handle(crossterm::event::Event::Key(
            crossterm::event::KeyEvent::new(code, crossterm::event::KeyModifiers::NONE),
        ));
    };
    use crossterm::event::KeyCode;
    draw(&mut a, 30, 10);
    key(&mut a, KeyCode::End);
    draw(&mut a, 30, 10);
    let last = a.modal.as_ref().unwrap().at;
    key(&mut a, KeyCode::Up);
    draw(&mut a, 20, 8);
    assert_eq!(a.modal.as_ref().unwrap().at, last.saturating_sub(1));
    a.handle(crossterm::event::Event::Paste("xxxxxxxxxxnoresult".into()));
    settle(&mut a, |a| {
        a.session.state().navigation().is_some_and(|s| !s.pending)
    });
    let shown = text(&draw(&mut a, 100, 26));
    assert!(shown.contains("No matching results"));
    assert_eq!(a.modal.as_ref().unwrap().at, 0);
    key(&mut a, KeyCode::Enter);
    assert!(a.modal.is_some());
    key(&mut a, KeyCode::Esc);
    assert!(a.modal.is_none());
    assert!(a.session.state().navigation().is_none());
}

#[test]
fn navigation_files_show_distinct_paths_and_disabled_actions_explain_why() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("src")).unwrap();
    std::fs::create_dir(dir.path().join("tests")).unwrap();
    for path in ["src/NetworkParserEngine.rs", "tests/NetworkParserEngine.rs"] {
        std::fs::write(dir.path().join(path), "// read only\n").unwrap();
    }
    let mut a = app(dir.path(), true);
    a.dispatch(Command::OpenRoot(dir.path().to_owned()));
    settle(&mut a, |a| {
        a.session
            .state()
            .project()
            .is_some_and(|p| matches!(p.coverage(), reader_core::project::Coverage::Complete(_)))
    });
    a.run(CommandId::FindFile);
    a.handle(crossterm::event::Event::Paste("NPE".into()));
    settle(&mut a, |a| {
        a.session.state().navigation().is_some_and(|s| !s.pending)
    });
    let shown = text(&draw(&mut a, 100, 26));
    for path in [
        "NetworkParserEngine.rs",
        "src/NetworkParserEngine.rs",
        "tests/NetworkParserEngine.rs",
        "File",
    ] {
        assert!(shown.contains(path), "missing {path}: {shown}");
    }
    a.run(CommandId::FindAction);
    settle(&mut a, |a| {
        a.session.state().navigation().is_some_and(|s| !s.pending)
    });
    let (index, reason) = a
        .session
        .state()
        .navigation()
        .unwrap()
        .rows
        .iter()
        .enumerate()
        .find_map(|(index, row)| row.disabled.map(|why| (index, why)))
        .expect("an empty reader has disabled actions");
    a.modal.as_mut().unwrap().at = index;
    let shown = text(&draw(&mut a, 100, 26));
    assert!(shown.contains(reason));
    a.handle(crossterm::event::Event::Key(
        crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        ),
    ));
    assert!(a.modal.is_some());
    assert!(a.session.state().current_source().is_none());
}

#[test]
fn navigation_long_unicode_query_keeps_typed_tail_and_caret_visible_in_tiny_frames() {
    let dir = tempfile::tempdir().unwrap();
    let mut a = app(dir.path(), true);
    a.run(CommandId::SearchEverywhere);
    let query = format!("{}α界e\u{301}終Z", "obsolete-prefix-".repeat(20));
    a.handle(crossterm::event::Event::Paste(query.clone()));
    for (width, tail) in [(12, "e\u{301}終Z"), (10, "終Z"), (8, "Z")] {
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, 8)).unwrap();
        terminal.draw(|frame| a.draw(frame)).unwrap();
        let content = inner(a.layout.modal);
        let y = content.y + 1;
        let mut x = content.x + 2;
        for grapheme in tail.graphemes(true) {
            assert_eq!(terminal.backend().buffer()[(x, y)].symbol(), grapheme);
            x += grapheme.width() as u16;
        }
        assert_eq!(terminal.get_cursor_position().unwrap(), (x, y).into());
        assert!(
            x < content.right(),
            "caret must remain inside the query input"
        );
        assert_eq!(a.modal.as_ref().unwrap().input, query);
    }
}

#[test]
fn recent_files_popup_shows_one_heading_file_names_paths_and_query_tail_in_both_appearances() {
    for dark in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let mut a = app(dir.path(), dark);
        for folder in ["first", "second"] {
            std::fs::create_dir_all(dir.path().join(folder)).unwrap();
            open(
                &mut a,
                dir.path(),
                &format!("{folder}/same.txt"),
                b"hello\n",
            );
        }
        a.run(CommandId::RecentFiles);
        settle(&mut a, |a| {
            a.session.state().navigation().is_some_and(|s| !s.pending)
        });
        let shown = text(&draw(&mut a, 100, 26));
        for expected in ["Recent Files", "same.txt", "first", "second", "File"] {
            assert!(shown.contains(expected), "missing {expected}: {shown}");
        }
        assert_eq!(a.layout.navigation_scopes.len(), 1);
        assert_eq!(a.layout.navigation_scopes[0].0, Scope::Recent);
        assert!(!shown.contains("Search Everywhere"));
        assert!(!shown.contains("Symbols"));
        assert!(!shown.contains("Actions"));
        a.handle(crossterm::event::Event::Paste("α界".repeat(40)));
        settle(&mut a, |a| {
            a.session.state().navigation().is_some_and(|s| !s.pending)
        });
        for (w, h) in [(20, 8), (4, 4), (1, 1), (0, 0), (100, 26)] {
            draw(&mut a, w, h);
        }
        a.handle(crossterm::event::Event::Key(
            crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Esc,
                crossterm::event::KeyModifiers::NONE,
            ),
        ));
        assert!(a.modal.is_none());
    }
}

#[test]
fn chrome_reserves_menu_position_commands_and_ellipsizes_unicode_status() {
    for dark in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let mut a = app(dir.path(), dark);
        open(&mut a, dir.path(), "sample.txt", b"hello\n");
        a.independent_view = Some((2, 3));
        a.notice = Some("界e\u{301} diagnostic\n".repeat(30));
        for (width, height) in [(120, 26), (60, 18), (28, 10), (8, 5), (1, 1), (0, 0), (120, 26)] {
            let buffer = draw(&mut a, width, height);
            for (_, rect) in &a.layout.controls {
                assert!(rect.right() <= width);
                assert!(rect.bottom() <= height);
            }
            for rect in a.layout.command_menus.iter().chain([&a.layout.app_menu]) {
                assert!(rect.right() <= width);
                assert!(rect.bottom() <= height);
            }
            if width >= 60 {
                let top: String = (0..width).map(|x| buffer[(x, 0)].symbol()).collect();
                assert!(top.contains("Menu"), "top: {top:?}");
                assert!(top.contains("Find file") && !top.contains("Open"), "top: {top:?}");
                let footer: String = (0..width).map(|x| buffer[(x, height - 1)].symbol()).collect();
                assert!(footer.contains("F10 Commands"));
                assert!(footer.contains("View 2/3"));
                assert!(footer.contains('…'));
                assert!(a.layout.panes[0].tabs.iter().all(|(_, rect)| rect.y == 1));
                let (_, message) = a.layout.controls.iter().find(|(id, _)| *id == CommandId::StatusDetails).unwrap();
                let shown: String = (message.x..message.right()).map(|x| buffer[(x, message.y)].symbol()).collect();
                assert!(shown.contains('界'));
                assert!(shown.contains("e\u{301}"));
                assert!(shown.ends_with('…'));
                let (id, back) = a.layout.controls.iter().find(|(id, _)| *id == CommandId::Back).unwrap();
                assert!(a.session.disabled(*id).is_some());
                assert_eq!(buffer[(back.x, back.y)].fg, color(a.session.state().palette().dim));
            }
        }
    }
}

#[test]
fn search_dialog_shows_labelled_fields_toggles_and_refusal_in_both_appearances_and_tiny_frames() {
    for dark in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let mut a = app(dir.path(), dark);
        a.dispatch(Command::OpenRoot(dir.path().to_owned()));
        settle(&mut a, |a| a.session.state().project().is_some());
        a.run(CommandId::SearchProject);
        a.handle(crossterm::event::Event::Paste("foo".into()));
        a.run(CommandId::MatchCase);
        let shown = text(&draw(&mut a, 100, 30));
        for expected in [
            "Search in Project",
            "Find    foo",
            "Files   All files",
            "✓ Aa Match case",
            "○ W Whole word",
            "○ .* Regular expression",
            "Enter search",
        ] {
            assert!(shown.contains(expected), "missing {expected}: {shown}");
        }
        assert_eq!(a.layout.search_fields.len(), 2);
        assert_eq!(a.layout.search_toggles.len(), 3);
        assert!(a.layout.modal.y > 1, "centered, not at the top: {:?}", a.layout.modal);
        a.run(CommandId::RegularExpression);
        a.handle(crossterm::event::Event::Paste("(".into()));
        let shown = text(&draw(&mut a, 100, 30));
        assert!(shown.contains("unclosed group — fix the expression"), "{shown}");
        for (width, height) in [(44, 16), (30, 10), (4, 4), (1, 1), (0, 0), (100, 30)] {
            a.handle(crossterm::event::Event::Resize(width, height));
            draw(&mut a, width, height);
            let area = Rect::new(0, 0, width, height);
            assert!(area.union(a.layout.modal) == area || a.layout.modal.is_empty(), "{width}x{height}: {:?}", a.layout.modal);
            for r in a.layout.search_fields.iter().chain(a.layout.search_toggles.iter().map(|(_, r)| r)) {
                assert!(a.layout.modal.union(*r) == a.layout.modal || r.is_empty(), "{width}x{height}: {r:?} in {:?}", a.layout.modal);
            }
        }
    }
}

#[test]
fn breadcrumbs_separate_crumbs_shrink_outer_ones_and_keep_the_focused_one_in_both_appearances() {
    use crate::app::{Focus, Prompt};
    use crossterm::event::{Event, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    use reader_core::breadcrumbs::Crumb;
    for dark in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("proj");
        std::fs::create_dir_all(root.join("crates/reader/src/ui")).unwrap();
        let mut a = app(dir.path(), dark);
        a.dispatch(Command::OpenRoot(root.clone()));
        settle(&mut a, |a| a.session.state().project().is_some());
        let source = b"struct S;\nimpl S {\n    fn render_toolbar(&self) {\n        let x = 1;\n    }\n}\n";
        open(&mut a, &root, "crates/reader/src/ui/widgets.rs", source);
        a.dispatch(Command::SetBreadcrumbs(true));
        a.dispatch(Command::GoToLine(lenz_content::goto::LineColumn { line: 4, column: None }));
        settle(&mut a, |a| {
            a.session.state().breadcrumbs_for(0).is_some_and(|b| {
                b.crumbs.first().is_some_and(|c| c.name() == "crates")
                    && b.crumbs.iter().filter(|c| matches!(c, Crumb::Decl { .. })).count() == 2
            })
        });
        let row = |buffer: &Buffer, y: u16| {
            (0..buffer.area.width).map(|x| buffer[(x, y)].symbol()).collect::<String>()
        };
        let buffer = draw(&mut a, 100, 8);
        let y = a.layout.panes[0].crumbs[0].1.y;
        let bar = row(&buffer, y);
        assert!(bar.contains(" crates › reader › src › ui › widgets.rs › impl S › render_toolbar "), "{bar}");
        assert!(!bar.contains("render_toolbar ›"), "no separator after the innermost: {bar}");
        let (_, innermost) = *a.layout.panes[0].crumbs.last().unwrap();
        assert!(buffer[(innermost.x + 1, y)].modifier.contains(Modifier::BOLD));
        // Narrow: folders, then the outer declaration, shrink to "…"; the file and the innermost stay.
        let shown = |a: &App| a.layout.panes[0].crumbs.iter().map(|c| c.0).collect::<Vec<_>>();
        let buffer = draw(&mut a, 60, 8);
        let bar = row(&buffer, y);
        assert!(bar.contains(" … › widgets.rs › impl S › render_toolbar "), "{bar}");
        assert_eq!(shown(&a), vec![4, 5, 6], "{bar}");
        let buffer = draw(&mut a, 52, 8);
        let bar = row(&buffer, y);
        assert!(bar.contains(" … › widgets.rs › … › render_toolbar"), "{bar}");
        assert_eq!(shown(&a), vec![4, 6], "{bar}");
        // A focused crumb stays shown, highlighted.
        a.focus = Focus::Breadcrumbs;
        a.crumb_at = 1;
        let buffer = draw(&mut a, 60, 8);
        let bar = row(&buffer, y);
        assert!(bar.contains(" reader "), "{bar}");
        let (_, r) = *a.layout.panes[0].crumbs.iter().find(|c| c.0 == 1).unwrap();
        assert_eq!(buffer[(r.x + 1, y)].bg, color(a.session.state().palette().selected_bg));
        a.handle(Event::Mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: r.x + 1, row: r.y, modifiers: KeyModifiers::NONE }));
        assert_eq!(a.modal.as_ref().map(|m| &m.kind), Some(&Prompt::Siblings { pane: 0, crumb: 1 }));
        for (width, height) in [(34, 8), (12, 4), (3, 3), (1, 2), (0, 0)] {
            a.handle(Event::Resize(width, height));
            draw(&mut a, width, height);
            for (_, r) in a.layout.panes.iter().flat_map(|p| &p.crumbs) {
                assert!(r.right() <= width && r.bottom() <= height, "{width}x{height}: {r:?}");
            }
        }
        assert_eq!(std::fs::read(root.join("crates/reader/src/ui/widgets.rs")).unwrap(), source);
    }
}
