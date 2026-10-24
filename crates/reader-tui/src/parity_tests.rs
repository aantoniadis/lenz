//! Replay actual keys in both adapters and compare the resulting semantic state and source bytes.
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use gpui::{AppContext, TestAppContext, VisualTestContext};
use reader_core::session::{Command, CommandId, Config, Session, State};
use reader_gui::ReaderView;
use std::{
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

use crate::app::App;

fn copied_selection(state: &reader_core::session::State) -> Option<String> {
    state.view(state.active()?.id())?.copy_selection()
}

fn config(base: &Path) -> Config {
    let providers = base.join("providers.json");
    std::fs::create_dir_all(base).unwrap();
    let providers_json: serde_json::Map<_, _> = reader_core::semantics::provider::PROVIDERS
        .iter()
        .map(|p| {
            (
                p.id.to_owned(),
                serde_json::json!({"executable": format!("/nonexistent/{}", p.id)}),
            )
        })
        .collect();
    std::fs::write(
        &providers,
        serde_json::json!({"providers": providers_json}).to_string(),
    )
    .unwrap();
    Config {
        themes: lenz_content::theme::ThemePaths::under(
            &base.join("config"),
            &base.join("state"),
            "lenz-reader",
        ),
        recent: base.join("state/recent.json"),
        settings: base.join("settings.json"),
        providers,
        cache: base.join("cache"),
        idle: reader_core::semantics::IDLE,
        clock: Arc::new(Instant::now),
        overrides: Default::default(),
        dark: true,
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Snapshot {
    tabs: Vec<(u8, String, bool)>,
    active: Option<String>,
    text: Option<String>,
    selection: Option<reader_core::session::Selection>,
    /// Every selection of the active view, in source order (several cursors included).
    selections: Vec<reader_core::session::Selection>,
    split: Option<reader_core::session::Split>,
    pane: u8,
    selected: Option<String>,
    find: Option<(String, usize, Option<usize>)>,
    wrap: bool,
}

fn snapshot(s: &State) -> Snapshot {
    Snapshot {
        tabs: s
            .tabs()
            .iter()
            .map(|t| (t.pane(), t.title(), t.retained()))
            .collect(),
        active: s.active().map(|t| t.title()),
        text: s.current_source().map(|d| d.copy_range(0..d.len_bytes()).unwrap()),
        selection: s.selection(),
        selections: s.active().map(|tab| {
            let mut all: Vec<_> = tab.selections().collect();
            all.sort_by_key(|selection| (selection.range().start, selection.range().end));
            all
        }).unwrap_or_default(),
        selected: copied_selection(s),
        split: s.split(),
        pane: s.focused_pane(),
        wrap: s.soft_wrap(),
        find: s
            .active()
            .and_then(|t| t.find())
            .map(|f| (f.query().into(), f.matches().len(), f.current())),
    }
}

#[gpui::test]
fn editor_input_printables_paste_read_only_and_modal_precedence_match(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap(); let path = dir.path().join("input.txt"); let original = "seed\r\n"; std::fs::write(&path, original).unwrap();
    let gui = cx.new(|cx| ReaderView::new(Session::new(config(&dir.path().join("gui"))), Some(path.clone()), cx));
    let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| gui.clone())).unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), cx); let focus = gui.read_with(&visual, |v, _| v.focus_handle()); visual.update(|w, cx| focus.focus(w, cx));
    let mut tui = App::new(config(&dir.path().join("tui")), Some(path.clone()));
    let deadline = Instant::now() + Duration::from_secs(5);
    while gui.read_with(&visual, |v, _| v.session().state().current_source().is_none()) || tui.session.state().current_source().is_none() {
        gui.update(&mut visual, |v, cx| v.poll(cx)); while let Ok(done) = tui.session.completions().try_recv() { tui.apply(done); } assert!(Instant::now() < deadline); std::thread::yield_now();
    }
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 24)).unwrap();
    let draw = |visual: &mut VisualTestContext, tui: &mut App, terminal: &mut ratatui::Terminal<ratatui::backend::TestBackend>| { visual.update(|w, cx| { let _ = w.draw(cx); }); terminal.draw(|f| tui.draw(f)).unwrap(); };
    let key = |tui: &mut App, code, modifiers| { tui.handle(Event::Key(KeyEvent::new(code, modifiers))); };
    let check = |visual: &VisualTestContext, tui: &App, expected: &str| { let state = snapshot(tui.session.state()); assert_eq!(state.text.as_deref(), Some(expected)); assert_eq!(state, gui.read_with(visual, |v, _| snapshot(v.session().state()))); };
    draw(&mut visual, &mut tui, &mut terminal); gui.update(&mut visual, |v, _| v.enable_draft_input_for_test()); tui.enable_draft_input_for_test();
    let pasted = "e\u{301}👩🏽‍💻\r\nqjk/ ";
    visual.update(|_, cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string(pasted.into()))); visual.simulate_keystrokes("ctrl-a ctrl-v"); key(&mut tui, KeyCode::Char('a'), KeyModifiers::CONTROL); tui.handle(Event::Paste(pasted.into())); check(&visual, &tui, pasted);
    visual.simulate_keystrokes("ctrl-z"); key(&mut tui, KeyCode::Char('z'), KeyModifiers::CONTROL); check(&visual, &tui, original);
    visual.simulate_keystrokes("ctrl-shift-z"); key(&mut tui, KeyCode::Char('z'), KeyModifiers::CONTROL | KeyModifiers::SHIFT); check(&visual, &tui, pasted);
    visual.simulate_keystrokes("q j k / space"); for c in ['q', 'j', 'k', '/', ' '] { key(&mut tui, KeyCode::Char(c), KeyModifiers::NONE); }
    let typed = "e\u{301}👩🏽‍💻\r\nqjk/ qjk/ "; check(&visual, &tui, typed); assert!(!tui.quit && tui.modal.is_none());
    visual.simulate_keystrokes("enter tab"); key(&mut tui, KeyCode::Enter, KeyModifiers::NONE); key(&mut tui, KeyCode::Tab, KeyModifiers::NONE); check(&visual, &tui, "e\u{301}👩🏽‍💻\r\nqjk/ qjk/ \r\n\t");
    visual.simulate_keystrokes("backspace ctrl-z"); key(&mut tui, KeyCode::Backspace, KeyModifiers::NONE); key(&mut tui, KeyCode::Char('z'), KeyModifiers::CONTROL); check(&visual, &tui, "e\u{301}👩🏽‍💻\r\nqjk/ qjk/ \r\n\t");
    // Real command menus toggle the same per-buffer permission in both adapters.
    visual.simulate_keystrokes("f10"); let at = gui.read_with(&visual, |v, _| v.modal_rows().iter().position(|row| row.0.starts_with(CommandId::ReadOnlyMode.label())).unwrap()); for _ in 0..at { visual.simulate_keystrokes("down"); } visual.simulate_keystrokes("enter");
    key(&mut tui, KeyCode::F(10), KeyModifiers::NONE); let at = tui.choices().iter().position(|choice| matches!(choice.target, crate::app::ChoiceTarget::Action(CommandId::ReadOnlyMode))).unwrap(); for _ in 0..at { key(&mut tui, KeyCode::Down, KeyModifiers::NONE); } key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    for disabled in [CommandId::Cut, CommandId::Paste, CommandId::Delete] { assert_eq!(tui.session.disabled(disabled), Some("Read-only mode is on")); assert_eq!(gui.read_with(&visual, |v, _| v.session().disabled(disabled)), Some("Read-only mode is on")); }
    let before = snapshot(tui.session.state()); visual.simulate_keystrokes("q delete ctrl-x ctrl-v"); key(&mut tui, KeyCode::Char('q'), KeyModifiers::NONE); key(&mut tui, KeyCode::Delete, KeyModifiers::NONE); key(&mut tui, KeyCode::Char('x'), KeyModifiers::CONTROL); tui.handle(Event::Paste("blocked".into())); assert_eq!(snapshot(tui.session.state()), before); assert_eq!(gui.read_with(&visual, |v, _| snapshot(v.session().state())), before); assert!(!tui.quit);
    visual.simulate_keystrokes("shift-f2 ctrl-v ctrl-x delete ctrl-s escape"); key(&mut tui, KeyCode::F(2), KeyModifiers::SHIFT); tui.handle(Event::Paste("blocked".into())); for c in ['x', 's'] { key(&mut tui, KeyCode::Char(c), KeyModifiers::CONTROL); } key(&mut tui, KeyCode::Delete, KeyModifiers::NONE); key(&mut tui, KeyCode::Esc, KeyModifiers::NONE); assert_eq!(snapshot(tui.session.state()), before); assert_eq!(gui.read_with(&visual, |v, _| snapshot(v.session().state())), before);
    draw(&mut visual, &mut tui, &mut terminal); assert_eq!(std::fs::read(path).unwrap(), original.as_bytes());
}

#[gpui::test]
fn several_cursors_replay_the_same_text_and_selections_in_both_adapters(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("carets.txt");
    let original = "aa bb aa\none\n";
    std::fs::write(&path, original).unwrap();
    let gui = cx.new(|cx| ReaderView::new(Session::new(config(&dir.path().join("gui"))), Some(path.clone()), cx));
    let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| gui.clone())).unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let focus = gui.read_with(&visual, |view, _| view.focus_handle());
    visual.update(|window, cx| focus.focus(window, cx));
    let mut tui = App::new(config(&dir.path().join("tui")), Some(path.clone()));
    let deadline = Instant::now() + Duration::from_secs(5);
    while gui.read_with(&visual, |view, _| view.session().state().current_source().is_none()) || tui.session.state().current_source().is_none() {
        gui.update(&mut visual, |view, cx| view.poll(cx));
        while let Ok(done) = tui.session.completions().try_recv() { tui.apply(done); }
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 24)).unwrap();
    let draw = |visual: &mut VisualTestContext, tui: &mut App, terminal: &mut ratatui::Terminal<ratatui::backend::TestBackend>| {
        visual.update(|window, cx| { let _ = window.draw(cx); });
        terminal.draw(|frame| tui.draw(frame)).unwrap();
    };
    let key = |tui: &mut App, code, modifiers| { tui.handle(Event::Key(KeyEvent::new(code, modifiers))); };
    let check = |visual: &VisualTestContext, tui: &App, expected: &str| {
        let state = snapshot(tui.session.state());
        assert_eq!(state.text.as_deref(), Some(expected));
        assert_eq!(state, gui.read_with(visual, |view, _| snapshot(view.session().state())));
    };
    draw(&mut visual, &mut tui, &mut terminal);
    gui.update(&mut visual, |view, _| view.enable_draft_input_for_test());
    tui.enable_draft_input_for_test();
    visual.simulate_keystrokes("right alt-shift-down");
    key(&mut tui, KeyCode::Right, KeyModifiers::NONE);
    key(&mut tui, KeyCode::Down, KeyModifiers::ALT | KeyModifiers::SHIFT);
    check(&visual, &tui, original);
    assert_eq!(snapshot(tui.session.state()).selections.len(), 2, "both adapters hold two cursors");
    visual.simulate_keystrokes("x y ctrl-z");
    for c in ['x', 'y'] { key(&mut tui, KeyCode::Char(c), KeyModifiers::NONE); }
    key(&mut tui, KeyCode::Char('z'), KeyModifiers::CONTROL);
    check(&visual, &tui, original);
    visual.simulate_keystrokes("x escape");
    key(&mut tui, KeyCode::Char('x'), KeyModifiers::NONE);
    key(&mut tui, KeyCode::Esc, KeyModifiers::NONE);
    check(&visual, &tui, "axa bb aa\noxne\n");
    assert_eq!(snapshot(tui.session.state()).selections.len(), 1, "Escape keeps one cursor in both adapters");
    draw(&mut visual, &mut tui, &mut terminal);
    assert_eq!(std::fs::read(path).unwrap(), original.as_bytes());
}

#[gpui::test]
fn gated_drafts_share_split_history_and_keep_views_and_sessions_independent(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("draft.txt");
    let original = "é界\r\nfirst\r\nsecond\r\nthird\r\n";
    let typed = "éλ\r\nfirst\r\nsecond\r\nthird\r\n";
    let pasted = "éλ👩🏽‍💻\r\nfirst\r\nsecond\r\nthird\r\n";
    std::fs::write(&path, original).unwrap();
    let gui = cx.new(|cx| ReaderView::new(Session::new(config(&dir.path().join("gui"))), Some(path.clone()), cx));
    let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| gui.clone())).unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let focus = gui.read_with(&visual, |view, _| view.focus_handle());
    visual.update(|window, cx| focus.focus(window, cx));
    let mut tui = App::new(config(&dir.path().join("tui")), Some(path.clone()));
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 24)).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while gui.read_with(&visual, |view, _| view.session().state().current_source().is_none()) || tui.session.state().current_source().is_none() {
        gui.update(&mut visual, |view, cx| view.poll(cx));
        while let Ok(done) = tui.session.completions().try_recv() { tui.apply(done); }
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    let draw = |visual: &mut VisualTestContext, tui: &mut App, terminal: &mut ratatui::Terminal<ratatui::backend::TestBackend>| {
        visual.update(|window, cx| { let _ = window.draw(cx); });
        terminal.draw(|frame| tui.draw(frame)).unwrap();
    };
    let text = |state: &State| { let source = state.current_source().unwrap(); source.copy_range(0..source.len_bytes()).unwrap() };
    draw(&mut visual, &mut tui, &mut terminal);
    gui.update(&mut visual, |view, _| view.enable_draft_input_for_test());
    tui.enable_draft_input_for_test();
    visual.simulate_keystrokes("right shift-right λ");
    assert_eq!(text(tui.session.state()), original, "a different Session does not share the GUI draft");
    for (key, modifiers) in [(KeyCode::Right, KeyModifiers::NONE), (KeyCode::Right, KeyModifiers::SHIFT), (KeyCode::Char('λ'), KeyModifiers::NONE)] {
        tui.handle(Event::Key(KeyEvent::new(key, modifiers)));
    }
    visual.update(|_, cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string("👩🏽‍💻".into())));
    visual.simulate_keystrokes("ctrl-v ctrl-\\");
    tui.handle(Event::Paste("👩🏽‍💻".into()));
    tui.handle(Event::Key(KeyEvent::new(KeyCode::Char('\\'), KeyModifiers::CONTROL)));
    draw(&mut visual, &mut tui, &mut terminal);
    for state in [gui.read_with(&visual, |view, _| text(view.session().state())), text(tui.session.state())] { assert_eq!(state, pasted); }
    assert_eq!(snapshot(tui.session.state()), gui.read_with(&visual, |view, _| snapshot(view.session().state())));
    for pane in [0, 1] {
        visual.simulate_keystrokes(if pane == 0 { "ctrl-1" } else { "ctrl-2" });
        tui.handle(Event::Key(KeyEvent::new(KeyCode::Char(if pane == 0 { '1' } else { '2' }), KeyModifiers::CONTROL)));
        draw(&mut visual, &mut tui, &mut terminal);
        assert_eq!(gui.read_with(&visual, |view, _| view.visible_rows()[0].text.clone()), "éλ👩🏽‍💻");
    }
    visual.simulate_keystrokes("down down pagedown");
    for key in [KeyCode::Down, KeyCode::Down, KeyCode::PageDown] { tui.handle(Event::Key(KeyEvent::new(key, KeyModifiers::NONE))); }
    let positions = |state: &State| (0..2).map(|pane| {
        let tab = state.shown(pane).unwrap();
        (tab.selection().unwrap(), tab.top_line())
    }).collect::<Vec<_>>();
    let before = positions(tui.session.state());
    assert_eq!(before, gui.read_with(&visual, |view, _| positions(view.session().state())));
    assert_eq!(before[0].0.head, 19);
    assert_eq!(before[1].0.head, 33);
    assert_eq!(before[0].1, 0);
    assert!(before[1].1 > before[0].1, "only the focused pane scrolls");
    assert_eq!(tui.session.disabled(CommandId::Undo), None);
    assert_eq!(tui.session.disabled(CommandId::Redo), Some("Nothing to redo"));
    visual.simulate_keystrokes("ctrl-z");
    assert_eq!(text(tui.session.state()), pasted, "undo in another Session leaves this draft alone");
    tui.handle(Event::Key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL)));
    draw(&mut visual, &mut tui, &mut terminal);
    assert_eq!(text(tui.session.state()), typed);
    assert_eq!(snapshot(tui.session.state()), gui.read_with(&visual, |view, _| snapshot(view.session().state())));
    assert_eq!(positions(tui.session.state()), gui.read_with(&visual, |view, _| positions(view.session().state())));
    assert_eq!(positions(tui.session.state())[0].0.head, 4);
    assert_eq!(positions(tui.session.state())[1].0.head, 18);
    assert_eq!(positions(tui.session.state())[0].1, 0);
    let left = tui.session.state().view(tui.session.state().shown(0).unwrap().id()).unwrap();
    let right = tui.session.state().view(tui.session.state().shown(1).unwrap().id()).unwrap();
    assert!(Arc::ptr_eq(left.snapshot(), right.snapshot()), "split views use one current draft");
    visual.simulate_keystrokes("ctrl-shift-z");
    tui.handle(Event::Key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL | KeyModifiers::SHIFT)));
    draw(&mut visual, &mut tui, &mut terminal);
    assert_eq!(text(tui.session.state()), pasted);
    assert_eq!(snapshot(tui.session.state()), gui.read_with(&visual, |view, _| snapshot(view.session().state())));
    assert_eq!(positions(tui.session.state())[0].0.head, 19);
    assert_eq!(positions(tui.session.state())[1].0.head, 33);
    assert_eq!(std::fs::read_to_string(path).unwrap(), original);
}

#[gpui::test]
fn project_text_search_reads_shifted_drafts_and_refuses_stale_activation_in_both_adapters(cx: &mut TestAppContext) {
    replay_draft_project_query(cx, DraftQuery::Text);
}

#[gpui::test]
fn project_declaration_search_reads_shifted_drafts_and_refuses_stale_activation_in_both_adapters(cx: &mut TestAppContext) {
    replay_draft_project_query(cx, DraftQuery::Declarations);
}

#[gpui::test]
fn project_name_navigation_reads_shifted_drafts_and_refuses_stale_disk_symbols_in_both_adapters(cx: &mut TestAppContext) {
    replay_draft_project_query(cx, DraftQuery::Names);
}

enum DraftQuery { Text, Declarations, Names }

fn replay_draft_project_query(cx: &mut TestAppContext, mode: DraftQuery) {
    let declarations = matches!(mode, DraftQuery::Declarations);
    use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    std::fs::create_dir(&root).unwrap();
    let saved_a = "fn SavedOnlyA() {}\r\n";
    let saved_b = "fn SavedOnlyB() {}\r\n";
    let saved_c = "fn UnopenedDraftOnly() {}\r\n";
    let draft_a = "\r\n// é👩🏽‍💻\r\nfn DraftOnlyA() {}\r\n";
    let draft_b = "\r\n\r\n// 界\r\nfn DraftOnlyB() {}\r\n";
    for (name, text) in [("a.rs", saved_a), ("b.rs", saved_b), ("c.rs", saved_c)] { std::fs::write(root.join(name), text).unwrap(); }
    let gui = cx.new(|cx| ReaderView::new(Session::new(config(&dir.path().join("gui"))), Some(root.clone()), cx));
    let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| gui.clone())).unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    visual.simulate_resize(gpui::size(gpui::px(1100.0), gpui::px(700.0)));
    let focus = gui.read_with(&visual, |view, _| view.focus_handle());
    visual.update(|window, cx| focus.focus(window, cx));
    let mut tui = App::new(config(&dir.path().join("tui")), Some(root.clone()));
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 36)).unwrap();
    let key = |tui: &mut App, code, modifiers| tui.handle(Event::Key(KeyEvent::new(code, modifiers)));
    let draw = |visual: &mut VisualTestContext, tui: &mut App, terminal: &mut ratatui::Terminal<ratatui::backend::TestBackend>| {
        visual.update(|window, cx| { let _ = window.draw(cx); });
        terminal.draw(|frame| tui.draw(frame)).unwrap();
    };
    let wait = |visual: &mut VisualTestContext, tui: &mut App, terminal: &mut ratatui::Terminal<ratatui::backend::TestBackend>, done: &dyn Fn(&State) -> bool| {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            gui.update(visual, |view, cx| view.poll(cx));
            while let Ok(completion) = tui.session.completions().try_recv() { tui.apply(completion); }
            draw(visual, tui, terminal);
            if gui.read_with(visual, |view, _| done(view.session().state())) && done(tui.session.state()) { break; }
            assert!(Instant::now() < deadline, "draft query adapters did not settle: GUI {:?}; TUI {:?}", gui.read_with(visual, |view, _| snapshot(view.session().state())), snapshot(tui.session.state()));
            std::thread::yield_now();
        }
    };
    wait(&mut visual, &mut tui, &mut terminal, &|state| state.project().is_some_and(|project| matches!(project.coverage(), reader_core::project::Coverage::Complete(_))) && state.index_coverage().complete());
    gui.update(&mut visual, |view, _| view.enable_draft_input_for_test());
    tui.enable_draft_input_for_test();
    let paste = |visual: &mut VisualTestContext, tui: &mut App, text: &str| {
        visual.update(|_, cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string(text.into())));
        visual.simulate_keystrokes("ctrl-v");
        tui.handle(Event::Paste(text.into()));
    };
    for (name, text) in [("a.rs", draft_a), ("b.rs", draft_b)] {
        visual.simulate_keystrokes("ctrl-p");
        visual.simulate_keystrokes(&name.chars().map(|c| c.to_string()).collect::<Vec<_>>().join(" "));
        key(&mut tui, KeyCode::Char('p'), KeyModifiers::CONTROL);
        tui.handle(Event::Paste(name.into()));
        wait(&mut visual, &mut tui, &mut terminal, &|state| state.navigation().is_some_and(|search| !search.pending && !search.rows.is_empty()));
        visual.simulate_keystrokes("enter");
        key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
        wait(&mut visual, &mut tui, &mut terminal, &|state| state.current_source().is_some_and(|source| source.source().ends_with(name)) && state.loading().is_none());
        visual.simulate_keystrokes("ctrl-a");
        key(&mut tui, KeyCode::Char('a'), KeyModifiers::CONTROL);
        paste(&mut visual, &mut tui, text);
        assert_eq!(snapshot(tui.session.state()), gui.read_with(&visual, |view, _| snapshot(view.session().state())));
        assert_eq!(snapshot(tui.session.state()).text.as_deref(), Some(text));
    }
    if matches!(mode, DraftQuery::Names) {
        use reader_core::navigation::{Kind, Scope};
        let navigate = |visual: &mut VisualTestContext, tui: &mut App, terminal: &mut ratatui::Terminal<ratatui::backend::TestBackend>, scope: Scope, query: &str| {
            visual.simulate_keystrokes(if scope == Scope::All { "ctrl-k" } else { "ctrl-alt-shift-n" });
            key(tui, KeyCode::Char(if scope == Scope::All { 'k' } else { 'n' }), if scope == Scope::All { KeyModifiers::CONTROL } else { KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SHIFT });
            visual.simulate_keystrokes(&query.chars().map(|c| c.to_string()).collect::<Vec<_>>().join(" "));
            tui.handle(Event::Paste(query.into()));
            wait(visual, tui, terminal, &|state| state.navigation().is_some_and(|search| !search.pending && search.scope == scope && search.query == query));
        };
        let symbols = |state: &State| {
            let mut rows: Vec<_> = state.navigation().unwrap().rows.iter().filter(|row| row.kind == Kind::Symbol).map(|row| (row.label.clone(), row.detail.clone())).collect();
            rows.sort(); rows
        };
        for scope in [Scope::All, Scope::Symbols] {
            navigate(&mut visual, &mut tui, &mut terminal, scope, "draftonly");
            let rows = symbols(tui.session.state());
            assert_eq!(rows.iter().map(|row| row.0.as_str()).collect::<Vec<_>>(), ["DraftOnlyA", "DraftOnlyB", "UnopenedDraftOnly"]);
            assert!(rows[0].1.ends_with("a.rs:3"));
            assert!(rows[1].1.ends_with("b.rs:4"));
            assert!(rows[2].1.ends_with("c.rs:1"));
            assert_eq!(rows, gui.read_with(&visual, |view, _| symbols(view.session().state())));
            let labels = gui.read_with(&visual, |view, _| view.modal_rows().into_iter().map(|row| row.0).collect::<Vec<_>>().join("\n"));
            let shown: String = terminal.backend().buffer().content.iter().map(|cell| cell.symbol()).collect();
            for name in ["DraftOnlyA", "DraftOnlyB", "UnopenedDraftOnly"] { assert!(labels.contains(name)); assert!(shown.contains(name), "{shown}"); }
            visual.simulate_keystrokes("escape"); key(&mut tui, KeyCode::Esc, KeyModifiers::NONE);
            navigate(&mut visual, &mut tui, &mut terminal, scope, "savedonly");
            assert!(symbols(tui.session.state()).is_empty());
            assert!(gui.read_with(&visual, |view, _| symbols(view.session().state()).is_empty()));
            visual.simulate_keystrokes("escape"); key(&mut tui, KeyCode::Esc, KeyModifiers::NONE);
        }
        navigate(&mut visual, &mut tui, &mut terminal, Scope::Symbols, "draftonlya");
        visual.simulate_keystrokes("enter"); key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
        wait(&mut visual, &mut tui, &mut terminal, &|state| state.navigation().is_none() && state.current_source().is_some_and(|source| source.source().ends_with("a.rs")));
        assert_eq!(snapshot(tui.session.state()).text.as_deref(), Some(draft_a));
        assert_eq!(snapshot(tui.session.state()), gui.read_with(&visual, |view, _| snapshot(view.session().state())));
        assert_eq!(tui.session.state().selection().unwrap().head, draft_a.find("fn DraftOnlyA").unwrap());
        navigate(&mut visual, &mut tui, &mut terminal, Scope::Symbols, "unopeneddraftonly");
        let before = snapshot(tui.session.state());
        let rewritten = format!("// externally shifted\r\n{saved_c}");
        std::fs::write(root.join("c.rs"), &rewritten).unwrap();
        // No watcher polling between the fixture's external rewrite and the real Enter inputs.
        visual.simulate_keystrokes("enter"); key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            gui.update(&mut visual, |view, cx| view.poll(cx));
            while let Ok(completion) = tui.session.completions().try_recv() { tui.apply(completion); }
            draw(&mut visual, &mut tui, &mut terminal);
            if gui.read_with(&visual, |view, _| view.session().navigation_status().contains("search again")) && tui.session.navigation_status().contains("search again") { break; }
            assert!(Instant::now() < deadline, "stale symbol did not report its refusal");
        }
        assert_eq!(snapshot(tui.session.state()), before, "a changed disk symbol must not open its old line");
        assert_eq!(gui.read_with(&visual, |view, _| snapshot(view.session().state())), before);
        assert!(tui.modal.is_some(), "a refused activation keeps navigation open");
        let labels = gui.read_with(&visual, |view, _| view.modal_rows().into_iter().map(|row| row.0).collect::<Vec<_>>().join("\n"));
        assert!(labels.contains("search again"), "{labels}");
        let shown: String = terminal.backend().buffer().content.iter().map(|cell| cell.symbol()).collect();
        assert!(shown.contains("search again"), "{shown}");
        visual.simulate_keystrokes("escape"); key(&mut tui, KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(std::fs::read(root.join("a.rs")).unwrap(), saved_a.as_bytes());
        assert_eq!(std::fs::read(root.join("b.rs")).unwrap(), saved_b.as_bytes());
        assert_eq!(std::fs::read(root.join("c.rs")).unwrap(), rewritten.as_bytes(), "only the explicit external fixture rewrite changed disk");
        return;
    }
    let query = |visual: &mut VisualTestContext, tui: &mut App, terminal: &mut ratatui::Terminal<ratatui::backend::TestBackend>, text: &str| {
        visual.simulate_keystrokes(if declarations { "ctrl-t" } else { "ctrl-shift-f" });
        key(tui, KeyCode::Char(if declarations { 't' } else { 'f' }), if declarations { KeyModifiers::CONTROL } else { KeyModifiers::CONTROL | KeyModifiers::SHIFT });
        if !declarations {
            for _ in 0..9 { visual.simulate_keystrokes("backspace"); key(tui, KeyCode::Backspace, KeyModifiers::NONE); }
        }
        visual.simulate_keystrokes(&text.chars().map(|c| c.to_string()).collect::<Vec<_>>().join(" "));
        tui.handle(Event::Paste(text.into()));
        visual.simulate_keystrokes("enter");
        key(tui, KeyCode::Enter, KeyModifiers::NONE);
        wait(visual, tui, terminal, &|state| state.results().is_some_and(|results| results.done() && results.query() == text));
    };
    query(&mut visual, &mut tui, &mut terminal, "draftonly");
    let hits = |state: &State| { let mut hits: Vec<_> = state.results().unwrap().hits().iter().map(|hit| (hit.path.to_string(), hit.line, hit.range.clone(), hit.text.to_string())).collect(); hits.sort_by(|a, b| (&a.0, a.1).cmp(&(&b.0, b.1))); hits };
    let expected = if declarations {
        vec![("a.rs".into(), 2, 3..13, "DraftOnlyA".into()), ("b.rs".into(), 3, 3..13, "DraftOnlyB".into()), ("c.rs".into(), 0, 3..20, "UnopenedDraftOnly".into())]
    } else {
        vec![("a.rs".into(), 2, 3..12, "fn DraftOnlyA() {}".into()), ("b.rs".into(), 3, 3..12, "fn DraftOnlyB() {}".into()), ("c.rs".into(), 0, 11..20, "fn UnopenedDraftOnly() {}".into())]
    };
    assert_eq!(hits(tui.session.state()), expected);
    assert_eq!(hits(tui.session.state()), gui.read_with(&visual, |view, _| hits(view.session().state())));
    assert!(tui.session.state().tabs().iter().all(|tab| !tab.source().ends_with("c.rs")), "unopened result is read from disk");
    query(&mut visual, &mut tui, &mut terminal, "savedonly");
    assert!(tui.session.state().results().unwrap().hits().is_empty(), "removed disk matches disappear from both drafts");
    assert!(gui.read_with(&visual, |view, _| view.session().state().results().unwrap().hits().is_empty()));
    query(&mut visual, &mut tui, &mut terminal, "draftonly");
    let select_first = |visual: &mut VisualTestContext, tui: &mut App, terminal: &mut ratatui::Terminal<ratatui::backend::TestBackend>| {
        draw(visual, tui, terminal);
        let (x, y) = gui.read_with(visual, |view, _| view.result_point_of(0)).unwrap();
        visual.simulate_click(gpui::point(gpui::px(x), gpui::px(y)), gpui::Modifiers::default());
        let row = tui.session.state().results().unwrap().row_of_hit(0).unwrap();
        let area = tui.layout.results;
        tui.handle(Event::Mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: area.x + 8, row: area.y + row as u16, modifiers: KeyModifiers::NONE }));
        wait(visual, tui, terminal, &|state| state.results().unwrap().preview().is_some_and(|preview| preview.index == 0));
    };
    select_first(&mut visual, &mut tui, &mut terminal);
    let preview = |state: &State| state.results().unwrap().preview().unwrap().doc.text().to_string();
    assert_eq!(preview(tui.session.state()), draft_a);
    assert_eq!(gui.read_with(&visual, |view, _| preview(view.session().state())), draft_a);
    assert!(tui.session.state().current_source().unwrap().source().ends_with("b.rs"), "preview does not move the editor");
    let (_, rows) = gui.update(&mut visual, |view, _| view.panel_contents(16, 16));
    assert!(rows.iter().any(|row| row.number == 3 && row.text == "fn DraftOnlyA() {}"));
    let area = tui.layout.results;
    let preview_cells: String = (area.y..area.bottom()).flat_map(|y| (area.right() + 1..120).map(move |x| (x, y))).map(|point| terminal.backend().buffer()[point].symbol()).collect();
    assert!(preview_cells.contains("fn DraftOnlyA() {}"), "{preview_cells}");
    visual.simulate_keystrokes("ctrl-shift-tab");
    key(&mut tui, KeyCode::Tab, KeyModifiers::CONTROL | KeyModifiers::SHIFT);
    wait(&mut visual, &mut tui, &mut terminal, &|state| state.current_source().is_some_and(|source| source.source().ends_with("a.rs")));
    let caret = gui.read_with(&visual, |view, _| view.session().state().selection().unwrap().head);
    let (x, y) = gui.read_with(&visual, |view, _| view.text_point_of(caret)).unwrap();
    visual.simulate_click(gpui::point(gpui::px(x), gpui::px(y)), gpui::Modifiers::default());
    let area = tui.layout.panes[0].text;
    tui.handle(Event::Mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: area.x, row: area.y, modifiers: KeyModifiers::NONE }));
    visual.simulate_keystrokes("ctrl-a");
    key(&mut tui, KeyCode::Char('a'), KeyModifiers::CONTROL);
    let changed = format!("// later\r\n{draft_a}");
    paste(&mut visual, &mut tui, &changed);
    assert!(tui.session.state().results().unwrap().is_stale(0));
    assert!(gui.read_with(&visual, |view, _| view.session().state().results().unwrap().is_stale(0)));
    assert_eq!(preview(tui.session.state()), draft_a, "preview retains the exact query snapshot after another edit");
    assert_eq!(gui.read_with(&visual, |view, _| preview(view.session().state())), draft_a);
    let before = snapshot(tui.session.state());
    assert_eq!(before, gui.read_with(&visual, |view, _| snapshot(view.session().state())));
    select_first(&mut visual, &mut tui, &mut terminal);
    visual.simulate_keystrokes("enter");
    key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        gui.update(&mut visual, |view, cx| view.poll(cx));
        while let Ok(completion) = tui.session.completions().try_recv() { tui.apply(completion); }
        draw(&mut visual, &mut tui, &mut terminal);
        if gui.read_with(&visual, |view, _| view.session().status_message().contains("rerun the search")) && tui.session.status_message().contains("rerun the search") { break; }
        assert!(Instant::now() < deadline, "stale result did not report its refusal");
    }
    assert_eq!(snapshot(tui.session.state()), before, "stale result must not navigate to old coordinates");
    assert_eq!(gui.read_with(&visual, |view, _| snapshot(view.session().state())), before);
    let (rows, _) = gui.update(&mut visual, |view, _| view.panel_contents(16, 16));
    assert!(rows.iter().any(|row| row.hit == Some(0) && row.stale && row.label.contains("changed since")));
    let shown: String = terminal.backend().buffer().content.iter().map(|cell| cell.symbol()).collect();
    assert!(shown.contains("[changed]"), "{shown}");
    let list = tui.layout.results;
    let header: String = (list.x..120).map(|x| terminal.backend().buffer()[(x, list.y - 1)].symbol()).collect();
    assert!(header.contains("Open drafts changed"), "the stale-query error stays above the result rows: {header}");
    assert!(gui.read_with(&visual, |view, _| view.results_status().unwrap().contains("Open drafts changed")));
    for (name, text) in [("a.rs", saved_a), ("b.rs", saved_b), ("c.rs", saved_c)] { assert_eq!(std::fs::read(root.join(name)).unwrap(), text.as_bytes()); }
}

#[gpui::test]
fn markdown_header_buttons_target_their_own_pane_and_respect_modals(cx: &mut TestAppContext) {
    use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    use reader_core::session::{Rendering, Split};
    for dark in [true, false] {
        for split in [Split::Columns, Split::Rows] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("a-long-markdown-filename-to-clip-in-the-header.md");
            let bytes = "# Title\n\nSome **Markdown**.\n";
            std::fs::write(&path, bytes).unwrap();
            let mut gc = config(&dir.path().join("gui")); gc.dark = dark;
            let mut tc = config(&dir.path().join("tui")); tc.dark = dark;
            let gui = cx.new(|cx| ReaderView::new(Session::new(gc), Some(path.clone()), cx));
            let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| gui.clone())).unwrap();
            let mut visual = VisualTestContext::from_window(window.into(), cx);
            let focus = gui.read_with(&visual, |v, _| v.focus_handle());
            visual.update(|w, cx| focus.focus(w, cx));
            let mut tui = App::new(tc, Some(path.clone()));
            let settle = |visual: &mut VisualTestContext, tui: &mut App| {
                let deadline = Instant::now() + Duration::from_secs(5);
                loop {
                    gui.update(visual, |v, cx| v.poll(cx));
                    while let Ok(c) = tui.session.completions().try_recv() { tui.apply(c); }
                    let ready = |s: &State| s.current_source().is_some() && (0..2).all(|pane| !s.shown(pane).is_some_and(|t| s.view(t.id()).is_some_and(|view| matches!(view.rendering(), Some(Rendering::Pending)))));
                    if gui.read_with(visual, |v, _| ready(v.session().state())) && ready(tui.session.state()) { break; }
                    assert!(Instant::now() < deadline);
                    std::thread::sleep(Duration::from_millis(2));
                }
            };
            let draw = |visual: &mut VisualTestContext, tui: &mut App, w: f32, h: f32, cols: u16, rows: u16| {
                visual.simulate_resize(gpui::size(gpui::px(w), gpui::px(h)));
                visual.update(|w, cx| { let _ = w.draw(cx); });
                let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(cols, rows)).unwrap();
                terminal.draw(|f| tui.draw(f)).unwrap();
                if tui.modal.is_none() {
                    for pane in &tui.layout.panes {
                        for (_, b) in &pane.controls {
                            let text: String = (b.x..b.right()).map(|x| terminal.backend().buffer()[(x, b.y)].symbol()).collect();
                            assert!(text.contains(tui.session.preview_control(pane.pane).unwrap().label));
                        }
                    }
                }
            };
            let click = |visual: &mut VisualTestContext, tui: &mut App, pane| {
                let point = gui.read_with(visual, |v, _| {
                    let g = *v.panes.iter().find(|g| g.pane == pane).unwrap();
                    let (button, _) = v.preview_button(g).unwrap();
                    assert!(button.left() >= gpui::px(g.bar.0) && button.right() <= gpui::px(g.bar.1));
                    button.center()
                });
                visual.simulate_click(point, gpui::Modifiers::default());
                let g = tui.layout.panes.iter().find(|p| p.pane == pane).unwrap();
                let (_, b) = g.controls.iter().find(|(id, _)| *id == CommandId::MarkdownSource).unwrap();
                assert!(g.tabs.iter().all(|(_, tab)| tab.right() <= b.x));
                tui.handle(Event::Mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: b.x + 1, row: b.y, modifiers: KeyModifiers::NONE }));
            };
            settle(&mut visual, &mut tui);
            gui.update_in(&mut visual, |v, w, cx| v.run(if split == Split::Columns { CommandId::SplitRight } else { CommandId::SplitDown }, w, cx));
            tui.dispatch(Command::Split(split));
            for pane in [0, 1, 0, 1] {
                // Force focus elsewhere: the clicked header must choose the target before toggling.
                visual.simulate_keystrokes(if pane == 0 { "ctrl-2" } else { "ctrl-1" });
                tui.dispatch(Command::FocusPane(1 - pane));
                draw(&mut visual, &mut tui, 900., 500., 100, 30);
                assert!(gui.read_with(&visual, |v, _| v.nav_buttons().iter().all(|b| b.2 != CommandId::MarkdownSource)));
                let before: Vec<_> = (0..2).map(|p| tui.session.state().shown(p).unwrap().markdown_source()).collect();
                click(&mut visual, &mut tui, pane);
                settle(&mut visual, &mut tui);
                for p in 0..2 {
                    let expected = if p == pane { !before[p as usize] } else { before[p as usize] };
                    assert_eq!(tui.session.state().shown(p).unwrap().markdown_source(), expected);
                    assert_eq!(gui.read_with(&visual, |v, _| v.session().state().shown(p).unwrap().markdown_source()), expected);
                }
                assert_eq!(tui.session.state().focused_pane(), pane);
                assert_eq!(gui.read_with(&visual, |v, _| v.session().state().focused_pane()), pane);
            }
            draw(&mut visual, &mut tui, 900., 500., 100, 30);
            visual.simulate_keystrokes("f2");
            tui.handle(Event::Key(KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE)));
            draw(&mut visual, &mut tui, 900., 500., 100, 30);
            click(&mut visual, &mut tui, 0); // dismisses the modal; never reaches the header underneath
            assert!((0..2).all(|p| tui.session.state().shown(p).unwrap().markdown_source()));
            assert!(gui.read_with(&visual, |v, _| (0..2).all(|p| v.session().state().shown(p).unwrap().markdown_source())));
            visual.simulate_keystrokes("escape");
            tui.handle(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
            for (w, h, cols, rows) in [(380., 180., 34, 12), (120., 50., 8, 4), (1., 1., 1, 1), (900., 500., 100, 30)] {
                draw(&mut visual, &mut tui, w, h, cols, rows);
                gui.read_with(&visual, |v, _| {
                    for g in &v.panes {
                        if let Some((b, _)) = v.preview_button(*g) { assert!(b.left() >= gpui::px(0.) && b.right() <= gpui::px(w)); }
                    }
                });
            }
            assert_eq!(std::fs::read_to_string(&path).unwrap(), bytes);
        }
    }
}

#[gpui::test]
fn gui_and_terminal_project_lsp_actions_save_and_reset_the_same_scope(cx: &mut TestAppContext) {
    for dark in [true, false] {
        let scratch = tempfile::tempdir().unwrap();
        let root = scratch.path().join("rails");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("Gemfile"), "").unwrap();
        let source = "class Application; end\n";
        let path = root.join("application.rb");
        std::fs::write(&path, source).unwrap();
        let mut gc = config(&scratch.path().join("gui"));
        let mut tc = config(&scratch.path().join("tui"));
        gc.dark = dark;
        tc.dark = dark;
        let gui_file = gc.providers.clone();
        let tui_file = tc.providers.clone();
        let gui = cx.new(|cx| ReaderView::new(Session::new(gc), Some(path.clone()), cx));
        let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| gui.clone())).unwrap();
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        let focus = gui.read_with(&visual, |v, _| v.focus_handle());
        visual.update(|w, cx| focus.focus(w, cx));
        let mut tui = App::new(tc, Some(path.clone()));
        let key = |app: &mut App, code| { app.handle(Event::Key(KeyEvent::new(code, KeyModifiers::NONE))); };
        let settle = |visual: &mut VisualTestContext, tui: &mut App, configured: bool, excluded: bool| {
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                gui.update(visual, |v, cx| v.poll(cx));
                while let Ok(c) = tui.session.completions().try_recv() { tui.apply(c); }
                let done = |s: &Session| s.state().provider().is_some_and(|v| matches!(v.status, reader_core::semantics::Status::Missing(_)))
                    && s.disabled(CommandId::ExcludeRubyTooling).is_none()
                    && s.disabled(CommandId::ResetProjectServerSettings).is_none() == configured
                    && s.actions().iter().find(|a| a.id == CommandId::ExcludeRubyTooling).and_then(|a| a.on) == Some(excluded);
                if gui.read_with(visual, |v, _| done(v.session())) && done(&tui.session) { break; }
                assert!(Instant::now() < deadline, "GUI {:?}; TUI {:?}", gui.read_with(visual, |v, _| v.session().state().notices()), tui.session.state().notices());
                std::thread::sleep(Duration::from_millis(2));
            }
            assert_eq!(gui.read_with(visual, |v, _| snapshot(v.session().state())), snapshot(tui.session.state()));
            assert_eq!(std::fs::read_to_string(&path).unwrap(), source);
        };
        settle(&mut visual, &mut tui, false, true);
        // Both adapters open the same frozen report from the first F2 action, then copy it after scrolling.
        visual.simulate_keystrokes("f2 enter");
        key(&mut tui, KeyCode::F(2)); key(&mut tui, KeyCode::Enter);
        for (width, height) in [(26, 8), (1, 1), (80, 24)] {
            let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
            terminal.draw(|f| tui.draw(f)).unwrap();
            key(&mut tui, KeyCode::End); key(&mut tui, KeyCode::Up);
        }
        for (width, height) in [(180., 90.), (1., 1.), (800., 400.)] {
            visual.simulate_resize(gpui::size(gpui::px(width), gpui::px(height)));
            visual.simulate_keystrokes("end up");
            visual.update(|w, cx| { let _ = w.draw(cx); });
        }
        visual.simulate_keystrokes("ctrl-c");
        tui.handle(Event::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)));
        let copied = visual.update(|_, cx| cx.read_from_clipboard().and_then(|c| c.text())).unwrap();
        let normalize = |s: &str| s.replace(&gui_file.display().to_string(), "providers.json").replace(&tui_file.display().to_string(), "providers.json");
        assert_eq!(normalize(&copied), normalize(tui.copy.as_deref().unwrap()));
        assert!(copied.contains("Ruby tooling gem exclusions: enabled") && copied.contains("Workspace:"));
        visual.simulate_keystrokes("escape"); key(&mut tui, KeyCode::Esc);
        assert!(tui.modal.is_none());
        settle(&mut visual, &mut tui, false, true);
        for (id, excluded) in [(CommandId::ExcludeRubyTooling, false), (CommandId::ExcludeRubyTooling, true), (CommandId::ResetProjectServerSettings, true)] {
            visual.simulate_keystrokes("f2 home");
            key(&mut tui, KeyCode::F(2));
            key(&mut tui, KeyCode::Home);
            for (width, height) in [(26, 8), (1, 1), (80, 24)] {
                let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
                terminal.draw(|f| tui.draw(f)).unwrap();
            }
            for (width, height) in [(180., 90.), (1., 1.), (800., 400.)] {
                visual.simulate_resize(gpui::size(gpui::px(width), gpui::px(height)));
                visual.update(|w, cx| { let _ = w.draw(cx); });
            }
            let at = tui.session.language_server_rows().iter().position(|(_, action)| *action == Some(id)).unwrap();
            for _ in 0..at { visual.simulate_keystrokes("down"); key(&mut tui, KeyCode::Down); }
            visual.simulate_keystrokes("enter");
            key(&mut tui, KeyCode::Enter);
            settle(&mut visual, &mut tui, id == CommandId::ExcludeRubyTooling, excluded);
            let g: serde_json::Value = serde_json::from_slice(&std::fs::read(&gui_file).unwrap()).unwrap();
            let t: serde_json::Value = serde_json::from_slice(&std::fs::read(&tui_file).unwrap()).unwrap();
            assert_eq!(g, t);
            let ruby = &g["providers"]["ruby-lsp"];
            assert!(ruby.get("settings").is_none(), "global settings were changed");
            if id == CommandId::ExcludeRubyTooling {
                assert_eq!(ruby["projects"][root.to_str().unwrap()]["indexing"]["excludedGems"].as_array().unwrap().contains(&serde_json::json!("parser")), excluded);
                let details = tui.session.language_server_details();
                assert!(details.contains("Project settings for") && details.contains("project overrides"));
                assert!(details.contains(if excluded { "Ruby tooling gem exclusions: enabled" } else { "Ruby tooling gem exclusions: not applied" }));
            } else { assert!(ruby.get("projects").is_none()); }
        }
    }
}

#[gpui::test]
fn gui_and_terminal_index_settings_and_rebuild_are_accessible_through_f2(cx: &mut TestAppContext) {
    for dark in [true, false] {
        let scratch = tempfile::tempdir().unwrap();
        let root = scratch.path().join("project"); std::fs::create_dir(&root).unwrap();
        let path = root.join("a.rs"); let source = "struct Alpha;\n"; std::fs::write(&path, source).unwrap();
        let mut gc = config(&scratch.path().join("gui")); let mut tc = config(&scratch.path().join("tui"));
        gc.dark = dark; tc.dark = dark;
        let gs = gc.settings.clone(); let ts = tc.settings.clone();
        let mut core = Session::new(gc); core.dispatch(Command::OpenRoot(root.clone()));
        let gui = cx.new(|cx| ReaderView::new(core, Some(path.clone()), cx));
        let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| gui.clone())).unwrap();
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        let focus = gui.read_with(&visual, |v, _| v.focus_handle()); visual.update(|w, cx| focus.focus(w, cx));
        let mut tui = App::new(tc, Some(root.clone()));
        tui.session.dispatch(Command::Open(path.clone()));
        let key = |app: &mut App, code| { app.handle(Event::Key(KeyEvent::new(code, KeyModifiers::NONE))); };
        for (id, enabled) in [(None, true), (Some(CommandId::PersistentIndex), false), (Some(CommandId::PersistentIndex), true), (Some(CommandId::RebuildIndex), true)] {
            if let Some(id) = id {
                visual.simulate_keystrokes("f2 home"); key(&mut tui, KeyCode::F(2)); key(&mut tui, KeyCode::Home);
                for (width, height) in [(26, 8), (1, 1), (80, 24)] {
                    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap(); terminal.draw(|f| tui.draw(f)).unwrap();
                }
                for (width, height) in [(180., 90.), (1., 1.), (800., 400.)] {
                    visual.simulate_resize(gpui::size(gpui::px(width), gpui::px(height))); visual.update(|w, cx| { let _ = w.draw(cx); });
                }
                let at = tui.session.language_server_rows().iter().position(|(_, action)| *action == Some(id)).unwrap();
                for _ in 0..at { visual.simulate_keystrokes("down"); key(&mut tui, KeyCode::Down); }
                visual.simulate_keystrokes("enter"); key(&mut tui, KeyCode::Enter);
            }
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                gui.update(&mut visual, |v, cx| v.poll(cx));
                while let Ok(c) = tui.session.completions().try_recv() { tui.apply(c); }
                let done = |s: &Session| s.state().index_coverage().indexed == 1 && s.state().index_coverage().complete()
                    && s.actions().iter().find(|a| a.id == CommandId::PersistentIndex).and_then(|a| a.on) == Some(enabled)
                    && s.disabled(CommandId::RebuildIndex).is_none()
                    && (id != Some(CommandId::RebuildIndex) || s.state().notices().iter().any(|n| n.contains("Saved declarations cleared")));
                if gui.read_with(&visual, |v, _| done(v.session())) && done(&tui.session) { break; }
                assert!(Instant::now() < deadline, "index setting/rebuild did not settle: {:?}", tui.session.state().notices());
                std::thread::sleep(Duration::from_millis(2));
            }
            assert_eq!(gui.read_with(&visual, |v, _| snapshot(v.session().state())), snapshot(tui.session.state()));
            assert_eq!(std::fs::read_to_string(&path).unwrap(), source);
            if id.is_some() {
                let deadline = Instant::now() + Duration::from_secs(5);
                loop {
                    let saved = |p: &std::path::PathBuf| std::fs::read(p).ok().and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok()).is_some_and(|v| v["persistent_index"] == enabled);
                    if saved(&gs) && saved(&ts) { break; }
                    assert!(Instant::now() < deadline); std::thread::sleep(Duration::from_millis(2));
                }
            }
        }
    }
}

#[gpui::test]
fn gui_and_terminal_keys_share_selection_find_split_reload_and_modal_outcomes(
    cx: &mut TestAppContext,
) {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path().join("project");
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("a.txt");
    let original = "héllo\r\nneedle here\r\nother needle\r\n";
    std::fs::write(&path, original).unwrap();
    let gui = cx.new(|cx| {
        ReaderView::new(
            Session::new(config(&scratch.path().join("gui"))),
            Some(path.clone()),
            cx,
        )
    });
    let window = cx
        .update(|cx| cx.open_window(Default::default(), |_, _| gui.clone()))
        .unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let focus = gui.read_with(&visual, |v, _| v.focus_handle());
    visual.update(|w, cx| focus.focus(w, cx));
    let mut tui = App::new(config(&scratch.path().join("tui")), Some(path.clone()));
    // Match input capabilities explicitly; platform-default policy has separate constructor coverage.
    gui.update(&mut visual, |view, cx| { view.enable_draft_input_for_test(); cx.notify(); });
    tui.enable_draft_input_for_test();
    assert!(gui.read_with(&visual, |view, _| view.session().text_input_enabled()));
    assert!(tui.session.text_input_enabled());
    let key = |app: &mut App, code, modifiers| {
        app.handle(Event::Key(KeyEvent::new(code, modifiers)));
    };
    let settle = |visual: &mut VisualTestContext, tui: &mut App| {
        let until = Instant::now() + Duration::from_secs(10);
        loop {
            gui.update(visual, |v, cx| v.poll(cx));
            while let Ok(c) = tui.session.completions().try_recv() {
                tui.apply(c);
            }
            let ready = |s: &State| {
                s.loading().is_none()
                    && s.current_source().is_some()
                    && s.active()
                        .and_then(|t| t.find())
                        .is_none_or(|f| !f.pending())
            };
            if gui.read_with(visual, |v, _| ready(v.session().state()))
                && ready(tui.session.state())
            {
                break;
            }
            assert!(Instant::now() < until, "adapters did not settle");
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(
            gui.read_with(visual, |v, _| snapshot(v.session().state())),
            snapshot(tui.session.state())
        );
        assert_eq!(std::fs::read(&path).unwrap(), original.as_bytes());
    };
    settle(&mut visual, &mut tui);
    visual.simulate_keystrokes("ctrl-a");
    key(&mut tui, KeyCode::Char('a'), KeyModifiers::CONTROL);
    settle(&mut visual, &mut tui);
    assert_eq!(copied_selection(tui.session.state()).as_deref(), Some(original));

    // Both tab bars expose the same navigation, and actual right-clicks expose the same context actions.
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
    terminal.draw(|f| tui.draw(f)).unwrap();
    visual.update(|window, cx| {
        let _ = window.draw(cx);
    });
    let gui_nav: Vec<_> = gui.read_with(&visual, |v, _| {
        v.nav_buttons().iter().map(|b| b.2).collect()
    });
    assert_eq!(
        tui.layout
            .controls
            .iter()
            .filter(|(_, rect)| rect.y == 0)
            .map(|(id, _)| *id)
            .collect::<Vec<_>>(),
        gui_nav
    );
    visual.simulate_event(gpui::MouseDownEvent {
        button: gpui::MouseButton::Right,
        position: gpui::point(gpui::px(60.0), gpui::px(70.0)),
        modifiers: gpui::Modifiers::default(),
        click_count: 1,
        first_mouse: false,
    });
    let text = tui.layout.panes[0].text;
    tui.handle(Event::Mouse(crossterm::event::MouseEvent {
        kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Right),
        column: text.x + 1,
        row: text.y,
        modifiers: KeyModifiers::NONE,
    }));
    let gui_rows = gui.read_with(&visual, |v, _| {
        v.modal_rows()
            .into_iter()
            .filter(|r| r.0 != "—")
            .collect::<Vec<_>>()
    });
    assert!(!gui_rows.is_empty());
    assert_eq!(
        tui.choices()
            .into_iter()
            .map(|c| (c.label, c.disabled))
            .collect::<Vec<_>>(),
        gui_rows
    );
    visual.update(|window, cx| {
        let _ = window.draw(cx);
    });
    let (bounds, row) = gui.read_with(&visual, |v, _| {
        (
            v.context_bounds().unwrap(),
            v.context_row_y("Copy").unwrap(),
        )
    });
    visual.simulate_click(
        gpui::point(bounds.origin.x + gpui::px(5.0), gpui::px(row + 1.0)),
        gpui::Modifiers::default(),
    );
    terminal.draw(|f| tui.draw(f)).unwrap();
    let copy = tui
        .choices()
        .iter()
        .position(|c| c.label == "Copy")
        .unwrap();
    let row = tui
        .layout
        .modal_items
        .iter()
        .position(|i| *i == copy)
        .unwrap();
    tui.handle(Event::Mouse(crossterm::event::MouseEvent {
        kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
        column: tui.layout.modal_rows.x + 1,
        row: tui.layout.modal_rows.y + row as u16,
        modifiers: KeyModifiers::NONE,
    }));
    assert_eq!(
        visual
            .read_from_clipboard()
            .and_then(|c| c.text())
            .as_deref(),
        Some(original)
    );
    assert_eq!(tui.copy.take().as_deref(), Some(original));
    assert!(tui.modal.is_none());
    settle(&mut visual, &mut tui);

    visual.simulate_keystrokes("f2 ctrl-c");
    key(&mut tui, KeyCode::F(2), KeyModifiers::NONE);
    key(&mut tui, KeyCode::Char('c'), KeyModifiers::CONTROL);
    let gui_details = visual.read_from_clipboard().and_then(|c| c.text()).unwrap();
    let tui_details = tui.copy.take().unwrap();
    assert!(gui_details.contains("No language server supports this document"));
    assert_eq!(
        gui_details.split("\n\nProvider settings:").next(),
        tui_details.split("\n\nProvider settings:").next()
    );
    visual.simulate_keystrokes("escape");
    key(&mut tui, KeyCode::Esc, KeyModifiers::NONE);
    settle(&mut visual, &mut tui);

    visual.simulate_keystrokes("ctrl-f");
    key(&mut tui, KeyCode::Char('f'), KeyModifiers::CONTROL);
    for c in "needle".chars() {
        visual.simulate_keystrokes(&c.to_string());
        key(&mut tui, KeyCode::Char(c), KeyModifiers::NONE);
    }
    settle(&mut visual, &mut tui);
    assert_eq!(
        tui.session
            .state()
            .active()
            .unwrap()
            .find()
            .unwrap()
            .matches()
            .len(),
        2
    );
    visual.simulate_keystrokes("f3");
    key(&mut tui, KeyCode::F(3), KeyModifiers::NONE);
    settle(&mut visual, &mut tui);
    // GUI's find box and terminal find prompt dismiss through the shared close command.
    visual.simulate_keystrokes("escape");
    key(&mut tui, KeyCode::Esc, KeyModifiers::NONE);
    settle(&mut visual, &mut tui);

    visual.simulate_keystrokes("f10");
    key(&mut tui, KeyCode::F(10), KeyModifiers::NONE);
    let before = snapshot(tui.session.state());
    visual.simulate_keystrokes("ctrl-a f5");
    key(&mut tui, KeyCode::Char('a'), KeyModifiers::CONTROL);
    key(&mut tui, KeyCode::F(5), KeyModifiers::NONE);
    assert_eq!(snapshot(tui.session.state()), before);
    visual.simulate_keystrokes("escape");
    key(&mut tui, KeyCode::Esc, KeyModifiers::NONE);
    settle(&mut visual, &mut tui);

    for id in [
        CommandId::SplitRight,
        CommandId::MoveTabToOtherPane,
        CommandId::Unsplit,
        CommandId::SoftWrap,
        CommandId::Reload,
        CommandId::OpenInEditor,
        CommandId::OpenWithDefaultApp,
    ] {
        // Reach the same action with the same keys through both adapters' application menus.
        visual.simulate_click(
            gpui::point(gpui::px(12.0), gpui::px(10.0)),
            gpui::Modifiers::default(),
        );
        terminal.draw(|f| tui.draw(f)).unwrap();
        let button = tui.layout.app_menu;
        tui.handle(Event::Mouse(crossterm::event::MouseEvent {
            kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
            column: button.x + 1,
            row: button.y,
            modifiers: KeyModifiers::NONE,
        }));
        let group = reader_core::actions::CommandGroup::ALL
            .iter()
            .position(|g| *g == id.group())
            .unwrap();
        visual.simulate_keystrokes(&format!("{}enter", "down ".repeat(group)));
        for _ in 0..group {
            key(&mut tui, KeyCode::Down, KeyModifiers::NONE);
        }
        key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
        let gui_rows = gui.read_with(&visual, |v, _| v.modal_rows());
        let tui_rows: Vec<_> = tui.choices().into_iter().map(|c| (c.label, c.disabled)).collect();
        assert_eq!(tui_rows, gui_rows, "{id:?}");
        let row = CommandId::ALL
            .iter()
            .filter(|c| c.group() == id.group())
            .position(|c| *c == id)
            .unwrap();
        visual.simulate_keystrokes(&format!("{}enter", "down ".repeat(row)));
        for _ in 0..row {
            key(&mut tui, KeyCode::Down, KeyModifiers::NONE);
        }
        key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
        settle(&mut visual, &mut tui);
    }
    // Both handed the same editor and default-app launches over (recorded, not started).
    assert_eq!(tui.launched.len(), 2);
    assert_eq!(gui.read_with(&visual, |v, _| v.launched.clone()), tui.launched);
    // A pending navigation can be cancelled in either adapter without changing the source.
    tui.dispatch(Command::CancelSemantic);
    assert_eq!(std::fs::read(&path).unwrap(), original.as_bytes());
}

#[gpui::test]
#[cfg(unix)]
fn gui_and_tui_server_runtime_repairs_produce_the_same_settings(cx: &mut TestAppContext) {
    use std::os::unix::fs::PermissionsExt;
    for (name, source, runtime_name, flag, version, provider) in [
        (
            "Main.java",
            "class Main {}\n",
            "java",
            "-version",
            "",
            "jdtls",
        ),
        (
            "main.py",
            "def main(): pass\n",
            "node",
            "--version",
            "v26.8.1",
            "pyright",
        ),
    ] {
        let scratch = tempfile::tempdir().unwrap();
        let path = scratch.path().join(name);
        std::fs::write(&path, source).unwrap();
        let java = scratch
            .path()
            .join("runtime with spaces/bin")
            .join(runtime_name);
        std::fs::create_dir_all(java.parent().unwrap()).unwrap();
        std::fs::write(
            &java,
            format!("#!/bin/sh\ntest \"$1\" = {flag} || exit 2\nprintf '%s\\n' '{version}'\n"),
        )
        .unwrap();
        if provider == "pyright" {
            let server = java.parent().unwrap().join("pyright-langserver");
            std::fs::write(&server, "#!/bin/sh\nexit 1\n").unwrap();
            std::fs::set_permissions(&server, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        std::fs::set_permissions(&java, std::fs::Permissions::from_mode(0o755)).unwrap();
        let gui_config = config(&scratch.path().join("gui"));
        let gui_settings = gui_config.providers.clone();
        let gui = cx.new(|cx| ReaderView::new(Session::new(gui_config), Some(path.clone()), cx));
        let window = cx
            .update(|cx| cx.open_window(Default::default(), |_, _| gui.clone()))
            .unwrap();
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        let focus = gui.read_with(&visual, |v, _| v.focus_handle());
        visual.update(|w, cx| focus.focus(w, cx));
        let tui_config = config(&scratch.path().join("tui"));
        let tui_settings = tui_config.providers.clone();
        let mut tui = App::new(tui_config, Some(path.clone()));
        let key = |app: &mut App, code| {
            app.handle(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
        };
        let settle = |visual: &mut VisualTestContext, tui: &mut App, saved: bool| {
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                gui.update(visual, |v, cx| v.poll(cx));
                while let Ok(c) = tui.session.completions().try_recv() {
                    tui.apply(c);
                }
                let done = |s: &State| {
                    if saved {
                        s.server_runtime()
                            .is_some_and(|m| m.message.contains("saved."))
                    } else {
                        s.current_source().is_some()
                    }
                };
                if gui.read_with(visual, |v, _| done(v.session().state()))
                    && done(tui.session.state())
                {
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "{provider}: GUI {:?}; TUI {:?}",
                    gui.read_with(visual, |v, _| v
                        .session()
                        .state()
                        .server_runtime()
                        .map(|m| m.message.clone())),
                    tui.session.state().server_runtime().map(|m| &m.message)
                );
                std::thread::sleep(Duration::from_millis(2));
            }
            assert_eq!(
                gui.read_with(visual, |v, _| snapshot(v.session().state())),
                snapshot(tui.session.state())
            );
        };
        settle(&mut visual, &mut tui, false);
        visual.simulate_keystrokes("f2");
        key(&mut tui, KeyCode::F(2));
        let at = tui
            .session
            .language_server_rows()
            .iter()
            .position(|(_, action)| *action == Some(CommandId::ServerRuntime))
            .unwrap();
        for _ in 0..at {
            visual.simulate_keystrokes("down");
            key(&mut tui, KeyCode::Down);
        }
        visual.simulate_keystrokes("enter");
        key(&mut tui, KeyCode::Enter);
        visual.update(|_, cx| {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                java.to_str().unwrap().into(),
            ))
        });
        visual.simulate_keystrokes("ctrl-v enter");
        tui.handle(Event::Paste(java.to_str().unwrap().into()));
        key(&mut tui, KeyCode::Enter);
        settle(&mut visual, &mut tui, true);
        let read = |path| {
            serde_json::from_slice::<serde_json::Value>(&std::fs::read(path).unwrap()).unwrap()
        };
        let saved = read(gui_settings);
        assert_eq!(saved, read(tui_settings));
        if provider == "pyright" {
            assert_eq!(saved["providers"][provider]["node"], java.to_str().unwrap());
        }
        visual.simulate_keystrokes("escape");
        key(&mut tui, KeyCode::Esc);
        assert!(tui.session.state().server_runtime().is_none());
        assert!(gui.read_with(&visual, |v, _| {
            v.session().state().server_runtime().is_none()
        }));
        assert_eq!(std::fs::read_to_string(path).unwrap(), source);
    }
}

#[gpui::test]
fn workspace_reopens_from_tui_to_gui_and_back_with_real_input(cx: &mut TestAppContext) {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path().join("project");
    std::fs::create_dir_all(&root).unwrap();
    let source = (0..120)
        .map(|i| format!("line_{i:03} héllo workspace\n"))
        .collect::<String>();
    for name in ["a.txt", "b.txt"] {
        std::fs::write(root.join(name), &source).unwrap();
    }
    let base = scratch.path().join("preferences");
    let mut tui = App::new(config(&base), Some(root.clone()));
    let key = |app: &mut App, code, modifiers| {
        app.handle(Event::Key(KeyEvent::new(code, modifiers)));
    };
    let wait = |app: &mut App, done: &dyn Fn(&State) -> bool| {
        let end = Instant::now() + Duration::from_secs(10);
        while !done(app.session.state()) {
            assert!(
                Instant::now() < end,
                "workspace wait: {:?}, {:?}, {:?}",
                app.session.state().loading(),
                app.session.state().notices(),
                app.session.state().current_source().map(|d| d.source())
            );
            if let Ok(c) = app.session.completions().try_recv() {
                app.apply(c);
            } else {
                std::thread::yield_now();
            }
        }
    };
    wait(&mut tui, &|s| {
        s.project()
            .is_some_and(|p| matches!(p.coverage(), reader_core::project::Coverage::Complete(_)))
    });
    let open = |app: &mut App, name: &str| {
        key(app, KeyCode::Char('p'), KeyModifiers::CONTROL);
        app.handle(Event::Paste(name.into()));
        wait(app, &|s| {
            s.navigation()
                .is_some_and(|n| !n.pending && !n.rows.is_empty())
        });
        key(app, KeyCode::Enter, KeyModifiers::NONE);
        wait(app, &|s| {
            s.current_source().is_some_and(|d| d.source().ends_with(name))
        });
    };
    open(&mut tui, "a.txt");
    key(&mut tui, KeyCode::Char('g'), KeyModifiers::CONTROL);
    tui.handle(Event::Paste("42:3".into()));
    key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    key(&mut tui, KeyCode::Char('\\'), KeyModifiers::CONTROL);
    key(&mut tui, KeyCode::Char('2'), KeyModifiers::CONTROL);
    open(&mut tui, "b.txt");
    key(&mut tui, KeyCode::Char('g'), KeyModifiers::CONTROL);
    tui.handle(Event::Paste("65:4".into()));
    key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    key(&mut tui, KeyCode::Char('b'), KeyModifiers::CONTROL);
    let expected = snapshot(tui.session.state());
    assert!(!tui.tree_shown);
    drop(tui);
    let gui = cx.new(|cx| ReaderView::new(Session::new(config(&base)), Some(root.clone()), cx));
    let window = cx
        .update(|cx| cx.open_window(Default::default(), |_, _| gui.clone()))
        .unwrap();
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        cx.run_until_parked();
        gui.update(cx, |r, cx| r.poll(cx));
        if gui.read_with(cx, |r, _| {
            r.session()
                .state()
                .current_source()
                .is_some_and(|d| d.source().ends_with("b.txt"))
                && r.session()
                    .state()
                    .shown(0)
                    .is_some_and(|t| t.loaded())
        }) {
            break;
        }
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(
        gui.read_with(cx, |r, _| snapshot(r.session().state())),
        expected
    );
    assert!(!gui.read_with(cx, |r, _| r.session().state().explorer().visible));
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let focus = gui.read_with(&visual, |r, _| r.focus_handle());
    visual.update(|w, cx| focus.focus(w, cx));
    visual.simulate_keystrokes("ctrl-b ctrl-1 ctrl-g 7 enter");
    let changed = gui.read_with(&visual, |r, _| snapshot(r.session().state()));
    assert_eq!(changed.active.as_deref(), Some("a.txt"));
    assert!(gui.read_with(&visual, |r, _| r.session().state().explorer().visible));
    visual.update(|w, cx| {
        let _ = w.draw(cx);
    });
    visual.update(|w, _| w.remove_window());
    drop(visual);
    drop(gui);
    cx.run_until_parked();
    let saved_file = reader_core::places::file_for(&base.join("state/projects"), &root);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&saved_file).unwrap()).unwrap();
        if saved["workspace"]["focused"] == 0 && saved["workspace"]["explorer"]["visible"] == true {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "closing the GUI did not persist its workspace: {saved}"
        );
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(2));
    }
    let mut tui = App::new(config(&base), Some(root.clone()));
    wait(&mut tui, &|s| {
        s.current_source().is_some_and(|d| d.source().ends_with("a.txt"))
    });
    assert_eq!(snapshot(tui.session.state()), changed);
    assert!(tui.tree_shown);
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(26, 8)).unwrap();
    terminal.draw(|f| tui.draw(f)).unwrap();
    for name in ["a.txt", "b.txt"] {
        assert_eq!(std::fs::read_to_string(root.join(name)).unwrap(), source);
    }
}

#[gpui::test]
fn unified_name_search_replays_files_symbols_actions_caret_and_recents_in_both_adapters(
    cx: &mut TestAppContext,
) {
    use reader_core::navigation::{Kind, Scope};
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path().join("project");
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("wrap.rs");
    let source = "// fixture\npub fn wrap() {}\n";
    std::fs::write(&path, source).unwrap();
    let other = root.join("other.txt");
    std::fs::write(&other, "other\n").unwrap();
    let gui = cx.new(|cx| {
        ReaderView::new(
            Session::new(config(&scratch.path().join("gui"))),
            Some(root.clone()),
            cx,
        )
    });
    let window = cx
        .update(|cx| cx.open_window(Default::default(), |_, _| gui.clone()))
        .unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let focus = gui.read_with(&visual, |v, _| v.focus_handle());
    visual.update(|w, cx| focus.focus(w, cx));
    let mut tui = App::new(config(&scratch.path().join("tui")), Some(root.clone()));
    let key = |app: &mut App, code, modifiers| {
        app.handle(Event::Key(KeyEvent::new(code, modifiers)));
    };
    let ready = |s: &State| {
        s.project()
            .is_some_and(|p| matches!(p.coverage(), reader_core::project::Coverage::Complete(_)))
            && s.index_coverage().complete()
            && s.navigation().is_none_or(|n| !n.pending)
            && s.loading().is_none()
    };
    let settle = |visual: &mut VisualTestContext, tui: &mut App| {
        let until = Instant::now() + Duration::from_secs(10);
        loop {
            gui.update(visual, |r, cx| r.poll(cx));
            while let Ok(c) = tui.session.completions().try_recv() {
                tui.apply(c);
            }
            if gui.read_with(visual, |r, _| ready(r.session().state()))
                && ready(tui.session.state())
            {
                break;
            }
            assert!(Instant::now() < until, "navigation adapters did not settle");
            std::thread::yield_now();
        }
        visual.update(|w, cx| {
            let _ = w.draw(cx);
        });
        let rows = |s: &State| {
            s.navigation().map(|n| {
                (
                    n.scope,
                    n.query.clone(),
                    n.rows
                        .iter()
                        .map(|r| (r.kind, r.label.clone(), r.detail.clone(), r.disabled, r.on))
                        .collect::<Vec<_>>(),
                )
            })
        };
        assert_eq!(
            gui.read_with(visual, |r, _| rows(r.session().state())),
            rows(tui.session.state())
        );
        assert_eq!(
            gui.read_with(visual, |r, _| snapshot(r.session().state())),
            snapshot(tui.session.state())
        );
        assert_eq!(std::fs::read(&path).unwrap(), source.as_bytes());
        assert_eq!(std::fs::read(&other).unwrap(), b"other\n");
    };
    settle(&mut visual, &mut tui);
    visual.simulate_keystrokes("ctrl-k w r a p");
    key(&mut tui, KeyCode::Char('k'), KeyModifiers::CONTROL);
    tui.handle(Event::Paste("wrap".into()));
    settle(&mut visual, &mut tui);
    assert_eq!(tui.session.state().navigation().unwrap().scope, Scope::All);
    for kind in [Kind::File, Kind::Symbol, Kind::Action] {
        assert!(
            tui.session
                .state()
                .navigation()
                .unwrap()
                .rows
                .iter()
                .any(|r| r.kind == kind)
        );
    }
    for scope in [Scope::Files, Scope::Symbols, Scope::Actions, Scope::All] {
        visual.simulate_keystrokes("tab");
        key(&mut tui, KeyCode::Tab, KeyModifiers::NONE);
        settle(&mut visual, &mut tui);
        assert_eq!(tui.session.state().navigation().unwrap().scope, scope);
        assert_eq!(tui.session.state().navigation().unwrap().query, "wrap");
    }
    visual.simulate_keystrokes("escape ctrl-p w r a p");
    key(&mut tui, KeyCode::Esc, KeyModifiers::NONE);
    key(&mut tui, KeyCode::Char('p'), KeyModifiers::CONTROL);
    tui.handle(Event::Paste("wrap".into()));
    // Enter accepts only a completed row; wait for both scopes before opening it.
    settle(&mut visual, &mut tui);
    visual.simulate_keystrokes("enter");
    key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    settle(&mut visual, &mut tui);
    assert!(tui.session.state().current_source().is_some());
    visual.simulate_keystrokes("ctrl-alt-shift-n w r a p");
    key(
        &mut tui,
        KeyCode::Char('n'),
        KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SHIFT,
    );
    tui.handle(Event::Paste("wrap".into()));
    settle(&mut visual, &mut tui);
    visual.simulate_keystrokes("enter");
    key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    settle(&mut visual, &mut tui);
    assert_eq!(
        tui.session.state().selection().unwrap().head,
        "// fixture\n".len()
    );
    // Actual plain arrows collapse the caret in both adapters, including vertical movement.
    for (gui_key, code, head) in [
        ("right", KeyCode::Right, 12),
        ("left", KeyCode::Left, 11),
        ("up", KeyCode::Up, 0),
        ("down", KeyCode::Down, 11),
    ] {
        visual.simulate_keystrokes(gui_key);
        key(&mut tui, code, KeyModifiers::NONE);
        settle(&mut visual, &mut tui);
        assert_eq!(
            tui.session.state().selection(),
            Some(reader_core::session::Selection { anchor: head, head })
        );
    }
    visual.simulate_keystrokes("ctrl-shift-a s o f t space w r a p");
    key(
        &mut tui,
        KeyCode::Char('a'),
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    );
    tui.handle(Event::Paste("soft wrap".into()));
    settle(&mut visual, &mut tui);
    visual.simulate_keystrokes("enter");
    key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    settle(&mut visual, &mut tui);
    assert!(tui.session.state().soft_wrap());

    // Keep each tab so Recent Files must restore its existing caret instead of resetting it.
    visual.simulate_keystrokes("ctrl-shift-k right ctrl-p o t h e r");
    key(
        &mut tui,
        KeyCode::Char('k'),
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    );
    key(&mut tui, KeyCode::Right, KeyModifiers::NONE);
    key(&mut tui, KeyCode::Char('p'), KeyModifiers::CONTROL);
    tui.handle(Event::Paste("other".into()));
    settle(&mut visual, &mut tui);
    visual.simulate_keystrokes("enter");
    key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    settle(&mut visual, &mut tui);
    assert_eq!(tui.session.state().active().unwrap().source(), other);
    visual.simulate_keystrokes("ctrl-shift-k right right ctrl-e");
    key(
        &mut tui,
        KeyCode::Char('k'),
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    );
    key(&mut tui, KeyCode::Right, KeyModifiers::NONE);
    key(&mut tui, KeyCode::Right, KeyModifiers::NONE);
    key(&mut tui, KeyCode::Char('e'), KeyModifiers::CONTROL);
    settle(&mut visual, &mut tui);
    assert_eq!(
        tui.session.state().navigation().unwrap().scope,
        Scope::Recent
    );
    assert_eq!(
        tui.session.state().navigation().unwrap().rows[0].label,
        "wrap.rs"
    );
    let selection = tui.session.state().selection();
    let top = tui.session.state().top_line();
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(40, 10)).unwrap();
    terminal.draw(|f| tui.draw(f)).unwrap();
    visual.simulate_event(gpui::ScrollWheelEvent {
        position: gpui::point(gpui::px(400.0), gpui::px(200.0)),
        delta: gpui::ScrollDelta::Lines(gpui::point(0.0, -1.0)),
        ..Default::default()
    });
    tui.handle(Event::Mouse(crossterm::event::MouseEvent {
        kind: crossterm::event::MouseEventKind::ScrollDown,
        column: 0,
        row: 0,
        modifiers: KeyModifiers::NONE,
    }));
    settle(&mut visual, &mut tui);
    assert_eq!(tui.session.state().selection(), selection);
    assert_eq!(tui.session.state().top_line(), top);
    visual.simulate_keystrokes("enter");
    key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    settle(&mut visual, &mut tui);
    assert_eq!(tui.session.state().active().unwrap().source(), path);
    assert_eq!(
        tui.session.state().selection(),
        Some(reader_core::session::Selection {
            anchor: 12,
            head: 12
        })
    );

    visual.simulate_keystrokes("ctrl-e o h r");
    key(&mut tui, KeyCode::Char('e'), KeyModifiers::CONTROL);
    tui.handle(Event::Paste("ohr".into()));
    settle(&mut visual, &mut tui);
    assert_eq!(tui.session.state().navigation().unwrap().rows.len(), 1);
    assert_eq!(
        tui.session.state().navigation().unwrap().rows[0].label,
        "other.txt"
    );
    terminal.draw(|f| tui.draw(f)).unwrap();
    visual.simulate_keystrokes("delete");
    key(&mut tui, KeyCode::Delete, KeyModifiers::NONE);
    settle(&mut visual, &mut tui);
    assert!(tui.session.state().navigation().unwrap().rows.is_empty());
    visual.simulate_keystrokes("escape ctrl-e");
    key(&mut tui, KeyCode::Esc, KeyModifiers::NONE);
    key(&mut tui, KeyCode::Char('e'), KeyModifiers::CONTROL);
    settle(&mut visual, &mut tui);
    assert_eq!(tui.session.state().navigation().unwrap().rows.len(), 1);
    assert_eq!(
        tui.session.state().navigation().unwrap().rows[0].label,
        "wrap.rs"
    );
}

#[gpui::test]
fn dependency_definitions_reveal_browsable_libraries_in_both_explorers(cx: &mut TestAppContext) {
    use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    use reader_core::{project::Coverage, semantics::Status};
    let profile = std::env::current_exe().unwrap().parent().unwrap().parent().unwrap().to_owned();
    assert!(std::process::Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .args(["build", "-q", "--locked", "-p", "reader-core", "--example", "fake_lsp"])
        .current_dir(env!("CARGO_MANIFEST_DIR")).status().unwrap().success());
    let fake = profile.join("examples/fake_lsp");
    for dark in [true, false] {
        for folder in [true, false] {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().join("project");
            let dep = dir.path().join(".cargo/registry/src/index/dep-1.0");
            let main = root.join("main.rs");
            let target = dep.join("src/lib.rs");
            let sibling = dep.join("src/sibling.rs");
            let files = [
                (root.join("Cargo.toml"), "[package]\nname='test'\nversion='0.1.0'\n"),
                (main.clone(), "fn main() { from_dep(); }\n"),
                (target.clone(), "pub fn from_dep() {}\n"),
                (sibling.clone(), "pub fn sibling() {}\n"),
                (dep.join("unopened/nested.rs"), "pub fn nested() {}\n"),
            ];
            for (path, text) in &files { std::fs::create_dir_all(path.parent().unwrap()).unwrap(); std::fs::write(path, text).unwrap(); }
            let make = |name| {
                let mut c = config(&dir.path().join(name)); c.dark = dark;
                let mut settings: serde_json::Value = serde_json::from_slice(&std::fs::read(&c.providers).unwrap()).unwrap();
                settings["providers"]["rust-analyzer"] = serde_json::json!({"executable":fake,"env":{"FAKE_LSP_EXTRA":dep}});
                std::fs::write(&c.providers, settings.to_string()).unwrap(); c
            };
            let path = if folder { root.clone() } else { main.clone() };
            let gui = cx.new(|cx| ReaderView::new(Session::new(make("gui")), Some(path.clone()), cx));
            let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| gui.clone())).unwrap();
            let mut visual = VisualTestContext::from_window(window.into(), cx);
            let focus = gui.read_with(&visual, |v, _| v.focus_handle());
            visual.update(|w, cx| focus.focus(w, cx));
            let mut tui = App::new(make("tui"), Some(path));
            let settle = |visual: &mut VisualTestContext, tui: &mut App, ready: &dyn Fn(&State) -> bool| {
                let until = Instant::now() + Duration::from_secs(10);
                loop {
                    gui.update(visual, |v, cx| v.poll(cx));
                    while let Ok(c) = tui.session.completions().try_recv() { tui.apply(c); }
                    if gui.read_with(visual, |v, _| ready(v.session().state())) && ready(tui.session.state()) { break; }
                    assert!(Instant::now() < until, "{:?}", tui.session.state().notices());
                    std::thread::sleep(Duration::from_millis(2));
                }
            };
            if folder {
                settle(&mut visual, &mut tui, &|s| s.project().is_some_and(|p| matches!(p.coverage(), Coverage::Complete(_))));
                visual.simulate_keystrokes("ctrl-shift-e end enter");
                for (key, modifiers) in [(KeyCode::Char('e'), KeyModifiers::CONTROL | KeyModifiers::SHIFT), (KeyCode::End, KeyModifiers::NONE), (KeyCode::Enter, KeyModifiers::NONE)] { tui.handle(Event::Key(KeyEvent::new(key, modifiers))); }
            }
            settle(&mut visual, &mut tui, &|s| s.provider().is_some_and(|p| p.status == Status::Ready));
            visual.simulate_keystrokes("ctrl-home");
            tui.handle(Event::Key(KeyEvent::new(KeyCode::Home, KeyModifiers::CONTROL)));
            for _ in 0..13 { visual.simulate_keystrokes("right"); tui.handle(Event::Key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE))); }
            visual.simulate_keystrokes("f12");
            tui.handle(Event::Key(KeyEvent::new(KeyCode::F(12), KeyModifiers::NONE)));
            settle(&mut visual, &mut tui, &|s| s.current_source().is_some_and(|d| d.source() == target) && s.tree_rows().any(|r| r.name.as_ref() == "sibling.rs") && s.tree_rows().all(|r| !r.name.contains("loading")));
            let tree = |s: &State| s.tree_rows().map(|r| (r.depth, r.path.to_string(), r.name.to_string(), r.dir)).collect::<Vec<_>>();
            assert_eq!(gui.read_with(&visual, |v, _| tree(v.session().state())), tree(tui.session.state()));
            assert_eq!(tui.session.state().project().map(|p| p.root()), folder.then_some(root.as_path()));
            assert_eq!(tui.session.state().tree_rows().nth(tui.tree_at).unwrap().path.as_ref(), target.to_str().unwrap());
            assert_eq!(gui.read_with(&visual, |v, _| v.tree_sel.clone()).unwrap().as_ref(), target.to_str().unwrap());
            assert!(!tui.session.state().tree_rows().any(|r| r.name.as_ref() == "nested.rs"));
            let draw = |visual: &mut VisualTestContext, tui: &mut App, width, height, cols, rows| {
                visual.simulate_resize(gpui::size(gpui::px(width), gpui::px(height)));
                visual.update(|w, cx| { let _ = w.draw(cx); });
                let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(cols, rows)).unwrap();
                term.draw(|f| tui.draw(f)).unwrap();
                term.backend().buffer().content.iter().map(|c| c.symbol()).collect::<String>()
            };
            let rendered = draw(&mut visual, &mut tui, 900., 500., 100, 30);
            assert!(rendered.contains("Libraries") && rendered.contains("dep-1.0"));
            assert!(gui.read_with(&visual, |v, _| v.tree_lines(v.tree_at()).iter().any(|r| r.name == "Libraries")));
            let select = |visual: &mut VisualTestContext, tui: &mut App, path: &Path| {
                let at = tui.session.state().tree_rows().position(|r| r.path.as_ref() == path.to_str().unwrap()).unwrap();
                visual.simulate_keystrokes("ctrl-shift-e home");
                tui.handle(Event::Key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL | KeyModifiers::SHIFT)));
                tui.handle(Event::Key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE)));
                for _ in 0..at { visual.simulate_keystrokes("down"); tui.handle(Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))); }
            };
            select(&mut visual, &mut tui, &dep);
            visual.simulate_keystrokes("left right");
            for key in [KeyCode::Left, KeyCode::Right] { tui.handle(Event::Key(KeyEvent::new(key, KeyModifiers::NONE))); }
            settle(&mut visual, &mut tui, &|s| s.tree_rows().any(|r| r.name.as_ref() == "unopened"));
            select(&mut visual, &mut tui, &dep.join("unopened"));
            visual.simulate_keystrokes("right");
            tui.handle(Event::Key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE)));
            settle(&mut visual, &mut tui, &|s| s.tree_rows().any(|r| r.name.as_ref() == "nested.rs"));
            select(&mut visual, &mut tui, &dep.join("unopened/nested.rs"));
            visual.simulate_keystrokes("enter");
            tui.handle(Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)));
            settle(&mut visual, &mut tui, &|s| s.current_source().is_some_and(|d| d.source().ends_with("nested.rs")));
            assert_eq!(gui.read_with(&visual, |v, _| snapshot(v.session().state())), snapshot(tui.session.state()));
            // A menu owns input, including clicks on the explorer behind it.
            draw(&mut visual, &mut tui, 900., 500., 100, 30);
            visual.simulate_keystrokes("f2");
            tui.handle(Event::Key(KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE)));
            visual.simulate_click(gpui::point(gpui::px(30.), gpui::px(60.)), gpui::Modifiers::default());
            tui.handle(Event::Mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: tui.layout.tree.x + 1, row: tui.layout.tree.y, modifiers: KeyModifiers::NONE }));
            assert!(tui.session.state().current_source().unwrap().source().ends_with("nested.rs"));
            assert!(gui.read_with(&visual, |v, _| v.session().state().current_source().unwrap().source().ends_with("nested.rs")));
            visual.simulate_keystrokes("escape ctrl-shift-e end up");
            for (key, modifiers) in [(KeyCode::Esc, KeyModifiers::NONE), (KeyCode::Char('e'), KeyModifiers::CONTROL | KeyModifiers::SHIFT), (KeyCode::End, KeyModifiers::NONE), (KeyCode::Up, KeyModifiers::NONE)] { tui.handle(Event::Key(KeyEvent::new(key, modifiers))); }
            for (w, h, cols, rows) in [(280., 180., 35, 8), (70., 55., 8, 3), (900., 500., 100, 30)] { draw(&mut visual, &mut tui, w, h, cols, rows); }
            for (path, text) in &files { assert_eq!(std::fs::read_to_string(path).unwrap(), *text); }
        }
    }
}

#[gpui::test]
fn gui_and_terminal_double_and_triple_clicks_select_the_same_word_and_group(cx: &mut TestAppContext) {
    use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.rs");
    let src = "fn f() { g(\"hello world\", 1); }\n";
    std::fs::write(&path, src).unwrap();
    let gui = cx.new(|cx| ReaderView::new(Session::new(config(&dir.path().join("gui"))), Some(path.clone()), cx));
    let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| gui.clone())).unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let mut tui = App::new(config(&dir.path().join("tui")), Some(path.clone()));
    visual.simulate_resize(gpui::size(gpui::px(800.0), gpui::px(400.0)));
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 20)).unwrap();
    let mut settle = |visual: &mut VisualTestContext, tui: &mut App, done: &dyn Fn(&State) -> bool| {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            gui.update(visual, |v, cx| v.poll(cx));
            while let Ok(c) = tui.session.completions().try_recv() { tui.apply(c); }
            // Completion handling can invalidate source geometry (for example, late declarations).
            // Replay the event loop's render phase before accepting another click or a settled outcome.
            visual.update(|w, cx| { let _ = w.draw(cx); });
            terminal.draw(|f| tui.draw(f)).unwrap();
            if gui.read_with(visual, |v, _| done(v.session().state())) && done(tui.session.state()) { break; }
            assert!(Instant::now() < deadline, "adapters did not settle: GUI selection {:?}, TUI selection {:?}",
                gui.read_with(visual, |v, _| copied_selection(v.session().state())),
                copied_selection(tui.session.state()).as_deref());
            std::thread::sleep(Duration::from_millis(2));
        }
    };
    settle(&mut visual, &mut tui, &|s| s.current_source().is_some());
    let at = src.find("world").unwrap() + 2;
    let (gx, gy) = gui.read_with(&visual, |v, _| v.text_point_of(at)).unwrap();
    let text = tui.layout.panes[0].text;
    let selected = |s: &State| copied_selection(s).unwrap_or_default();
    for clicks in 1..=3 {
        visual.simulate_event(gpui::MouseDownEvent { button: gpui::MouseButton::Left, position: gpui::point(gpui::px(gx), gpui::px(gy)), modifiers: gpui::Modifiers::default(), click_count: clicks, first_mouse: false });
        visual.simulate_event(gpui::MouseUpEvent { button: gpui::MouseButton::Left, position: gpui::point(gpui::px(gx), gpui::px(gy)), modifiers: gpui::Modifiers::default(), click_count: clicks });
        tui.handle(Event::Mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: text.x + at as u16, row: text.y, modifiers: KeyModifiers::NONE }));
        let want = ["", "world", "hello world"][clicks - 1];
        settle(&mut visual, &mut tui, &|s| selected(s) == want);
        assert_eq!(gui.read_with(&visual, |v, _| snapshot(v.session().state())), snapshot(tui.session.state()), "{clicks} clicks");
    }
    assert_eq!(std::fs::read_to_string(&path).unwrap(), src);
}

#[gpui::test]
fn gui_and_terminal_blame_through_their_menus_show_the_same_commit(cx: &mut TestAppContext) {
    use lenz_content::git::testing::{commit_all, git, repo, write};
    let r = repo();
    write(r.path(), "a.txt", "one\ntwo\n");
    commit_all(r.path(), "First line\n\nThe body.");
    let sha = git(r.path(), &["rev-parse", "HEAD"]).trim().to_string();
    let path = r.path().join("a.txt");
    let state = tempfile::tempdir().unwrap();
    let gui = cx.new(|cx| ReaderView::new(Session::new(config(&state.path().join("gui"))), Some(path.clone()), cx));
    let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| gui.clone())).unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let focus = gui.read_with(&visual, |v, _| v.focus_handle());
    visual.update(|w, cx| focus.focus(w, cx));
    let mut tui = App::new(config(&state.path().join("tui")), Some(path.clone()));
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 24)).unwrap();
    let key = |app: &mut App, code, modifiers| { app.handle(Event::Key(KeyEvent::new(code, modifiers))); };
    let settle = |visual: &mut VisualTestContext, tui: &mut App, done: &dyn Fn(&Session) -> bool| {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            gui.update(visual, |v, cx| v.poll(cx));
            while let Ok(c) = tui.session.completions().try_recv() { tui.apply(c); }
            if gui.read_with(visual, |v, _| done(v.session())) && done(&tui.session) { break; }
            assert!(Instant::now() < deadline, "adapters did not settle: gui {} / tui {}", gui.read_with(visual, |v, _| { let s = v.session(); format!("{:?} {:?} {}", s.state().active().map(|t| (t.annotation_cols(), t.annotation(0).map(|a| a.label), s.state().view(t.id()).map(|view| view.line_count()))), s.disabled(CommandId::AnnotateBlame), s.status_message()) }), { let s = &tui.session; format!("{:?} {:?} {}", s.state().active().map(|t| (t.annotation_cols(), t.annotation(0).map(|a| a.label), s.state().view(t.id()).map(|view| view.line_count()))), s.disabled(CommandId::AnnotateBlame), s.status_message()) });
            std::thread::sleep(Duration::from_millis(2));
        }
    };
    settle(&mut visual, &mut tui, &|s| s.state().current_source().is_some());
    visual.update(|w, cx| { let _ = w.draw(cx); });
    terminal.draw(|f| tui.draw(f)).unwrap();
    // The same command from each adapter's application menu: its group, then its row.
    let menu = |visual: &mut VisualTestContext, tui: &mut App, terminal: &mut ratatui::Terminal<ratatui::backend::TestBackend>, id: CommandId| {
        visual.simulate_click(gpui::point(gpui::px(12.0), gpui::px(10.0)), gpui::Modifiers::default());
        terminal.draw(|f| tui.draw(f)).unwrap();
        let button = tui.layout.app_menu;
        tui.handle(Event::Mouse(crossterm::event::MouseEvent { kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left), column: button.x + 1, row: button.y, modifiers: KeyModifiers::NONE }));
        let group = reader_core::actions::CommandGroup::ALL.iter().position(|g| *g == id.group()).unwrap();
        let row = CommandId::ALL.iter().filter(|c| c.group() == id.group()).position(|c| *c == id).unwrap();
        visual.simulate_keystrokes(&format!("{}enter {}enter", "down ".repeat(group), "down ".repeat(row)));
        for n in [group, row] {
            for _ in 0..n { key(tui, KeyCode::Down, KeyModifiers::NONE); }
            key(tui, KeyCode::Enter, KeyModifiers::NONE);
        }
    };
    menu(&mut visual, &mut tui, &mut terminal, CommandId::AnnotateBlame);
    settle(&mut visual, &mut tui, &|s| s.state().active().is_some_and(|t| t.annotation(1).is_some()));
    let labels = |s: &Session| (0..2).map(|l| s.annotation(s.state().active().unwrap().id(), l).unwrap().label).collect::<Vec<_>>();
    assert_eq!(gui.read_with(&visual, |v, _| labels(v.session())), labels(&tui.session));
    // Show Commit… on the caret's line (the first): the same popup text, and Copy Revision copies the same id.
    visual.simulate_keystrokes("down up");
    for code in [KeyCode::Down, KeyCode::Up] { key(&mut tui, code, KeyModifiers::NONE); }
    assert_eq!(gui.read_with(&visual, |v, _| v.session().state().selection()), tui.session.state().selection());
    assert_eq!(tui.session.state().selection().map(|s| s.head), Some(0));
    menu(&mut visual, &mut tui, &mut terminal, CommandId::ShowCommit);
    settle(&mut visual, &mut tui, &|s| s.state().commit_popup().is_some_and(|c| c.details.is_some()));
    visual.update(|w, cx| { let _ = w.draw(cx); });
    terminal.draw(|f| tui.draw(f)).unwrap();
    let text = tui.session.commit_text().unwrap();
    assert!(text.starts_with(&format!("commit {sha}")), "{text}");
    assert_eq!(gui.read_with(&visual, |v, _| v.session().commit_text()), Some(text));
    visual.simulate_keystrokes("ctrl-c");
    key(&mut tui, KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert_eq!(visual.update(|_, cx| cx.read_from_clipboard().and_then(|c| c.text())), Some(sha.clone()));
    assert_eq!(tui.copy.take(), Some(sha.clone()));
    visual.simulate_keystrokes("escape");
    key(&mut tui, KeyCode::Esc, KeyModifiers::NONE);
    settle(&mut visual, &mut tui, &|s| s.state().commit_popup().is_none());
    // Show Diff: neither finds Lenz Diff beside the test binary, and both say so the same way.
    menu(&mut visual, &mut tui, &mut terminal, CommandId::ShowCommitDiff);
    let said = tui.session.status_message();
    assert!(said.starts_with("Show Diff: Lenz Diff is not beside Lenz Code"), "{said}");
    assert_eq!(gui.read_with(&visual, |v, _| v.session().status_message()), said.replace("lenz-diff-tui", "lenz-diff"));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "one\ntwo\n");
}

#[gpui::test]
fn gui_and_terminal_git_branches_list_the_same_rows_and_check_out_the_same_way(cx: &mut TestAppContext) {
    use lenz_content::git::testing::{commit_all, git, repo, write};
    // Two identical repositories: one per adapter (a checkout in one must not change the other's).
    let make = || {
        let r = repo();
        git(r.path(), &["config", "user.name", "t"]);
        git(r.path(), &["config", "user.email", "t@t"]);
        write(r.path(), "a.txt", "main\n");
        commit_all(r.path(), "main");
        git(r.path(), &["tag", "v1"]);
        git(r.path(), &["switch", "-q", "-c", "other"]);
        write(r.path(), "a.txt", "other\n");
        commit_all(r.path(), "other");
        git(r.path(), &["switch", "-q", "main"]);
        r
    };
    let (gr, tr) = (make(), make());
    let state = tempfile::tempdir().unwrap();
    let gui = cx.new(|cx| ReaderView::new(Session::new(config(&state.path().join("gui"))), Some(gr.path().to_path_buf()), cx));
    let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| gui.clone())).unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let focus = gui.read_with(&visual, |v, _| v.focus_handle());
    visual.update(|w, cx| focus.focus(w, cx));
    let mut tui = App::new(config(&state.path().join("tui")), Some(tr.path().to_path_buf()));
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 24)).unwrap();
    let key = |app: &mut App, code, modifiers| { app.handle(Event::Key(KeyEvent::new(code, modifiers))); };
    let settle = |visual: &mut VisualTestContext, tui: &mut App, done: &dyn Fn(&Session) -> bool| {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            gui.update(visual, |v, cx| v.poll(cx));
            while let Ok(c) = tui.session.completions().try_recv() { tui.apply(c); }
            if gui.read_with(visual, |v, _| done(v.session())) && done(&tui.session) { break; }
            assert!(Instant::now() < deadline, "adapters did not settle");
            std::thread::sleep(Duration::from_millis(2));
        }
    };
    settle(&mut visual, &mut tui, &|s| s.branch_label().is_some());
    visual.update(|w, cx| { let _ = w.draw(cx); });
    terminal.draw(|f| tui.draw(f)).unwrap();
    // Git Branches… from each application menu.
    visual.simulate_click(gpui::point(gpui::px(12.0), gpui::px(10.0)), gpui::Modifiers::default());
    let button = tui.layout.app_menu;
    tui.handle(Event::Mouse(crossterm::event::MouseEvent { kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left), column: button.x + 1, row: button.y, modifiers: KeyModifiers::NONE }));
    let group = reader_core::actions::CommandGroup::ALL.iter().position(|g| *g == reader_core::actions::CommandGroup::Git).unwrap();
    let row = CommandId::ALL.iter().filter(|c| c.group() == reader_core::actions::CommandGroup::Git).position(|c| *c == CommandId::GitBranches).unwrap();
    visual.simulate_keystrokes(&format!("{}enter {}enter", "down ".repeat(group), "down ".repeat(row)));
    for n in [group, row] {
        for _ in 0..n { key(&mut tui, KeyCode::Down, KeyModifiers::NONE); }
        key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    }
    settle(&mut visual, &mut tui, &|s| s.state().branches().is_some_and(|b| !b.loading));
    visual.update(|w, cx| { let _ = w.draw(cx); });
    terminal.draw(|f| tui.draw(f)).unwrap();
    let gui_rows: Vec<String> = gui.read_with(&visual, |v, _| v.modal_rows().into_iter().skip(2).map(|r| r.0).collect());
    let tui_rows: Vec<String> = tui.choices().into_iter().map(|c| c.label).collect();
    assert_eq!(gui_rows, tui_rows);
    assert!(tui_rows.iter().any(|r| r.contains("● main")) && tui_rows.iter().any(|r| r.ends_with("v1")));
    // The same keys check out `other` in both.
    visual.simulate_keystrokes("o t h e r enter");
    for c in "other".chars() { key(&mut tui, KeyCode::Char(c), KeyModifiers::NONE); }
    key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    settle(&mut visual, &mut tui, &|s| !s.state().checking_out() && s.branch_label().as_deref() == Some("⎇ other"));
    assert_eq!(std::fs::read(gr.path().join("a.txt")).unwrap(), std::fs::read(tr.path().join("a.txt")).unwrap());
    assert_eq!(std::fs::read_to_string(tr.path().join("a.txt")).unwrap(), "other\n");
}

#[gpui::test]
fn gui_and_terminal_git_comparisons_start_the_same_lenz_diff(cx: &mut TestAppContext) {
    use lenz_content::git::testing::{commit_all, git, repo, write};
    let r = repo();
    write(r.path(), "a.txt", "main\n");
    commit_all(r.path(), "main");
    git(r.path(), &["tag", "v1"]);
    write(r.path(), "a.txt", "edited\n");
    let bin = tempfile::tempdir().unwrap();
    for name in ["lenz-diff", "lenz-diff-tui"] {
        std::fs::write(bin.path().join(name), "").unwrap();
    }
    let state = tempfile::tempdir().unwrap();
    let gui = cx.new(|cx| ReaderView::new(Session::new(config(&state.path().join("gui"))), Some(r.path().to_path_buf()), cx));
    let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| gui.clone())).unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let focus = gui.read_with(&visual, |v, _| v.focus_handle());
    visual.update(|w, cx| focus.focus(w, cx));
    gui.update(&mut visual, |v, _| v.exe = Some(bin.path().join("lenz-code")));
    let mut tui = App::new(config(&state.path().join("tui")), Some(r.path().to_path_buf()));
    tui.exe = Some(bin.path().join("lenz-code"));
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 24)).unwrap();
    let key = |app: &mut App, code, modifiers| { app.handle(Event::Key(KeyEvent::new(code, modifiers))); };
    let settle = |visual: &mut VisualTestContext, tui: &mut App, done: &dyn Fn(&Session) -> bool| {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            gui.update(visual, |v, cx| v.poll(cx));
            while let Ok(c) = tui.session.completions().try_recv() { tui.apply(c); }
            if gui.read_with(visual, |v, _| done(v.session())) && done(&tui.session) { break; }
            assert!(Instant::now() < deadline, "adapters did not settle");
            std::thread::sleep(Duration::from_millis(2));
        }
    };
    settle(&mut visual, &mut tui, &|s| s.branch_label().is_some());
    visual.update(|w, cx| { let _ = w.draw(cx); });
    terminal.draw(|f| tui.draw(f)).unwrap();
    // Show Local Changes from each application menu.
    visual.simulate_click(gpui::point(gpui::px(12.0), gpui::px(10.0)), gpui::Modifiers::default());
    let button = tui.layout.app_menu;
    tui.handle(Event::Mouse(crossterm::event::MouseEvent { kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left), column: button.x + 1, row: button.y, modifiers: KeyModifiers::NONE }));
    let group = reader_core::actions::CommandGroup::ALL.iter().position(|g| *g == reader_core::actions::CommandGroup::Git).unwrap();
    let row = CommandId::ALL.iter().filter(|c| c.group() == reader_core::actions::CommandGroup::Git).position(|c| *c == CommandId::ShowLocalChanges).unwrap();
    visual.simulate_keystrokes(&format!("{}enter {}enter", "down ".repeat(group), "down ".repeat(row)));
    for n in [group, row] {
        for _ in 0..n { key(&mut tui, KeyCode::Down, KeyModifiers::NONE); }
        key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    }
    let gui_args = |visual: &mut VisualTestContext| gui.read_with(visual, |v, _| v.launched.last().map(|l| (l.args.clone(), l.cwd.clone())));
    let tui_args = |tui: &mut App| tui.foreground.take().map(|l| (l.args, l.cwd));
    let local = tui_args(&mut tui);
    assert_eq!(gui_args(&mut visual), local);
    assert_eq!(local.map(|l| l.0), Some(vec!["HEAD".to_string()]));
    // Compare with Working Tree from tag v1's row menu.
    visual.simulate_keystrokes("ctrl-shift-`");
    tui.run(CommandId::GitBranches);
    settle(&mut visual, &mut tui, &|s| s.state().branches().is_some_and(|b| !b.loading));
    visual.simulate_keystrokes("v 1 right down down enter");
    for c in "v1".chars() { key(&mut tui, KeyCode::Char(c), KeyModifiers::NONE); }
    key(&mut tui, KeyCode::Right, KeyModifiers::NONE);
    for _ in 0..2 { key(&mut tui, KeyCode::Down, KeyModifiers::NONE); }
    key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    let tag = tui_args(&mut tui);
    assert_eq!(gui_args(&mut visual), tag);
    assert_eq!(tag.map(|l| l.0), Some(vec!["refs/tags/v1".to_string()]));
    assert_eq!(std::fs::read_to_string(r.path().join("a.txt")).unwrap(), "edited\n");
}

#[gpui::test]
fn draft_bookmarks_and_back_follow_unicode_edits_in_both_adapters(cx: &mut TestAppContext) {
    use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    for dark in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("anchors.txt");
        let original = "first\r\nα😀 target\r\nlast\r\n";
        let prefix = "é😀 new\r\n";
        std::fs::write(&path, original).unwrap();
        let mut gc = config(&dir.path().join("gui")); gc.dark = dark;
        let mut tc = config(&dir.path().join("tui")); tc.dark = dark;
        let gui = cx.new(|cx| ReaderView::new(Session::new(gc), Some(path.clone()), cx));
        let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| gui.clone())).unwrap();
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        let focus = gui.read_with(&visual, |view, _| view.focus_handle());
        visual.update(|window, cx| focus.focus(window, cx));
        let mut tui = App::new(tc, Some(path.clone()));
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 24)).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while gui.read_with(&visual, |view, _| view.session().state().current_source().is_none()) || tui.session.state().current_source().is_none() {
            gui.update(&mut visual, |view, cx| view.poll(cx));
            while let Ok(done) = tui.session.completions().try_recv() { tui.apply(done); }
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        let draw = |visual: &mut VisualTestContext, tui: &mut App, terminal: &mut ratatui::Terminal<ratatui::backend::TestBackend>| {
            visual.update(|window, cx| { let _ = window.draw(cx); });
            terminal.draw(|frame| tui.draw(frame)).unwrap();
        };
        let key = |tui: &mut App, code, modifiers| { tui.handle(Event::Key(KeyEvent::new(code, modifiers))); };
        draw(&mut visual, &mut tui, &mut terminal);
        gui.update(&mut visual, |view, _| view.enable_draft_input_for_test()); tui.enable_draft_input_for_test();
        visual.simulate_keystrokes("ctrl-home down right right right ctrl-shift-2");
        for (code, modifiers) in [(KeyCode::Home, KeyModifiers::CONTROL), (KeyCode::Down, KeyModifiers::NONE), (KeyCode::Right, KeyModifiers::NONE), (KeyCode::Right, KeyModifiers::NONE), (KeyCode::Right, KeyModifiers::NONE), (KeyCode::Char('2'), KeyModifiers::CONTROL | KeyModifiers::SHIFT)] { key(&mut tui, code, modifiers); }
        let marked = original.find("target").unwrap();
        assert_eq!(tui.session.state().selection().unwrap().head, marked);
        assert_eq!(snapshot(tui.session.state()), gui.read_with(&visual, |view, _| snapshot(view.session().state())));
        visual.simulate_keystrokes("ctrl-g 3 : 2 enter");
        key(&mut tui, KeyCode::Char('g'), KeyModifiers::CONTROL);
        tui.handle(Event::Paste("3:2".into())); key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
        draw(&mut visual, &mut tui, &mut terminal);
        let (x, y) = gui.read_with(&visual, |view, _| view.text_point_of(0)).unwrap();
        visual.simulate_click(gpui::point(gpui::px(x), gpui::px(y)), gpui::Modifiers::default());
        let area = tui.layout.panes[0].text;
        tui.handle(Event::Mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: area.x, row: area.y, modifiers: KeyModifiers::NONE }));
        visual.simulate_keystrokes("left"); key(&mut tui, KeyCode::Left, KeyModifiers::NONE);
        visual.update(|_, cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string(prefix.into())));
        visual.simulate_keystrokes("ctrl-v"); tui.handle(Event::Paste(prefix.into()));
        assert!(tui.modal.is_none(), "{:?}", tui.modal.as_ref().map(|modal| (&modal.kind, &modal.input, &modal.error)));
        assert_eq!(snapshot(tui.session.state()).text.as_deref(), Some(format!("{prefix}{original}").as_str()));
        assert_eq!(snapshot(tui.session.state()), gui.read_with(&visual, |view, _| snapshot(view.session().state())));
        visual.simulate_keystrokes("alt-left"); key(&mut tui, KeyCode::Left, KeyModifiers::ALT);
        draw(&mut visual, &mut tui, &mut terminal);
        assert_eq!(tui.session.state().selection().unwrap().head, marked + prefix.len());
        assert_eq!(snapshot(tui.session.state()), gui.read_with(&visual, |view, _| snapshot(view.session().state())));
        visual.simulate_keystrokes("alt-2"); key(&mut tui, KeyCode::Char('2'), KeyModifiers::ALT);
        draw(&mut visual, &mut tui, &mut terminal);
        assert_eq!(tui.session.state().selection().unwrap().head, marked + prefix.len());
        assert_eq!(gui.read_with(&visual, |view, _| view.bookmark_marks().into_iter().map(|(row, key)| (view.visible_rows()[row].number, key)).collect::<Vec<_>>()), [(3, Some('2'))]);
        let rendered = terminal.backend().buffer().content.iter().map(|cell| cell.symbol()).collect::<String>();
        assert!(rendered.contains("target") && rendered.contains('α') && rendered.contains('😀'), "{rendered}");
        assert_eq!(tui.session.state().places().bookmarks()[0].line, 2);
        visual.simulate_keystrokes("ctrl-z"); key(&mut tui, KeyCode::Char('z'), KeyModifiers::CONTROL);
        draw(&mut visual, &mut tui, &mut terminal);
        assert_eq!(snapshot(tui.session.state()).text.as_deref(), Some(original));
        assert_eq!(snapshot(tui.session.state()), gui.read_with(&visual, |view, _| snapshot(view.session().state())));
        assert_eq!(gui.read_with(&visual, |view, _| view.bookmark_marks().into_iter().map(|(row, key)| (view.visible_rows()[row].number, key)).collect::<Vec<_>>()), [(2, Some('2'))]);
        assert_eq!(tui.session.state().places().bookmarks()[0].line, 1);
        assert_eq!(std::fs::read(path).unwrap(), original.as_bytes());
    }
}

#[gpui::test]
fn semantic_draft_targets_keep_peek_and_activation_consistent_in_both_adapters(cx: &mut TestAppContext) {
    use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    use reader_core::{project::Coverage, search::ResultsKind, semantics::Status};
    let profile = std::env::current_exe().unwrap().parent().unwrap().parent().unwrap().to_owned();
    assert!(std::process::Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .args(["build", "-q", "--locked", "-p", "reader-core", "--example", "fake_lsp"])
        .current_dir(env!("CARGO_MANIFEST_DIR")).status().unwrap().success());
    let fake = profile.join("examples/fake_lsp");
    let dir = tempfile::tempdir().unwrap(); let root = dir.path().join("project"); std::fs::create_dir(&root).unwrap();
    let a = root.join("main.rs"); let b = root.join("lib.rs"); let other = root.join("other.rs");
    let a_text = "fn main() {\r\n    helper();\r\n}\r\n";
    let b_text = "pub fn helper() {}\r\n";
    let prefix = "// é😀 draft\r\n";
    let later = "// newer target\r\n";
    for (path, text) in [(root.join("Cargo.toml"), "[package]\nname='draft-target'\nversion='0.1.0'\n"), (a.clone(), a_text), (b.clone(), b_text), (other.clone(), "pub fn helper() {}\n")] { std::fs::write(path, text).unwrap(); }
    let make = |name| {
        let c = config(&dir.path().join(name));
        let mut settings: serde_json::Value = serde_json::from_slice(&std::fs::read(&c.providers).unwrap()).unwrap();
        settings["providers"]["rust-analyzer"] = serde_json::json!({"executable":fake,"env":{"FAKE_LSP_NO_DECOR":"1"}});
        std::fs::write(&c.providers, settings.to_string()).unwrap(); c
    };
    let mut core = Session::new(make("gui")); core.dispatch(Command::OpenRoot(root.clone()));
    let gui = cx.new(|cx| ReaderView::new(core, Some(a.clone()), cx));
    let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| gui.clone())).unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let focus = gui.read_with(&visual, |view, _| view.focus_handle()); visual.update(|window, cx| focus.focus(window, cx));
    let mut tui = App::new(make("tui"), Some(root.clone())); tui.session.dispatch(Command::Open(a.clone()));
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 40)).unwrap();
    let draw = |visual: &mut VisualTestContext, tui: &mut App, terminal: &mut ratatui::Terminal<ratatui::backend::TestBackend>| {
        visual.update(|window, cx| { let _ = window.draw(cx); }); terminal.draw(|frame| tui.draw(frame)).unwrap();
    };
    let wait = |visual: &mut VisualTestContext, tui: &mut App, terminal: &mut ratatui::Terminal<ratatui::backend::TestBackend>, done: &dyn Fn(&State) -> bool| {
        let until = Instant::now() + Duration::from_secs(10);
        loop {
            gui.update(visual, |view, cx| view.poll(cx)); while let Ok(completion) = tui.session.completions().try_recv() { tui.apply(completion); }
            draw(visual, tui, terminal);
            if gui.read_with(visual, |view, _| done(view.session().state())) && done(tui.session.state()) { break; }
            assert!(Instant::now() < until, "GUI {:?}; TUI {:?}", gui.read_with(visual, |view, _| view.session().state().notices()), tui.session.state().notices());
            std::thread::yield_now();
        }
    };
    let key = |tui: &mut App, code, modifiers| { tui.handle(Event::Key(KeyEvent::new(code, modifiers))); };
    let paste = |visual: &mut VisualTestContext, tui: &mut App, text: &str| {
        visual.update(|_, cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string(text.into())));
        visual.simulate_keystrokes("ctrl-v"); tui.handle(Event::Paste(text.into()));
    };
    wait(&mut visual, &mut tui, &mut terminal, &|state| state.project().is_some_and(|project| matches!(project.coverage(), Coverage::Complete(_))) && state.current_source().is_some_and(|source| source.source() == a) && state.provider().is_some_and(|provider| provider.status == Status::Ready));
    gui.update(&mut visual, |view, _| view.enable_draft_input_for_test()); tui.enable_draft_input_for_test();
    visual.simulate_keystrokes("ctrl-p l i b . r s"); key(&mut tui, KeyCode::Char('p'), KeyModifiers::CONTROL); tui.handle(Event::Paste("lib.rs".into()));
    wait(&mut visual, &mut tui, &mut terminal, &|state| state.navigation().is_some_and(|search| !search.pending && !search.rows.is_empty()));
    visual.simulate_keystrokes("enter"); key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    wait(&mut visual, &mut tui, &mut terminal, &|state| state.current_source().is_some_and(|source| source.source() == b));
    visual.simulate_keystrokes("ctrl-home"); key(&mut tui, KeyCode::Home, KeyModifiers::CONTROL); paste(&mut visual, &mut tui, prefix);
    visual.simulate_keystrokes("ctrl-\\ ctrl-1 ctrl-p m a i n . r s");
    for (code, modifiers) in [(KeyCode::Char('\\'), KeyModifiers::CONTROL), (KeyCode::Char('1'), KeyModifiers::CONTROL), (KeyCode::Char('p'), KeyModifiers::CONTROL)] { key(&mut tui, code, modifiers); }
    tui.handle(Event::Paste("main.rs".into()));
    wait(&mut visual, &mut tui, &mut terminal, &|state| state.navigation().is_some_and(|search| !search.pending && !search.rows.is_empty()));
    visual.simulate_keystrokes("enter"); key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    wait(&mut visual, &mut tui, &mut terminal, &|state| state.current_source().is_some_and(|source| source.source() == a));
    visual.simulate_keystrokes("ctrl-home down right right right right alt-f12");
    for (code, modifiers) in [(KeyCode::Home, KeyModifiers::CONTROL), (KeyCode::Down, KeyModifiers::NONE), (KeyCode::Right, KeyModifiers::NONE), (KeyCode::Right, KeyModifiers::NONE), (KeyCode::Right, KeyModifiers::NONE), (KeyCode::Right, KeyModifiers::NONE), (KeyCode::F(12), KeyModifiers::ALT)] { key(&mut tui, code, modifiers); }
    wait(&mut visual, &mut tui, &mut terminal, &|state| state.results().is_some_and(|results| results.results_kind() == ResultsKind::Peek && results.hits().len() == 2 && results.preview().is_some()));
    let preview = |state: &State| state.results().unwrap().preview().unwrap().doc.text().to_string();
    let captured = format!("{prefix}{b_text}");
    assert_eq!(preview(tui.session.state()), captured); assert_eq!(gui.read_with(&visual, |view, _| preview(view.session().state())), captured);
    visual.simulate_keystrokes("ctrl-2 ctrl-g 1 : 1 enter"); key(&mut tui, KeyCode::Char('2'), KeyModifiers::CONTROL);
    key(&mut tui, KeyCode::Char('g'), KeyModifiers::CONTROL); tui.handle(Event::Paste("1:1".into())); key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    draw(&mut visual, &mut tui, &mut terminal);
    let (x, y) = gui.read_with(&visual, |view, _| view.text_point_of(0)).unwrap(); visual.simulate_click(gpui::point(gpui::px(x), gpui::px(y)), gpui::Modifiers::default());
    let area = tui.layout.panes[1].text; tui.handle(Event::Mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: area.x, row: area.y, modifiers: KeyModifiers::NONE }));
    visual.simulate_keystrokes("left"); key(&mut tui, KeyCode::Left, KeyModifiers::NONE); paste(&mut visual, &mut tui, later);
    assert_eq!(snapshot(tui.session.state()).text.as_deref(), Some(format!("{later}{prefix}{b_text}").as_str()));
    assert_eq!(snapshot(tui.session.state()), gui.read_with(&visual, |view, _| snapshot(view.session().state())));
    assert!(tui.session.state().results().unwrap().is_stale(0));
    visual.simulate_keystrokes("ctrl-1"); key(&mut tui, KeyCode::Char('1'), KeyModifiers::CONTROL);
    let select = |visual: &mut VisualTestContext, tui: &mut App, terminal: &mut ratatui::Terminal<ratatui::backend::TestBackend>, index| {
        draw(visual, tui, terminal);
        let (x, y) = gui.read_with(visual, |view, _| view.result_point_of(index)).unwrap(); visual.simulate_click(gpui::point(gpui::px(x), gpui::px(y)), gpui::Modifiers::default());
        let row = tui.session.state().results().unwrap().row_of_hit(index).unwrap(); let area = tui.layout.results;
        tui.handle(Event::Mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: area.x + 8, row: area.y + row as u16, modifiers: KeyModifiers::NONE }));
        wait(visual, tui, terminal, &|state| state.results().unwrap().preview().is_some_and(|preview| preview.index == index));
    };
    select(&mut visual, &mut tui, &mut terminal, 1); select(&mut visual, &mut tui, &mut terminal, 0);
    assert_eq!(preview(tui.session.state()), captured); assert_eq!(gui.read_with(&visual, |view, _| preview(view.session().state())), captured);
    let (_, rows) = gui.update(&mut visual, |view, _| view.panel_contents(16, 16)); assert!(rows.iter().any(|row| row.number == 2 && row.text == "pub fn helper() {}"));
    let rendered = terminal.backend().buffer().content.iter().map(|cell| cell.symbol()).collect::<String>(); assert!(rendered.contains("Open drafts changed") && rendered.contains("pub fn helper() {}"), "{rendered}");
    assert!(gui.read_with(&visual, |view, _| view.results_status().unwrap().contains("Open drafts changed")));
    let before = snapshot(tui.session.state()); assert_eq!(before, gui.read_with(&visual, |view, _| snapshot(view.session().state())));
    visual.simulate_keystrokes("enter"); key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    wait(&mut visual, &mut tui, &mut terminal, &|state| state.notices().iter().any(|notice| notice.contains("captured draft changed")));
    assert_eq!(snapshot(tui.session.state()), before); assert_eq!(gui.read_with(&visual, |view, _| snapshot(view.session().state())), before);
    draw(&mut visual, &mut tui, &mut terminal);
    let (x, y) = gui.read_with(&visual, |view, _| view.text_point_of(0)).unwrap(); visual.simulate_click(gpui::point(gpui::px(x), gpui::px(y)), gpui::Modifiers::default());
    let area = tui.layout.panes[0].text; tui.handle(Event::Mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: area.x, row: area.y, modifiers: KeyModifiers::NONE }));
    visual.simulate_keystrokes("left down right right right right f12");
    for (code, modifiers) in [(KeyCode::Left, KeyModifiers::NONE), (KeyCode::Down, KeyModifiers::NONE), (KeyCode::Right, KeyModifiers::NONE), (KeyCode::Right, KeyModifiers::NONE), (KeyCode::Right, KeyModifiers::NONE), (KeyCode::Right, KeyModifiers::NONE), (KeyCode::F(12), KeyModifiers::NONE)] { key(&mut tui, code, modifiers); }
    wait(&mut visual, &mut tui, &mut terminal, &|state| state.results().is_some_and(|results| results.results_kind() == ResultsKind::Definitions && results.hits().len() == 2 && results.preview().is_some()));
    select(&mut visual, &mut tui, &mut terminal, 0); visual.simulate_keystrokes("enter"); key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    wait(&mut visual, &mut tui, &mut terminal, &|state| state.current_source().is_some_and(|source| source.source() == b) && copied_selection(state).as_deref() == Some("helper"));
    assert_eq!(snapshot(tui.session.state()).text.as_deref(), Some(format!("{later}{prefix}{b_text}").as_str()));
    assert_eq!(snapshot(tui.session.state()), gui.read_with(&visual, |view, _| snapshot(view.session().state())));
    visual.simulate_keystrokes("shift-f12"); key(&mut tui, KeyCode::F(12), KeyModifiers::SHIFT);
    wait(&mut visual, &mut tui, &mut terminal, &|state| state.results().is_some_and(|results| results.results_kind() == ResultsKind::Usages && results.hits().len() == 1 && results.preview().is_some()));
    select(&mut visual, &mut tui, &mut terminal, 0); visual.simulate_keystrokes("enter"); key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    wait(&mut visual, &mut tui, &mut terminal, &|state| state.current_source().is_some_and(|source| source.source() == a) && copied_selection(state).as_deref() == Some("helper"));
    assert_eq!(snapshot(tui.session.state()), gui.read_with(&visual, |view, _| snapshot(view.session().state())));
    assert_eq!(std::fs::read(a).unwrap(), a_text.as_bytes()); assert_eq!(std::fs::read(b).unwrap(), b_text.as_bytes()); assert_eq!(std::fs::read(other).unwrap(), b"pub fn helper() {}\n");
}

#[gpui::test]
fn long_line_split_edits_keep_absolute_unicode_coordinates_in_both_adapters(cx: &mut TestAppContext) {
    for (dark, prefix) in [(true, "a".repeat(12_000)), (false, "aé😀e\u{301}\t\x07 ".repeat(80_000))] {
    let dir = tempfile::tempdir().unwrap(); let path = dir.path().join("long-edit.txt");
    let column = prefix.chars().count() + 1;
    let tail = "😀e\u{301}\t\x07END\r\n";
    let original = format!("{prefix}{tail}"); std::fs::write(&path, &original).unwrap();
    let mut gc = config(&dir.path().join("gui")); gc.dark = dark;
    let mut tc = config(&dir.path().join("tui")); tc.dark = dark;
    let gui = cx.new(|cx| ReaderView::new(Session::new(gc), Some(path.clone()), cx));
    let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| gui.clone())).unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let focus = gui.read_with(&visual, |view, _| view.focus_handle()); visual.update(|window, cx| focus.focus(window, cx));
    let mut tui = App::new(tc, Some(path.clone()));
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 24)).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while gui.read_with(&visual, |view, _| view.session().state().current_source().is_none()) || tui.session.state().current_source().is_none() {
        gui.update(&mut visual, |view, cx| view.poll(cx));
        while let Ok(done) = tui.session.completions().try_recv() { tui.apply(done); }
        assert!(Instant::now() < deadline); std::thread::yield_now();
    }
    let draw = |visual: &mut VisualTestContext, tui: &mut App, terminal: &mut ratatui::Terminal<ratatui::backend::TestBackend>| {
        visual.update(|window, cx| { let _ = window.draw(cx); }); terminal.draw(|frame| tui.draw(frame)).unwrap();
    };
    let key = |app: &mut App, code, modifiers| { app.handle(Event::Key(KeyEvent::new(code, modifiers))); };
    draw(&mut visual, &mut tui, &mut terminal);
    gui.update(&mut visual, |view, _| view.enable_draft_input_for_test()); tui.enable_draft_input_for_test();
    let conversions = reader_core::text::TextSnapshot::conversion_calls_on_thread_for_test();
    let query = format!("1:{column}");
    let keys = query.chars().map(|c| c.to_string()).collect::<Vec<_>>().join(" ");
    visual.simulate_keystrokes(&format!("ctrl-g {keys} enter ctrl-\\ shift-right β"));
    key(&mut tui, KeyCode::Char('g'), KeyModifiers::CONTROL); tui.handle(Event::Paste(query));
    key(&mut tui, KeyCode::Enter, KeyModifiers::NONE); key(&mut tui, KeyCode::Char('\\'), KeyModifiers::CONTROL);
    key(&mut tui, KeyCode::Right, KeyModifiers::SHIFT); key(&mut tui, KeyCode::Char('β'), KeyModifiers::NONE);
    assert_eq!(snapshot(tui.session.state()).text.as_deref(), Some(format!("{prefix}βe\u{301}\t\x07END\r\n").as_str()));
    assert_eq!(snapshot(tui.session.state()), gui.read_with(&visual, |view, _| snapshot(view.session().state())));
    assert_eq!(tui.session.state().selection().unwrap().head, prefix.len() + 2);
    visual.simulate_keystrokes("ctrl-1 ctrl-z"); key(&mut tui, KeyCode::Char('1'), KeyModifiers::CONTROL); key(&mut tui, KeyCode::Char('z'), KeyModifiers::CONTROL);
    assert_eq!(snapshot(tui.session.state()).text.as_deref(), Some(original.as_str()));
    assert_eq!(snapshot(tui.session.state()), gui.read_with(&visual, |view, _| snapshot(view.session().state())));
    visual.update(|_, cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string("終😀".into())));
    visual.simulate_keystrokes("ctrl-v"); tui.handle(Event::Paste("終😀".into()));
    assert_eq!(snapshot(tui.session.state()), gui.read_with(&visual, |view, _| snapshot(view.session().state())));
    let expected = format!("{prefix}😀終😀e\u{301}\t\x07END\r\n");
    assert_eq!(snapshot(tui.session.state()).text.as_deref(), Some(expected.as_str()));
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        gui.update(&mut visual, |view, cx| view.poll(cx));
        while let Ok(done) = tui.session.completions().try_recv() { tui.apply(done); }
        draw(&mut visual, &mut tui, &mut terminal);
        let shown = terminal.backend().buffer().content.iter().map(|cell| cell.symbol()).collect::<String>();
        if shown.contains('終') && gui.read_with(&visual, |view, _| view.visible_rows().iter().any(|row| row.text.contains('終'))) { break; }
        assert!(Instant::now() < deadline, "the edited Unicode tail must become visible in both split views"); std::thread::yield_now();
    }
    for pane in [0, 1] {
        let state = tui.session.state(); let tab = state.shown(pane).unwrap();
        assert_eq!(state.view(tab.id()).unwrap().snapshot().copy_range(0..expected.len()).unwrap(), expected);
    }
    // Reuse the actual IME geometry callback for a native source point, then exercise real clicks.
    let target = expected.find("END").unwrap();
    let reveal = format!("1:{}", expected[..target].chars().count() + 1);
    let keys = reveal.chars().map(|c| c.to_string()).collect::<Vec<_>>().join(" ");
    visual.simulate_keystrokes(&format!("ctrl-g {keys} enter"));
    key(&mut tui, KeyCode::Char('g'), KeyModifiers::CONTROL); tui.handle(Event::Paste(reveal)); key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    let unit = tui.session.state().current_source().unwrap().byte_to_utf16(target).unwrap();
    let entity = gui.clone();
    let mut input = visual.update(|window, cx| gpui::PlatformInputHandler::new(window.to_async(cx), Box::new(gpui::ElementInputHandler::new(gpui::Bounds::default(), entity))));
    let deadline = Instant::now() + Duration::from_secs(5);
    let point = loop {
        gui.update(&mut visual, |view, cx| view.poll(cx));
        while let Ok(done) = tui.session.completions().try_recv() { tui.apply(done); }
        draw(&mut visual, &mut tui, &mut terminal);
        let hit = tui.layout.panes.iter().find(|pane| pane.pane == tui.session.state().focused_pane()).and_then(|pane| pane.hits.iter().find(|hit| hit.byte == target)).cloned();
        if let Some(bounds) = input.bounds_for_range(unit..unit) && let Some(hit) = hit {
            break (gpui::point(bounds.origin.x + gpui::px(0.1), bounds.origin.y + gpui::px(5.0)), hit.x, hit.y);
        }
        assert!(Instant::now() < deadline, "tail hit geometry never became current: target={target} GUI={:?} selected={:?} TUI={:?} rows={:?}", input.bounds_for_range(unit..unit), input.selected_text_range(false).map(|s|s.range), tui.layout.panes.iter().map(|p|(p.pane,p.hits.first().map(|h|h.byte),p.hits.last().map(|h|h.byte))).collect::<Vec<_>>(), gui.read_with(&visual, |view,_|view.visible_rows().iter().map(|r|r.text.clone()).collect::<Vec<_>>())); std::thread::yield_now();
    };
    visual.simulate_click(point.0, gpui::Modifiers::default());
    tui.handle(Event::Mouse(crossterm::event::MouseEvent { kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left), column: point.1, row: point.2, modifiers: KeyModifiers::NONE }));
    visual.simulate_keystrokes("shift-right"); key(&mut tui, KeyCode::Right, KeyModifiers::SHIFT);
    assert_eq!(copied_selection(tui.session.state()).as_deref(), Some("E"));
    assert_eq!(snapshot(tui.session.state()), gui.read_with(&visual, |view, _| snapshot(view.session().state())));
    assert_eq!(reader_core::text::TextSnapshot::conversion_calls_on_thread_for_test(), conversions);
    assert_eq!(std::fs::read(path).unwrap(), original.as_bytes());
    }
}

#[gpui::test]
fn new_file_save_as_and_replace_decisions_match_in_both_adapters(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let targets = [dir.path().join("gui"), dir.path().join("tui")].map(|d| { std::fs::create_dir_all(&d).unwrap(); let t = d.join("note.txt"); std::fs::write(&t, b"keep\n").unwrap(); t });
    let gui = cx.new(|cx| ReaderView::new(Session::new(config(&dir.path().join("gui-state"))), None, cx));
    let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| gui.clone())).unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), cx); let focus = gui.read_with(&visual, |v, _| v.focus_handle()); visual.update(|w, cx| focus.focus(w, cx));
    let mut tui = App::new(config(&dir.path().join("tui-state")), None);
    gui.update(&mut visual, |v, _| v.enable_draft_input_for_test()); tui.enable_draft_input_for_test();
    let key = |tui: &mut App, code, modifiers| { tui.handle(Event::Key(KeyEvent::new(code, modifiers))); };
    let save_as = |visual: &mut VisualTestContext, tui: &mut App, cx: &mut TestAppContext| {
        visual.simulate_keystrokes("ctrl-s"); let chosen = targets[0].clone(); cx.simulate_new_path_selection(move |_| Some(chosen)); cx.run_until_parked();
        key(tui, KeyCode::Char('s'), KeyModifiers::CONTROL); key(tui, KeyCode::Char('u'), KeyModifiers::CONTROL); tui.handle(Event::Paste(targets[1].display().to_string())); key(tui, KeyCode::Enter, KeyModifiers::NONE);
    };
    let both = |visual: &VisualTestContext, tui: &App| { let state = snapshot(tui.session.state()); assert_eq!(state, gui.read_with(visual, |v, _| snapshot(v.session().state()))); state };
    visual.simulate_keystrokes("ctrl-n"); key(&mut tui, KeyCode::Char('n'), KeyModifiers::CONTROL);
    visual.update(|_, cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string("draft λ\r\n".into()))); visual.simulate_keystrokes("ctrl-v"); tui.handle(Event::Paste("draft λ\r\n".into()));
    let state = both(&visual, &tui);
    assert_eq!((state.active.as_deref(), state.text.as_deref()), (Some("Untitled-1"), Some("draft λ\r\n")));
    save_as(&mut visual, &mut tui, cx);
    assert!(gui.read_with(&visual, |v, _| v.session().save_as_prompt().is_some()) && tui.session.save_as_prompt().is_some());
    visual.simulate_keystrokes("escape"); key(&mut tui, KeyCode::Esc, KeyModifiers::NONE);
    assert!(gui.read_with(&visual, |v, _| v.session().save_as_prompt().is_none()) && tui.session.save_as_prompt().is_none());
    for target in &targets { assert_eq!(std::fs::read(target).unwrap(), b"keep\n"); }
    assert_eq!(both(&visual, &tui).active.as_deref(), Some("Untitled-1"));
    save_as(&mut visual, &mut tui, cx);
    visual.simulate_keystrokes("r"); key(&mut tui, KeyCode::Char('r'), KeyModifiers::NONE);
    let deadline = Instant::now() + Duration::from_secs(5);
    let clean = |s: &State| s.active().is_some_and(|t| !t.untitled() && !s.tab_dirty(t.id()));
    while !gui.read_with(&visual, |v, _| clean(v.session().state())) || !clean(tui.session.state()) {
        gui.update(&mut visual, |v, cx| v.poll(cx)); while let Ok(done) = tui.session.completions().try_recv() { tui.apply(done); } assert!(Instant::now() < deadline); std::thread::yield_now();
    }
    assert_eq!(both(&visual, &tui).active.as_deref(), Some("note.txt"));
    for target in &targets { assert_eq!(std::fs::read(target).unwrap(), "draft λ\r\n".as_bytes()); }
    visual.simulate_keystrokes("ctrl-z"); key(&mut tui, KeyCode::Char('z'), KeyModifiers::CONTROL);
    assert_eq!(both(&visual, &tui).text.as_deref(), Some(""));
    assert!(tui.session.state().tab_dirty(tui.session.state().active().unwrap().id()));
}

#[gpui::test]
fn completion_typing_acceptance_and_dismissal_match_in_both_adapters(cx: &mut TestAppContext) {
    let profile = std::env::current_exe().unwrap().parent().unwrap().parent().unwrap().to_owned();
    assert!(std::process::Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .args(["build", "-q", "--locked", "-p", "reader-core", "--example", "fake_lsp"])
        .current_dir(env!("CARGO_MANIFEST_DIR")).status().unwrap().success());
    let fake = profile.join("examples/fake_lsp");
    let dir = tempfile::tempdir().unwrap(); let root = dir.path().join("project"); std::fs::create_dir(&root).unwrap();
    let path = root.join("main.rs");
    let text = "fn main() {  }\nfn helper() {}\nfn hello() {}\n";
    std::fs::write(root.join("Cargo.toml"), "[package]\nname='completion'\nversion='0.1.0'\n").unwrap(); std::fs::write(&path, text).unwrap();
    let make = |name| {
        let c = config(&dir.path().join(name));
        let mut settings: serde_json::Value = serde_json::from_slice(&std::fs::read(&c.providers).unwrap()).unwrap();
        settings["providers"]["rust-analyzer"] = serde_json::json!({"executable":fake,"env":{"FAKE_LSP_NO_DECOR":"1"}});
        std::fs::write(&c.providers, settings.to_string()).unwrap(); c
    };
    let gui = cx.new(|cx| ReaderView::new(Session::new(make("gui")), Some(path.clone()), cx));
    let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| gui.clone())).unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let focus = gui.read_with(&visual, |view, _| view.focus_handle()); visual.update(|window, cx| focus.focus(window, cx));
    let mut tui = App::new(make("tui"), Some(root.clone())); tui.session.dispatch(Command::Open(path.clone()));
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
    let wait = |visual: &mut VisualTestContext, tui: &mut App, terminal: &mut ratatui::Terminal<ratatui::backend::TestBackend>, done: &dyn Fn(&State) -> bool| {
        let until = Instant::now() + Duration::from_secs(10);
        loop {
            gui.update(visual, |view, cx| view.poll(cx)); while let Ok(completion) = tui.session.completions().try_recv() { tui.apply(completion); }
            visual.update(|window, cx| { let _ = window.draw(cx); }); terminal.draw(|frame| tui.draw(frame)).unwrap();
            if gui.read_with(visual, |view, _| done(view.session().state())) && done(tui.session.state()) { break; }
            assert!(Instant::now() < until, "GUI {:?}; TUI {:?}", gui.read_with(visual, |view, _| view.session().state().notices()), tui.session.state().notices());
            std::thread::yield_now();
        }
    };
    let ready = |state: &State| state.provider().is_some_and(|p| p.status == reader_core::semantics::Status::Ready) && state.view(state.active().unwrap().id()).is_some_and(|v| v.highlights().is_some());
    wait(&mut visual, &mut tui, &mut terminal, &ready);
    gui.update(&mut visual, |view, _| view.enable_draft_input_for_test()); tui.enable_draft_input_for_test();
    let key = |tui: &mut App, code| { tui.handle(Event::Key(KeyEvent::new(code, KeyModifiers::NONE))); };
    // Inside the braces, by a click in each.
    let at = text.find("{  }").unwrap() + 2;
    let (x, y) = gui.read_with(&visual, |view, _| view.text_point_of(at)).unwrap(); visual.simulate_click(gpui::point(gpui::px(x), gpui::px(y)), gpui::Modifiers::default());
    let hit = tui.layout.panes[0].hits.iter().find(|h| h.byte == at).unwrap().clone();
    tui.handle(Event::Mouse(crossterm::event::MouseEvent { kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left), column: hit.x, row: hit.y, modifiers: KeyModifiers::NONE }));
    tui.handle(Event::Mouse(crossterm::event::MouseEvent { kind: crossterm::event::MouseEventKind::Up(crossterm::event::MouseButton::Left), column: hit.x, row: hit.y, modifiers: KeyModifiers::NONE }));
    assert_eq!(tui.session.state().selection().map(|s| s.head), Some(at));
    assert_eq!(gui.read_with(&visual, |view, _| view.session().state().selection().map(|s| s.head)), Some(at));
    let labels = |state: &State| state.completion().map(|c| (c.rows().map(|r| r.label.clone()).collect::<Vec<_>>(), c.selected()));
    let same = |visual: &mut VisualTestContext, tui: &App| {
        let gui_state = gui.read_with(visual, |view, _| (snapshot(view.session().state()), labels(view.session().state())));
        assert_eq!((snapshot(tui.session.state()), labels(tui.session.state())), gui_state);
    };
    // Typing opens the same suggestions; Down and Enter accept the same one.
    visual.simulate_keystrokes("h e"); key(&mut tui, KeyCode::Char('h')); key(&mut tui, KeyCode::Char('e'));
    wait(&mut visual, &mut tui, &mut terminal, &|state| state.completion().is_some());
    same(&mut visual, &tui);
    visual.simulate_keystrokes("down enter"); key(&mut tui, KeyCode::Down); key(&mut tui, KeyCode::Enter);
    same(&mut visual, &tui);
    assert_eq!(snapshot(tui.session.state()).text.as_deref(), Some(text.replacen("{  }", "{ helper }", 1).as_str()));
    // A trigger character opens it in both; Escape dismisses alike; punctuation types without accepting.
    visual.simulate_keystrokes("."); key(&mut tui, KeyCode::Char('.'));
    wait(&mut visual, &mut tui, &mut terminal, &|state| state.completion().is_some());
    same(&mut visual, &tui);
    visual.simulate_keystrokes("escape"); key(&mut tui, KeyCode::Esc);
    same(&mut visual, &tui);
    assert!(tui.session.state().completion().is_none());
    visual.simulate_keystrokes("h"); key(&mut tui, KeyCode::Char('h'));
    wait(&mut visual, &mut tui, &mut terminal, &|state| state.completion().is_some());
    visual.simulate_keystrokes("("); key(&mut tui, KeyCode::Char('('));
    same(&mut visual, &tui);
    assert_eq!(snapshot(tui.session.state()).text.as_deref(), Some(text.replacen("{  }", "{ helper.h( }", 1).as_str()));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
}

#[gpui::test]
fn gui_and_terminal_choose_and_show_project_editing_preferences_through_find_action(cx: &mut TestAppContext) {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path().join("project"); std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join(".editorconfig"), "[*.rs]\nindent_size = 2\n").unwrap();
    let path = root.join("a.rs"); let source = "fn private_body() {}\n"; std::fs::write(&path, source).unwrap();
    let (gc, tc) = (config(&scratch.path().join("gui")), config(&scratch.path().join("tui")));
    let (gs, ts) = (gc.settings.clone(), tc.settings.clone());
    let mut core = Session::new(gc); core.dispatch(Command::OpenRoot(root.clone()));
    let gui = cx.new(|cx| ReaderView::new(core, Some(path.clone()), cx));
    let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| gui.clone())).unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let focus = gui.read_with(&visual, |v, _| v.focus_handle()); visual.update(|w, cx| focus.focus(w, cx));
    let mut tui = App::new(tc, Some(root.clone()));
    tui.session.dispatch(Command::Open(path.clone()));
    let key = |app: &mut App, code, modifiers| { app.handle(Event::Key(KeyEvent::new(code, modifiers))); };
    let settle = |visual: &mut VisualTestContext, tui: &mut App| {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            gui.update(visual, |v, cx| v.poll(cx));
            while let Ok(c) = tui.session.completions().try_recv() { tui.apply(c); }
            let ready = |s: &State| s.current_source().is_some_and(|d| d.source() == path) && s.navigation().is_none_or(|n| !n.pending);
            if gui.read_with(visual, |v, _| ready(v.session().state())) && ready(tui.session.state()) { break; }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
    };
    let find_action = |visual: &mut VisualTestContext, tui: &mut App, query: &str| {
        let typed: Vec<String> = query.chars().map(|c| if c == ' ' { "space".into() } else { c.to_string() }).collect();
        visual.simulate_keystrokes(&format!("ctrl-shift-a {}", typed.join(" ")));
        key(tui, KeyCode::Char('a'), KeyModifiers::CONTROL | KeyModifiers::SHIFT);
        tui.handle(Event::Paste(query.into()));
        settle(visual, tui);
        // Enter accepts the revision the GUI drew.
        visual.update(|w, cx| { let _ = w.draw(cx); });
        visual.simulate_keystrokes("enter");
        key(tui, KeyCode::Enter, KeyModifiers::NONE);
        settle(visual, tui);
    };
    let on = |s: &Session, id| s.actions().iter().find(|a| a.id == id).and_then(|a| a.on);
    settle(&mut visual, &mut tui);
    find_action(&mut visual, &mut tui, "indentation tabs");
    find_action(&mut visual, &mut tui, "format on save for this project");
    for id in [CommandId::IndentTabs, CommandId::FormatOnSave] {
        assert_eq!(gui.read_with(&visual, |v, _| on(v.session(), id)), Some(true), "{id:?} through the GUI");
        assert_eq!(on(&tui.session, id), Some(true), "{id:?} through the terminal");
    }
    find_action(&mut visual, &mut tui, "editing preferences");
    let expected = [
        "Indent style: tabs (this project's choice)".to_owned(),
        format!("Indent size: 2 (EditorConfig {})", root.join(".editorconfig").display()),
        "Format on save: on for this project (Lenz does not format yet)".to_owned(),
    ];
    let shown = gui.read_with(&visual, |v, _| v.modal_rows()).into_iter().map(|(row, _)| row).collect::<Vec<_>>();
    assert_eq!(shown[0], "Editing preferences");
    for row in &expected { assert!(shown.contains(row), "GUI modal lacks {row}: {shown:?}"); }
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(200, 40)).unwrap();
    terminal.draw(|f| tui.draw(f)).unwrap();
    let screen: String = terminal.backend().buffer().content().chunks(200).map(|r| r.iter().map(|c| c.symbol()).collect::<String>() + "\n").collect();
    assert!(screen.contains("Editing preferences"), "{screen}");
    for row in &expected { assert!(screen.contains(row.as_str()), "terminal lacks {row}:\n{screen}"); }
    let Some(crate::app::Prompt::Details { body, .. }) = tui.modal.as_ref().map(|m| &m.kind) else { panic!("no terminal details") };
    assert!(!body.contains("private_body") && !shown.iter().any(|r| r.contains("private_body")), "the rows never quote the source");
    // Tiny layouts and dismissal leave the documents and project files as they were.
    for (w, h) in [(1u16, 1u16), (26, 8)] { ratatui::Terminal::new(ratatui::backend::TestBackend::new(w, h)).unwrap().draw(|f| tui.draw(f)).unwrap(); }
    for (w, h) in [(1., 1.), (180., 90.), (800., 400.)] { visual.simulate_resize(gpui::size(gpui::px(w), gpui::px(h))); visual.update(|w, cx| { let _ = w.draw(cx); }); }
    visual.simulate_keystrokes("escape"); key(&mut tui, KeyCode::Esc, KeyModifiers::NONE); settle(&mut visual, &mut tui);
    assert!(gui.read_with(&visual, |v, _| v.modal_rows()).is_empty() && tui.modal_title().is_empty());
    assert_eq!(gui.read_with(&visual, |v, _| snapshot(v.session().state())), snapshot(tui.session.state()));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), source);
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 2, "nothing was written into the project");
    let deadline = Instant::now() + Duration::from_secs(5);
    let saved = |p: &std::path::PathBuf| std::fs::read(p).ok().and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok()).map(|v| v["projects"].clone());
    while saved(&gs) != Some(serde_json::json!({ root.to_str().unwrap(): { "indent": "Tabs", "format_on_save": true } })) || saved(&gs) != saved(&ts) {
        assert!(Instant::now() < deadline, "{:?} {:?}", saved(&gs), saved(&ts)); std::thread::sleep(Duration::from_millis(2));
    }
}

#[gpui::test]
fn gui_and_terminal_show_the_same_git_status_and_dirty_marks_through_edit_and_save(cx: &mut TestAppContext) {
    use lenz_content::git::testing::{commit_all, git, repo, write};
    let make = || {
        let r = repo();
        write(r.path(), "clean.txt", "clean\n");
        write(r.path(), "edited.txt", "base\n");
        write(r.path(), "staged.txt", "base\n");
        commit_all(r.path(), "base");
        write(r.path(), "edited.txt", "edited\n");
        write(r.path(), "staged.txt", "staged\n");
        git(r.path(), &["add", "staged.txt"]);
        write(r.path(), "loose/new.txt", "new\n");
        r
    };
    let (gr, tr) = (make(), make());
    let state = tempfile::tempdir().unwrap();
    use reader_core::session::Command;
    use lenz_content::branches::FileStatus;
    let mut core = Session::new(config(&state.path().join("gui")));
    core.dispatch(Command::OpenRoot(gr.path().to_path_buf()));
    let gui = cx.new(|cx| ReaderView::new(core, Some(gr.path().join("clean.txt")), cx));
    let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| gui.clone())).unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let focus = gui.read_with(&visual, |v, _| v.focus_handle());
    visual.update(|w, cx| focus.focus(w, cx));
    let mut tui = App::new(config(&state.path().join("tui")), Some(tr.path().to_path_buf()));
    tui.session.dispatch(Command::OpenRetained(tr.path().join("clean.txt")));
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 24)).unwrap();
    // Rows and tabs by name, with their Git status and unsaved mark: comparable across the two repositories.
    let marks = |s: &State| {
        let rows = s.tree_rows().map(|r| (r.path.to_string(), s.row_status(r), s.row_dirty(r))).collect::<Vec<_>>();
        let tabs = s.tabs().iter().map(|t| (t.title(), s.tab_status(t.id()), s.tab_dirty(t.id()))).collect::<Vec<_>>();
        (rows, tabs)
    };
    let settle = |visual: &mut VisualTestContext, tui: &mut App, done: &dyn Fn(&State) -> bool| {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            gui.update(visual, |v, cx| v.poll(cx));
            while let Ok(c) = tui.session.completions().try_recv() { tui.apply(c); }
            if gui.read_with(visual, |v, _| done(v.session().state())) && done(tui.session.state()) { break; }
            assert!(Instant::now() < deadline, "adapters did not settle: {:?} / {:?}", gui.read_with(visual, |v, _| marks(v.session().state())), marks(tui.session.state()));
            std::thread::sleep(Duration::from_millis(2));
        }
    };
    settle(&mut visual, &mut tui, &|s| s.tree_rows().any(|r| s.row_status(r) == Some(FileStatus::Untracked)));
    gui.update(&mut visual, |v, _| v.enable_draft_input_for_test());
    tui.enable_draft_input_for_test();
    settle(&mut visual, &mut tui, &|s| s.current_source().is_some());
    visual.update(|w, cx| { let _ = w.draw(cx); });
    terminal.draw(|f| tui.draw(f)).unwrap();
    visual.simulate_keystrokes("x");
    tui.handle(Event::Key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE)));
    let dirty = gui.read_with(&visual, |v, _| marks(v.session().state()));
    assert_eq!(dirty, marks(tui.session.state()));
    let m = |p: &str, s: Option<FileStatus>, d: bool| (p.to_string(), s, d);
    assert_eq!(dirty, (
        vec![m("loose", Some(FileStatus::Untracked), false), m("clean.txt", None, true), m("edited.txt", Some(FileStatus::Modified), false), m("staged.txt", Some(FileStatus::Modified), false)],
        vec![("clean.txt".to_string(), None, true)],
    ));
    visual.simulate_keystrokes("ctrl-s");
    tui.handle(Event::Key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)));
    settle(&mut visual, &mut tui, &|s| s.tabs().iter().all(|t| !s.tab_dirty(t.id()) && s.tab_status(t.id()) == Some(FileStatus::Modified)));
    assert_eq!(gui.read_with(&visual, |v, _| marks(v.session().state())), marks(tui.session.state()));
    for r in [&gr, &tr] {
        assert_eq!(std::fs::read_to_string(r.path().join("clean.txt")).unwrap(), "xclean\n");
        assert_eq!(git(r.path(), &["diff", "--cached", "--name-only"]), "staged.txt\n", "Save leaves the index alone");
    }
}
