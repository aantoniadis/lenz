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
    settle(app, |a| a.session.state().document().is_some());
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
            .is_some_and(|t| t.folds().is_some())
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
            .is_some_and(|t| t.declarations().is_some())
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
    settle(&mut app, |a| a.session.state().document().is_some());
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
        tab.version(),
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
    for width in [160, 18, 1000, 1, 160] {
        let layout = MarkdownLayout::new(preview.clone(), width, &source_doc);
        assert!(
            layout.rows.len() <= 21,
            "image layout rows: {}",
            layout.rows.len()
        );
        assert_eq!(layout.source_lines.len(), layout.rows.len());
    }
}
