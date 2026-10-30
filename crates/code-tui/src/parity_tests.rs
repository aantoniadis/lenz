//! Replay actual keys in both adapters and compare the resulting semantic state and source bytes.
mod blank_line_completion;
mod enter_indent;
mod json;
mod vim;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use gpui::{AppContext, TestAppContext, VisualTestContext};
use code_core::session::{Command, CommandId, Config, Session, State};
use code_gui::ReaderView;
use std::{
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

use crate::app::App;

fn menu_path(session: &Session, id: CommandId) -> Vec<usize> {
    use code_core::chrome::MenuTarget;
    let group = id.group();
    let mut parent = None;
    let mut steps = Vec::new();
    for group in group.parent().into_iter().chain([group]) {
        steps.push(session.menu(parent).iter().position(|r| r.target == MenuTarget::Group(group)).unwrap());
        parent = Some(group);
    }
    steps.push(session.menu(parent).iter().position(|r| r.target == MenuTarget::Run(id)).unwrap());
    steps
}

fn copied_selection(state: &code_core::session::State) -> Option<String> {
    state.view(state.active()?.id())?.copy_selection()
}

fn config(base: &Path) -> Config {
    let providers = base.join("providers.json");
    std::fs::create_dir_all(base).unwrap();
    let providers_json: serde_json::Map<_, _> = code_core::semantics::provider::PROVIDERS
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
            "lenz-code",
        ),
        recent: base.join("state/recent.json"),
        settings: base.join("settings.json"),
        providers,
        cache: base.join("cache"),
        idle: code_core::semantics::IDLE,
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
    selection: Option<code_core::session::Selection>,
    /// Every selection of the active view, in source order (several cursors included).
    selections: Vec<code_core::session::Selection>,
    split: Option<code_core::session::Split>,
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
    // Tab indents with the file's step (a plain text file's default is four spaces), not a bare tab.
    visual.simulate_keystrokes("enter tab"); key(&mut tui, KeyCode::Enter, KeyModifiers::NONE); key(&mut tui, KeyCode::Tab, KeyModifiers::NONE); check(&visual, &tui, "e\u{301}👩🏽‍💻\r\nqjk/ qjk/ \r\n    ");
    visual.simulate_keystrokes("backspace ctrl-z"); key(&mut tui, KeyCode::Backspace, KeyModifiers::NONE); key(&mut tui, KeyCode::Char('z'), KeyModifiers::CONTROL); check(&visual, &tui, "e\u{301}👩🏽‍💻\r\nqjk/ qjk/ \r\n    ");
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
    visual.simulate_keystrokes("right ctrl-alt-down");
    key(&mut tui, KeyCode::Right, KeyModifiers::NONE);
    key(&mut tui, KeyCode::Down, KeyModifiers::CONTROL | KeyModifiers::ALT);
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
    wait(&mut visual, &mut tui, &mut terminal, &|state| state.project().is_some_and(|project| matches!(project.coverage(), code_core::project::Coverage::Complete(_))) && state.index_coverage().complete());
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
        use code_core::navigation::{Kind, Scope};
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
    use code_core::session::{Rendering, Split};
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
                let done = |s: &Session| s.state().provider().is_some_and(|v| matches!(v.status, code_core::semantics::Status::Missing(_)))
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
        // A disabled action now keeps its explanation open; dismiss it before opening another group.
        visual.simulate_keystrokes("escape escape escape");
        key(&mut tui, KeyCode::Esc, KeyModifiers::NONE);
        key(&mut tui, KeyCode::Esc, KeyModifiers::NONE);
        key(&mut tui, KeyCode::Esc, KeyModifiers::NONE);
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
        for row in menu_path(&tui.session, id) {
            visual.simulate_keystrokes(&format!("home {}enter", "down ".repeat(row)));
            key(&mut tui, KeyCode::Home, KeyModifiers::NONE);
            for _ in 0..row { key(&mut tui, KeyCode::Down, KeyModifiers::NONE); }
            key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
        }
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
            .is_some_and(|p| matches!(p.coverage(), code_core::project::Coverage::Complete(_)))
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
    let saved_file = code_core::places::file_for(&base.join("state/projects"), &root);
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
    use code_core::navigation::{Kind, Scope};
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
            .is_some_and(|p| matches!(p.coverage(), code_core::project::Coverage::Complete(_)))
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
            Some(code_core::session::Selection { anchor: head, head })
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
        Some(code_core::session::Selection {
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
    use code_core::{project::Coverage, semantics::Status};
    let profile = std::env::current_exe().unwrap().parent().unwrap().parent().unwrap().to_owned();
    assert!(std::process::Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .args(["build", "-q", "--locked", "-p", "code-core", "--example", "fake_lsp"])
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
        for row in menu_path(&tui.session, id) {
            visual.simulate_keystrokes(&format!("home {}enter", "down ".repeat(row)));
            key(tui, KeyCode::Home, KeyModifiers::NONE);
            for _ in 0..row { key(tui, KeyCode::Down, KeyModifiers::NONE); }
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
    for row in menu_path(&tui.session, CommandId::GitBranches) {
        visual.simulate_keystrokes(&format!("home {}enter", "down ".repeat(row)));
        key(&mut tui, KeyCode::Home, KeyModifiers::NONE);
        for _ in 0..row { key(&mut tui, KeyCode::Down, KeyModifiers::NONE); }
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
    let group = code_core::actions::CommandGroup::ALL.iter().filter(|g| g.parent().is_none()).position(|g| *g == code_core::actions::CommandGroup::Git).unwrap();
    let row = CommandId::ALL.iter().filter(|c| c.group() == code_core::actions::CommandGroup::Git).position(|c| *c == CommandId::ShowLocalChanges).unwrap();
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
    use code_core::{project::Coverage, search::ResultsKind, semantics::Status};
    let profile = std::env::current_exe().unwrap().parent().unwrap().parent().unwrap().to_owned();
    assert!(std::process::Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .args(["build", "-q", "--locked", "-p", "code-core", "--example", "fake_lsp"])
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
    let conversions = code_core::text::TextSnapshot::conversion_calls_on_thread_for_test();
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
    assert_eq!(code_core::text::TextSnapshot::conversion_calls_on_thread_for_test(), conversions);
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
        visual.simulate_keystrokes("ctrl-s ctrl-a"); visual.update(|_, cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string(targets[0].display().to_string()))); visual.simulate_keystrokes("ctrl-v enter"); cx.run_until_parked();
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
        .args(["build", "-q", "--locked", "-p", "code-core", "--example", "fake_lsp"])
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
    let ready = |state: &State| state.provider().is_some_and(|p| p.status == code_core::semantics::Status::Ready) && state.view(state.active().unwrap().id()).is_some_and(|v| v.highlights().is_some());
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
    // A trigger character opens it in both; Escape dismisses alike; punctuation types (here with its pair) without accepting.
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
    assert_eq!(snapshot(tui.session.state()).text.as_deref(), Some(text.replacen("{  }", "{ helper.h() }", 1).as_str()));
    // A suggestion with an import: Enter applies the name and the import in both, and one Undo takes both back.
    visual.simulate_keystrokes("w i t h"); for c in "with".chars() { key(&mut tui, KeyCode::Char(c)); }
    wait(&mut visual, &mut tui, &mut terminal, &|state| state.completion().is_some_and(|c| c.rows().any(|r| r.label == "with_import")));
    same(&mut visual, &tui);
    visual.simulate_keystrokes("enter"); key(&mut tui, KeyCode::Enter);
    same(&mut visual, &tui);
    let imported = format!("use extra::with_import;\n{}", text.replacen("{  }", "{ helper.h(with_import) }", 1));
    assert_eq!(snapshot(tui.session.state()).text.as_deref(), Some(imported.as_str()));
    visual.simulate_keystrokes("ctrl-z"); tui.handle(Event::Key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL)));
    same(&mut visual, &tui);
    assert_eq!(snapshot(tui.session.state()).text.as_deref(), Some(text.replacen("{  }", "{ helper.h(with) }", 1).as_str()));
    // Parameter information: a trigger character opens the same signature in both, drawn in each; a comma moves both
    // to the second overload; Escape closes both.
    let signature = |state: &State| state.signature_help().and_then(|h| h.active_signature().map(|s| (s.label.clone(), s.active_parameter, h.active)));
    visual.simulate_keystrokes("space p i c k ("); for c in " pick(".chars() { key(&mut tui, KeyCode::Char(c)); }
    wait(&mut visual, &mut tui, &mut terminal, &|state| state.signature_help().is_some());
    assert_eq!(signature(tui.session.state()), Some(("fn pick(a: i32) -> i32".to_string(), Some(0), 0)));
    assert_eq!(gui.read_with(&visual, |view, _| signature(view.session().state())), signature(tui.session.state()));
    assert!(gui.read_with(&visual, |view, _| view.signature_bounds().is_some()) && tui.layout.signature.width > 0, "both draw the box");
    visual.simulate_keystrokes("1 ,"); key(&mut tui, KeyCode::Char('1')); key(&mut tui, KeyCode::Char(','));
    wait(&mut visual, &mut tui, &mut terminal, &|state| state.signature_help().is_some_and(|h| h.active == 1));
    assert_eq!(signature(tui.session.state()), Some(("fn pick(größe: f64, b: &str) -> f64".to_string(), Some(1), 1)));
    assert_eq!(gui.read_with(&visual, |view, _| signature(view.session().state())), signature(tui.session.state()));
    same(&mut visual, &tui);
    visual.simulate_keystrokes("escape"); key(&mut tui, KeyCode::Esc);
    wait(&mut visual, &mut tui, &mut terminal, &|state| state.signature_help().is_none());
    same(&mut visual, &tui);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
}

#[gpui::test]
fn problems_and_quick_fixes_show_navigate_and_apply_alike_in_both_adapters(cx: &mut TestAppContext) {
    let profile = std::env::current_exe().unwrap().parent().unwrap().parent().unwrap().to_owned();
    assert!(std::process::Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .args(["build", "-q", "--locked", "-p", "code-core", "--example", "fake_lsp"])
        .current_dir(env!("CARGO_MANIFEST_DIR")).status().unwrap().success());
    let fake = profile.join("examples/fake_lsp");
    let dir = tempfile::tempdir().unwrap(); let root = dir.path().join("project"); std::fs::create_dir(&root).unwrap();
    let path = root.join("main.rs");
    let text = "fn main() {\n    let unused_x = bad_name;\n}\n";
    std::fs::write(root.join("Cargo.toml"), "[package]\nname='problems'\nversion='0.1.0'\n").unwrap(); std::fs::write(&path, text).unwrap();
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
    let problems = |state: &State| state.active().and_then(|t| state.diagnostics(t.id())).map_or(0, |d| d.items.len());
    wait(&mut visual, &mut tui, &mut terminal, &|state| problems(state) == 2);
    gui.update(&mut visual, |view, _| view.enable_draft_input_for_test()); tui.enable_draft_input_for_test();
    let key = |tui: &mut App, code, modifiers| { tui.handle(Event::Key(KeyEvent::new(code, modifiers))); };
    let same = |visual: &mut VisualTestContext, tui: &App| {
        assert_eq!(gui.read_with(visual, |view, _| snapshot(view.session().state())), snapshot(tui.session.state()));
    };
    // Each draws the two problems: the warning and the error, in their colours.
    let palette = tui.session.state().palette();
    assert_eq!(gui.read_with(&visual, |view, _| view.problem_marks()), vec![("unused_x".to_string(), palette.accent), ("bad_name".to_string(), palette.error)]);
    let screen = terminal.backend().buffer().clone();
    // Terminal hits name a line and the byte within it.
    let marked = |byte: usize| {
        let line = text[..byte].matches('\n').count();
        let within = byte - text[..byte].rfind('\n').map_or(0, |i| i + 1);
        tui.layout.panes[0].hits.iter().find(|h| h.line == line && h.byte == within).map(|h| screen[(h.x, h.y)].style())
    };
    let style = |byte| marked(byte).unwrap();
    assert!(style(text.find("bad_name").unwrap()).add_modifier.contains(ratatui::style::Modifier::UNDERLINED));
    assert_eq!(style(text.find("bad_name").unwrap()).fg, Some(crate::render::color(palette.error)));
    assert!(!style(text.find("let").unwrap()).add_modifier.contains(ratatui::style::Modifier::UNDERLINED), "only the problems are marked");
    // F8 moves both carets to the first problem, then the next; both name it in the status line.
    visual.simulate_keystrokes("f8"); key(&mut tui, KeyCode::F(8), KeyModifiers::NONE);
    assert_eq!(tui.session.state().selection().map(|s| s.head), Some(text.find("unused_x").unwrap()));
    same(&mut visual, &tui);
    visual.simulate_keystrokes("f8"); key(&mut tui, KeyCode::F(8), KeyModifiers::NONE);
    assert_eq!(tui.session.state().selection().map(|s| s.head), Some(text.find("bad_name").unwrap()));
    same(&mut visual, &tui);
    assert_eq!(tui.session.state().problem_at_caret().as_deref(), Some("Error: cannot find value `bad_name` in this scope (fake E0425)"));
    // Alt+Enter lists the same fixes, drawn in each; the import (another file) is refused alike.
    let fixes = |state: &State| state.code_actions().map(|c| (c.actions.iter().map(|a| a.title.clone()).collect::<Vec<_>>(), c.selected));
    visual.simulate_keystrokes("alt-enter"); key(&mut tui, KeyCode::Enter, KeyModifiers::ALT);
    wait(&mut visual, &mut tui, &mut terminal, &|state| state.code_actions().is_some());
    assert_eq!(gui.read_with(&visual, |view, _| fixes(view.session().state())), fixes(tui.session.state()));
    assert_eq!(fixes(tui.session.state()).unwrap().0[0], "Replace with `good_name`");
    assert!(gui.read_with(&visual, |view, _| view.code_actions_bounds().is_some()) && tui.layout.code_actions.width > 0, "both draw the list");
    visual.simulate_keystrokes("down enter"); key(&mut tui, KeyCode::Down, KeyModifiers::NONE); key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    same(&mut visual, &tui);
    assert_eq!(snapshot(tui.session.state()).text.as_deref(), Some(text), "a fix that changes another file is not applied");
    assert!(tui.session.state().notices().iter().any(|n| n.starts_with("Quick fixes: Import `bad_name` from `other`: Changes other files too")));
    // Escape dismisses alike; asked again, Enter applies the preferred fix as one edit, and Ctrl+Z takes it back.
    visual.simulate_keystrokes("alt-enter"); key(&mut tui, KeyCode::Enter, KeyModifiers::ALT);
    wait(&mut visual, &mut tui, &mut terminal, &|state| state.code_actions().is_some());
    // Tiny layouts draw the open list without panicking and keep it open.
    for (w, h) in [(1u16, 1u16), (26, 8)] { ratatui::Terminal::new(ratatui::backend::TestBackend::new(w, h)).unwrap().draw(|f| tui.draw(f)).unwrap(); }
    for (w, h) in [(1., 1.), (180., 90.), (800., 600.)] { visual.simulate_resize(gpui::size(gpui::px(w), gpui::px(h))); visual.update(|w, cx| { let _ = w.draw(cx); }); }
    terminal.draw(|frame| tui.draw(frame)).unwrap();
    assert!(tui.session.state().code_actions().is_some() && gui.read_with(&visual, |view, _| view.session().state().code_actions().is_some()));
    visual.simulate_keystrokes("escape"); key(&mut tui, KeyCode::Esc, KeyModifiers::NONE);
    assert!(tui.session.state().code_actions().is_none() && gui.read_with(&visual, |view, _| view.session().state().code_actions().is_none()));
    visual.simulate_keystrokes("alt-enter"); key(&mut tui, KeyCode::Enter, KeyModifiers::ALT);
    wait(&mut visual, &mut tui, &mut terminal, &|state| state.code_actions().is_some());
    visual.simulate_keystrokes("enter"); key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    same(&mut visual, &tui);
    assert_eq!(snapshot(tui.session.state()).text.as_deref(), Some(text.replace("bad_name", "good_name").as_str()));
    visual.simulate_keystrokes("ctrl-z"); key(&mut tui, KeyCode::Char('z'), KeyModifiers::CONTROL);
    same(&mut visual, &tui);
    assert_eq!(snapshot(tui.session.state()).text.as_deref(), Some(text));
    // Problems uses the shared navigable results panel in both adapters.
    wait(&mut visual, &mut tui, &mut terminal, &|state| problems(state) == 2);
    gui.update(&mut visual, |view, cx| view.dispatch_for_test(Command::CloseCodeActions, cx));
    visual.update(|window, cx| gui.update(cx, |view, cx| view.run(CommandId::ShowProblems, window, cx)));
    tui.run(CommandId::ShowProblems);
    wait(&mut visual, &mut tui, &mut terminal, &|state| state.results().is_some_and(|r| r.results_kind() == code_core::search::ResultsKind::Problems && r.hits().len() == 2));
    let reports = |state: &State| state.results().unwrap().hits().iter().map(|h| (root.join(h.path.as_ref()), h.line, h.text.to_string())).collect::<Vec<_>>();
    assert_eq!(gui.read_with(&visual, |view, _| reports(view.session().state())), reports(tui.session.state()));
    let shown = gui.update(&mut visual, |view, _| view.panel_contents(30, 0).0.into_iter().map(|row| row.label).collect::<Vec<_>>()).join("\n");
    let screen = terminal.backend().buffer().content.iter().map(|cell| cell.symbol()).collect::<String>();
    for message in ["unused variable", "cannot find value"] {
        assert!(shown.contains(message), "GUI lacks {message}: {shown}");
        assert!(screen.contains(message), "terminal lacks {message}: {screen}");
    }
    assert!(tui.layout.results.width > 0 && gui.read_with(&visual, |view, _| view.results_focus));
    visual.simulate_keystrokes("escape"); key(&mut tui, KeyCode::Esc, KeyModifiers::NONE);
    assert!(gui.read_with(&visual, |view, _| view.session().state().results().is_none()) && tui.session.state().results().is_none());
    same(&mut visual, &tui);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    assert!(!root.join("src/other.rs").exists());
}

/// Templates (ticket 57) through both adapters, the fake standing in for Ruby LSP and the HTML server: the problems of
/// both views of an HTML ERB template drawn alike; a Ruby name typed into an output tag and a word of the markup each
/// completed by the server of the view the caret is in (Enter), each one Ctrl+Z; the Ruby quick fix by Alt+Enter;
/// Ctrl+Alt+L running the project's erb_lint (a stand-in binstub, ticket 58) on the draft; Ctrl+Space on a tag's
/// delimiter refused alike; Server Details' template rows (the Rails view context, the formatter) the same in both. The
/// same draft in both, and the files untouched.
#[gpui::test]
fn template_assistance_routes_through_its_projections_alike_in_both_adapters(cx: &mut TestAppContext) {
    let profile = std::env::current_exe().unwrap().parent().unwrap().parent().unwrap().to_owned();
    assert!(std::process::Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .args(["build", "-q", "--locked", "-p", "code-core", "--example", "fake_lsp"])
        .current_dir(env!("CARGO_MANIFEST_DIR")).status().unwrap().success());
    let fake = profile.join("examples/fake_lsp");
    let text = "<ul>\n<li>𝄞 <% let greeting = 1 %></li>\n<li title=\"<%=  %>\">mar</li>\n<p>bad_name</p> <%= bad_name %>\n</ul>\n";
    let dir = tempfile::tempdir().unwrap();
    let make = |name: &str| {
        let root = dir.path().join(name).join("project");
        std::fs::create_dir_all(root.join("app/views")).unwrap();
        std::fs::write(root.join("Gemfile"), "").unwrap();
        std::fs::write(root.join("names.js"), "let markup_name = 1;\n").unwrap();
        std::fs::write(root.join("app/views/page.html.erb"), text).unwrap();
        std::fs::write(root.join(".erb_lint.yml"), "---\nEnableDefaultLinters: true\n").unwrap();
        // A stand-in for erb_lint 0.9.0's `--stdin FILE --autocorrect`: its report, the header, the corrected text.
        shell_tool(&root.join("bin/erb_lint"), "#!/bin/sh\ninput=$(cat; printf x)\ninput=${input%x}\necho 'Linting and autocorrecting 1 files with 14 linters (11 autocorrectable)...'\nprintf '================ %s ==================\\n' \"$4\"\nprintf '%s' \"$input\" | sed 's/<%=  */<%= /g'\n");
        let c = config(&dir.path().join(name).join("config"));
        let mut settings: serde_json::Value = serde_json::from_slice(&std::fs::read(&c.providers).unwrap()).unwrap();
        for id in ["ruby-lsp", "vscode-html"] {
            settings["providers"][id] = serde_json::json!({"executable": fake, "env": {"FAKE_LSP_NO_DECOR": "1", "FAKE_LSP_LINE_FORMAT": "1"}});
        }
        std::fs::write(&c.providers, settings.to_string()).unwrap(); (c, root)
    };
    let ((gc, groot), (tc, troot)) = (make("gui"), make("tui"));
    let file = |root: &std::path::Path| root.join("app/views/page.html.erb");
    let mut core = Session::new(gc); core.dispatch(Command::OpenRoot(groot.clone()));
    let gui = cx.new(|cx| ReaderView::new(core, Some(file(&groot)), cx));
    let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| gui.clone())).unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let focus = gui.read_with(&visual, |view, _| view.focus_handle()); visual.update(|window, cx| focus.focus(window, cx));
    let mut tui = App::new(tc, Some(troot.clone())); tui.session.dispatch(Command::Open(file(&troot)));
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
    let wait = |visual: &mut VisualTestContext, tui: &mut App, terminal: &mut ratatui::Terminal<ratatui::backend::TestBackend>, done: &dyn Fn(&State) -> bool| {
        let until = Instant::now() + Duration::from_secs(15);
        loop {
            gui.update(visual, |view, cx| view.poll(cx)); while let Ok(completion) = tui.session.completions().try_recv() { tui.apply(completion); }
            visual.update(|window, cx| { let _ = window.draw(cx); }); terminal.draw(|frame| tui.draw(frame)).unwrap();
            if gui.read_with(visual, |view, _| done(view.session().state())) && done(tui.session.state()) { break; }
            assert!(Instant::now() < until, "GUI {:?}; TUI {:?}", gui.read_with(visual, |view, _| view.session().state().notices()), tui.session.state().notices());
            std::thread::yield_now();
        }
    };
    // Both views' problems: the markup's word (HTML server) and the output tag's code (Ruby server).
    let problems = |state: &State| state.active().and_then(|t| state.diagnostics(t.id())).map_or(0, |d| d.items.len());
    wait(&mut visual, &mut tui, &mut terminal, &|state| problems(state) == 2);
    gui.update(&mut visual, |view, _| view.enable_draft_input_for_test()); tui.enable_draft_input_for_test();
    let key = |tui: &mut App, code, modifiers| { tui.handle(Event::Key(KeyEvent::new(code, modifiers))); };
    let current = |state: &State| state.current_source().map(|d| d.copy_range(0..d.len_bytes()).unwrap());
    let same = |visual: &mut VisualTestContext, tui: &App| {
        assert_eq!(gui.read_with(visual, |view, _| snapshot(view.session().state())), snapshot(tui.session.state()));
    };
    let palette = tui.session.state().palette();
    assert_eq!(gui.read_with(&visual, |view, _| view.problem_marks()), vec![("bad_name".to_string(), palette.error), ("bad_name".to_string(), palette.error)]);
    let screen = terminal.backend().buffer().clone();
    let style = |byte: usize| {
        let line = text[..byte].matches('\n').count();
        let within = byte - text[..byte].rfind('\n').map_or(0, |i| i + 1);
        tui.layout.panes[0].hits.iter().find(|h| h.line == line && h.byte == within).map(|h| screen[(h.x, h.y)].style()).unwrap()
    };
    for at in [text.find("bad_name").unwrap(), text.rfind("bad_name").unwrap()] {
        assert!(style(at).add_modifier.contains(ratatui::style::Modifier::UNDERLINED), "the terminal underlines the problem at {at}");
    }
    assert!(!style(text.find("mar<").unwrap()).add_modifier.contains(ratatui::style::Modifier::UNDERLINED));
    // The caret: a click in the terminal; in the GUI through its command seam (its click helper reads the first line).
    let click = |visual: &mut VisualTestContext, tui: &mut App, terminal: &mut ratatui::Terminal<ratatui::backend::TestBackend>, at: usize| {
        gui.update(visual, |view, cx| view.dispatch_for_test(Command::Select(code_core::session::Selection { anchor: at, head: at }), cx));
        terminal.draw(|frame| tui.draw(frame)).unwrap();
        let line = text[..at].matches('\n').count();
        let within = at - text[..at].rfind('\n').map_or(0, |i| i + 1);
        let hit = tui.layout.panes[0].hits.iter().find(|h| h.line == line && h.byte == within).unwrap().clone();
        for kind in [crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left), crossterm::event::MouseEventKind::Up(crossterm::event::MouseButton::Left)] {
            tui.handle(Event::Mouse(crossterm::event::MouseEvent { kind, column: hit.x, row: hit.y, modifiers: KeyModifiers::NONE }));
        }
        assert_eq!(tui.session.state().selection().map(|s| s.head), Some(at));
        assert_eq!(gui.read_with(visual, |view, _| view.session().state().selection().map(|s| s.head)), Some(at));
    };
    let labels = |state: &State| state.completion().map(|c| c.rows().map(|r| r.label.clone()).collect::<Vec<_>>());
    // Ruby, typed into the empty output tag: the Ruby server's name.
    click(&mut visual, &mut tui, &mut terminal, text.find("<%=  %>").unwrap() + 4);
    visual.simulate_keystrokes("g r e"); for c in "gre".chars() { key(&mut tui, KeyCode::Char(c), KeyModifiers::NONE); }
    wait(&mut visual, &mut tui, &mut terminal, &|state| labels(state).is_some_and(|l| l == ["greeting"]));
    assert!(gui.read_with(&visual, |view, _| view.completion_bounds().is_some()) && terminal.backend().buffer().content().iter().map(|c| c.symbol()).collect::<String>().contains("greeting"), "both draw the suggestion");
    visual.simulate_keystrokes("enter"); key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    same(&mut visual, &tui);
    assert_eq!(current(tui.session.state()).as_deref(), Some(text.replacen("<%=  %>", "<%= greeting %>", 1).as_str()));
    visual.simulate_keystrokes("ctrl-z"); key(&mut tui, KeyCode::Char('z'), KeyModifiers::CONTROL);
    same(&mut visual, &tui);
    assert_eq!(current(tui.session.state()).as_deref(), Some(text.replacen("<%=  %>", "<%= gre %>", 1).as_str()), "one Ctrl+Z takes the suggestion back");
    visual.simulate_keystrokes("ctrl-z"); key(&mut tui, KeyCode::Char('z'), KeyModifiers::CONTROL);
    assert_eq!(current(tui.session.state()).as_deref(), Some(text));
    // Markup, typed after its word: the HTML server's name.
    click(&mut visual, &mut tui, &mut terminal, text.find("mar<").unwrap() + 3);
    visual.simulate_keystrokes("k"); key(&mut tui, KeyCode::Char('k'), KeyModifiers::NONE);
    wait(&mut visual, &mut tui, &mut terminal, &|state| labels(state).is_some_and(|l| l == ["markup_name"]));
    visual.simulate_keystrokes("enter"); key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    same(&mut visual, &tui);
    assert_eq!(current(tui.session.state()).as_deref(), Some(text.replacen(">mar<", ">markup_name<", 1).as_str()));
    for _ in 0..2 { visual.simulate_keystrokes("ctrl-z"); key(&mut tui, KeyCode::Char('z'), KeyModifiers::CONTROL); }
    assert_eq!(current(tui.session.state()).as_deref(), Some(text));
    same(&mut visual, &tui);
    // The Ruby problem's quick fix, listed and drawn alike, applied inside its tag; the markup's word stays.
    wait(&mut visual, &mut tui, &mut terminal, &|state| problems(state) == 2);
    click(&mut visual, &mut tui, &mut terminal, text.rfind("bad_name").unwrap() + 3);
    visual.simulate_keystrokes("alt-enter"); key(&mut tui, KeyCode::Enter, KeyModifiers::ALT);
    wait(&mut visual, &mut tui, &mut terminal, &|state| state.code_actions().is_some());
    let fixes = |state: &State| state.code_actions().map(|c| c.actions.iter().map(|a| a.title.clone()).collect::<Vec<_>>());
    assert_eq!(gui.read_with(&visual, |view, _| fixes(view.session().state())), fixes(tui.session.state()));
    assert!(gui.read_with(&visual, |view, _| view.code_actions_bounds().is_some()) && tui.layout.code_actions.width > 0, "both draw the list");
    visual.simulate_keystrokes("enter"); key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    same(&mut visual, &tui);
    assert_eq!(current(tui.session.state()).as_deref(), Some(text.replacen("<%= bad_name %>", "<%= good_name %>", 1).as_str()));
    visual.simulate_keystrokes("ctrl-z"); key(&mut tui, KeyCode::Char('z'), KeyModifiers::CONTROL);
    assert_eq!(current(tui.session.state()).as_deref(), Some(text));
    // Ctrl+Alt+L: the project's erb_lint corrects the draft (no language server is asked); one Ctrl+Z.
    visual.simulate_keystrokes("ctrl-alt-l"); key(&mut tui, KeyCode::Char('l'), KeyModifiers::CONTROL | KeyModifiers::ALT);
    let formatted = text.replacen("<%=  %>", "<%= %>", 1);
    wait(&mut visual, &mut tui, &mut terminal, &|state| current(state).as_deref() == Some(formatted.as_str()) && state.notices().iter().any(|n| n == "Format Document: done with erb_lint"));
    visual.simulate_keystrokes("ctrl-z"); key(&mut tui, KeyCode::Char('z'), KeyModifiers::CONTROL);
    same(&mut visual, &tui);
    assert_eq!(current(tui.session.state()).as_deref(), Some(text));
    // Ctrl+Space on a tag's own delimiter asks neither server, and says so alike.
    click(&mut visual, &mut tui, &mut terminal, text.rfind("<%= bad_name").unwrap() + 1);
    visual.simulate_keystrokes("ctrl-space"); key(&mut tui, KeyCode::Char(' '), KeyModifiers::CONTROL);
    let why = "Code completion: the caret is on one of the ERB template's own delimiters or comments, which no language server is asked about";
    wait(&mut visual, &mut tui, &mut terminal, &|state| state.notices().iter().any(|n| n == why));
    // Server Details (through Find Action) shows the same template rows in both.
    visual.simulate_keystrokes("ctrl-shift-a s e r v e r space d e t a i l s");
    key(&mut tui, KeyCode::Char('a'), KeyModifiers::CONTROL | KeyModifiers::SHIFT);
    tui.handle(Event::Paste("server details".into()));
    wait(&mut visual, &mut tui, &mut terminal, &|state| state.navigation().is_some_and(|n| !n.pending));
    visual.simulate_keystrokes("enter"); key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    let shown = gui.read_with(&visual, |v, _| v.modal_rows()).into_iter().map(|(row, _)| row).collect::<Vec<_>>();
    let Some(crate::app::Prompt::Details { body, .. }) = tui.modal.as_ref().map(|m| &m.kind) else { panic!("no terminal details: {:?}", tui.modal_title()) };
    let body = body.clone();
    for row in ["ERB template Ruby: 3 regions mapped exactly to the template, served by the Ruby provider; edits apply only inside them",
        "ERB template view context: a Rails view (Action View), with ApplicationHelper; controller instance variables and route helpers are not known to Ruby LSP's index",
        "ERB template markup: 4 regions mapped exactly to the template, served by the HTML provider (Ruby tags hidden); edits apply only inside them"] {
        assert!(shown.iter().any(|r| r == row), "GUI lacks {row}: {shown:?}");
        assert!(body.contains(row), "terminal lacks {row}: {body}");
    }
    let formatter = |root: &std::path::Path| format!("Format Document runs the project's erb_lint, not a language server: erb_lint (configured by {}); run as {} with --autocorrect", root.join(".erb_lint.yml").display(), root.join("bin/erb_lint").display());
    assert!(shown.iter().any(|r| *r == formatter(&groot)), "GUI lacks the formatter: {shown:?}");
    assert!(body.contains(&formatter(&troot)), "terminal lacks the formatter: {body}");
    visual.simulate_keystrokes("escape"); key(&mut tui, KeyCode::Esc, KeyModifiers::NONE);
    assert!(gui.read_with(&visual, |v, _| v.modal_rows()).is_empty() && tui.modal_title().is_empty());
    same(&mut visual, &tui);
    for root in [&groot, &troot] { assert_eq!(std::fs::read_to_string(file(root)).unwrap(), text); }
}

/// Real Ruby LSP and the HTML server behind both adapters on an ERB template (ticket 57): Ruby's problem at the
/// template's own `]` underlined in both; `Greeter.gre` typed into an output tag and `greet` chosen with Enter from
/// the suggestions typing opened, drawn in both; parameter information by Ctrl+Shift+Space inside a call; a tag name
/// in the markup from the HTML server; Ctrl+Alt+L saying no ERB formatter is configured (ticket 58: Ruby LSP does not
/// format templates). Each change undone with Ctrl+Z; the same draft in both and the files untouched. Needs the `ruby-lsp` and
/// `rubocop` gems and `vscode-html-language-server` (npm `vscode-langservers-extracted`) on PATH.
#[gpui::test]
#[ignore = "needs installed ruby-lsp and rubocop gems and vscode-html-language-server (Node.js); run alone"]
fn real_erb_template_assistance_matches_in_both_adapters(cx: &mut TestAppContext) {
    let Some(ruby_lsp) = std::env::var_os("READER_RUBY_LSP").map(std::path::PathBuf::from)
        .or_else(|| code_core::semantics::provider::find_executable(std::path::Path::new("ruby-lsp"), None)) else {
        println!("SKIPPED: Ruby LSP is not installed");
        return;
    };
    let Some(html) = code_core::semantics::provider::find_executable(std::path::Path::new("vscode-html-language-server"), None) else {
        println!("SKIPPED: vscode-html-language-server is not installed");
        return;
    };
    let greeter = "# frozen_string_literal: true\n\nmodule Greeter\n  def self.greet(name, excited: false)\n    excited ? \"#{name}!\" : name\n  end\nend\n";
    let text = "<ul class=\"𝄞\">\r\n<% names = [\"é\", \"𝄞\"] %>\r\n<li><%=  %></li>\r\n<li><%= Greeter.greet(names.first, excited: true) %></li>\r\n<p>𝄞 </p>\r\n<%= names] %>\r\n</ul>\r\n";
    let dir = tempfile::tempdir().unwrap();
    let make = |name: &str| {
        let root = dir.path().join(name).join("project");
        std::fs::create_dir_all(root.join("lib")).unwrap();
        std::fs::create_dir_all(root.join("app/views")).unwrap();
        std::fs::write(root.join("Gemfile"), "source \"https://rubygems.org\"\n\ngem \"rubocop\"\n").unwrap();
        std::fs::write(root.join(".rubocop.yml"), "AllCops:\n  NewCops: disable\n  SuggestExtensions: false\n").unwrap();
        std::fs::write(root.join("lib/greeter.rb"), greeter).unwrap();
        std::fs::write(root.join("app/views/greet.html.erb"), text).unwrap();
        assert!(std::process::Command::new("bundle").args(["lock", "--local"]).current_dir(&root).status().is_ok_and(|s| s.success()));
        let c = config(&dir.path().join(name).join("config"));
        let mut settings: serde_json::Value = serde_json::from_slice(&std::fs::read(&c.providers).unwrap()).unwrap();
        settings["providers"]["ruby-lsp"] = serde_json::json!({ "executable": ruby_lsp });
        settings["providers"]["vscode-html"] = serde_json::json!({ "executable": html });
        std::fs::write(&c.providers, settings.to_string()).unwrap(); (c, root)
    };
    let ((gc, groot), (tc, troot)) = (make("gui"), make("tui"));
    let file = |root: &std::path::Path| root.join("app/views/greet.html.erb");
    let mut core = Session::new(gc); core.dispatch(Command::OpenRoot(groot.clone()));
    let gui = cx.new(|cx| ReaderView::new(core, Some(file(&groot)), cx));
    let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| gui.clone())).unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let focus = gui.read_with(&visual, |view, _| view.focus_handle()); visual.update(|window, cx| focus.focus(window, cx));
    let mut tui = App::new(tc, Some(troot.clone())); tui.session.dispatch(Command::Open(file(&troot)));
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 40)).unwrap();
    let wait = |visual: &mut VisualTestContext, tui: &mut App, terminal: &mut ratatui::Terminal<ratatui::backend::TestBackend>, limit: u64, done: &dyn Fn(&State) -> bool| {
        let until = Instant::now() + Duration::from_secs(limit);
        loop {
            gui.update(visual, |view, cx| view.poll(cx)); while let Ok(completion) = tui.session.completions().try_recv() { tui.apply(completion); }
            visual.update(|window, cx| { let _ = window.draw(cx); }); terminal.draw(|frame| tui.draw(frame)).unwrap();
            if gui.read_with(visual, |view, _| done(view.session().state())) && done(tui.session.state()) { break; }
            assert!(Instant::now() < until, "GUI {:?}; TUI {:?} {:?}", gui.read_with(visual, |view, _| view.session().state().notices()), tui.session.state().notices(), tui.modal_title());
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    let screen = |terminal: &ratatui::Terminal<ratatui::backend::TestBackend>| terminal.backend().buffer().content().chunks(120).map(|r| r.iter().map(|c| c.symbol()).collect::<String>() + "\n").collect::<String>();
    let problems = |state: &State| state.active().and_then(|t| state.diagnostics(t.id())).map(|d| d.items.iter().map(|p| p.range.clone()).collect::<Vec<_>>()).unwrap_or_default();
    let close = text.find("names]").unwrap() + 5;
    wait(&mut visual, &mut tui, &mut terminal, 120, &|state| problems(state).first() == Some(&(close..close + 1)));
    let palette = tui.session.state().palette();
    assert!(gui.read_with(&visual, |view, _| view.problem_marks()).iter().all(|m| *m == ("]".to_string(), palette.error)), "the GUI marks the `]`");
    gui.update(&mut visual, |view, _| view.enable_draft_input_for_test()); tui.enable_draft_input_for_test();
    let key = |tui: &mut App, code, modifiers| { tui.handle(Event::Key(KeyEvent::new(code, modifiers))); };
    let current = |state: &State| state.current_source().map(|d| d.copy_range(0..d.len_bytes()).unwrap());
    let caret = |visual: &mut VisualTestContext, tui: &mut App, at: usize| {
        gui.update(visual, |view, cx| view.dispatch_for_test(Command::Select(code_core::session::Selection { anchor: at, head: at }), cx));
        tui.session.dispatch(Command::Select(code_core::session::Selection { anchor: at, head: at }));
    };
    let both = |visual: &mut VisualTestContext, tui: &App, expected: &str| {
        assert_eq!(current(tui.session.state()).as_deref(), Some(expected));
        assert_eq!(gui.read_with(visual, |view, _| current(view.session().state())).as_deref(), Some(expected));
    };
    let rows = |state: &State| state.completion().map(|c| c.rows().map(|r| r.label.clone()).collect::<Vec<_>>());
    let undo_to = |visual: &mut VisualTestContext, tui: &mut App, expected: &str| {
        for _ in 0..4 {
            if current(tui.session.state()).as_deref() == Some(expected) { break; }
            visual.simulate_keystrokes("ctrl-z"); tui.handle(Event::Key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL)));
            assert_eq!(gui.read_with(visual, |view, _| current(view.session().state())), current(tui.session.state()), "the same draft after each Ctrl+Z");
        }
        both(visual, tui, expected);
    };
    println!("step: Ruby completion");
    caret(&mut visual, &mut tui, text.find("<%=  %>").unwrap() + 4);
    visual.simulate_keystrokes("G r e e t e r . g r e"); for c in "Greeter.gre".chars() { key(&mut tui, KeyCode::Char(c), KeyModifiers::NONE); }
    let typed = text.replacen("<%=  %>", "<%= Greeter.gre %>", 1);
    wait(&mut visual, &mut tui, &mut terminal, 10, &|state| current(state).as_deref() == Some(typed.as_str()));
    // Ruby LSP may answer the typing's requests with nothing while it indexes: Ctrl+Space asks again in both.
    let offered = |state: &State| rows(state).is_some_and(|r| r.iter().any(|l| l == "greet"));
    for _ in 0..20 {
        let until = Instant::now() + Duration::from_secs(3);
        while Instant::now() < until && !(gui.read_with(&visual, |view, _| offered(view.session().state())) && offered(tui.session.state())) {
            gui.update(&mut visual, |view, cx| view.poll(cx)); while let Ok(completion) = tui.session.completions().try_recv() { tui.apply(completion); }
            visual.update(|window, cx| { let _ = window.draw(cx); }); terminal.draw(|frame| tui.draw(frame)).unwrap();
            std::thread::sleep(Duration::from_millis(5));
        }
        if gui.read_with(&visual, |view, _| offered(view.session().state())) && offered(tui.session.state()) { break; }
        if !gui.read_with(&visual, |view, _| offered(view.session().state())) { visual.simulate_keystrokes("ctrl-space"); }
        if !offered(tui.session.state()) { key(&mut tui, KeyCode::Char(' '), KeyModifiers::CONTROL); }
    }
    wait(&mut visual, &mut tui, &mut terminal, 5, &offered);
    let labels = rows(tui.session.state()).unwrap();
    assert_eq!(gui.read_with(&visual, |view, _| rows(view.session().state())), Some(labels.clone()), "the same suggestions in both");
    assert!(gui.read_with(&visual, |view, _| view.completion_bounds().is_some()) && screen(&terminal).contains("greet"), "both draw them");
    let row = labels.iter().position(|l| l == "greet").unwrap();
    for _ in 0..row { visual.simulate_keystrokes("down"); key(&mut tui, KeyCode::Down, KeyModifiers::NONE); }
    visual.simulate_keystrokes("enter"); key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    let accepted = text.replacen("<%=  %>", "<%= Greeter.greet %>", 1);
    wait(&mut visual, &mut tui, &mut terminal, 30, &|state| current(state).as_deref() == Some(accepted.as_str()));
    visual.simulate_keystrokes("ctrl-z"); key(&mut tui, KeyCode::Char('z'), KeyModifiers::CONTROL);
    both(&mut visual, &tui, &typed);
    undo_to(&mut visual, &mut tui, text);
    println!("step: parameter information");
    caret(&mut visual, &mut tui, text.find("first, excited").unwrap() + 7);
    // GUI: Ctrl+Shift+Space; the terminal: its command menu's Parameter Info (terminals rarely report Ctrl+Shift+Space).
    // Right after edits Ruby LSP may answer with nothing: asked again until it answers.
    let signature = |state: &State| state.signature_help().and_then(|h| h.active_signature().map(|s| (s.label.clone(), s.active_parameter)));
    for _ in 0..20 {
        if gui.read_with(&visual, |view, _| view.session().state().signature_help().is_none()) { visual.simulate_keystrokes("ctrl-shift-space"); }
        if tui.session.state().signature_help().is_none() { tui.run(CommandId::ParameterInfo); }
        let until = Instant::now() + Duration::from_secs(2);
        while Instant::now() < until && !(gui.read_with(&visual, |view, _| view.session().state().signature_help().is_some()) && tui.session.state().signature_help().is_some()) {
            gui.update(&mut visual, |view, cx| view.poll(cx)); while let Ok(completion) = tui.session.completions().try_recv() { tui.apply(completion); }
            visual.update(|window, cx| { let _ = window.draw(cx); }); terminal.draw(|frame| tui.draw(frame)).unwrap();
            std::thread::sleep(Duration::from_millis(5));
        }
        if gui.read_with(&visual, |view, _| view.session().state().signature_help().is_some()) && tui.session.state().signature_help().is_some() { break; }
    }
    wait(&mut visual, &mut tui, &mut terminal, 5, &|state| state.signature_help().is_some());
    assert_eq!(signature(tui.session.state()), Some(("greet(name, excited: <default>)".to_string(), Some(1))));
    assert_eq!(gui.read_with(&visual, |view, _| signature(view.session().state())), signature(tui.session.state()));
    assert!(gui.read_with(&visual, |view, _| view.signature_bounds().is_some()) && tui.layout.signature.width > 0, "both draw the box");
    visual.simulate_keystrokes("escape"); key(&mut tui, KeyCode::Esc, KeyModifiers::NONE);
    wait(&mut visual, &mut tui, &mut terminal, 10, &|state| state.signature_help().is_none());
    println!("step: markup completion");
    caret(&mut visual, &mut tui, text.find("<p>𝄞 ").unwrap() + "<p>𝄞 ".len());
    visual.simulate_keystrokes("< s p"); for c in "<sp".chars() { key(&mut tui, KeyCode::Char(c), KeyModifiers::NONE); }
    wait(&mut visual, &mut tui, &mut terminal, 30, &|state| rows(state).is_some_and(|r| r.iter().any(|l| l == "span")));
    let labels = rows(tui.session.state()).unwrap();
    assert_eq!(gui.read_with(&visual, |view, _| rows(view.session().state())), Some(labels.clone()));
    let row = labels.iter().position(|l| l == "span").unwrap();
    for _ in 0..row { visual.simulate_keystrokes("down"); key(&mut tui, KeyCode::Down, KeyModifiers::NONE); }
    visual.simulate_keystrokes("enter"); key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    let named = text.replacen("<p>𝄞 </p>", "<p>𝄞 <span</p>", 1);
    wait(&mut visual, &mut tui, &mut terminal, 30, &|state| current(state).as_deref() == Some(named.as_str()));
    undo_to(&mut visual, &mut tui, text);
    println!("step: formatting");
    visual.simulate_keystrokes("ctrl-alt-l"); key(&mut tui, KeyCode::Char('l'), KeyModifiers::CONTROL | KeyModifiers::ALT);
    wait(&mut visual, &mut tui, &mut terminal, 60, &|state| state.notices().iter().any(|n| n.starts_with("Format Document: no ERB formatter is configured for this project")));
    both(&mut visual, &tui, text);
    for root in [&groot, &troot] {
        assert_eq!(std::fs::read_to_string(file(root)).unwrap(), text);
        assert_eq!(std::fs::read_to_string(root.join("lib/greeter.rb")).unwrap(), greeter);
    }
}

#[gpui::test]
fn formatting_organize_imports_and_format_on_save_match_in_both_adapters(cx: &mut TestAppContext) {
    let profile = std::env::current_exe().unwrap().parent().unwrap().parent().unwrap().to_owned();
    assert!(std::process::Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .args(["build", "-q", "--locked", "-p", "code-core", "--example", "fake_lsp"])
        .current_dir(env!("CARGO_MANIFEST_DIR")).status().unwrap().success());
    let fake = profile.join("examples/fake_lsp");
    let dir = tempfile::tempdir().unwrap();
    let make = |name: &str, text: &str| {
        let root = dir.path().join(name).join("project"); std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("Cargo.toml"), "[package]\nname='format'\nversion='0.1.0'\n").unwrap();
        std::fs::write(root.join(".editorconfig"), "root = true\n[*.rs]\nindent_style = space\nindent_size = 2\n").unwrap();
        std::fs::write(root.join("main.rs"), text).unwrap();
        std::fs::write(root.join("unrelated.rs"), "fn   untouched( ) {}\n").unwrap();
        let c = config(&dir.path().join(name).join("config"));
        let mut settings: serde_json::Value = serde_json::from_slice(&std::fs::read(&c.providers).unwrap()).unwrap();
        settings["providers"]["rust-analyzer"] = serde_json::json!({"executable":fake,"env":{"FAKE_LSP_NO_DECOR":"1"}});
        std::fs::write(&c.providers, settings.to_string()).unwrap(); (c, root)
    };
    let text = "use b;\nuse a;\nfn main() {\nlet x = 1;   \n        x;\n}\n";
    let ((gc, groot), (tc, troot)) = (make("gui", text), make("tui", text));
    let mut core = Session::new(gc); core.dispatch(Command::OpenRoot(groot.clone()));
    let gui = cx.new(|cx| ReaderView::new(core, Some(groot.join("main.rs")), cx));
    let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| gui.clone())).unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let focus = gui.read_with(&visual, |view, _| view.focus_handle()); visual.update(|window, cx| focus.focus(window, cx));
    let mut tui = App::new(tc, Some(troot.clone())); tui.session.dispatch(Command::Open(troot.join("main.rs")));
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
    let ready = |state: &State| state.provider().is_some_and(|p| p.status == code_core::semantics::Status::Ready && p.supports.contains(&code_core::semantics::Question::Formatting));
    wait(&mut visual, &mut tui, &mut terminal, &ready);
    gui.update(&mut visual, |view, _| view.enable_draft_input_for_test()); tui.enable_draft_input_for_test();
    let key = |tui: &mut App, code, modifiers| { tui.handle(Event::Key(KeyEvent::new(code, modifiers))); };
    let current = |state: &State| state.current_source().map(|d| d.copy_range(0..d.len_bytes()).unwrap());
    let same = |visual: &mut VisualTestContext, tui: &App| {
        assert_eq!(gui.read_with(visual, |view, _| current(view.session().state())), current(tui.session.state()));
    };
    // Ctrl+Alt+L formats with the project's two-space indentation in both; one Ctrl+Z takes it back.
    let formatted = "use b;\nuse a;\nfn main() {\n  let x = 1;\n  x;\n}\n";
    visual.simulate_keystrokes("ctrl-alt-l"); key(&mut tui, KeyCode::Char('l'), KeyModifiers::CONTROL | KeyModifiers::ALT);
    wait(&mut visual, &mut tui, &mut terminal, &|state| current(state).as_deref() == Some(formatted));
    visual.simulate_keystrokes("ctrl-z"); key(&mut tui, KeyCode::Char('z'), KeyModifiers::CONTROL);
    same(&mut visual, &tui);
    assert_eq!(current(tui.session.state()).as_deref(), Some(text));
    // Ctrl+Alt+O organizes the imports alike.
    visual.simulate_keystrokes("ctrl-alt-o"); key(&mut tui, KeyCode::Char('o'), KeyModifiers::CONTROL | KeyModifiers::ALT);
    wait(&mut visual, &mut tui, &mut terminal, &|state| current(state).as_deref() == Some(&text.replacen("use b;\nuse a;", "use a;\nuse b;", 1)));
    // Format on save: Ctrl+S formats, then writes the formatted draft; the other file is untouched.
    gui.update(&mut visual, |view, cx| view.dispatch_for_test(Command::SetFormatOnSave(true), cx));
    tui.session.dispatch(Command::SetFormatOnSave(true));
    visual.simulate_keystrokes("ctrl-s"); key(&mut tui, KeyCode::Char('s'), KeyModifiers::CONTROL);
    let saved = "use a;\nuse b;\nfn main() {\n  let x = 1;\n  x;\n}\n";
    wait(&mut visual, &mut tui, &mut terminal, &|state| state.active().is_some_and(|t| !state.tab_dirty(t.id())) && current(state).as_deref() == Some(saved));
    for root in [&groot, &troot] {
        assert_eq!(std::fs::read_to_string(root.join("main.rs")).unwrap(), saved);
        assert_eq!(std::fs::read_to_string(root.join("unrelated.rs")).unwrap(), "fn   untouched( ) {}\n");
    }
    // A slow formatter shows its status in both; Escape cancels, and nothing is written.
    visual.simulate_keystrokes("ctrl-home"); key(&mut tui, KeyCode::Home, KeyModifiers::CONTROL);
    visual.simulate_keystrokes("/ / space s l o w _ f o r m a t enter"); for c in "// slow_format".chars() { key(&mut tui, KeyCode::Char(c), KeyModifiers::NONE); } key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    same(&mut visual, &tui);
    let draft = current(tui.session.state());
    visual.simulate_keystrokes("ctrl-s"); key(&mut tui, KeyCode::Char('s'), KeyModifiers::CONTROL);
    assert_eq!(tui.session.state().asking().as_deref(), Some("Formatting before saving…"));
    assert_eq!(gui.read_with(&visual, |view, _| view.session().state().asking()), tui.session.state().asking());
    terminal.draw(|frame| tui.draw(frame)).unwrap();
    let screen: String = terminal.backend().buffer().content().iter().map(|c| c.symbol()).collect();
    assert!(screen.contains("Formatting before saving…"), "the terminal status line says what it waits for");
    visual.simulate_keystrokes("escape"); key(&mut tui, KeyCode::Esc, KeyModifiers::NONE);
    assert!(tui.session.state().asking().is_none() && gui.read_with(&visual, |view, _| view.session().state().asking().is_none()));
    let until = Instant::now() + Duration::from_millis(700);
    while Instant::now() < until { gui.update(&mut visual, |view, cx| view.poll(cx)); while let Ok(c) = tui.session.completions().try_recv() { tui.apply(c); } std::thread::yield_now(); }
    same(&mut visual, &tui);
    assert_eq!(current(tui.session.state()), draft, "a late answer is dropped");
    for root in [&groot, &troot] { assert_eq!(std::fs::read_to_string(root.join("main.rs")).unwrap(), saved); }
}

#[gpui::test]
fn server_details_explain_missing_assistance_alike_and_hide_environment_values(cx: &mut TestAppContext) {
    let profile = std::env::current_exe().unwrap().parent().unwrap().parent().unwrap().to_owned();
    assert!(std::process::Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .args(["build", "-q", "--locked", "-p", "code-core", "--example", "fake_lsp"])
        .current_dir(env!("CARGO_MANIFEST_DIR")).status().unwrap().success());
    let fake = profile.join("examples/fake_lsp");
    let dir = tempfile::tempdir().unwrap(); let root = dir.path().join("project"); std::fs::create_dir(&root).unwrap();
    let path = root.join("main.rs"); let text = "fn main() { let private_body = 1; }\n";
    std::fs::write(root.join("Cargo.toml"), "[package]\nname='details'\nversion='0.1.0'\n").unwrap(); std::fs::write(&path, text).unwrap();
    let make = |name| {
        let c = config(&dir.path().join(name));
        let mut settings: serde_json::Value = serde_json::from_slice(&std::fs::read(&c.providers).unwrap()).unwrap();
        settings["providers"]["rust-analyzer"] = serde_json::json!({"executable":fake,"env":{"FAKE_LSP_NO_DECOR":"1","FAKE_LSP_NO_FORMAT":"1","API_TOKEN":"hunter2-secret"}});
        std::fs::write(&c.providers, settings.to_string()).unwrap(); c
    };
    let gui = cx.new(|cx| ReaderView::new(Session::new(make("gui")), Some(path.clone()), cx));
    let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| gui.clone())).unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let focus = gui.read_with(&visual, |view, _| view.focus_handle()); visual.update(|window, cx| focus.focus(window, cx));
    let mut tui = App::new(make("tui"), Some(root.clone())); tui.session.dispatch(Command::Open(path.clone()));
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 50)).unwrap();
    let until = Instant::now() + Duration::from_secs(10);
    let known = |state: &State| state.provider().is_some_and(|p| !p.supports.is_empty());
    while !(gui.read_with(&visual, |view, _| known(view.session().state())) && known(tui.session.state())) {
        gui.update(&mut visual, |view, cx| view.poll(cx)); while let Ok(c) = tui.session.completions().try_recv() { tui.apply(c); }
        visual.update(|window, cx| { let _ = window.draw(cx); }); terminal.draw(|frame| tui.draw(frame)).unwrap();
        assert!(Instant::now() < until); std::thread::yield_now();
    }
    visual.update(|window, cx| gui.update(cx, |view, cx| view.run(CommandId::ServerDetails, window, cx)));
    tui.run(CommandId::ServerDetails);
    let shown = gui.read_with(&visual, |v, _| v.modal_rows()).into_iter().map(|(row, _)| row).collect::<Vec<_>>();
    let Some(crate::app::Prompt::Details { body, .. }) = tui.modal.as_ref().map(|m| &m.kind) else { panic!("no terminal details") };
    let body = body.clone();
    let offered = shown.iter().find(|r| r.starts_with("Offered by rust-analyzer: ")).unwrap_or_else(|| panic!("{shown:?}"));
    assert!(offered.contains("Not offered by this server: type hierarchy, formatting;"), "{offered}");
    assert!(body.contains(offered.as_str()), "both adapters show the same assistance row");
    let env_row = shown.iter().find(|r| r.starts_with("Environment set by provider settings: ")).unwrap_or_else(|| panic!("{shown:?}"));
    for name in ["FAKE_LSP_NO_DECOR", "FAKE_LSP_NO_FORMAT", "API_TOKEN"] { assert!(env_row.contains(name) && body.contains(name), "{env_row}"); }
    assert!(env_row.ends_with("(values not shown)"));
    terminal.draw(|frame| tui.draw(frame)).unwrap();
    let screen: String = terminal.backend().buffer().content().iter().map(|c| c.symbol()).collect();
    assert!(!screen.contains("hunter2-secret"), "no environment value is drawn");
    for shown_text in [body.as_str(), &shown.join("\n")] {
        assert!(!shown_text.contains("hunter2-secret") && !shown_text.contains("private_body"), "the details carry no environment value or source text");
    }
    // The format command explains itself the same way in both.
    gui.update(&mut visual, |view, _| view.enable_draft_input_for_test()); tui.enable_draft_input_for_test();
    assert_eq!(gui.read_with(&visual, |v, _| v.session().disabled(CommandId::FormatDocument)), Some("The language server does not offer formatting"));
    assert_eq!(tui.session.disabled(CommandId::FormatDocument), Some("The language server does not offer formatting"));
    visual.simulate_keystrokes("escape"); tui.handle(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
    assert!(gui.read_with(&visual, |v, _| v.modal_rows()).is_empty() && tui.modal_title().is_empty());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
}

/// Real JDT LS behind both adapters: a Maven dependency's import fix through Alt+Enter, Format Document through
/// Ctrl+Alt+L and Ctrl+Z, each leaving the same draft in both and the file untouched. Needs `jdtls` on PATH (or
/// `READER_JDTLS`), Java 21+ and Maven Central for the fixture's dependency.
#[gpui::test]
#[ignore = "needs an installed jdtls, Java 21+ and network access to Maven Central; run alone"]
fn real_jdtls_import_fix_and_format_match_in_both_adapters(cx: &mut TestAppContext) {
    let Some(jdtls) = std::env::var_os("READER_JDTLS").map(std::path::PathBuf::from)
        .or_else(|| code_core::semantics::provider::find_executable(std::path::Path::new("jdtls"), None)) else {
        println!("SKIPPED: jdtls is not installed");
        return;
    };
    let pom = "<project xmlns=\"http://maven.apache.org/POM/4.0.0\">\n  <modelVersion>4.0.0</modelVersion>\n  <groupId>demo</groupId>\n  <artifactId>demo</artifactId>\n  <version>1</version>\n  <properties>\n    <maven.compiler.release>21</maven.compiler.release>\n  </properties>\n  <dependencies>\n    <dependency>\n      <groupId>com.google.code.gson</groupId>\n      <artifactId>gson</artifactId>\n      <version>2.11.0</version>\n    </dependency>\n  </dependencies>\n</project>\n";
    let text = "package demo;\n\npublic class App {\n    public static void main(String[] args) {\n        Gson g = new Gson();\n            String s = g.toJson(args);\n        System.out.println(s);\n    }\n}\n";
    let dir = tempfile::tempdir().unwrap();
    let make = |name: &str| {
        let root = dir.path().join(name).join("project");
        std::fs::create_dir_all(root.join("src/main/java/demo")).unwrap();
        std::fs::write(root.join("pom.xml"), pom).unwrap();
        std::fs::write(root.join("src/main/java/demo/App.java"), text).unwrap();
        let c = config(&dir.path().join(name).join("config"));
        let mut settings: serde_json::Value = serde_json::from_slice(&std::fs::read(&c.providers).unwrap()).unwrap();
        settings["providers"]["jdtls"] = serde_json::json!({ "executable": jdtls });
        std::fs::write(&c.providers, settings.to_string()).unwrap(); (c, root)
    };
    let ((gc, groot), (tc, troot)) = (make("gui"), make("tui"));
    let file = |root: &std::path::Path| root.join("src/main/java/demo/App.java");
    let mut core = Session::new(gc); core.dispatch(Command::OpenRoot(groot.clone()));
    let gui = cx.new(|cx| ReaderView::new(core, Some(file(&groot)), cx));
    let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| gui.clone())).unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let focus = gui.read_with(&visual, |view, _| view.focus_handle()); visual.update(|window, cx| focus.focus(window, cx));
    let mut tui = App::new(tc, Some(troot.clone())); tui.session.dispatch(Command::Open(file(&troot)));
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 40)).unwrap();
    let wait = |visual: &mut VisualTestContext, tui: &mut App, terminal: &mut ratatui::Terminal<ratatui::backend::TestBackend>, limit: u64, done: &dyn Fn(&State) -> bool| {
        let until = Instant::now() + Duration::from_secs(limit);
        loop {
            gui.update(visual, |view, cx| view.poll(cx)); while let Ok(completion) = tui.session.completions().try_recv() { tui.apply(completion); }
            visual.update(|window, cx| { let _ = window.draw(cx); }); terminal.draw(|frame| tui.draw(frame)).unwrap();
            if gui.read_with(visual, |view, _| done(view.session().state())) && done(tui.session.state()) { break; }
            assert!(Instant::now() < until, "GUI {:?}; TUI {:?}", gui.read_with(visual, |view, _| view.session().state().notices()), tui.session.state().notices());
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    let unresolved = |state: &State| state.active().and_then(|t| state.diagnostics(t.id())).is_some_and(|d| d.items.iter().any(|p| p.message.contains("Gson")));
    wait(&mut visual, &mut tui, &mut terminal, 240, &unresolved);
    gui.update(&mut visual, |view, _| view.enable_draft_input_for_test()); tui.enable_draft_input_for_test();
    let key = |tui: &mut App, code, modifiers| { tui.handle(Event::Key(KeyEvent::new(code, modifiers))); };
    let current = |state: &State| state.current_source().map(|d| d.copy_range(0..d.len_bytes()).unwrap());
    let at = text.find("Gson g").unwrap() + 1;
    gui.update(&mut visual, |view, cx| view.dispatch_for_test(Command::Select(code_core::session::Selection { anchor: at, head: at }), cx));
    tui.session.dispatch(Command::Select(code_core::session::Selection { anchor: at, head: at }));
    visual.simulate_keystrokes("alt-enter"); key(&mut tui, KeyCode::Enter, KeyModifiers::ALT);
    wait(&mut visual, &mut tui, &mut terminal, 60, &|state| state.code_actions().is_some());
    let import = |state: &State| state.code_actions().and_then(|c| c.actions.iter().position(|a| a.title.contains("com.google.gson") && a.unavailable.is_none()));
    let row = import(tui.session.state()).expect("an import fix from the Maven dependency");
    assert_eq!(gui.read_with(&visual, |view, _| import(view.session().state())), Some(row));
    for _ in 0..row { visual.simulate_keystrokes("down"); key(&mut tui, KeyCode::Down, KeyModifiers::NONE); }
    visual.simulate_keystrokes("enter"); key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    let imported = current(tui.session.state()).unwrap();
    assert!(imported.contains("import com.google.gson.Gson;"), "{imported}");
    assert_eq!(gui.read_with(&visual, |view, _| current(view.session().state())).as_deref(), Some(imported.as_str()));
    visual.simulate_keystrokes("ctrl-alt-l"); key(&mut tui, KeyCode::Char('l'), KeyModifiers::CONTROL | KeyModifiers::ALT);
    wait(&mut visual, &mut tui, &mut terminal, 60, &|state| current(state).is_some_and(|t| t.contains("\n        String s = g.toJson(args);")));
    assert_eq!(gui.read_with(&visual, |view, _| current(view.session().state())), current(tui.session.state()));
    visual.simulate_keystrokes("ctrl-z"); key(&mut tui, KeyCode::Char('z'), KeyModifiers::CONTROL);
    assert_eq!(current(tui.session.state()).as_deref(), Some(imported.as_str()));
    assert_eq!(gui.read_with(&visual, |view, _| current(view.session().state())).as_deref(), Some(imported.as_str()));
    for root in [&groot, &troot] { assert_eq!(std::fs::read_to_string(file(root)).unwrap(), text); }
}

#[gpui::test]
fn server_command_fixes_and_completion_apply_alike_in_both_adapters(cx: &mut TestAppContext) {
    let profile = std::env::current_exe().unwrap().parent().unwrap().parent().unwrap().to_owned();
    assert!(std::process::Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .args(["build", "-q", "--locked", "-p", "code-core", "--example", "fake_lsp"])
        .current_dir(env!("CARGO_MANIFEST_DIR")).status().unwrap().success());
    let fake = profile.join("examples/fake_lsp");
    // Served as Kotlin LSP serves Kotlin: every fix and suggestion through its own allowlisted command.
    let text = "package a\n\nfn main() {\n    bad_name;\n    he\n}\nfn helper() {}\n";
    let dir = tempfile::tempdir().unwrap();
    let make = |name: &str| {
        let root = dir.path().join(name).join("project");
        std::fs::create_dir_all(root.join("src/main/kotlin/a")).unwrap();
        std::fs::write(root.join("pom.xml"), "<project/>\n").unwrap();
        std::fs::write(root.join("src/main/kotlin/a/A.kt"), text).unwrap();
        let c = config(&dir.path().join(name).join("config"));
        let mut settings: serde_json::Value = serde_json::from_slice(&std::fs::read(&c.providers).unwrap()).unwrap();
        settings["providers"]["kotlin-lsp"] = serde_json::json!({"executable":fake,"env":{"FAKE_LSP_NO_DECOR":"1","FAKE_LSP_KOTLIN_COMMANDS":"1"}});
        std::fs::write(&c.providers, settings.to_string()).unwrap(); (c, root)
    };
    let ((gc, groot), (tc, troot)) = (make("gui"), make("tui"));
    let file = |root: &std::path::Path| root.join("src/main/kotlin/a/A.kt");
    let mut core = Session::new(gc); core.dispatch(Command::OpenRoot(groot.clone()));
    let gui = cx.new(|cx| ReaderView::new(core, Some(file(&groot)), cx));
    let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| gui.clone())).unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let focus = gui.read_with(&visual, |view, _| view.focus_handle()); visual.update(|window, cx| focus.focus(window, cx));
    let mut tui = App::new(tc, Some(troot.clone())); tui.session.dispatch(Command::Open(file(&troot)));
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
    let problems = |state: &State| state.active().and_then(|t| state.diagnostics(t.id())).map_or(0, |d| d.items.len());
    wait(&mut visual, &mut tui, &mut terminal, &|state| problems(state) == 1);
    gui.update(&mut visual, |view, _| view.enable_draft_input_for_test()); tui.enable_draft_input_for_test();
    let key = |tui: &mut App, code, modifiers| { tui.handle(Event::Key(KeyEvent::new(code, modifiers))); };
    let current = |state: &State| state.current_source().map(|d| d.copy_range(0..d.len_bytes()).unwrap());
    let select = |visual: &mut VisualTestContext, tui: &mut App, at: usize| {
        gui.update(visual, |view, cx| view.dispatch_for_test(Command::Select(code_core::session::Selection { anchor: at, head: at }), cx));
        tui.session.dispatch(Command::Select(code_core::session::Selection { anchor: at, head: at }));
    };
    // Alt+Enter: the allowlisted fix is offered, the other command refused alike; Enter runs it and its edit applies.
    select(&mut visual, &mut tui, text.find("bad_name").unwrap() + 2);
    visual.simulate_keystrokes("alt-enter"); key(&mut tui, KeyCode::Enter, KeyModifiers::ALT);
    wait(&mut visual, &mut tui, &mut terminal, &|state| state.code_actions().is_some());
    let listed = |state: &State| state.code_actions().map(|c| c.actions.iter().map(|a| (a.title.clone(), a.unavailable.is_some())).collect::<Vec<_>>());
    assert_eq!(listed(tui.session.state()), Some(vec![("Replace with `good_name`".to_owned(), false), ("Run something else".to_owned(), true)]));
    assert_eq!(gui.read_with(&visual, |view, _| listed(view.session().state())), listed(tui.session.state()));
    visual.simulate_keystrokes("enter"); key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    let fixed = text.replace("bad_name", "good_name");
    wait(&mut visual, &mut tui, &mut terminal, &|state| current(state).as_deref() == Some(fixed.as_str()));
    visual.simulate_keystrokes("ctrl-z"); key(&mut tui, KeyCode::Char('z'), KeyModifiers::CONTROL);
    assert_eq!(current(tui.session.state()).as_deref(), Some(text));
    assert_eq!(gui.read_with(&visual, |view, _| current(view.session().state())).as_deref(), Some(text));
    // Ctrl+Space and Enter: the suggestion inserts through its command.
    select(&mut visual, &mut tui, text.find("he\n}").unwrap() + 2);
    visual.simulate_keystrokes("ctrl-space"); key(&mut tui, KeyCode::Char(' '), KeyModifiers::CONTROL);
    wait(&mut visual, &mut tui, &mut terminal, &|state| state.completion().is_some_and(|c| c.rows().any(|r| r.label == "helper")));
    let row = tui.session.state().completion().unwrap().rows().position(|r| r.label == "helper").unwrap();
    for _ in 0..row { visual.simulate_keystrokes("down"); key(&mut tui, KeyCode::Down, KeyModifiers::NONE); }
    visual.simulate_keystrokes("enter"); key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    let completed = text.replacen("    he\n", "    helper\n", 1);
    wait(&mut visual, &mut tui, &mut terminal, &|state| current(state).as_deref() == Some(completed.as_str()));
    for root in [&groot, &troot] { assert_eq!(std::fs::read_to_string(file(root)).unwrap(), text); }
}

/// Real rust-analyzer behind both adapters: its auto-import fix through Alt+Enter and rustfmt through Ctrl+Alt+L,
/// each one Ctrl+Z, the same draft in both and the file untouched. Needs `rust-analyzer` (with rust-src) on PATH or `READER_RUST_ANALYZER`.
#[gpui::test]
#[ignore = "needs an installed rust-analyzer with rust-src and rustfmt; run alone"]
fn real_rust_analyzer_fix_and_format_match_in_both_adapters(cx: &mut TestAppContext) {
    let Some(ra) = std::env::var_os("READER_RUST_ANALYZER").map(std::path::PathBuf::from)
        .or_else(|| code_core::semantics::provider::find_executable(std::path::Path::new("rust-analyzer"), None)) else {
        println!("SKIPPED: rust-analyzer is not installed");
        return;
    };
    let text = "fn main() {\n    let mut m = HashMap::<u32, u32>::new();\n        m.insert(1, 2);\n    println!(\"{}\", m.len());\n}\n";
    let dir = tempfile::tempdir().unwrap();
    let make = |name: &str| {
        let root = dir.path().join(name).join("project");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"edit\"\nversion = \"0.1.0\"\nedition = \"2021\"\n").unwrap();
        std::fs::write(root.join("src/main.rs"), text).unwrap();
        assert!(std::process::Command::new("cargo").args(["generate-lockfile", "--offline"]).current_dir(&root).status().is_ok_and(|s| s.success()));
        let c = config(&dir.path().join(name).join("config"));
        let mut settings: serde_json::Value = serde_json::from_slice(&std::fs::read(&c.providers).unwrap()).unwrap();
        settings["providers"]["rust-analyzer"] = serde_json::json!({ "executable": ra });
        std::fs::write(&c.providers, settings.to_string()).unwrap(); (c, root)
    };
    let ((gc, groot), (tc, troot)) = (make("gui"), make("tui"));
    let file = |root: &std::path::Path| root.join("src/main.rs");
    let mut core = Session::new(gc); core.dispatch(Command::OpenRoot(groot.clone()));
    let gui = cx.new(|cx| ReaderView::new(core, Some(file(&groot)), cx));
    let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| gui.clone())).unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let focus = gui.read_with(&visual, |view, _| view.focus_handle()); visual.update(|window, cx| focus.focus(window, cx));
    let mut tui = App::new(tc, Some(troot.clone())); tui.session.dispatch(Command::Open(file(&troot)));
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 40)).unwrap();
    let wait = |visual: &mut VisualTestContext, tui: &mut App, terminal: &mut ratatui::Terminal<ratatui::backend::TestBackend>, limit: u64, done: &dyn Fn(&State) -> bool| {
        let until = Instant::now() + Duration::from_secs(limit);
        loop {
            gui.update(visual, |view, cx| view.poll(cx)); while let Ok(completion) = tui.session.completions().try_recv() { tui.apply(completion); }
            visual.update(|window, cx| { let _ = window.draw(cx); }); terminal.draw(|frame| tui.draw(frame)).unwrap();
            if gui.read_with(visual, |view, _| done(view.session().state())) && done(tui.session.state()) { break; }
            assert!(Instant::now() < until, "GUI {:?}; TUI {:?}", gui.read_with(visual, |view, _| view.session().state().notices()), tui.session.state().notices());
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    let ready = |state: &State| state.provider().is_some_and(|p| p.status == code_core::semantics::Status::Ready);
    wait(&mut visual, &mut tui, &mut terminal, 180, &ready);
    gui.update(&mut visual, |view, _| view.enable_draft_input_for_test()); tui.enable_draft_input_for_test();
    let key = |tui: &mut App, code, modifiers| { tui.handle(Event::Key(KeyEvent::new(code, modifiers))); };
    let current = |state: &State| state.current_source().map(|d| d.copy_range(0..d.len_bytes()).unwrap());
    let select = |visual: &mut VisualTestContext, tui: &mut App, at: usize| {
        gui.update(visual, |view, cx| view.dispatch_for_test(Command::Select(code_core::session::Selection { anchor: at, head: at }), cx));
        tui.session.dispatch(Command::Select(code_core::session::Selection { anchor: at, head: at }));
    };
    // Alt+Enter at `HashMap`: the import fix, picked by keys.
    select(&mut visual, &mut tui, text.find("HashMap::<").unwrap() + 3);
    visual.simulate_keystrokes("alt-enter"); key(&mut tui, KeyCode::Enter, KeyModifiers::ALT);
    let import = |state: &State| state.code_actions().and_then(|c| c.actions.iter().position(|a| a.title.contains("std::collections::HashMap") && a.unavailable.is_none()));
    wait(&mut visual, &mut tui, &mut terminal, 60, &|state| import(state).is_some());
    let row = import(tui.session.state()).unwrap();
    assert_eq!(gui.read_with(&visual, |view, _| import(view.session().state())), Some(row));
    for _ in 0..row { visual.simulate_keystrokes("down"); key(&mut tui, KeyCode::Down, KeyModifiers::NONE); }
    visual.simulate_keystrokes("enter"); key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    let imported = format!("use std::collections::HashMap;\n\n{text}");
    assert_eq!(current(tui.session.state()).as_deref(), Some(imported.as_str()));
    assert_eq!(gui.read_with(&visual, |view, _| current(view.session().state())).as_deref(), Some(imported.as_str()));
    // Ctrl+Alt+L formats alike; Ctrl+Z takes it back in both.
    visual.simulate_keystrokes("ctrl-alt-l"); key(&mut tui, KeyCode::Char('l'), KeyModifiers::CONTROL | KeyModifiers::ALT);
    wait(&mut visual, &mut tui, &mut terminal, 60, &|state| current(state).is_some_and(|t| t.contains("\n    m.insert(1, 2);\n")));
    assert_eq!(gui.read_with(&visual, |view, _| current(view.session().state())), current(tui.session.state()));
    visual.simulate_keystrokes("ctrl-z"); key(&mut tui, KeyCode::Char('z'), KeyModifiers::CONTROL);
    assert_eq!(current(tui.session.state()).as_deref(), Some(imported.as_str()));
    assert_eq!(gui.read_with(&visual, |view, _| current(view.session().state())).as_deref(), Some(imported.as_str()));
    for root in [&groot, &troot] { assert_eq!(std::fs::read_to_string(file(root)).unwrap(), text); }
}

/// Real gopls behind both adapters: Organize Imports through Ctrl+Alt+O and gofmt through Ctrl+Alt+L, each one
/// Ctrl+Z, the same draft in both and the file untouched. Needs `gopls` and `go` on PATH (or `READER_GOPLS`).
#[gpui::test]
#[ignore = "needs an installed gopls and Go; run alone"]
fn real_gopls_organize_and_format_match_in_both_adapters(cx: &mut TestAppContext) {
    let Some(gopls) = std::env::var_os("READER_GOPLS").map(std::path::PathBuf::from)
        .or_else(|| code_core::semantics::provider::find_executable(std::path::Path::new("gopls"), None)) else {
        println!("SKIPPED: gopls is not installed");
        return;
    };
    let text = "package main\n\nimport \"fmt\"\n\nfunc main() {\n\t\tfmt.Println(strings.ToUpper(\"x\"))\n}\n";
    let dir = tempfile::tempdir().unwrap();
    let make = |name: &str| {
        let root = dir.path().join(name).join("project");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("go.mod"), "module example.com/demo\n\ngo 1.24\n").unwrap();
        std::fs::write(root.join("main.go"), text).unwrap();
        let c = config(&dir.path().join(name).join("config"));
        let mut settings: serde_json::Value = serde_json::from_slice(&std::fs::read(&c.providers).unwrap()).unwrap();
        settings["providers"]["gopls"] = serde_json::json!({ "executable": gopls });
        std::fs::write(&c.providers, settings.to_string()).unwrap(); (c, root)
    };
    let ((gc, groot), (tc, troot)) = (make("gui"), make("tui"));
    let file = |root: &std::path::Path| root.join("main.go");
    let mut core = Session::new(gc); core.dispatch(Command::OpenRoot(groot.clone()));
    let gui = cx.new(|cx| ReaderView::new(core, Some(file(&groot)), cx));
    let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| gui.clone())).unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let focus = gui.read_with(&visual, |view, _| view.focus_handle()); visual.update(|window, cx| focus.focus(window, cx));
    let mut tui = App::new(tc, Some(troot.clone())); tui.session.dispatch(Command::Open(file(&troot)));
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 40)).unwrap();
    let wait = |visual: &mut VisualTestContext, tui: &mut App, terminal: &mut ratatui::Terminal<ratatui::backend::TestBackend>, limit: u64, done: &dyn Fn(&State) -> bool| {
        let until = Instant::now() + Duration::from_secs(limit);
        loop {
            gui.update(visual, |view, cx| view.poll(cx)); while let Ok(completion) = tui.session.completions().try_recv() { tui.apply(completion); }
            visual.update(|window, cx| { let _ = window.draw(cx); }); terminal.draw(|frame| tui.draw(frame)).unwrap();
            if gui.read_with(visual, |view, _| done(view.session().state())) && done(tui.session.state()) { break; }
            assert!(Instant::now() < until, "GUI {:?}; TUI {:?}", gui.read_with(visual, |view, _| view.session().state().notices()), tui.session.state().notices());
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    let problem = |state: &State| state.active().and_then(|t| state.diagnostics(t.id())).is_some_and(|d| d.items.iter().any(|p| p.message.contains("strings")));
    wait(&mut visual, &mut tui, &mut terminal, 120, &problem);
    gui.update(&mut visual, |view, _| view.enable_draft_input_for_test()); tui.enable_draft_input_for_test();
    let key = |tui: &mut App, code, modifiers| { tui.handle(Event::Key(KeyEvent::new(code, modifiers))); };
    let current = |state: &State| state.current_source().map(|d| d.copy_range(0..d.len_bytes()).unwrap());
    visual.simulate_keystrokes("ctrl-alt-o"); key(&mut tui, KeyCode::Char('o'), KeyModifiers::CONTROL | KeyModifiers::ALT);
    wait(&mut visual, &mut tui, &mut terminal, 60, &|state| current(state).is_some_and(|t| t.contains("\"strings\"")));
    let organized = current(tui.session.state()).unwrap();
    assert_eq!(gui.read_with(&visual, |view, _| current(view.session().state())).as_deref(), Some(organized.as_str()));
    visual.simulate_keystrokes("ctrl-alt-l"); key(&mut tui, KeyCode::Char('l'), KeyModifiers::CONTROL | KeyModifiers::ALT);
    wait(&mut visual, &mut tui, &mut terminal, 60, &|state| current(state).is_some_and(|t| t.contains("\n\tfmt.Println(")));
    assert_eq!(gui.read_with(&visual, |view, _| current(view.session().state())), current(tui.session.state()));
    visual.simulate_keystrokes("ctrl-z"); key(&mut tui, KeyCode::Char('z'), KeyModifiers::CONTROL);
    assert_eq!(current(tui.session.state()).as_deref(), Some(organized.as_str()));
    assert_eq!(gui.read_with(&visual, |view, _| current(view.session().state())).as_deref(), Some(organized.as_str()));
    visual.simulate_keystrokes("ctrl-z"); key(&mut tui, KeyCode::Char('z'), KeyModifiers::CONTROL);
    assert_eq!(current(tui.session.state()).as_deref(), Some(text));
    assert_eq!(gui.read_with(&visual, |view, _| current(view.session().state())).as_deref(), Some(text));
    for root in [&groot, &troot] { assert_eq!(std::fs::read_to_string(file(root)).unwrap(), text); }
}

/// Real typescript-language-server behind both adapters: the add-import fix through Alt+Enter and formatting through
/// Ctrl+Alt+L, each one Ctrl+Z, the same draft in both and the files untouched. Needs `typescript-language-server`
/// and TypeScript (the global npm one, else `READER_TSSERVER`).
#[gpui::test]
#[ignore = "needs an installed typescript-language-server and TypeScript; run alone"]
fn real_typescript_fix_and_format_match_in_both_adapters(cx: &mut TestAppContext) {
    let Some(tsls) = code_core::semantics::provider::find_executable(std::path::Path::new("typescript-language-server"), None) else {
        println!("SKIPPED: typescript-language-server is not installed");
        return;
    };
    let tsserver = std::env::var("READER_TSSERVER").ok().or_else(|| {
        let root = std::process::Command::new("npm").args(["root", "-g"]).output().ok()?;
        let lib = std::path::PathBuf::from(String::from_utf8_lossy(&root.stdout).trim()).join("typescript/lib");
        lib.join("tsserver.js").is_file().then(|| lib.to_string_lossy().into_owned())
    });
    let util = "export function formatName(first: string, last: string): string {\n  return `${first} ${last}`;\n}\n";
    let text = "const s = \"𝄞\";\n    console.log(s, formatName(\"a\", \"b\"));\n";
    let dir = tempfile::tempdir().unwrap();
    let make = |name: &str| {
        let root = dir.path().join(name).join("project");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("package.json"), "{ \"name\": \"edit\", \"private\": true }\n").unwrap();
        std::fs::write(root.join("tsconfig.json"), "{ \"compilerOptions\": { \"strict\": true, \"noEmit\": true }, \"include\": [\"src\"] }\n").unwrap();
        std::fs::write(root.join("src/util.ts"), util).unwrap();
        std::fs::write(root.join("src/app.ts"), text).unwrap();
        let c = config(&dir.path().join(name).join("config"));
        let mut settings: serde_json::Value = serde_json::from_slice(&std::fs::read(&c.providers).unwrap()).unwrap();
        settings["providers"]["typescript-language-server"] = serde_json::json!({ "executable": tsls });
        if let Some(path) = &tsserver { settings["providers"]["typescript-language-server"]["settings"] = serde_json::json!({ "tsserver": { "path": path } }); }
        std::fs::write(&c.providers, settings.to_string()).unwrap(); (c, root)
    };
    let ((gc, groot), (tc, troot)) = (make("gui"), make("tui"));
    let file = |root: &std::path::Path| root.join("src/app.ts");
    let mut core = Session::new(gc); core.dispatch(Command::OpenRoot(groot.clone()));
    let gui = cx.new(|cx| ReaderView::new(core, Some(file(&groot)), cx));
    let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| gui.clone())).unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let focus = gui.read_with(&visual, |view, _| view.focus_handle()); visual.update(|window, cx| focus.focus(window, cx));
    let mut tui = App::new(tc, Some(troot.clone())); tui.session.dispatch(Command::Open(file(&troot)));
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 40)).unwrap();
    let wait = |visual: &mut VisualTestContext, tui: &mut App, terminal: &mut ratatui::Terminal<ratatui::backend::TestBackend>, limit: u64, done: &dyn Fn(&State) -> bool| {
        let until = Instant::now() + Duration::from_secs(limit);
        loop {
            gui.update(visual, |view, cx| view.poll(cx)); while let Ok(completion) = tui.session.completions().try_recv() { tui.apply(completion); }
            visual.update(|window, cx| { let _ = window.draw(cx); }); terminal.draw(|frame| tui.draw(frame)).unwrap();
            if gui.read_with(visual, |view, _| done(view.session().state())) && done(tui.session.state()) { break; }
            assert!(Instant::now() < until, "GUI {:?}; TUI {:?}", gui.read_with(visual, |view, _| view.session().state().notices()), tui.session.state().notices());
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    let problem = |state: &State| state.active().and_then(|t| state.diagnostics(t.id())).is_some_and(|d| d.items.iter().any(|p| p.message.contains("formatName")));
    wait(&mut visual, &mut tui, &mut terminal, 120, &problem);
    gui.update(&mut visual, |view, _| view.enable_draft_input_for_test()); tui.enable_draft_input_for_test();
    let key = |tui: &mut App, code, modifiers| { tui.handle(Event::Key(KeyEvent::new(code, modifiers))); };
    let current = |state: &State| state.current_source().map(|d| d.copy_range(0..d.len_bytes()).unwrap());
    let at = text.find("formatName").unwrap() + 3;
    gui.update(&mut visual, |view, cx| view.dispatch_for_test(Command::Select(code_core::session::Selection { anchor: at, head: at }), cx));
    tui.session.dispatch(Command::Select(code_core::session::Selection { anchor: at, head: at }));
    visual.simulate_keystrokes("alt-enter"); key(&mut tui, KeyCode::Enter, KeyModifiers::ALT);
    let import = |state: &State| state.code_actions().and_then(|c| c.actions.iter().position(|a| a.title.contains("./util") && a.unavailable.is_none()));
    wait(&mut visual, &mut tui, &mut terminal, 60, &|state| import(state).is_some());
    let row = import(tui.session.state()).unwrap();
    assert_eq!(gui.read_with(&visual, |view, _| import(view.session().state())), Some(row));
    for _ in 0..row { visual.simulate_keystrokes("down"); key(&mut tui, KeyCode::Down, KeyModifiers::NONE); }
    visual.simulate_keystrokes("enter"); key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    let fixed = current(tui.session.state()).unwrap();
    assert!(fixed.starts_with("import { formatName } from \"./util\";"), "{fixed}");
    assert_eq!(gui.read_with(&visual, |view, _| current(view.session().state())).as_deref(), Some(fixed.as_str()));
    visual.simulate_keystrokes("ctrl-alt-l"); key(&mut tui, KeyCode::Char('l'), KeyModifiers::CONTROL | KeyModifiers::ALT);
    wait(&mut visual, &mut tui, &mut terminal, 60, &|state| current(state).is_some_and(|t| t.contains("\nconsole.log(s, formatName")));
    assert_eq!(gui.read_with(&visual, |view, _| current(view.session().state())), current(tui.session.state()));
    visual.simulate_keystrokes("ctrl-z"); key(&mut tui, KeyCode::Char('z'), KeyModifiers::CONTROL);
    visual.simulate_keystrokes("ctrl-z"); key(&mut tui, KeyCode::Char('z'), KeyModifiers::CONTROL);
    assert_eq!(current(tui.session.state()).as_deref(), Some(text));
    assert_eq!(gui.read_with(&visual, |view, _| current(view.session().state())).as_deref(), Some(text));
    for root in [&groot, &troot] { assert_eq!(std::fs::read_to_string(file(root)).unwrap(), text); }
}

/// Real Ruby LSP with RuboCop behind both adapters: RuboCop's fix through Alt+Enter and its formatting through
/// Ctrl+Alt+L, each one Ctrl+Z, the same draft in both and the file untouched. Needs the `ruby-lsp` and `rubocop` gems.
#[gpui::test]
#[ignore = "needs installed ruby-lsp and rubocop gems; run alone"]
fn real_ruby_lsp_fix_and_format_match_in_both_adapters(cx: &mut TestAppContext) {
    let Some(ruby_lsp) = std::env::var_os("READER_RUBY_LSP").map(std::path::PathBuf::from)
        .or_else(|| code_core::semantics::provider::find_executable(std::path::Path::new("ruby-lsp"), None)) else {
        println!("SKIPPED: Ruby LSP is not installed");
        return;
    };
    let text = "# frozen_string_literal: true\n\nunused = 1\n    puts \"x\"\n";
    let dir = tempfile::tempdir().unwrap();
    let make = |name: &str| {
        let root = dir.path().join(name).join("project");
        std::fs::create_dir_all(root.join("lib")).unwrap();
        std::fs::write(root.join("Gemfile"), "source \"https://rubygems.org\"\n\ngem \"rubocop\"\n").unwrap();
        std::fs::write(root.join(".rubocop.yml"), "AllCops:\n  NewCops: disable\n  SuggestExtensions: false\n").unwrap();
        std::fs::write(root.join("lib/app.rb"), text).unwrap();
        assert!(std::process::Command::new("bundle").args(["lock", "--local"]).current_dir(&root).status().is_ok_and(|s| s.success()));
        let c = config(&dir.path().join(name).join("config"));
        let mut settings: serde_json::Value = serde_json::from_slice(&std::fs::read(&c.providers).unwrap()).unwrap();
        settings["providers"]["ruby-lsp"] = serde_json::json!({ "executable": ruby_lsp });
        std::fs::write(&c.providers, settings.to_string()).unwrap(); (c, root)
    };
    let ((gc, groot), (tc, troot)) = (make("gui"), make("tui"));
    let file = |root: &std::path::Path| root.join("lib/app.rb");
    let mut core = Session::new(gc); core.dispatch(Command::OpenRoot(groot.clone()));
    let gui = cx.new(|cx| ReaderView::new(core, Some(file(&groot)), cx));
    let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| gui.clone())).unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let focus = gui.read_with(&visual, |view, _| view.focus_handle()); visual.update(|window, cx| focus.focus(window, cx));
    let mut tui = App::new(tc, Some(troot.clone())); tui.session.dispatch(Command::Open(file(&troot)));
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 40)).unwrap();
    let wait = |visual: &mut VisualTestContext, tui: &mut App, terminal: &mut ratatui::Terminal<ratatui::backend::TestBackend>, limit: u64, done: &dyn Fn(&State) -> bool| {
        let until = Instant::now() + Duration::from_secs(limit);
        loop {
            gui.update(visual, |view, cx| view.poll(cx)); while let Ok(completion) = tui.session.completions().try_recv() { tui.apply(completion); }
            visual.update(|window, cx| { let _ = window.draw(cx); }); terminal.draw(|frame| tui.draw(frame)).unwrap();
            if gui.read_with(visual, |view, _| done(view.session().state())) && done(tui.session.state()) { break; }
            assert!(Instant::now() < until, "GUI {:?}; TUI {:?}", gui.read_with(visual, |view, _| view.session().state().notices()), tui.session.state().notices());
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    let flagged = |state: &State| state.active().and_then(|t| state.diagnostics(t.id())).is_some_and(|d| d.items.iter().any(|p| p.message.starts_with("Lint/UselessAssignment")));
    wait(&mut visual, &mut tui, &mut terminal, 120, &flagged);
    gui.update(&mut visual, |view, _| view.enable_draft_input_for_test()); tui.enable_draft_input_for_test();
    let key = |tui: &mut App, code, modifiers| { tui.handle(Event::Key(KeyEvent::new(code, modifiers))); };
    let current = |state: &State| state.current_source().map(|d| d.copy_range(0..d.len_bytes()).unwrap());
    let at = text.find("unused").unwrap() + 2;
    gui.update(&mut visual, |view, cx| view.dispatch_for_test(Command::Select(code_core::session::Selection { anchor: at, head: at }), cx));
    tui.session.dispatch(Command::Select(code_core::session::Selection { anchor: at, head: at }));
    visual.simulate_keystrokes("alt-enter"); key(&mut tui, KeyCode::Enter, KeyModifiers::ALT);
    let fix = |state: &State| state.code_actions().and_then(|c| c.actions.iter().position(|a| a.title == "Autocorrect Lint/UselessAssignment" && a.unavailable.is_none()));
    wait(&mut visual, &mut tui, &mut terminal, 60, &|state| fix(state).is_some());
    let row = fix(tui.session.state()).unwrap();
    assert_eq!(gui.read_with(&visual, |view, _| fix(view.session().state())), Some(row));
    for _ in 0..row { visual.simulate_keystrokes("down"); key(&mut tui, KeyCode::Down, KeyModifiers::NONE); }
    visual.simulate_keystrokes("enter"); key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    let fixed = current(tui.session.state()).unwrap();
    assert!(!fixed.contains("unused = 1"), "{fixed}");
    assert_eq!(gui.read_with(&visual, |view, _| current(view.session().state())).as_deref(), Some(fixed.as_str()));
    visual.simulate_keystrokes("ctrl-z"); key(&mut tui, KeyCode::Char('z'), KeyModifiers::CONTROL);
    assert_eq!(current(tui.session.state()).as_deref(), Some(text));
    visual.simulate_keystrokes("ctrl-alt-l"); key(&mut tui, KeyCode::Char('l'), KeyModifiers::CONTROL | KeyModifiers::ALT);
    wait(&mut visual, &mut tui, &mut terminal, 60, &|state| current(state).is_some_and(|t| t.contains("\nputs 'x'\n")));
    assert_eq!(gui.read_with(&visual, |view, _| current(view.session().state())), current(tui.session.state()));
    visual.simulate_keystrokes("ctrl-z"); key(&mut tui, KeyCode::Char('z'), KeyModifiers::CONTROL);
    assert_eq!(current(tui.session.state()).as_deref(), Some(text));
    assert_eq!(gui.read_with(&visual, |view, _| current(view.session().state())).as_deref(), Some(text));
    for root in [&groot, &troot] { assert_eq!(std::fs::read_to_string(file(root)).unwrap(), text); }
}

/// Real Ruby LSP with its Rails add-on behind both adapters, on a Rails application created for the test: the model's
/// columns from the booted application in Quick Documentation (through Find Action), its columns as `where`
/// suggestions while typing, each drawn with what it filters and accepted by Enter, rubocop-rails' fix through
/// Alt+Enter and RuboCop formatting through Ctrl+Alt+L, each one Ctrl+Z; the same draft in both and the files
/// untouched. Needs the `ruby-lsp`, `ruby-lsp-rails`, `rails`, `sqlite3`, `rubocop` and `rubocop-rails` gems.
#[gpui::test]
#[ignore = "needs installed ruby-lsp, ruby-lsp-rails, rails, sqlite3, rubocop and rubocop-rails gems; run alone"]
fn real_rails_assistance_matches_in_both_adapters(cx: &mut TestAppContext) {
    let Some(ruby_lsp) = std::env::var_os("READER_RUBY_LSP").map(std::path::PathBuf::from)
        .or_else(|| code_core::semantics::provider::find_executable(std::path::Path::new("ruby-lsp"), None)) else {
        println!("SKIPPED: Ruby LSP is not installed");
        return;
    };
    let listed = std::process::Command::new("gem").args(["list", "--local"]).output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    let missing: Vec<&str> = ["ruby-lsp-rails", "railties", "sqlite3", "rubocop", "rubocop-rails"].into_iter().filter(|g| !listed.lines().any(|l| l.starts_with(&format!("{g} (")))).collect();
    if !missing.is_empty() {
        println!("SKIPPED: gems not installed: {missing:?}");
        return;
    }
    let text = "# frozen_string_literal: true\n\nclass PostsController < ApplicationController\n  def index\n    @posts = Post.where(ti)\n    @checked = Time.now\n      redirect_to posts_path\n  end\nend\n";
    let files = [
        ("Gemfile", "source \"https://rubygems.org\"\n\ngem \"rails\"\ngem \"sqlite3\"\ngem \"rubocop\"\ngem \"rubocop-rails\"\n"),
        (".rubocop.yml", "plugins:\n  - rubocop-rails\nAllCops:\n  NewCops: disable\n  SuggestExtensions: false\nStyle/Documentation:\n  Enabled: false\n"),
        ("Rakefile", "require_relative \"config/application\"\nRails.application.load_tasks\n"),
        ("bin/rails", "#!/usr/bin/env ruby\nAPP_PATH = File.expand_path(\"../config/application\", __dir__)\nrequire_relative \"../config/boot\"\nrequire \"rails/commands\"\n"),
        ("config/boot.rb", "ENV[\"BUNDLE_GEMFILE\"] ||= File.expand_path(\"../Gemfile\", __dir__)\nrequire \"bundler/setup\"\n"),
        ("config/application.rb", "require_relative \"boot\"\nrequire \"rails\"\nrequire \"active_model/railtie\"\nrequire \"active_record/railtie\"\nrequire \"action_controller/railtie\"\nrequire \"action_view/railtie\"\nBundler.require(*Rails.groups)\n\nmodule Blog\n  class Application < Rails::Application\n    config.load_defaults 8.1\n    config.eager_load = false\n    config.logger = Logger.new(nil)\n  end\nend\n"),
        ("config/environment.rb", "require_relative \"application\"\nRails.application.initialize!\n"),
        ("config/database.yml", "development:\n  adapter: sqlite3\n  database: db/development.sqlite3\n"),
        ("config/routes.rb", "Rails.application.routes.draw do\n  resources :posts\nend\n"),
        ("db/schema.rb", "ActiveRecord::Schema[8.1].define(version: 1) do\n  create_table \"posts\" do |t|\n    t.string \"title\", null: false\n    t.text \"body\"\n    t.datetime \"created_at\", null: false\n    t.datetime \"updated_at\", null: false\n  end\nend\n"),
        ("app/models/application_record.rb", "class ApplicationRecord < ActiveRecord::Base\n  primary_abstract_class\nend\n"),
        ("app/models/post.rb", "class Post < ApplicationRecord\n  validates :title, presence: true\nend\n"),
        ("app/controllers/application_controller.rb", "class ApplicationController < ActionController::Base\nend\n"),
        ("app/controllers/posts_controller.rb", text),
    ];
    let dir = tempfile::tempdir().unwrap();
    let make = |name: &str| {
        let root = dir.path().join(name).join("blog");
        for (file, body) in files {
            std::fs::create_dir_all(root.join(file).parent().unwrap()).unwrap();
            std::fs::write(root.join(file), body).unwrap();
        }
        std::fs::set_permissions(root.join("bin/rails"), std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        for args in [&["lock", "--local"][..], &["exec", "rails", "db:schema:load"]] {
            let out = std::process::Command::new("bundle").args(args).current_dir(&root).env("BUNDLE_GEMFILE", root.join("Gemfile")).output().unwrap();
            assert!(out.status.success(), "bundle {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        }
        let c = config(&dir.path().join(name).join("config"));
        let mut settings: serde_json::Value = serde_json::from_slice(&std::fs::read(&c.providers).unwrap()).unwrap();
        settings["providers"]["ruby-lsp"] = serde_json::json!({ "executable": ruby_lsp });
        std::fs::write(&c.providers, settings.to_string()).unwrap(); (c, root)
    };
    let ((gc, groot), (tc, troot)) = (make("gui"), make("tui"));
    let file = |root: &std::path::Path| root.join("app/controllers/posts_controller.rb");
    let mut core = Session::new(gc); core.dispatch(Command::OpenRoot(groot.clone()));
    let gui = cx.new(|cx| ReaderView::new(core, Some(file(&groot)), cx));
    let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| gui.clone())).unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let focus = gui.read_with(&visual, |view, _| view.focus_handle()); visual.update(|window, cx| focus.focus(window, cx));
    let mut tui = App::new(tc, Some(troot.clone())); tui.session.dispatch(Command::Open(file(&troot)));
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 40)).unwrap();
    let wait = |visual: &mut VisualTestContext, tui: &mut App, terminal: &mut ratatui::Terminal<ratatui::backend::TestBackend>, limit: u64, done: &dyn Fn(&State) -> bool| {
        let until = Instant::now() + Duration::from_secs(limit);
        loop {
            gui.update(visual, |view, cx| view.poll(cx)); while let Ok(completion) = tui.session.completions().try_recv() { tui.apply(completion); }
            visual.update(|window, cx| { let _ = window.draw(cx); }); terminal.draw(|frame| tui.draw(frame)).unwrap();
            if gui.read_with(visual, |view, _| done(view.session().state())) && done(tui.session.state()) { break; }
            assert!(Instant::now() < until, "GUI {:?} {:?}; TUI {:?} {:?} {:?}", gui.read_with(visual, |view, _| view.session().state().notices()), gui.read_with(visual, |view, _| view.session().state().provider().map(|p| p.status.clone())), tui.session.state().notices(), tui.session.state().provider().map(|p| p.status.clone()), tui.modal_title());
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    let screen = |terminal: &ratatui::Terminal<ratatui::backend::TestBackend>| terminal.backend().buffer().content().chunks(120).map(|r| r.iter().map(|c| c.symbol()).collect::<String>() + "\n").collect::<String>();
    let flagged = |state: &State| state.active().and_then(|t| state.diagnostics(t.id())).is_some_and(|d| d.items.iter().any(|p| p.message.starts_with("Rails/TimeZone")));
    wait(&mut visual, &mut tui, &mut terminal, 120, &flagged);
    gui.update(&mut visual, |view, _| view.enable_draft_input_for_test()); tui.enable_draft_input_for_test();
    let key = |tui: &mut App, code, modifiers| { tui.handle(Event::Key(KeyEvent::new(code, modifiers))); };
    let current = |state: &State| state.current_source().map(|d| d.copy_range(0..d.len_bytes()).unwrap());
    let caret = |visual: &mut VisualTestContext, tui: &mut App, at: usize| {
        gui.update(visual, |view, cx| view.dispatch_for_test(Command::Select(code_core::session::Selection { anchor: at, head: at }), cx));
        tui.session.dispatch(Command::Select(code_core::session::Selection { anchor: at, head: at }));
    };
    println!("step: documentation");
    // Quick Documentation of the model through Find Action, until the booted application adds its columns.
    caret(&mut visual, &mut tui, text.find("Post.where").unwrap() + 1);
    let documented = |state: &State| state.documentation().map(|d| d.preview.doc.text.clone());
    let asked = Instant::now();
    loop {
        visual.simulate_keystrokes("ctrl-shift-a q u i c k space d o c u m e n t a t i o n");
        key(&mut tui, KeyCode::Char('a'), KeyModifiers::CONTROL | KeyModifiers::SHIFT);
        tui.handle(Event::Paste("quick documentation".into()));
        wait(&mut visual, &mut tui, &mut terminal, 10, &|state| state.navigation().is_some_and(|n| !n.pending));
        visual.simulate_keystrokes("enter"); key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
        wait(&mut visual, &mut tui, &mut terminal, 60, &|state| state.documentation().is_some());
        let both = [gui.read_with(&visual, |view, _| documented(view.session().state())), documented(tui.session.state())];
        if both.iter().all(|d| d.as_deref().is_some_and(|d| d.contains("title: string - not null"))) { break; }
        println!("documentation not complete yet: {both:?}");
        assert!(asked.elapsed() < Duration::from_secs(60), "{both:?}");
        visual.simulate_keystrokes("escape"); key(&mut tui, KeyCode::Esc, KeyModifiers::NONE);
        wait(&mut visual, &mut tui, &mut terminal, 10, &|state| state.documentation().is_none());
        std::thread::sleep(Duration::from_millis(500));
    }
    assert_eq!(gui.read_with(&visual, |view, _| documented(view.session().state())), documented(tui.session.state()));
    assert!(gui.read_with(&visual, |view, _| view.doc_popup_bounds().is_some()), "the GUI draws its documentation");
    let shown = screen(&terminal);
    assert!(tui.modal.is_some() && shown.contains("Documentation") && shown.contains("title: string - not null") && shown.contains("body: text"), "{shown}");
    visual.simulate_keystrokes("escape"); key(&mut tui, KeyCode::Esc, KeyModifiers::NONE);
    wait(&mut visual, &mut tui, &mut terminal, 10, &|state| state.documentation().is_none());
    println!("step: where completion");
    // Typing in `where(ti` suggests the model's column, drawn with what it filters; Enter accepts it.
    caret(&mut visual, &mut tui, text.find("where(ti)").unwrap() + 8);
    visual.simulate_keystrokes("t"); key(&mut tui, KeyCode::Char('t'), KeyModifiers::NONE);
    let typed = text.replacen("where(ti)", "where(tit)", 1);
    let rows = |state: &State| state.completion().map(|c| c.rows().map(|r| (r.label.clone(), r.detail.clone())).collect::<Vec<_>>());
    // Ruby LSP 0.26.11 with the Rails add-on sometimes answers a completion asked right after an edit with nothing
    // (seen outside Lenz too); then Ctrl+Space asks again, in both.
    let titled = |state: &State| rows(state).is_some_and(|r| r.iter().any(|(l, _)| l == "title"));
    let asked = Instant::now();
    loop {
        let until = Instant::now() + Duration::from_secs(3);
        while Instant::now() < until && !(gui.read_with(&visual, |view, _| titled(view.session().state())) && titled(tui.session.state())) {
            gui.update(&mut visual, |view, cx| view.poll(cx)); while let Ok(completion) = tui.session.completions().try_recv() { tui.apply(completion); }
            visual.update(|window, cx| { let _ = window.draw(cx); }); terminal.draw(|frame| tui.draw(frame)).unwrap();
            std::thread::sleep(Duration::from_millis(5));
        }
        let both = (gui.read_with(&visual, |view, _| rows(view.session().state())), rows(tui.session.state()));
        if both.0.is_some() && both.0 == both.1 && titled(tui.session.state()) { break; }
        println!("suggestions at {:.1} s: GUI {:?}; TUI {:?}", asked.elapsed().as_secs_f64(), both.0, both.1);
        assert!(asked.elapsed() < Duration::from_secs(60), "{both:?}");
        visual.simulate_keystrokes("ctrl-space"); key(&mut tui, KeyCode::Char(' '), KeyModifiers::CONTROL);
    }
    let expected = Some(vec![("title".to_owned(), Some("Filter Post records by title".to_owned()))]);
    assert_eq!(rows(tui.session.state()), expected);
    assert_eq!(gui.read_with(&visual, |view, _| rows(view.session().state())), expected);
    assert!(gui.read_with(&visual, |view, _| view.completion_bounds().is_some()), "the GUI draws its suggestions");
    let shown = screen(&terminal);
    assert!(shown.contains(" title  Filter Post records by title "), "{shown}");
    visual.simulate_keystrokes("enter"); key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    let completed = text.replacen("where(ti)", "where(title: )", 1);
    wait(&mut visual, &mut tui, &mut terminal, 30, &|state| state.completion().is_none() && current(state).as_deref() != Some(typed.as_str()));
    assert_eq!(current(tui.session.state()).as_deref(), Some(completed.as_str()));
    assert_eq!(gui.read_with(&visual, |view, _| current(view.session().state())).as_deref(), Some(completed.as_str()));
    visual.simulate_keystrokes("ctrl-z"); key(&mut tui, KeyCode::Char('z'), KeyModifiers::CONTROL);
    assert_eq!(current(tui.session.state()).as_deref(), Some(typed.as_str()));
    assert_eq!(gui.read_with(&visual, |view, _| current(view.session().state())).as_deref(), Some(typed.as_str()));
    visual.simulate_keystrokes("ctrl-z"); key(&mut tui, KeyCode::Char('z'), KeyModifiers::CONTROL);
    assert_eq!(current(tui.session.state()).as_deref(), Some(text));
    println!("step: quick fix");
    // rubocop-rails' fix through Alt+Enter.
    wait(&mut visual, &mut tui, &mut terminal, 60, &flagged);
    caret(&mut visual, &mut tui, text.find("Time.now").unwrap() + 6);
    visual.simulate_keystrokes("alt-enter"); key(&mut tui, KeyCode::Enter, KeyModifiers::ALT);
    let fix = |state: &State| state.code_actions().and_then(|c| c.actions.iter().position(|a| a.title == "Autocorrect Rails/TimeZone" && a.unavailable.is_none()));
    wait(&mut visual, &mut tui, &mut terminal, 60, &|state| fix(state).is_some());
    let row = fix(tui.session.state()).unwrap();
    assert_eq!(gui.read_with(&visual, |view, _| fix(view.session().state())), Some(row));
    for _ in 0..row { visual.simulate_keystrokes("down"); key(&mut tui, KeyCode::Down, KeyModifiers::NONE); }
    visual.simulate_keystrokes("enter"); key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    let fixed = text.replacen("Time.now", "Time.zone.now", 1);
    assert_eq!(current(tui.session.state()).as_deref(), Some(fixed.as_str()));
    assert_eq!(gui.read_with(&visual, |view, _| current(view.session().state())).as_deref(), Some(fixed.as_str()));
    visual.simulate_keystrokes("ctrl-z"); key(&mut tui, KeyCode::Char('z'), KeyModifiers::CONTROL);
    assert_eq!(current(tui.session.state()).as_deref(), Some(text));
    println!("step: formatting");
    // RuboCop formatting through Ctrl+Alt+L.
    visual.simulate_keystrokes("ctrl-alt-l"); key(&mut tui, KeyCode::Char('l'), KeyModifiers::CONTROL | KeyModifiers::ALT);
    wait(&mut visual, &mut tui, &mut terminal, 60, &|state| current(state).is_some_and(|t| t.contains("\n    redirect_to posts_path\n")));
    assert_eq!(gui.read_with(&visual, |view, _| current(view.session().state())), current(tui.session.state()));
    visual.simulate_keystrokes("ctrl-z"); key(&mut tui, KeyCode::Char('z'), KeyModifiers::CONTROL);
    assert_eq!(current(tui.session.state()).as_deref(), Some(text));
    assert_eq!(gui.read_with(&visual, |view, _| current(view.session().state())).as_deref(), Some(text));
    for root in [&groot, &troot] { assert_eq!(std::fs::read_to_string(file(root)).unwrap(), text); }
}

/// Real Ruby LSP with its Rails add-on, the HTML server and erb_lint behind both adapters, on a Rails view of an
/// application created for the test (ticket 58): Ruby's problem at the view's `]` (CRLF, non-BMP text) underlined in
/// both; `link_t` typed into an output tag completed to Action View's `link_to`, and `post_l` to the application's own
/// helper `post_link`, chosen with Enter from the suggestions drawn in both; parameter information for `link_to`
/// (Ctrl+Shift+Space / the terminal's Parameter Info); Ctrl+Alt+L running the project's erb_lint from its bundle, the
/// CRLF line ends kept; Server Details (Find Action) showing the view context alike. Each change undone with Ctrl+Z;
/// the same draft in both and the files untouched. Needs the gems of `real_rails_assistance_matches_in_both_adapters`,
/// `erb_lint` and `vscode-html-language-server`.
#[gpui::test]
#[ignore = "needs installed ruby-lsp, ruby-lsp-rails, rails, sqlite3, rubocop, rubocop-rails and erb_lint gems and vscode-html-language-server (Node.js); run alone"]
fn real_rails_view_assistance_matches_in_both_adapters(cx: &mut TestAppContext) {
    let Some(ruby_lsp) = std::env::var_os("READER_RUBY_LSP").map(std::path::PathBuf::from)
        .or_else(|| code_core::semantics::provider::find_executable(std::path::Path::new("ruby-lsp"), None)) else {
        println!("SKIPPED: Ruby LSP is not installed");
        return;
    };
    let Some(html) = code_core::semantics::provider::find_executable(std::path::Path::new("vscode-html-language-server"), None) else {
        println!("SKIPPED: vscode-html-language-server is not installed");
        return;
    };
    let listed = std::process::Command::new("gem").args(["list", "--local"]).output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    let missing: Vec<&str> = ["ruby-lsp-rails", "railties", "sqlite3", "rubocop", "rubocop-rails", "erb_lint"].into_iter().filter(|g| !listed.lines().any(|l| l.starts_with(&format!("{g} (")))).collect();
    if !missing.is_empty() {
        println!("SKIPPED: gems not installed: {missing:?}");
        return;
    }
    let text = "<h1 title=\"𝄞\">Posts 𝄞</h1>\r\n<% @posts.each do |post| %>\r\n  <p><%= link_to(\"x\", posts_path, class: \"a\") %></p>\r\n  <%=post_link(post)%>\r\n  <li><%=  %></li>\r\n<% end %>\r\n<%= [1].sum] %>\r\n";
    let helper = "module PostsHelper\n  def post_link(post)\n    link_to(post.title, post_path(post))\n  end\nend\n";
    let files = [
        ("Gemfile", "source \"https://rubygems.org\"\n\ngem \"rails\"\ngem \"sqlite3\"\ngem \"rubocop\"\ngem \"rubocop-rails\"\ngem \"erb_lint\"\n"),
        (".erb_lint.yml", "---\nEnableDefaultLinters: true\nlinters:\n  TrailingWhitespace:\n    enabled: false\n"),
        ("Rakefile", "require_relative \"config/application\"\nRails.application.load_tasks\n"),
        ("bin/rails", "#!/usr/bin/env ruby\nAPP_PATH = File.expand_path(\"../config/application\", __dir__)\nrequire_relative \"../config/boot\"\nrequire \"rails/commands\"\n"),
        ("config/boot.rb", "ENV[\"BUNDLE_GEMFILE\"] ||= File.expand_path(\"../Gemfile\", __dir__)\nrequire \"bundler/setup\"\n"),
        ("config/application.rb", "require_relative \"boot\"\nrequire \"rails\"\nrequire \"active_model/railtie\"\nrequire \"active_record/railtie\"\nrequire \"action_controller/railtie\"\nrequire \"action_view/railtie\"\nBundler.require(*Rails.groups)\n\nmodule Blog\n  class Application < Rails::Application\n    config.load_defaults 8.1\n    config.eager_load = false\n    config.logger = Logger.new(nil)\n  end\nend\n"),
        ("config/environment.rb", "require_relative \"application\"\nRails.application.initialize!\n"),
        ("config/database.yml", "development:\n  adapter: sqlite3\n  database: db/development.sqlite3\n"),
        ("config/routes.rb", "Rails.application.routes.draw do\n  resources :posts\nend\n"),
        ("db/schema.rb", "ActiveRecord::Schema[8.1].define(version: 1) do\n  create_table \"posts\" do |t|\n    t.string \"title\", null: false\n  end\nend\n"),
        ("app/models/application_record.rb", "class ApplicationRecord < ActiveRecord::Base\n  primary_abstract_class\nend\n"),
        ("app/models/post.rb", "class Post < ApplicationRecord\nend\n"),
        ("app/controllers/application_controller.rb", "class ApplicationController < ActionController::Base\nend\n"),
        ("app/controllers/posts_controller.rb", "class PostsController < ApplicationController\n  def index\n    @posts = Post.all\n  end\nend\n"),
        ("app/helpers/application_helper.rb", "module ApplicationHelper\nend\n"),
        ("app/helpers/posts_helper.rb", helper),
        ("app/views/posts/index.html.erb", text),
    ];
    let dir = tempfile::tempdir().unwrap();
    let make = |name: &str| {
        let root = dir.path().join(name).join("blog");
        for (file, body) in files {
            std::fs::create_dir_all(root.join(file).parent().unwrap()).unwrap();
            std::fs::write(root.join(file), body).unwrap();
        }
        std::fs::set_permissions(root.join("bin/rails"), std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        for args in [&["lock", "--local"][..], &["exec", "rails", "db:schema:load"]] {
            let out = std::process::Command::new("bundle").args(args).current_dir(&root).env("BUNDLE_GEMFILE", root.join("Gemfile")).output().unwrap();
            assert!(out.status.success(), "bundle {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        }
        let c = config(&dir.path().join(name).join("config"));
        let mut settings: serde_json::Value = serde_json::from_slice(&std::fs::read(&c.providers).unwrap()).unwrap();
        settings["providers"]["ruby-lsp"] = serde_json::json!({ "executable": ruby_lsp });
        settings["providers"]["vscode-html"] = serde_json::json!({ "executable": html });
        std::fs::write(&c.providers, settings.to_string()).unwrap(); (c, root)
    };
    let ((gc, groot), (tc, troot)) = (make("gui"), make("tui"));
    let file = |root: &std::path::Path| root.join("app/views/posts/index.html.erb");
    let mut core = Session::new(gc); core.dispatch(Command::OpenRoot(groot.clone()));
    let gui = cx.new(|cx| ReaderView::new(core, Some(file(&groot)), cx));
    let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| gui.clone())).unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let focus = gui.read_with(&visual, |view, _| view.focus_handle()); visual.update(|window, cx| focus.focus(window, cx));
    let mut tui = App::new(tc, Some(troot.clone())); tui.session.dispatch(Command::Open(file(&troot)));
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 40)).unwrap();
    let pump = |visual: &mut VisualTestContext, tui: &mut App, terminal: &mut ratatui::Terminal<ratatui::backend::TestBackend>| {
        gui.update(visual, |view, cx| view.poll(cx)); while let Ok(completion) = tui.session.completions().try_recv() { tui.apply(completion); }
        visual.update(|window, cx| { let _ = window.draw(cx); }); terminal.draw(|frame| tui.draw(frame)).unwrap();
    };
    let wait = |visual: &mut VisualTestContext, tui: &mut App, terminal: &mut ratatui::Terminal<ratatui::backend::TestBackend>, limit: u64, done: &dyn Fn(&State) -> bool| {
        let until = Instant::now() + Duration::from_secs(limit);
        loop {
            pump(visual, tui, terminal);
            if gui.read_with(visual, |view, _| done(view.session().state())) && done(tui.session.state()) { break; }
            assert!(Instant::now() < until, "GUI {:?}; TUI {:?} {:?}", gui.read_with(visual, |view, _| view.session().state().notices()), tui.session.state().notices(), tui.modal_title());
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    let screen = |terminal: &ratatui::Terminal<ratatui::backend::TestBackend>| terminal.backend().buffer().content().chunks(120).map(|r| r.iter().map(|c| c.symbol()).collect::<String>() + "\n").collect::<String>();
    let problems = |state: &State| state.active().and_then(|t| state.diagnostics(t.id())).map(|d| d.items.iter().map(|p| p.range.clone()).collect::<Vec<_>>()).unwrap_or_default();
    let close = text.find("sum]").unwrap() + 3;
    wait(&mut visual, &mut tui, &mut terminal, 180, &|state| problems(state).first() == Some(&(close..close + 1)));
    let palette = tui.session.state().palette();
    assert!(gui.read_with(&visual, |view, _| view.problem_marks()).iter().all(|m| *m == ("]".to_string(), palette.error)), "the GUI marks the `]`");
    gui.update(&mut visual, |view, _| view.enable_draft_input_for_test()); tui.enable_draft_input_for_test();
    let key = |tui: &mut App, code, modifiers| { tui.handle(Event::Key(KeyEvent::new(code, modifiers))); };
    let current = |state: &State| state.current_source().map(|d| d.copy_range(0..d.len_bytes()).unwrap());
    let caret = |visual: &mut VisualTestContext, tui: &mut App, at: usize| {
        gui.update(visual, |view, cx| view.dispatch_for_test(Command::Select(code_core::session::Selection { anchor: at, head: at }), cx));
        tui.session.dispatch(Command::Select(code_core::session::Selection { anchor: at, head: at }));
    };
    let both = |visual: &mut VisualTestContext, tui: &App, expected: &str| {
        assert_eq!(current(tui.session.state()).as_deref(), Some(expected));
        assert_eq!(gui.read_with(visual, |view, _| current(view.session().state())).as_deref(), Some(expected));
    };
    let rows = |state: &State| state.completion().map(|c| c.rows().map(|r| r.label.clone()).collect::<Vec<_>>());
    let undo_to = |visual: &mut VisualTestContext, tui: &mut App, expected: &str| {
        for _ in 0..8 {
            if current(tui.session.state()).as_deref() == Some(expected) { break; }
            visual.simulate_keystrokes("ctrl-z"); tui.handle(Event::Key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL)));
            assert_eq!(gui.read_with(visual, |view, _| current(view.session().state())), current(tui.session.state()), "the same draft after each Ctrl+Z");
        }
        both(visual, tui, expected);
    };
    // A helper typed into the empty output tag, chosen from the suggestions (asked again with Ctrl+Space while Ruby
    // LSP still indexes the bundle's gems).
    let complete = |visual: &mut VisualTestContext, tui: &mut App, terminal: &mut ratatui::Terminal<ratatui::backend::TestBackend>, typed: &str, label: &str| {
        println!("step: {label}");
        caret(visual, tui, text.find("<%=  %>").unwrap() + 4);
        let keys = typed.chars().map(String::from).collect::<Vec<_>>().join(" ");
        visual.simulate_keystrokes(&keys); for c in typed.chars() { key(tui, KeyCode::Char(c), KeyModifiers::NONE); }
        let draft = text.replacen("<%=  %>", &format!("<%= {typed} %>"), 1);
        wait(visual, tui, terminal, 10, &|state| current(state).as_deref() == Some(draft.as_str()));
        let offered = |state: &State| rows(state).is_some_and(|r| r.iter().any(|l| l == label));
        for _ in 0..40 {
            let until = Instant::now() + Duration::from_secs(3);
            while Instant::now() < until && !(gui.read_with(visual, |view, _| offered(view.session().state())) && offered(tui.session.state())) {
                pump(visual, tui, terminal); std::thread::sleep(Duration::from_millis(5));
            }
            if gui.read_with(visual, |view, _| offered(view.session().state())) && offered(tui.session.state()) { break; }
            if !gui.read_with(visual, |view, _| offered(view.session().state())) { visual.simulate_keystrokes("ctrl-space"); }
            if !offered(tui.session.state()) { key(tui, KeyCode::Char(' '), KeyModifiers::CONTROL); }
        }
        wait(visual, tui, terminal, 5, &offered);
        let labels = rows(tui.session.state()).unwrap();
        println!("{label}: {labels:?}");
        assert_eq!(gui.read_with(visual, |view, _| rows(view.session().state())), Some(labels.clone()), "the same suggestions in both");
        assert!(gui.read_with(visual, |view, _| view.completion_bounds().is_some()) && screen(terminal).contains(label), "both draw them");
        let row = labels.iter().position(|l| l == label).unwrap();
        for _ in 0..row { visual.simulate_keystrokes("down"); key(tui, KeyCode::Down, KeyModifiers::NONE); }
        visual.simulate_keystrokes("enter"); key(tui, KeyCode::Enter, KeyModifiers::NONE);
        let accepted = text.replacen("<%=  %>", &format!("<%= {label} %>"), 1);
        wait(visual, tui, terminal, 30, &|state| current(state).as_deref() == Some(accepted.as_str()));
        visual.simulate_keystrokes("ctrl-z"); key(tui, KeyCode::Char('z'), KeyModifiers::CONTROL);
        both(visual, tui, &draft);
        undo_to(visual, tui, text);
    };
    complete(&mut visual, &mut tui, &mut terminal, "link_t", "link_to");
    complete(&mut visual, &mut tui, &mut terminal, "post_l", "post_link");
    println!("step: parameter information");
    caret(&mut visual, &mut tui, text.find("posts_path, class").unwrap() + 13);
    let signature = |state: &State| state.signature_help().and_then(|h| h.active_signature().map(|s| (s.label.clone(), s.active_parameter)));
    for _ in 0..20 {
        if gui.read_with(&visual, |view, _| view.session().state().signature_help().is_none()) { visual.simulate_keystrokes("ctrl-shift-space"); }
        if tui.session.state().signature_help().is_none() { tui.run(CommandId::ParameterInfo); }
        let until = Instant::now() + Duration::from_secs(2);
        while Instant::now() < until && !(gui.read_with(&visual, |view, _| view.session().state().signature_help().is_some()) && tui.session.state().signature_help().is_some()) {
            pump(&mut visual, &mut tui, &mut terminal); std::thread::sleep(Duration::from_millis(5));
        }
        if gui.read_with(&visual, |view, _| view.session().state().signature_help().is_some()) && tui.session.state().signature_help().is_some() { break; }
    }
    wait(&mut visual, &mut tui, &mut terminal, 5, &|state| state.signature_help().is_some());
    assert_eq!(signature(tui.session.state()), Some(("link_to(name = <default>, options = <default>, html_options = <default>, &block)".to_string(), Some(2))));
    assert_eq!(gui.read_with(&visual, |view, _| signature(view.session().state())), signature(tui.session.state()));
    assert!(gui.read_with(&visual, |view, _| view.signature_bounds().is_some()) && tui.layout.signature.width > 0, "both draw the box");
    visual.simulate_keystrokes("escape"); key(&mut tui, KeyCode::Esc, KeyModifiers::NONE);
    wait(&mut visual, &mut tui, &mut terminal, 10, &|state| state.signature_help().is_none());
    println!("step: formatting");
    visual.simulate_keystrokes("ctrl-alt-l"); key(&mut tui, KeyCode::Char('l'), KeyModifiers::CONTROL | KeyModifiers::ALT);
    let formatted = text.replacen("<%=post_link(post)%>", "<%= post_link(post) %>", 1).replacen("<%=  %>", "<%= %>", 1);
    wait(&mut visual, &mut tui, &mut terminal, 60, &|state| current(state).as_deref() == Some(formatted.as_str()) && state.notices().iter().any(|n| n == "Format Document: done with erb_lint"));
    visual.simulate_keystrokes("ctrl-z"); key(&mut tui, KeyCode::Char('z'), KeyModifiers::CONTROL);
    both(&mut visual, &tui, text);
    println!("step: details");
    visual.simulate_keystrokes("ctrl-shift-a s e r v e r space d e t a i l s");
    key(&mut tui, KeyCode::Char('a'), KeyModifiers::CONTROL | KeyModifiers::SHIFT);
    tui.handle(Event::Paste("server details".into()));
    wait(&mut visual, &mut tui, &mut terminal, 10, &|state| state.navigation().is_some_and(|n| !n.pending));
    visual.simulate_keystrokes("enter"); key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    let shown = gui.read_with(&visual, |v, _| v.modal_rows()).into_iter().map(|(row, _)| row).collect::<Vec<_>>();
    let Some(crate::app::Prompt::Details { body, .. }) = tui.modal.as_ref().map(|m| &m.kind) else { panic!("no terminal details: {:?}", tui.modal_title()) };
    let body = body.clone();
    let row = "ERB template view context: a Rails view (Action View), with ApplicationHelper and PostsHelper; controller instance variables and route helpers are not known to Ruby LSP's index";
    assert!(shown.iter().any(|r| r == row), "GUI lacks {row}: {shown:?}");
    assert!(body.contains(row), "terminal lacks {row}: {body}");
    visual.simulate_keystrokes("escape"); key(&mut tui, KeyCode::Esc, KeyModifiers::NONE);
    for root in [&groot, &troot] {
        assert_eq!(std::fs::read_to_string(file(root)).unwrap(), text);
        assert_eq!(std::fs::read_to_string(root.join("app/helpers/posts_helper.rb")).unwrap(), helper);
    }
}

/// Real VS Code HTML server behind both adapters, on a page with a stylesheet and a script of its own: the stylesheet's
/// problem shown in both; typed `<li>` closed by the server with the caret inside, and `</` closing it again after an
/// Undo; a tag name chosen with Down and Enter in the typing-opened suggestions, drawn in both; Format Document
/// through Ctrl+Alt+L; each one Ctrl+Z; then the missing quick fixes explained by Alt+Enter and Server Details (Find
/// Action) alike. The same draft in both and the files untouched. Needs `vscode-html-language-server` (npm
/// `vscode-langservers-extracted`) on PATH.
#[gpui::test]
#[ignore = "needs vscode-html-language-server (vscode-langservers-extracted) and Node.js; run alone"]
fn real_html_assistance_matches_in_both_adapters(cx: &mut TestAppContext) {
    let Some(server) = code_core::semantics::provider::find_executable(std::path::Path::new("vscode-html-language-server"), None) else {
        println!("SKIPPED: vscode-html-language-server is not installed");
        return;
    };
    let text = "<!doctype html>\n<ul>\n<li>𝄞 one</li>\n\n</ul>\n<p>\n</p>\n<style>.card { colr: red; }</style>\n<script>\nfunction greet(name) { return name; }\n</script>\n";
    let dir = tempfile::tempdir().unwrap();
    let make = |name: &str| {
        let root = dir.path().join(name).join("site");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("index.html"), text).unwrap();
        let c = config(&dir.path().join(name).join("config"));
        let mut settings: serde_json::Value = serde_json::from_slice(&std::fs::read(&c.providers).unwrap()).unwrap();
        settings["providers"]["vscode-html"] = serde_json::json!({ "executable": server, "settings": { "format": { "indentInnerHtml": true } } });
        std::fs::write(&c.providers, settings.to_string()).unwrap(); (c, root)
    };
    let ((gc, groot), (tc, troot)) = (make("gui"), make("tui"));
    let file = |root: &std::path::Path| root.join("index.html");
    let mut core = Session::new(gc); core.dispatch(Command::OpenRoot(groot.clone()));
    let gui = cx.new(|cx| ReaderView::new(core, Some(file(&groot)), cx));
    let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| gui.clone())).unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let focus = gui.read_with(&visual, |view, _| view.focus_handle()); visual.update(|window, cx| focus.focus(window, cx));
    let mut tui = App::new(tc, Some(troot.clone())); tui.session.dispatch(Command::Open(file(&troot)));
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 40)).unwrap();
    let wait = |visual: &mut VisualTestContext, tui: &mut App, terminal: &mut ratatui::Terminal<ratatui::backend::TestBackend>, limit: u64, done: &dyn Fn(&State) -> bool| {
        let until = Instant::now() + Duration::from_secs(limit);
        loop {
            gui.update(visual, |view, cx| view.poll(cx)); while let Ok(completion) = tui.session.completions().try_recv() { tui.apply(completion); }
            visual.update(|window, cx| { let _ = window.draw(cx); }); terminal.draw(|frame| tui.draw(frame)).unwrap();
            if gui.read_with(visual, |view, _| done(view.session().state())) && done(tui.session.state()) { break; }
            assert!(Instant::now() < until, "GUI {:?}; TUI {:?} {:?}", gui.read_with(visual, |view, _| view.session().state().notices()), tui.session.state().notices(), tui.modal_title());
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    let screen = |terminal: &ratatui::Terminal<ratatui::backend::TestBackend>| terminal.backend().buffer().content().chunks(120).map(|r| r.iter().map(|c| c.symbol()).collect::<String>() + "\n").collect::<String>();
    let flagged = |state: &State| state.active().and_then(|t| state.diagnostics(t.id())).is_some_and(|d| d.items.iter().any(|p| p.message == "Unknown property: 'colr'"));
    wait(&mut visual, &mut tui, &mut terminal, 60, &flagged);
    gui.update(&mut visual, |view, _| view.enable_draft_input_for_test()); tui.enable_draft_input_for_test();
    let key = |tui: &mut App, code, modifiers| { tui.handle(Event::Key(KeyEvent::new(code, modifiers))); };
    let current = |state: &State| state.current_source().map(|d| d.copy_range(0..d.len_bytes()).unwrap());
    let caret = |visual: &mut VisualTestContext, tui: &mut App, at: usize| {
        gui.update(visual, |view, cx| view.dispatch_for_test(Command::Select(code_core::session::Selection { anchor: at, head: at }), cx));
        tui.session.dispatch(Command::Select(code_core::session::Selection { anchor: at, head: at }));
    };
    let both = |visual: &mut VisualTestContext, tui: &App, expected: &str| {
        assert_eq!(current(tui.session.state()).as_deref(), Some(expected));
        assert_eq!(gui.read_with(visual, |view, _| current(view.session().state())).as_deref(), Some(expected));
    };
    let selection = |state: &State| state.selection().map(|s| s.range());
    println!("step: closing tags");
    // `<li>` typed on the empty line: the server closes it, the caret stays inside.
    let line = text.find("</li>\n\n").unwrap() + "</li>\n".len();
    caret(&mut visual, &mut tui, line);
    visual.simulate_keystrokes("< l i >"); for c in "<li>".chars() { key(&mut tui, KeyCode::Char(c), KeyModifiers::NONE); }
    let closed = text.replacen("</li>\n\n", "</li>\n<li></li>\n", 1);
    wait(&mut visual, &mut tui, &mut terminal, 30, &|state| current(state).as_deref() == Some(closed.as_str()));
    let inside = line + "<li>".len();
    assert_eq!(selection(tui.session.state()), Some(inside..inside));
    assert_eq!(gui.read_with(&visual, |view, _| selection(view.session().state())), Some(inside..inside));
    assert!(screen(&terminal).contains("<li></li>"), "{}", screen(&terminal));
    // Ctrl+Z takes the closing tag back; `x` then `</` closes the element again.
    visual.simulate_keystrokes("ctrl-z"); key(&mut tui, KeyCode::Char('z'), KeyModifiers::CONTROL);
    let opened = text.replacen("</li>\n\n", "</li>\n<li>\n", 1);
    both(&mut visual, &tui, &opened);
    visual.simulate_keystrokes("x < /"); for c in "x</".chars() { key(&mut tui, KeyCode::Char(c), KeyModifiers::NONE); }
    let reclosed = text.replacen("</li>\n\n", "</li>\n<li>x</li>\n", 1);
    wait(&mut visual, &mut tui, &mut terminal, 30, &|state| current(state).as_deref() == Some(reclosed.as_str()) && state.completion().is_none());
    for _ in 0..3 { visual.simulate_keystrokes("ctrl-z"); key(&mut tui, KeyCode::Char('z'), KeyModifiers::CONTROL); }
    both(&mut visual, &tui, text);
    println!("step: tag completion");
    // A tag name from the suggestions typing opens, chosen with Down and Enter.
    caret(&mut visual, &mut tui, text.find("<p>\n").unwrap() + "<p>".len());
    visual.simulate_keystrokes("< s p"); for c in "<sp".chars() { key(&mut tui, KeyCode::Char(c), KeyModifiers::NONE); }
    let rows = |state: &State| state.completion().map(|c| c.rows().map(|r| r.label.clone()).collect::<Vec<_>>());
    wait(&mut visual, &mut tui, &mut terminal, 30, &|state| rows(state).is_some_and(|r| r.iter().any(|l| l == "span")));
    let labels = rows(tui.session.state()).unwrap();
    assert_eq!(gui.read_with(&visual, |view, _| rows(view.session().state())), Some(labels.clone()), "the same suggestions in both");
    assert!(gui.read_with(&visual, |view, _| view.completion_bounds().is_some()), "the GUI draws its suggestions");
    let shown = screen(&terminal);
    assert!(shown.contains(" span "), "{shown}");
    let row = labels.iter().position(|l| l == "span").unwrap();
    for _ in 0..row { visual.simulate_keystrokes("down"); key(&mut tui, KeyCode::Down, KeyModifiers::NONE); }
    visual.simulate_keystrokes("enter"); key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    let named = text.replacen("<p>\n", "<p><span\n", 1);
    wait(&mut visual, &mut tui, &mut terminal, 30, &|state| current(state).as_deref() == Some(named.as_str()));
    visual.simulate_keystrokes("ctrl-z"); key(&mut tui, KeyCode::Char('z'), KeyModifiers::CONTROL);
    visual.simulate_keystrokes("ctrl-z"); key(&mut tui, KeyCode::Char('z'), KeyModifiers::CONTROL);
    both(&mut visual, &tui, text);
    println!("step: formatting");
    visual.simulate_keystrokes("ctrl-alt-l"); key(&mut tui, KeyCode::Char('l'), KeyModifiers::CONTROL | KeyModifiers::ALT);
    wait(&mut visual, &mut tui, &mut terminal, 60, &|state| current(state).is_some_and(|t| t.contains("\n    <li>𝄞 one</li>\n")));
    let formatted = current(tui.session.state()).unwrap();
    println!("formatted:\n{formatted}");
    assert!(formatted.contains("\n<style>\n    .card {\n        colr: red;\n    }\n</style>\n"), "{formatted}");
    assert_eq!(gui.read_with(&visual, |view, _| current(view.session().state())).as_deref(), Some(formatted.as_str()));
    visual.simulate_keystrokes("ctrl-z"); key(&mut tui, KeyCode::Char('z'), KeyModifiers::CONTROL);
    both(&mut visual, &tui, text);
    println!("step: missing assistance");
    // Alt+Enter on the stylesheet's problem says there are no quick fixes, in both.
    caret(&mut visual, &mut tui, text.find("colr").unwrap() + 1);
    visual.simulate_keystrokes("alt-enter"); key(&mut tui, KeyCode::Enter, KeyModifiers::ALT);
    // (Both adapters show a disabled command's reason as a session notice.)
    let why = "The language server does not offer quick fixes";
    let said = gui.read_with(&visual, |view, _| view.session().state().notices());
    assert!(said.iter().any(|n| n == why), "{said:?}");
    assert!(tui.session.state().notices().iter().any(|n| n == why));
    terminal.draw(|frame| tui.draw(frame)).unwrap();
    assert!(screen(&terminal).contains(why), "{}", screen(&terminal));
    // Server Details (through Find Action) names what is offered, what is not, and the page's embedded languages.
    visual.simulate_keystrokes("ctrl-shift-a s e r v e r space d e t a i l s");
    key(&mut tui, KeyCode::Char('a'), KeyModifiers::CONTROL | KeyModifiers::SHIFT);
    tui.handle(Event::Paste("server details".into()));
    wait(&mut visual, &mut tui, &mut terminal, 10, &|state| state.navigation().is_some_and(|n| !n.pending));
    visual.simulate_keystrokes("enter"); key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    let shown = gui.read_with(&visual, |v, _| v.modal_rows()).into_iter().map(|(row, _)| row).collect::<Vec<_>>();
    let Some(crate::app::Prompt::Details { body, .. }) = tui.modal.as_ref().map(|m| &m.kind) else { panic!("no terminal details: {:?}", tui.modal_title()) };
    let body = body.clone();
    let offered = shown.iter().find(|r| r.starts_with("Offered by VS Code HTML: ")).unwrap_or_else(|| panic!("{shown:?}"));
    assert!(offered.contains("parameter information, formatting, automatic closing tags.") && offered.contains("Not offered by this server: ") && offered.contains("quick fixes"), "{offered}");
    let limits = shown.iter().find(|r| r.starts_with("Provider limits: ")).unwrap_or_else(|| panic!("{shown:?}"));
    assert!(limits.contains("CSS in <style> and style attributes and JavaScript in <script> and event attributes are served (no TypeScript, JSX or template languages; no quick fixes or organize imports)"), "{limits}");
    for row in [offered, limits] { assert!(body.contains(row.as_str()), "both adapters show the same row: {row}"); }
    visual.simulate_keystrokes("escape"); key(&mut tui, KeyCode::Esc, KeyModifiers::NONE);
    assert!(gui.read_with(&visual, |v, _| v.modal_rows()).is_empty() && tui.modal_title().is_empty());
    both(&mut visual, &tui, text);
    for root in [&groot, &troot] { assert_eq!(std::fs::read_to_string(file(root)).unwrap(), text); }
}

/// Enter in a YAML draft through both adapters, with no language server: the new line keeps its line's indentation and
/// opens a block one step (the project's `.editorconfig` size) deeper after a key; Tab inserts spaces even where
/// EditorConfig asks for tabs; each step one Ctrl+Z. The same draft in both and the file untouched.
#[gpui::test]
fn yaml_enter_and_tab_indent_alike_in_both_adapters(cx: &mut TestAppContext) {
    let text = "server:\n";
    let dir = tempfile::tempdir().unwrap();
    let make = |name: &str| {
        let root = dir.path().join(name).join("project");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join(".editorconfig"), "root = true\n[*]\nindent_style = tab\nindent_size = 3\n").unwrap();
        std::fs::write(root.join("app.yaml"), text).unwrap();
        (config(&dir.path().join(name).join("config")), root)
    };
    let ((gc, groot), (tc, troot)) = (make("gui"), make("tui"));
    let file = |root: &Path| root.join("app.yaml");
    let mut core = Session::new(gc); core.dispatch(Command::OpenRoot(groot.clone()));
    let gui = cx.new(|cx| ReaderView::new(core, Some(file(&groot)), cx));
    let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| gui.clone())).unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let focus = gui.read_with(&visual, |view, _| view.focus_handle()); visual.update(|window, cx| focus.focus(window, cx));
    let mut tui = App::new(tc, Some(troot.clone())); tui.session.dispatch(Command::Open(file(&troot)));
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
    let current = |state: &State| state.current_source().map(|d| d.copy_range(0..d.len_bytes()).unwrap());
    let until = Instant::now() + Duration::from_secs(20);
    loop {
        gui.update(&mut visual, |view, cx| view.poll(cx)); while let Ok(completion) = tui.session.completions().try_recv() { tui.apply(completion); }
        if gui.read_with(&visual, |view, _| current(view.session().state())).as_deref() == Some(text) && current(tui.session.state()).as_deref() == Some(text) { break; }
        assert!(Instant::now() < until, "the file never opened");
        std::thread::sleep(Duration::from_millis(5));
    }
    gui.update(&mut visual, |view, _| view.enable_draft_input_for_test()); tui.enable_draft_input_for_test();
    let key = |tui: &mut App, code, modifiers| { tui.handle(Event::Key(KeyEvent::new(code, modifiers))); };
    let at = "server:".len();
    gui.update(&mut visual, |view, cx| view.dispatch_for_test(Command::Select(code_core::session::Selection { anchor: at, head: at }), cx));
    tui.session.dispatch(Command::Select(code_core::session::Selection { anchor: at, head: at }));
    let both = |visual: &mut VisualTestContext, tui: &App, expected: &str| {
        assert_eq!(current(tui.session.state()).as_deref(), Some(expected));
        assert_eq!(gui.read_with(visual, |view, _| current(view.session().state())).as_deref(), Some(expected));
    };
    let ctrl_z = |visual: &mut VisualTestContext, tui: &mut App| { visual.simulate_keystrokes("ctrl-z"); key(tui, KeyCode::Char('z'), KeyModifiers::CONTROL); };
    // Enter after the key: three spaces (EditorConfig's size), then `- web` and Enter keeps the line's own.
    visual.simulate_keystrokes("enter"); key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    both(&mut visual, &tui, "server:\n   \n");
    visual.simulate_keystrokes("- space w e b enter"); for c in "- web".chars() { key(&mut tui, KeyCode::Char(c), KeyModifiers::NONE); } key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    both(&mut visual, &tui, "server:\n   - web\n     \n");
    // Tab: spaces, never the tab EditorConfig asks for.
    visual.simulate_keystrokes("tab"); key(&mut tui, KeyCode::Tab, KeyModifiers::NONE);
    both(&mut visual, &tui, "server:\n   - web\n        \n");
    terminal.draw(|frame| tui.draw(frame)).unwrap(); visual.update(|window, cx| { let _ = window.draw(cx); });
    ctrl_z(&mut visual, &mut tui);
    both(&mut visual, &tui, "server:\n   - web\n     \n");
    // (Typing within half a second is one Undo step.)
    for _ in 0..3 { if current(tui.session.state()).as_deref() != Some(text) { ctrl_z(&mut visual, &mut tui); } }
    both(&mut visual, &tui, text);
    for root in [&groot, &troot] { assert_eq!(std::fs::read_to_string(file(root)).unwrap(), text); }
}

/// Real yaml-language-server behind both adapters, with the user's settings mapping `config/*.yaml` to a local schema:
/// the same schema problems in both; Ctrl+Space after a typed key prefix shows the schema's key with its description,
/// both drawn in both, and Enter accepts it; Enter after a new key indents with the project's `.editorconfig` size; Format
/// Document through Ctrl+Alt+L; each one Ctrl+Z; Server Details (Find Action) says where schemas come from and that
/// nothing is downloaded, alike. The same draft in both and the files untouched. Needs `yaml-language-server` on PATH.
#[gpui::test]
#[ignore = "needs yaml-language-server and Node.js; run alone"]
fn real_yaml_schema_assistance_matches_in_both_adapters(cx: &mut TestAppContext) {
    let Some(server) = code_core::semantics::provider::find_executable(Path::new("yaml-language-server"), None) else {
        println!("SKIPPED: yaml-language-server is not installed");
        return;
    };
    let schema = r#"{ "type": "object", "additionalProperties": false, "properties": {
        "name": { "type": "string", "description": "The service name." },
        "mode": { "type": "string", "enum": ["fast", "safe"], "description": "How it runs." },
        "retries": { "type": "integer", "description": "How often to retry." },
        "server": { "type": "object", "properties": { "port": { "type": "integer", "description": "Port number." } } } } }"#;
    let text = "# 𝄞 service\nname: \"café\"\nmode: slow\nserver:\n  port: 80\n";
    let dir = tempfile::tempdir().unwrap();
    let make = |name: &str| {
        let root = dir.path().join(name).join("project");
        std::fs::create_dir_all(root.join("config")).unwrap();
        std::fs::create_dir_all(root.join("schemas")).unwrap();
        std::fs::write(root.join("schemas/service.json"), schema).unwrap();
        std::fs::write(root.join("config/app.yaml"), text).unwrap();
        std::fs::write(root.join(".editorconfig"), "root = true\n[*.yaml]\nindent_size = 4\n").unwrap();
        let c = config(&dir.path().join(name).join("config"));
        let mut settings: serde_json::Value = serde_json::from_slice(&std::fs::read(&c.providers).unwrap()).unwrap();
        settings["providers"]["yaml-language-server"] = serde_json::json!({ "executable": server, "settings": {
            "schemas": { "./schemas/service.json": "config/*.yaml" }, "format": { "singleQuote": true } } });
        std::fs::write(&c.providers, settings.to_string()).unwrap(); (c, root)
    };
    let ((gc, groot), (tc, troot)) = (make("gui"), make("tui"));
    let file = |root: &Path| root.join("config/app.yaml");
    let mut core = Session::new(gc); core.dispatch(Command::OpenRoot(groot.clone()));
    let gui = cx.new(|cx| ReaderView::new(core, Some(file(&groot)), cx));
    let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| gui.clone())).unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let focus = gui.read_with(&visual, |view, _| view.focus_handle()); visual.update(|window, cx| focus.focus(window, cx));
    let mut tui = App::new(tc, Some(troot.clone())); tui.session.dispatch(Command::Open(file(&troot)));
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 40)).unwrap();
    let wait = |visual: &mut VisualTestContext, tui: &mut App, terminal: &mut ratatui::Terminal<ratatui::backend::TestBackend>, limit: u64, done: &dyn Fn(&State) -> bool| {
        let until = Instant::now() + Duration::from_secs(limit);
        loop {
            gui.update(visual, |view, cx| view.poll(cx)); while let Ok(completion) = tui.session.completions().try_recv() { tui.apply(completion); }
            visual.update(|window, cx| { let _ = window.draw(cx); }); terminal.draw(|frame| tui.draw(frame)).unwrap();
            if gui.read_with(visual, |view, _| done(view.session().state())) && done(tui.session.state()) { break; }
            assert!(Instant::now() < until, "GUI {:?}; TUI {:?} {:?}", gui.read_with(visual, |view, _| view.session().state().notices()), tui.session.state().notices(), tui.modal_title());
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    let screen = |terminal: &ratatui::Terminal<ratatui::backend::TestBackend>| terminal.backend().buffer().content().chunks(120).map(|r| r.iter().map(|c| c.symbol()).collect::<String>() + "\n").collect::<String>();
    let problems = |state: &State| state.active().and_then(|t| state.diagnostics(t.id())).map(|d| d.items.iter().map(|p| (p.message.clone(), p.range.clone())).collect::<Vec<_>>()).unwrap_or_default();
    wait(&mut visual, &mut tui, &mut terminal, 60, &|state| !problems(state).is_empty());
    let slow = text.find("slow").unwrap();
    let expected = vec![("Value is not accepted. Valid values: \"fast\", \"safe\".".to_owned(), slow..slow + 4)];
    assert_eq!(problems(tui.session.state()), expected);
    assert_eq!(gui.read_with(&visual, |view, _| problems(view.session().state())), expected);
    gui.update(&mut visual, |view, _| view.enable_draft_input_for_test()); tui.enable_draft_input_for_test();
    let key = |tui: &mut App, code, modifiers| { tui.handle(Event::Key(KeyEvent::new(code, modifiers))); };
    let current = |state: &State| state.current_source().map(|d| d.copy_range(0..d.len_bytes()).unwrap());
    let both = |visual: &mut VisualTestContext, tui: &App, expected: &str| {
        assert_eq!(current(tui.session.state()).as_deref(), Some(expected));
        assert_eq!(gui.read_with(visual, |view, _| current(view.session().state())).as_deref(), Some(expected));
    };
    let ctrl_z = |visual: &mut VisualTestContext, tui: &mut App| { visual.simulate_keystrokes("ctrl-z"); key(tui, KeyCode::Char('z'), KeyModifiers::CONTROL); };
    let undo_to = |visual: &mut VisualTestContext, tui: &mut App, expected: &str| {
        for _ in 0..4 { if current(tui.session.state()).as_deref() != Some(expected) { ctrl_z(visual, tui); } }
        both(visual, tui, expected);
    };
    let end = text.len();
    gui.update(&mut visual, |view, cx| view.dispatch_for_test(Command::Select(code_core::session::Selection { anchor: end, head: end }), cx));
    tui.session.dispatch(Command::Select(code_core::session::Selection { anchor: end, head: end }));
    println!("step: key completion");
    visual.simulate_keystrokes("r e"); for c in "re".chars() { key(&mut tui, KeyCode::Char(c), KeyModifiers::NONE); }
    let typed = format!("{text}re");
    both(&mut visual, &tui, &typed);
    visual.simulate_keystrokes("ctrl-space"); key(&mut tui, KeyCode::Char(' '), KeyModifiers::CONTROL);
    let rows = |state: &State| state.completion().map(|c| c.rows().map(|r| (r.label.clone(), r.documentation.clone())).collect::<Vec<_>>());
    wait(&mut visual, &mut tui, &mut terminal, 30, &|state| rows(state).is_some_and(|r| r.iter().any(|(l, _)| l == "retries")) && state.completion().is_some_and(|c| c.documentation_preview().is_some()));
    let documentation_deadline=Instant::now()+Duration::from_secs(10);
    while gui.read_with(&visual, |view, _| view.completion_documentation_lines()).is_none() {
        assert!(Instant::now()<documentation_deadline,"GUI documentation shaping did not finish");
        cx.run_until_parked();
        gui.update(&mut visual,|view,cx|view.poll(cx));
        visual.update(|window,cx|{let _=window.draw(cx);});
        std::thread::sleep(Duration::from_millis(5));
    }
    let listed = rows(tui.session.state()).unwrap();
    assert_eq!(listed, [("retries".to_owned(), Some("How often to retry.".to_owned()))]);
    assert_eq!(gui.read_with(&visual, |view, _| rows(view.session().state())), Some(listed), "the same suggestion in both");
    assert!(gui.read_with(&visual, |view, _| view.completion_bounds().is_some()), "the GUI draws its suggestions");
    assert_eq!(gui.read_with(&visual, |view, _| view.completion_documentation_lines()), Some(vec!["How often to retry.".to_owned()]), "and the description beside them");
    let shown = screen(&terminal);
    assert!(shown.contains("retries") && shown.contains("How often to retry."), "{shown}");
    visual.simulate_keystrokes("enter"); key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    let completed = format!("{text}retries: 0");
    wait(&mut visual, &mut tui, &mut terminal, 30, &|state| current(state).as_deref() == Some(completed.as_str()));
    undo_to(&mut visual, &mut tui, text);
    println!("step: indentation");
    visual.simulate_keystrokes("l a b e l s : enter"); for c in "labels:".chars() { key(&mut tui, KeyCode::Char(c), KeyModifiers::NONE); } key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    let opened = format!("{text}labels:\n    ");
    wait(&mut visual, &mut tui, &mut terminal, 10, &|state| current(state).as_deref() == Some(opened.as_str()) && state.completion().is_none());
    undo_to(&mut visual, &mut tui, text);
    println!("step: formatting");
    visual.simulate_keystrokes("ctrl-alt-l"); key(&mut tui, KeyCode::Char('l'), KeyModifiers::CONTROL | KeyModifiers::ALT);
    let formatted = "# 𝄞 service\nname: 'café'\nmode: slow\nserver:\n    port: 80\n";
    wait(&mut visual, &mut tui, &mut terminal, 60, &|state| current(state).as_deref() == Some(formatted));
    ctrl_z(&mut visual, &mut tui);
    both(&mut visual, &tui, text);
    println!("step: server details");
    visual.simulate_keystrokes("ctrl-shift-a s e r v e r space d e t a i l s");
    key(&mut tui, KeyCode::Char('a'), KeyModifiers::CONTROL | KeyModifiers::SHIFT);
    tui.handle(Event::Paste("server details".into()));
    wait(&mut visual, &mut tui, &mut terminal, 10, &|state| state.navigation().is_some_and(|n| !n.pending));
    visual.simulate_keystrokes("enter"); key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    let shown = gui.read_with(&visual, |v, _| v.modal_rows()).into_iter().map(|(row, _)| row).collect::<Vec<_>>();
    let Some(crate::app::Prompt::Details { body, .. }) = tui.modal.as_ref().map(|m| &m.kind) else { panic!("no terminal details: {:?}", tui.modal_title()) };
    let body = body.clone();
    let schemas = shown.iter().find(|r| r.starts_with("Schemas: ")).unwrap_or_else(|| panic!("{shown:?}"));
    assert_eq!(schemas, "Schemas: local files only (1 schema mapping in providers.json, `$schema` keys and modelines in files); remote schemas and the SchemaStore catalog are not downloaded (set `remoteSchemas` to true in its provider settings to allow it)");
    assert!(body.contains(schemas.as_str()), "both adapters show the same row: {body}");
    visual.simulate_keystrokes("escape"); key(&mut tui, KeyCode::Esc, KeyModifiers::NONE);
    assert!(gui.read_with(&visual, |v, _| v.modal_rows()).is_empty() && tui.modal_title().is_empty());
    both(&mut visual, &tui, text);
    for root in [&groot, &troot] { assert_eq!(std::fs::read_to_string(file(root)).unwrap(), text); }
}

/// Pyright's Python environment through both adapters (the fake server standing in for Pyright): Server Details,
/// reached through Find Action, names the configured environment alike; once that environment is gone, Restart
/// Language Server (Find Action) leaves Pyright stopped in both, with the same reason in the status bar and in Server
/// Details and no reinstall advice, while typing goes on alike. The files never change.
#[gpui::test]
fn python_environment_state_and_a_missing_environment_match_in_both_adapters(cx: &mut TestAppContext) {
    let profile = std::env::current_exe().unwrap().parent().unwrap().parent().unwrap().to_owned();
    assert!(std::process::Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .args(["build", "-q", "--locked", "-p", "code-core", "--example", "fake_lsp"])
        .current_dir(env!("CARGO_MANIFEST_DIR")).status().unwrap().success());
    let fake = profile.join("examples/fake_lsp");
    let text = "import dep\n";
    let pyrightconfig = "{ \"venvPath\": \".\", \"venv\": \"env\" }\n";
    let dir = tempfile::tempdir().unwrap();
    let make = |name: &str| {
        let root = dir.path().join(name).join("project");
        std::fs::create_dir_all(root.join("env/lib/python3.13/site-packages/dep")).unwrap();
        for (file, body) in [("pyrightconfig.json", pyrightconfig), ("env/pyvenv.cfg", "version = 3.13.5\n"), ("env/lib/python3.13/site-packages/dep/__init__.py", ""), ("app.py", text)] { std::fs::write(root.join(file), body).unwrap(); }
        let c = config(&dir.path().join(name).join("config"));
        let mut settings: serde_json::Value = serde_json::from_slice(&std::fs::read(&c.providers).unwrap()).unwrap();
        settings["providers"]["pyright"] = serde_json::json!({ "executable": fake, "env": { "FAKE_LSP_NO_DECOR": "1" } });
        std::fs::write(&c.providers, settings.to_string()).unwrap(); (c, root)
    };
    let ((gc, groot), (tc, troot)) = (make("gui"), make("tui"));
    let file = |root: &Path| root.join("app.py");
    let mut core = Session::new(gc); core.dispatch(Command::OpenRoot(groot.clone()));
    let gui = cx.new(|cx| ReaderView::new(core, Some(file(&groot)), cx));
    let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| gui.clone())).unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let focus = gui.read_with(&visual, |view, _| view.focus_handle()); visual.update(|window, cx| focus.focus(window, cx));
    let mut tui = App::new(tc, Some(troot.clone())); tui.session.dispatch(Command::Open(file(&troot)));
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(160, 40)).unwrap();
    let wait = |visual: &mut VisualTestContext, tui: &mut App, terminal: &mut ratatui::Terminal<ratatui::backend::TestBackend>, done: &dyn Fn(&State) -> bool| {
        let until = Instant::now() + Duration::from_secs(15);
        loop {
            gui.update(visual, |view, cx| view.poll(cx)); while let Ok(completion) = tui.session.completions().try_recv() { tui.apply(completion); }
            visual.update(|window, cx| { let _ = window.draw(cx); }); terminal.draw(|frame| tui.draw(frame)).unwrap();
            if gui.read_with(visual, |view, _| done(view.session().state())) && done(tui.session.state()) { break; }
            assert!(Instant::now() < until, "GUI {:?}; TUI {:?} {:?}", gui.read_with(visual, |view, _| view.session().state().notices()), tui.session.state().notices(), tui.modal_title());
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    let screen = |terminal: &ratatui::Terminal<ratatui::backend::TestBackend>| terminal.backend().buffer().content().chunks(160).map(|r| r.iter().map(|c| c.symbol()).collect::<String>() + "\n").collect::<String>();
    let key = |tui: &mut App, code, modifiers| { tui.handle(Event::Key(KeyEvent::new(code, modifiers))); };
    let action = |visual: &mut VisualTestContext, tui: &mut App, terminal: &mut ratatui::Terminal<ratatui::backend::TestBackend>, name: &str| {
        let keys = name.chars().map(|c| if c == ' ' { "space".to_owned() } else { c.to_string() }).collect::<Vec<_>>().join(" ");
        visual.simulate_keystrokes(&format!("ctrl-shift-a {keys}"));
        key(tui, KeyCode::Char('a'), KeyModifiers::CONTROL | KeyModifiers::SHIFT);
        tui.handle(Event::Paste(name.into()));
        wait(visual, tui, terminal, &|state| state.navigation().is_some_and(|n| !n.pending));
        visual.simulate_keystrokes("enter"); key(tui, KeyCode::Enter, KeyModifiers::NONE);
    };
    let details = |visual: &mut VisualTestContext, tui: &App| {
        let shown = gui.read_with(visual, |v, _| v.modal_rows()).into_iter().map(|(row, _)| row).collect::<Vec<_>>();
        let Some(crate::app::Prompt::Details { body, .. }) = tui.modal.as_ref().map(|m| &m.kind) else { panic!("no terminal details: {:?}", tui.modal_title()) };
        (shown, body.clone())
    };
    let environment = |root: &Path| format!("Python environment: {} (venv \"env\" with venvPath \".\" in {}), Python 3.13.5; installed packages from {}", root.join("env").display(), root.join("pyrightconfig.json").display(), root.join("env/lib/python3.13/site-packages").display());
    wait(&mut visual, &mut tui, &mut terminal, &|state| state.provider().is_some_and(|p| p.status == code_core::semantics::Status::Ready));
    action(&mut visual, &mut tui, &mut terminal, "server details");
    let (shown, body) = details(&mut visual, &tui);
    assert!(shown.contains(&environment(&groot)), "{shown:?}");
    assert!(body.contains(&format!("\n\n{}\n\n", environment(&troot))), "{body}");
    visual.simulate_keystrokes("escape"); key(&mut tui, KeyCode::Esc, KeyModifiers::NONE);
    // The environment goes away; a restart leaves Pyright stopped in both, saying why.
    for root in [&groot, &troot] { std::fs::rename(root.join("env"), root.join("moved")).unwrap(); }
    action(&mut visual, &mut tui, &mut terminal, "restart language server");
    let stopped = |state: &State| state.provider().is_some_and(|p| matches!(p.status, code_core::semantics::Status::Failed(_)));
    wait(&mut visual, &mut tui, &mut terminal, &stopped);
    let why = |root: &Path| format!("The configured Python environment {} does not exist (venv \"env\" with venvPath \".\" in {}). Lenz does not start Pyright with another environment: create it (for example `python3 -m venv {}`) or correct the setting, then choose Restart Language Server", root.join("env").display(), root.join("pyrightconfig.json").display(), root.join("env").display());
    assert_eq!(gui.read_with(&visual, |view, _| view.session().state().provider().map(|p| p.status)), Some(code_core::semantics::Status::Failed(why(&groot))));
    assert_eq!(tui.session.state().provider().map(|p| p.status), Some(code_core::semantics::Status::Failed(why(&troot))));
    assert_eq!(gui.read_with(&visual, |view, _| view.session().state().language_server_label()), "LSP stopped");
    assert!(screen(&terminal).contains(" LSP stopped [F2] "), "{}", screen(&terminal));
    action(&mut visual, &mut tui, &mut terminal, "server details");
    let (shown, body) = details(&mut visual, &tui);
    let advice = "The language server was not started: correct the project environment named above, then choose Restart Language Server. Lenz never creates environments or installs packages.";
    assert!(shown.contains(&format!("Pyright: stopped: {}", why(&groot))) && shown.iter().any(|r| r == advice), "{shown:?}");
    assert!(body.starts_with(&format!("Pyright: stopped: {}\n\n", why(&troot))) && body.contains(advice), "{body}");
    assert!(!shown.iter().any(|r| r.starts_with("Setup: ")) && !body.contains("Setup: "), "no reinstall advice");
    visual.simulate_keystrokes("escape"); key(&mut tui, KeyCode::Esc, KeyModifiers::NONE);
    assert!(gui.read_with(&visual, |v, _| v.modal_rows()).is_empty() && tui.modal_title().is_empty());
    // Typing goes on alike.
    gui.update(&mut visual, |view, _| view.enable_draft_input_for_test()); tui.enable_draft_input_for_test();
    visual.simulate_keystrokes("ctrl-home"); key(&mut tui, KeyCode::Home, KeyModifiers::CONTROL);
    visual.simulate_keystrokes("x"); key(&mut tui, KeyCode::Char('x'), KeyModifiers::NONE);
    let current = |state: &State| state.current_source().map(|d| d.copy_range(0..d.len_bytes()).unwrap());
    assert_eq!(current(tui.session.state()).as_deref(), Some("ximport dep\n"));
    assert_eq!(gui.read_with(&visual, |view, _| current(view.session().state())).as_deref(), Some("ximport dep\n"));
    for root in [&groot, &troot] { assert_eq!(std::fs::read_to_string(file(root)).unwrap(), text); assert_eq!(std::fs::read_to_string(root.join("pyrightconfig.json")).unwrap(), pyrightconfig); }
}

/// A virtual environment at `dir` from the installed `python3 -m venv`, with package `name` (`body` its `__init__.py`)
/// installed by the environment's own pip, offline, from a wheel written here.
fn python_venv(dir: &std::path::Path, name: &str, body: &str) {
    let made = std::process::Command::new("python3").args(["-m", "venv"]).arg(dir).output().expect("python3 is installed");
    assert!(made.status.success(), "{}", String::from_utf8_lossy(&made.stderr));
    let wheels = tempfile::tempdir().unwrap();
    let wheel = wheels.path().join(format!("{name}-1.0-py3-none-any.whl"));
    const WHEEL: &str = "import base64, hashlib, sys, zipfile\nname, body, out = sys.argv[1:4]\ninfo = f'{name}-1.0.dist-info'\nfiles = {f'{name}/__init__.py': body, f'{info}/METADATA': f'Metadata-Version: 2.1\\nName: {name}\\nVersion: 1.0\\n', f'{info}/WHEEL': 'Wheel-Version: 1.0\\nGenerator: lenz-test\\nRoot-Is-Purelib: true\\nTag: py3-none-any\\n'}\nrecord = []\nwith zipfile.ZipFile(out, 'w') as z:\n    for path, text in files.items():\n        data = text.encode()\n        z.writestr(path, data)\n        digest = base64.urlsafe_b64encode(hashlib.sha256(data).digest()).rstrip(b'=').decode()\n        record.append(f'{path},sha256={digest},{len(data)}')\n    record.append(f'{info}/RECORD,,')\n    z.writestr(f'{info}/RECORD', '\\n'.join(record) + '\\n')\n";
    let built = std::process::Command::new("python3").args(["-I", "-c", WHEEL, name, body]).arg(&wheel).output().unwrap();
    assert!(built.status.success(), "{}", String::from_utf8_lossy(&built.stderr));
    let pip = std::process::Command::new(dir.join("bin/python")).args(["-m", "pip", "install", "--no-index", "--no-deps", "--disable-pip-version-check", "-q"]).arg(&wheel).output().unwrap();
    assert!(pip.status.success(), "{}", String::from_utf8_lossy(&pip.stderr));
}

/// Real Pyright and ruff behind both adapters, in a project whose `pyrightconfig.json` names its own virtual
/// environment (holding a pip-installed dependency): Server Details through Find Action names that environment and
/// the formatter in both; typing `shapes_dep.a` offers the dependency's `area` in both (drawn in each), Down/Enter
/// accepts it and Ctrl+Z takes it back; Ctrl+Alt+L formats and Ctrl+Alt+O sorts imports with the project's ruff
/// configuration, each one Ctrl+Z. The same draft in both after every step; the files untouched. Needs Pyright
/// (`READER_PYRIGHT`, else on PATH), python3 with venv and pip, and ruff on PATH.
#[gpui::test]
#[ignore = "needs an installed Pyright, python3 with venv and pip, and ruff; run alone"]
fn real_pyright_environment_and_ruff_match_in_both_adapters(cx: &mut TestAppContext) {
    let Some(pyright) = std::env::var_os("READER_PYRIGHT").map(std::path::PathBuf::from)
        .or_else(|| code_core::semantics::provider::find_executable(std::path::Path::new("pyright-langserver"), None)) else {
        println!("SKIPPED: Pyright is not installed");
        return;
    };
    let text = "import sys\nimport os\nimport shapes_dep\n\nr = shapes_dep.area(2.0)\nprint(r, os.sep, sys.argv)\ns  =  \"𝄞\"\n";
    let pyproject = "[project]\nname = \"envfix\"\nversion = \"0\"\n\n[tool.ruff.format]\nquote-style = \"single\"\n";
    let pyrightconfig = "{ \"venvPath\": \".\", \"venv\": \"env\" }\n";
    let dir = tempfile::tempdir().unwrap();
    let make = |name: &str| {
        let root = dir.path().join(name).join("project");
        std::fs::create_dir_all(&root).unwrap();
        for (file, body) in [("pyrightconfig.json", pyrightconfig), ("pyproject.toml", pyproject), ("app.py", text)] { std::fs::write(root.join(file), body).unwrap(); }
        python_venv(&root.join("env"), "shapes_dep", "def area(radius: float) -> float:\n    \"\"\"The area of a circle.\"\"\"\n    return 3.14159 * radius * radius\n");
        let c = config(&dir.path().join(name).join("config"));
        let mut settings: serde_json::Value = serde_json::from_slice(&std::fs::read(&c.providers).unwrap()).unwrap();
        settings["providers"]["pyright"] = serde_json::json!({ "executable": pyright });
        std::fs::write(&c.providers, settings.to_string()).unwrap(); (c, root)
    };
    let ((gc, groot), (tc, troot)) = (make("gui"), make("tui"));
    let file = |root: &std::path::Path| root.join("app.py");
    let mut core = Session::new(gc); core.dispatch(Command::OpenRoot(groot.clone()));
    let gui = cx.new(|cx| ReaderView::new(core, Some(file(&groot)), cx));
    let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| gui.clone())).unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let focus = gui.read_with(&visual, |view, _| view.focus_handle()); visual.update(|window, cx| focus.focus(window, cx));
    let mut tui = App::new(tc, Some(troot.clone())); tui.session.dispatch(Command::Open(file(&troot)));
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(140, 40)).unwrap();
    let wait = |visual: &mut VisualTestContext, tui: &mut App, terminal: &mut ratatui::Terminal<ratatui::backend::TestBackend>, limit: u64, done: &dyn Fn(&State) -> bool| {
        let until = Instant::now() + Duration::from_secs(limit);
        loop {
            gui.update(visual, |view, cx| view.poll(cx)); while let Ok(completion) = tui.session.completions().try_recv() { tui.apply(completion); }
            visual.update(|window, cx| { let _ = window.draw(cx); }); terminal.draw(|frame| tui.draw(frame)).unwrap();
            if gui.read_with(visual, |view, _| done(view.session().state())) && done(tui.session.state()) { break; }
            assert!(Instant::now() < until, "GUI {:?}; TUI {:?} {:?}", gui.read_with(visual, |view, _| view.session().state().notices()), tui.session.state().notices(), tui.modal_title());
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    let screen = |terminal: &ratatui::Terminal<ratatui::backend::TestBackend>| terminal.backend().buffer().content().chunks(140).map(|r| r.iter().map(|c| c.symbol()).collect::<String>() + "\n").collect::<String>();
    let ready = |state: &State| state.provider().is_some_and(|p| p.status == code_core::semantics::Status::Ready && matches!(p.environment, Some(Ok(_))));
    wait(&mut visual, &mut tui, &mut terminal, 120, &ready);
    gui.update(&mut visual, |view, _| view.enable_draft_input_for_test()); tui.enable_draft_input_for_test();
    let key = |tui: &mut App, code, modifiers| { tui.handle(Event::Key(KeyEvent::new(code, modifiers))); };
    let current = |state: &State| state.current_source().map(|d| d.copy_range(0..d.len_bytes()).unwrap());
    let both = |visual: &mut VisualTestContext, tui: &App, expected: &str| {
        assert_eq!(current(tui.session.state()).as_deref(), Some(expected));
        assert_eq!(gui.read_with(visual, |view, _| current(view.session().state())).as_deref(), Some(expected));
    };
    println!("step: server details");
    // Server Details (through Find Action) names the configured environment and the formatter in both.
    visual.simulate_keystrokes("ctrl-shift-a s e r v e r space d e t a i l s");
    key(&mut tui, KeyCode::Char('a'), KeyModifiers::CONTROL | KeyModifiers::SHIFT);
    tui.handle(Event::Paste("server details".into()));
    wait(&mut visual, &mut tui, &mut terminal, 10, &|state| state.navigation().is_some_and(|n| !n.pending));
    visual.simulate_keystrokes("enter"); key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    let shown = gui.read_with(&visual, |v, _| v.modal_rows()).into_iter().map(|(row, _)| row).collect::<Vec<_>>();
    let Some(crate::app::Prompt::Details { body, .. }) = tui.modal.as_ref().map(|m| &m.kind) else { panic!("no terminal details: {:?}", tui.modal_title()) };
    let body = body.clone();
    let environment = |root: &std::path::Path| {
        let env = root.join("env");
        let version = std::fs::read_to_string(env.join("pyvenv.cfg")).unwrap().lines().find_map(|l| l.strip_prefix("version = ").map(str::to_owned)).unwrap();
        let lib = std::fs::read_dir(env.join("lib")).unwrap().next().unwrap().unwrap().path();
        format!("Python environment: {} (venv \"env\" with venvPath \".\" in {}), Python {version}; installed packages from {}", env.display(), root.join("pyrightconfig.json").display(), lib.join("site-packages").display())
    };
    assert!(shown.contains(&environment(&groot)), "{shown:?}");
    assert!(body.contains(&environment(&troot)), "{body}");
    let formatter = |root: &std::path::Path, ruff: &str| format!("Format Document and Organize Imports run the project's formatter, not the language server: ruff (configured by {}); executable {ruff}", root.join("pyproject.toml").display());
    let ruff = code_core::semantics::provider::find_executable(std::path::Path::new("ruff"), None).expect("ruff is on PATH").display().to_string();
    assert!(shown.contains(&formatter(&groot, &ruff)), "{shown:?}");
    assert!(body.contains(&formatter(&troot, &ruff)), "{body}");
    visual.simulate_keystrokes("escape"); key(&mut tui, KeyCode::Esc, KeyModifiers::NONE);
    assert!(gui.read_with(&visual, |v, _| v.modal_rows()).is_empty() && tui.modal_title().is_empty());
    println!("step: completion");
    // Typing `shapes_dep.a` at the end offers the installed dependency's `area` in both.
    let end = code_core::session::Selection { anchor: text.len(), head: text.len() };
    gui.update(&mut visual, |view, cx| view.dispatch_for_test(Command::Select(end), cx)); tui.session.dispatch(Command::Select(end));
    visual.simulate_keystrokes("s h a p e s _ d e p . a"); for c in "shapes_dep.a".chars() { key(&mut tui, KeyCode::Char(c), KeyModifiers::NONE); }
    let typed = format!("{text}shapes_dep.a");
    let rows = |state: &State| state.completion().map(|c| c.rows().map(|r| r.label.clone()).collect::<Vec<_>>());
    wait(&mut visual, &mut tui, &mut terminal, 60, &|state| current(state).as_deref() == Some(typed.as_str()) && rows(state).is_some_and(|r| r.iter().any(|l| l == "area")));
    let labels = rows(tui.session.state()).unwrap();
    println!("suggestions: {labels:?}");
    assert_eq!(gui.read_with(&visual, |view, _| rows(view.session().state())), Some(labels.clone()), "the same suggestions in both");
    assert!(gui.read_with(&visual, |view, _| view.completion_bounds().is_some()), "the GUI draws its suggestions");
    assert!(screen(&terminal).contains(" area"), "{}", screen(&terminal));
    let row = labels.iter().position(|l| l == "area").unwrap();
    for _ in 0..row { visual.simulate_keystrokes("down"); key(&mut tui, KeyCode::Down, KeyModifiers::NONE); }
    visual.simulate_keystrokes("enter"); key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    let accepted = format!("{text}shapes_dep.area");
    wait(&mut visual, &mut tui, &mut terminal, 30, &|state| current(state).as_deref() == Some(accepted.as_str()));
    visual.simulate_keystrokes("ctrl-z"); key(&mut tui, KeyCode::Char('z'), KeyModifiers::CONTROL);
    both(&mut visual, &tui, &typed);
    visual.simulate_keystrokes("ctrl-z"); key(&mut tui, KeyCode::Char('z'), KeyModifiers::CONTROL);
    both(&mut visual, &tui, text);
    println!("step: formatting");
    visual.simulate_keystrokes("ctrl-alt-l"); key(&mut tui, KeyCode::Char('l'), KeyModifiers::CONTROL | KeyModifiers::ALT);
    let formatted = text.replace("s  =  \"𝄞\"", "s = '𝄞'");
    wait(&mut visual, &mut tui, &mut terminal, 30, &|state| current(state).as_deref() == Some(formatted.as_str()));
    assert!(screen(&terminal).contains("s = '𝄞'"), "{}", screen(&terminal));
    visual.simulate_keystrokes("ctrl-z"); key(&mut tui, KeyCode::Char('z'), KeyModifiers::CONTROL);
    both(&mut visual, &tui, text);
    visual.simulate_keystrokes("ctrl-alt-o"); key(&mut tui, KeyCode::Char('o'), KeyModifiers::CONTROL | KeyModifiers::ALT);
    let organized = text.replacen("import sys\nimport os\nimport shapes_dep\n", "import os\nimport sys\n\nimport shapes_dep\n", 1);
    wait(&mut visual, &mut tui, &mut terminal, 30, &|state| current(state).as_deref() == Some(organized.as_str()));
    visual.simulate_keystrokes("ctrl-z"); key(&mut tui, KeyCode::Char('z'), KeyModifiers::CONTROL);
    both(&mut visual, &tui, text);
    for root in [&groot, &troot] { assert_eq!(std::fs::read_to_string(file(root)).unwrap(), text); assert!(!root.join(".ruff_cache").exists()); }
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
        "Format on save: on for this project (Save asks the language server to format first)".to_owned(),
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
fn gui_and_terminal_choose_and_show_the_sql_dialect_through_find_action_and_its_status_label(cx: &mut TestAppContext) {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path().join("project");
    std::fs::create_dir(&root).unwrap();
    let path = root.join("q.sql");
    std::fs::write(&path, "select 1;\n").unwrap();
    let (gc, tc) = (config(&scratch.path().join("gui")), config(&scratch.path().join("tui")));
    let (gs, ts) = (gc.settings.clone(), tc.settings.clone());
    let mut core = Session::new(gc);
    core.dispatch(Command::OpenRoot(root.clone()));
    let gui = cx.new(|cx| ReaderView::new(core, Some(path.clone()), cx));
    let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| gui.clone())).unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let focus = gui.read_with(&visual, |v, _| v.focus_handle());
    visual.update(|w, cx| focus.focus(w, cx));
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
        visual.update(|w, cx| { let _ = w.draw(cx); });
        visual.simulate_keystrokes("enter");
        key(tui, KeyCode::Enter, KeyModifiers::NONE);
        settle(visual, tui);
    };
    let on = |s: &Session, id| s.actions().iter().find(|a| a.id == id).and_then(|a| a.on);
    let screen = |tui: &mut App| {
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(200, 40)).unwrap();
        terminal.draw(|f| tui.draw(f)).unwrap();
        terminal.backend().buffer().content().chunks(200).map(|r| r.iter().map(|c| c.symbol()).collect::<String>() + "\n").collect::<String>()
    };
    // Fit the complete dialect and metadata labels before the narrow-layout checks below.
    visual.simulate_resize(gpui::size(gpui::px(1600.0), gpui::px(400.0)));
    visual.update(|w, cx| { let _ = w.draw(cx); });
    let sql_text = |visual: &mut VisualTestContext| gui.read_with(visual, |v, _| v.status_sql().1);
    settle(&mut visual, &mut tui);
    // Neither adapter assumes PostgreSQL.
    assert_eq!(sql_text(&mut visual), "SQL: not set · metadata off");
    assert!(screen(&mut tui).contains("SQL: not set"));

    find_action(&mut visual, &mut tui, "file: duckdb");
    assert_eq!(gui.read_with(&visual, |v, _| on(v.session(), CommandId::SqlDialectDuckDb)), Some(true));
    assert_eq!(on(&tui.session, CommandId::SqlDialectDuckDb), Some(true), "the terminal chose the same dialect");
    assert_eq!(sql_text(&mut visual), "SQL: DuckDB (this file) · metadata off");
    assert!(screen(&mut tui).contains("SQL: DuckDB (this file)"));

    // The project default applies again once the file follows it, in both adapters.
    find_action(&mut visual, &mut tui, "project sql dialect: postgresql");
    find_action(&mut visual, &mut tui, "file: follow project default");
    for id in [CommandId::ProjectSqlDialectPostgreSql, CommandId::SqlDialectFollowProject] {
        assert_eq!(gui.read_with(&visual, |v, _| on(v.session(), id)), Some(true), "GUI {id:?}");
        assert_eq!(on(&tui.session, id), Some(true), "terminal {id:?}");
    }
    assert_eq!(sql_text(&mut visual), "SQL: PostgreSQL (project) · metadata off");
    assert!(screen(&mut tui).contains("SQL: PostgreSQL (project)"));

    // Each status label is a control: clicking it shows the same details in both adapters.
    let cell = gui.read_with(&visual, |v, _| v.status_sql().0);
    visual.simulate_click(gpui::point(cell.origin.x + gpui::px(4.0), cell.origin.y + cell.size.height / 2.0), gpui::Modifiers::default());
    let shown: Vec<String> = gui.read_with(&visual, |v, _| v.modal_rows()).into_iter().map(|(row, _)| row).collect();
    assert_eq!(shown[0], "SQL dialect");
    assert!(shown.iter().any(|r| r == "Dialect: PostgreSQL (project default)"), "{shown:?}");
    assert!(shown.iter().any(|r| r == "Project default: PostgreSQL"), "{shown:?}");
    assert!(shown.iter().any(|r| r.starts_with("Assistance: PostgreSQL offline assistance — query-local completion")), "{shown:?}");
    visual.simulate_keystrokes("m");
    assert!(gui.read_with(&visual, |v, _| matches!(v.modal, Some(code_gui::Modal::CommandGroup { group: code_core::actions::CommandGroup::Sql, .. }))));
    visual.simulate_keystrokes("escape escape");

    let control = tui.layout.controls.iter().find(|(id, _)| *id == CommandId::SqlDialect).map(|(_, rect)| *rect).expect("the terminal draws the SQL status control");
    tui.handle(Event::Mouse(crossterm::event::MouseEvent { kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left), column: control.x + 1, row: control.y, modifiers: KeyModifiers::NONE }));
    assert_eq!(tui.modal_title(), "SQL dialect");
    let Some(crate::app::Prompt::Details { body, .. }) = tui.modal.as_ref().map(|m| &m.kind) else { panic!("no terminal details") };
    assert_eq!(body.lines().next(), Some(format!("SQL dialect for {}", path.display()).as_str()));
    assert!(body.contains("Dialect: PostgreSQL (project default)\n"), "{body}");
    let body = body.clone();
    let gui_body = gui.read_with(&visual, |v, _| v.session().sql_dialect_details());
    // Each adapter names its own settings file; every other row is the same.
    assert_eq!(gui_body.replace(&gs.display().to_string(), "<settings>"), body.replace(&ts.display().to_string(), "<settings>"), "both adapters show the same details");
    key(&mut tui, KeyCode::Char('m'), KeyModifiers::NONE);
    assert!(matches!(tui.modal.as_ref().map(|m| &m.kind), Some(crate::app::Prompt::Menu(Some(code_core::actions::CommandGroup::Sql)))));
    key(&mut tui, KeyCode::Esc, KeyModifiers::NONE);
    key(&mut tui, KeyCode::Esc, KeyModifiers::NONE);
    tui.run(CommandId::SqlDialect);
    // Tiny layouts and dismissal leave the documents and settings as they were.
    for (w, h) in [(1u16, 1u16), (26, 8)] { ratatui::Terminal::new(ratatui::backend::TestBackend::new(w, h)).unwrap().draw(|f| tui.draw(f)).unwrap(); }
    for (w, h) in [(1., 1.), (180., 90.), (800., 400.)] { visual.simulate_resize(gpui::size(gpui::px(w), gpui::px(h))); visual.update(|w, cx| { let _ = w.draw(cx); }); }
    key(&mut tui, KeyCode::Esc, KeyModifiers::NONE);
    assert!(tui.modal_title().is_empty() && gui.read_with(&visual, |v, _| v.modal_rows()).is_empty());
    assert_eq!(gui.read_with(&visual, |v, _| snapshot(v.session().state())), snapshot(tui.session.state()));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "select 1;\n");
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1, "nothing was written into the project");
    let deadline = Instant::now() + Duration::from_secs(5);
    let saved = |p: &std::path::PathBuf| std::fs::read(p).ok().and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok()).map(|v| v["projects"].clone());
    while saved(&gs) != Some(serde_json::json!({ root.to_str().unwrap(): { "format_on_save": false, "sql_dialect": "PostgreSql" } })) || saved(&gs) != saved(&ts) {
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
    use code_core::session::Command;
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

#[gpui::test]
fn keymap_presets_overrides_and_unsupported_delivery_match_across_adapters(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("keys.txt");
    let original = "one\ntwo\nthree\n";
    std::fs::write(&path, original).unwrap();
    let gui = cx.new(|cx| ReaderView::new(Session::new(config(&dir.path().join("gui"))), Some(path.clone()), cx));
    let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| gui.clone())).unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let focus = gui.read_with(&visual, |v, _| v.focus_handle());
    visual.update(|w, cx| focus.focus(w, cx));
    let mut tui = App::new(config(&dir.path().join("tui")), Some(path.clone()));
    let deadline = Instant::now() + Duration::from_secs(5);
    while gui.read_with(&visual, |v, _| v.session().state().current_source().is_none()) || tui.session.state().current_source().is_none() {
        gui.update(&mut visual, |v, cx| v.poll(cx));
        while let Ok(done) = tui.session.completions().try_recv() { tui.apply(done); }
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 24)).unwrap();
    let key = |tui: &mut App, code, modifiers| { tui.handle(Event::Key(KeyEvent::new(code, modifiers))); };
    let draw = |visual: &mut VisualTestContext, tui: &mut App, terminal: &mut ratatui::Terminal<ratatui::backend::TestBackend>| {
        visual.update(|w, cx| { let _ = w.draw(cx); });
        terminal.draw(|f| tui.draw(f)).unwrap();
    };
    let same = |visual: &VisualTestContext, tui: &App, why: &str| {
        let state = snapshot(tui.session.state());
        assert_eq!(state, gui.read_with(visual, |v, _| snapshot(v.session().state())), "{why}");
    };
    draw(&mut visual, &mut tui, &mut terminal);
    // The same palette route: F10, the "Keymap: VS Code" row, Enter.
    visual.simulate_keystrokes("f10");
    let at = gui.read_with(&visual, |v, _| v.modal_rows().iter().position(|r| r.0.starts_with(CommandId::KeymapVsCode.label())).unwrap());
    for _ in 0..at { visual.simulate_keystrokes("down"); }
    visual.simulate_keystrokes("enter");
    key(&mut tui, KeyCode::F(10), KeyModifiers::NONE);
    let at = tui.choices().iter().position(|c| matches!(c.target, crate::app::ChoiceTarget::Action(CommandId::KeymapVsCode))).unwrap();
    for _ in 0..at { key(&mut tui, KeyCode::Down, KeyModifiers::NONE); }
    key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    draw(&mut visual, &mut tui, &mut terminal);
    for preset in [gui.read_with(&visual, |v, _| v.session().keymap().preset()), tui.session.keymap().preset()] {
        assert_eq!(preset, code_core::keymap::KeymapPreset::VsCode);
    }
    assert_eq!(gui.read_with(&visual, |v, _| v.key_hint(CommandId::FindAction)), tui.session.key_hint(CommandId::FindAction), "menus share the resolved chord");
    // Ctrl+W closes the tab in both; neither writes the file.
    visual.simulate_keystrokes("ctrl-w");
    key(&mut tui, KeyCode::Char('w'), KeyModifiers::CONTROL);
    draw(&mut visual, &mut tui, &mut terminal);
    same(&visual, &tui, "Ctrl+W closes the tab under the VS Code keymap in both");
    assert!(tui.session.state().tabs().is_empty());
    assert_eq!(std::fs::read(&path).unwrap(), original.as_bytes());
    // Reopen the same file through each adapter's Ctrl+E recent files, then rebind Save with the same key.
    visual.simulate_keystrokes("ctrl-e");
    key(&mut tui, KeyCode::Char('e'), KeyModifiers::CONTROL);
    let rows_ready = |s: &State| s.navigation().is_some_and(|n| !n.pending && !n.rows.is_empty());
    let deadline = Instant::now() + Duration::from_secs(5);
    while !rows_ready(tui.session.state()) || !gui.read_with(&visual, |v, _| rows_ready(v.session().state())) {
        gui.update(&mut visual, |v, cx| v.poll(cx));
        while let Ok(done) = tui.session.completions().try_recv() { tui.apply(done); }
        assert!(Instant::now() < deadline, "recent files did not arrive");
        std::thread::yield_now();
    }
    visual.simulate_keystrokes("enter");
    key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    let deadline = Instant::now() + Duration::from_secs(5);
    while gui.read_with(&visual, |v, _| v.session().state().current_source().is_none()) || tui.session.state().current_source().is_none() {
        gui.update(&mut visual, |v, cx| v.poll(cx));
        while let Ok(done) = tui.session.completions().try_recv() { tui.apply(done); }
        assert!(Instant::now() < deadline, "the recent file did not reopen");
        std::thread::yield_now();
    }
    visual.simulate_keystrokes("f10"); let at = gui.read_with(&visual, |v, _| v.modal_rows().iter().position(|r| r.0.starts_with(CommandId::Keymap.label())).unwrap()); for _ in 0..at { visual.simulate_keystrokes("down"); } visual.simulate_keystrokes("enter");
    key(&mut tui, KeyCode::F(10), KeyModifiers::NONE); let at = tui.choices().iter().position(|c| matches!(c.target, crate::app::ChoiceTarget::Action(CommandId::Keymap))).unwrap(); for _ in 0..at { key(&mut tui, KeyCode::Down, KeyModifiers::NONE); } key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    let save = gui.read_with(&visual, |v, _| v.session().keymap().rows().iter().position(|r| r.command == CommandId::Save).unwrap());
    for _ in 0..save { visual.simulate_keystrokes("down"); key(&mut tui, KeyCode::Down, KeyModifiers::NONE); }
    visual.simulate_keystrokes("enter"); key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    visual.simulate_keystrokes("ctrl-alt-s"); key(&mut tui, KeyCode::Char('s'), KeyModifiers::CONTROL | KeyModifiers::ALT);
    draw(&mut visual, &mut tui, &mut terminal);
    assert_eq!(gui.read_with(&visual, |v, _| v.key_hint(CommandId::Save)), Some("Ctrl+Alt+S".to_string()));
    assert_eq!(tui.session.key_hint(CommandId::Save), Some("Ctrl+Alt+S".to_string()));
    visual.simulate_keystrokes("escape"); key(&mut tui, KeyCode::Esc, KeyModifiers::NONE);
    // A disabled command says why in both adapters instead of vanishing.
    visual.simulate_keystrokes("alt-h");
    key(&mut tui, KeyCode::Char('h'), KeyModifiers::ALT);
    assert!(gui.read_with(&visual, |v, _| v.session().state().notices().iter().any(|n| n == "No language server for this language")), "{:?}", gui.read_with(&visual, |v, _| v.session().state().notices()));
    assert!(tui.session.state().notices().iter().any(|n| n == "No language server for this language"));
    assert_eq!(std::fs::read(&path).unwrap(), original.as_bytes());
}

#[gpui::test]
fn find_and_replace_replay_the_same_text_results_and_undo_in_both_adapters(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.txt");
    let original = "Foo foo\nfood foo\n";
    std::fs::write(&path, original).unwrap();
    let gui = cx.new(|cx| ReaderView::new(Session::new(config(&dir.path().join("gui"))), Some(path.clone()), cx));
    let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| gui.clone())).unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let focus = gui.read_with(&visual, |view, _| view.focus_handle());
    visual.update(|window, cx| focus.focus(window, cx));
    let mut tui = App::new(config(&dir.path().join("tui")), Some(path.clone()));
    gui.update(&mut visual, |view, _| view.enable_draft_input_for_test());
    tui.enable_draft_input_for_test();
    let key = |tui: &mut App, code, modifiers| { tui.handle(Event::Key(KeyEvent::new(code, modifiers))); };
    let ready = |state: &State| state.loading().is_none() && state.current_source().is_some() && state.active().and_then(|t| t.find()).is_none_or(|f| !f.pending());
    let settle = |visual: &mut VisualTestContext, tui: &mut App| {
        let until = Instant::now() + Duration::from_secs(10);
        loop {
            gui.update(visual, |view, cx| view.poll(cx));
            while let Ok(done) = tui.session.completions().try_recv() { tui.apply(done); }
            if gui.read_with(visual, |view, _| ready(view.session().state())) && ready(tui.session.state()) { break; }
            assert!(Instant::now() < until, "adapters did not settle");
            std::thread::sleep(Duration::from_millis(2));
        }
    };
    let text = |state: &State| { let source = state.current_source().unwrap(); source.copy_range(0..source.len_bytes()).unwrap() };
    let check = |visual: &VisualTestContext, tui: &App, expected: &str| {
        assert_eq!(text(tui.session.state()), expected);
        assert_eq!(snapshot(tui.session.state()), gui.read_with(visual, |view, _| snapshot(view.session().state())));
        assert_eq!(tui.session.replace_text(), gui.read_with(visual, |view, _| view.session().replace_text().to_owned()));
    };
    settle(&mut visual, &mut tui);
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
    let draw = |visual: &mut VisualTestContext, tui: &mut App, terminal: &mut ratatui::Terminal<ratatui::backend::TestBackend>| {
        visual.update(|window, cx| { let _ = window.draw(cx); });
        terminal.draw(|frame| tui.draw(frame)).unwrap();
    };
    draw(&mut visual, &mut tui, &mut terminal);
    // Ctrl+F and the query in both adapters, then Tab to the replacement field and the replacement text.
    visual.simulate_keystrokes("ctrl-f");
    key(&mut tui, KeyCode::Char('f'), KeyModifiers::CONTROL);
    for c in "foo".chars() { visual.simulate_keystrokes(&c.to_string()); key(&mut tui, KeyCode::Char(c), KeyModifiers::NONE); }
    settle(&mut visual, &mut tui);
    check(&visual, &tui, original);
    // Alt+Enter in the find box lists every match in both (not Quick Fix, which the keymap binds to it elsewhere);
    // the box keeps its keys.
    visual.simulate_keystrokes("alt-enter");
    key(&mut tui, KeyCode::Enter, KeyModifiers::ALT);
    let listed = |state: &State| state.results().filter(|r| r.done()).map(|r| (r.query().to_owned(), r.rows().len()));
    let until = Instant::now() + Duration::from_secs(10);
    while listed(tui.session.state()).is_none() || gui.read_with(&visual, |view, _| listed(view.session().state())).is_none() {
        assert!(Instant::now() < until, "the matches were not listed");
        gui.update(&mut visual, |view, cx| view.poll(cx));
        while let Ok(done) = tui.session.completions().try_recv() { tui.apply(done); }
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(listed(tui.session.state()), gui.read_with(&visual, |view, _| listed(view.session().state())));
    assert_eq!(listed(tui.session.state()).map(|(query, _)| query), Some("foo".to_owned()));
    assert!(tui.session.state().active().unwrap().find().is_some() && gui.read_with(&visual, |view, _| view.session().state().active().unwrap().find().is_some()));
    check(&visual, &tui, original);
    visual.simulate_keystrokes("tab");
    key(&mut tui, KeyCode::Tab, KeyModifiers::NONE);
    for c in "bar".chars() { visual.simulate_keystrokes(&c.to_string()); key(&mut tui, KeyCode::Char(c), KeyModifiers::NONE); }
    check(&visual, &tui, original);
    assert_eq!(tui.session.replace_text(), "bar", "typing in the replacement field never edits the text");
    // Enter replaces the current match in both; the next match is selected and the query is unchanged.
    visual.simulate_keystrokes("enter");
    key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    settle(&mut visual, &mut tui);
    check(&visual, &tui, "bar foo\nfood foo\n");
    assert_eq!(tui.session.state().active().unwrap().find().unwrap().current(), Some(0));
    // Alt+A replaces every remaining match, and both report the same state afterwards.
    visual.simulate_keystrokes("alt-a");
    key(&mut tui, KeyCode::Char('a'), KeyModifiers::ALT);
    settle(&mut visual, &mut tui);
    check(&visual, &tui, "bar bar\nbard bar\n");
    assert_eq!(tui.session.disabled(CommandId::ReplaceAll), Some("No matches to replace"));
    assert_eq!(gui.read_with(&visual, |view, _| view.session().disabled(CommandId::ReplaceAll)), Some("No matches to replace"));
    draw(&mut visual, &mut tui, &mut terminal);
    // Escape closes the box and the prompt through the shared close command.
    visual.simulate_keystrokes("escape");
    key(&mut tui, KeyCode::Esc, KeyModifiers::NONE);
    settle(&mut visual, &mut tui);
    assert!(tui.session.state().active().unwrap().find().is_none() && tui.modal.is_none());
    check(&visual, &tui, "bar bar\nbard bar\n");
    // Replace All is one undo step in both, and the file is untouched.
    visual.simulate_keystrokes("ctrl-z");
    key(&mut tui, KeyCode::Char('z'), KeyModifiers::CONTROL);
    settle(&mut visual, &mut tui);
    check(&visual, &tui, "bar foo\nfood foo\n");
    assert_eq!(std::fs::read(&path).unwrap(), original.as_bytes());
}

#[gpui::test]
fn explorer_creation_of_a_file_and_a_folder_matches_in_both_adapters(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let roots = [dir.path().join("gui"), dir.path().join("tui")];
    for root in &roots {
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/a.txt"), b"keep\n").unwrap();
    }
    let gui = cx.new(|cx| ReaderView::new(Session::new(config(&dir.path().join("gui-state"))), None, cx));
    let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| gui.clone())).unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let focus = gui.read_with(&visual, |v, _| v.focus_handle());
    visual.update(|w, cx| focus.focus(w, cx));
    gui.update(&mut visual, |v, cx| v.open_root(roots[0].clone(), cx));
    let mut tui = App::new(config(&dir.path().join("tui-state")), Some(roots[1].clone()));
    gui.update(&mut visual, |v, _| v.enable_draft_input_for_test());
    tui.enable_draft_input_for_test();
    let key = |tui: &mut App, code, modifiers| { tui.handle(Event::Key(KeyEvent::new(code, modifiers))); };
    let settle_until = |visual: &mut VisualTestContext, tui: &mut App, done: &dyn Fn(&State) -> bool| {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !gui.read_with(visual, |v, _| done(v.session().state())) || !done(tui.session.state()) {
            gui.update(visual, |v, cx| v.poll(cx));
            while let Ok(c) = tui.session.completions().try_recv() { tui.apply(c); }
            assert!(Instant::now() < deadline, "timed out");
            std::thread::yield_now();
        }
    };
    let scanned = |s: &State| s.project().is_some_and(|p| matches!(p.coverage(), code_core::project::Coverage::Complete(_)));
    settle_until(&mut visual, &mut tui, &scanned);
    // Both adapters act on the explorer's highlighted row, which is `src` in each (folders lead, as in both).
    let highlight_src = |_visual: &mut VisualTestContext, _tui: &mut App| {};
    let both_selected = |visual: &VisualTestContext, tui: &App| {
        let gui_row = gui.read_with(visual, |v, _| v.tree_sel.as_deref().map(str::to_string));
        let tui_row = tui.session.state().tree_rows().nth(tui.tree_at).map(|r| r.path.to_string());
        assert_eq!(gui_row, tui_row, "the same explorer row is selected");
        gui_row
    };
    // A new file, named through each adapter's own command path and dialog.
    highlight_src(&mut visual, &mut tui);
    gui.update_in(&mut visual, |v, w, cx| v.run(CommandId::NewExplorerFile, w, cx));
    tui.run(CommandId::NewExplorerFile);
    assert!(matches!(tui.modal.as_ref().map(|m| &m.kind), Some(crate::app::Prompt::Create { dir, .. }) if dir == "src"));
    for c in "new.txt".chars() {
        visual.simulate_keystrokes(&c.to_string());
        key(&mut tui, KeyCode::Char(c), KeyModifiers::NONE);
    }
    visual.simulate_keystrokes("enter");
    key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    let opened = |s: &State| s.current_source().is_some() && s.active().is_some_and(|t| t.title() == "new.txt");
    settle_until(&mut visual, &mut tui, &opened);
    assert_eq!(snapshot(tui.session.state()), gui.read_with(&visual, |v, _| snapshot(v.session().state())));
    assert_eq!(both_selected(&visual, &tui).as_deref(), Some("src/new.txt"));
    assert!(tui.modal.is_none() && gui.read_with(&visual, |v, _| v.modal.is_none()));
    for root in &roots { assert_eq!(std::fs::read(root.join("src/new.txt")).unwrap(), b"", "an empty file in either project"); }
    // The created file is an ordinary document in both: typing reaches its draft, not the disk.
    visual.simulate_keystrokes("a");
    key(&mut tui, KeyCode::Char('a'), KeyModifiers::NONE);
    let typed = |s: &State| s.current_source().is_some_and(|d| d.copy_range(0..d.len_bytes()).unwrap() == "a");
    settle_until(&mut visual, &mut tui, &typed);
    assert_eq!(snapshot(tui.session.state()), gui.read_with(&visual, |v, _| snapshot(v.session().state())));
    for root in &roots { assert_eq!(std::fs::read(root.join("src/new.txt")).unwrap(), b""); }
    // A new folder from the same command: both select it and keep the document.
    highlight_src(&mut visual, &mut tui);
    gui.update_in(&mut visual, |v, w, cx| v.run(CommandId::NewExplorerFolder, w, cx));
    tui.run(CommandId::NewExplorerFolder);
    for c in "docs".chars() {
        visual.simulate_keystrokes(&c.to_string());
        key(&mut tui, KeyCode::Char(c), KeyModifiers::NONE);
    }
    visual.simulate_keystrokes("enter");
    key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    settle_until(&mut visual, &mut tui, &|s: &State| s.tree_rows().any(|r| &*r.path == "src/docs"));
    for root in &roots { assert!(root.join("src/docs").is_dir()); }
    assert_eq!(both_selected(&visual, &tui).as_deref(), Some("src/docs"));
    assert_eq!(snapshot(tui.session.state()), gui.read_with(&visual, |v, _| snapshot(v.session().state())));
    assert_eq!(tui.session.state().active().unwrap().title(), "new.txt");
    // Files the tests did not create are untouched.
    for root in &roots { assert_eq!(std::fs::read(root.join("src/a.txt")).unwrap(), b"keep\n"); }
}

#[gpui::test]
fn explorer_rename_and_move_carry_the_draft_identically_in_both_adapters(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let roots = [dir.path().join("gui"), dir.path().join("tui")];
    for root in &roots {
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("lib")).unwrap();
        std::fs::write(root.join("src/a.txt"), b"saved\n").unwrap();
        std::fs::write(root.join("src/b.txt"), b"other\n").unwrap();
        std::fs::write(root.join("lib/keep.txt"), b"keep\n").unwrap();
    }
    let gui = cx.new(|cx| ReaderView::new(Session::new(config(&dir.path().join("gui-state"))), None, cx));
    let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| gui.clone())).unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let focus = gui.read_with(&visual, |v, _| v.focus_handle());
    visual.update(|w, cx| focus.focus(w, cx));
    gui.update(&mut visual, |v, cx| v.open_root(roots[0].clone(), cx));
    let mut tui = App::new(config(&dir.path().join("tui-state")), Some(roots[1].clone()));
    gui.update(&mut visual, |v, _| v.enable_draft_input_for_test());
    tui.enable_draft_input_for_test();
    let key = |tui: &mut App, code, modifiers| { tui.handle(Event::Key(KeyEvent::new(code, modifiers))); };
    let settle_until = |visual: &mut VisualTestContext, tui: &mut App, done: &dyn Fn(&State) -> bool| {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !gui.read_with(visual, |v, _| done(v.session().state())) || !done(tui.session.state()) {
            gui.update(visual, |v, cx| v.poll(cx));
            while let Ok(c) = tui.session.completions().try_recv() { tui.apply(c); }
            assert!(Instant::now() < deadline, "timed out");
            std::thread::yield_now();
        }
    };
    let scanned = |s: &State| s.project().is_some_and(|p| matches!(p.coverage(), code_core::project::Coverage::Complete(_)));
    settle_until(&mut visual, &mut tui, &scanned);
    // The same document in each: a.txt, with a typed draft that is never written by a move.
    for (i, root) in roots.iter().enumerate() {
        let open = Command::OpenRetained(root.join("src/a.txt"));
        if i == 0 { gui.update(&mut visual, |v, cx| v.dispatch_for_test(open, cx)); } else { tui.dispatch(open); }
    }
    settle_until(&mut visual, &mut tui, &|s: &State| s.current_source().is_some_and(|d| d.source().ends_with("src/a.txt")));
    visual.simulate_keystrokes("x");
    key(&mut tui, KeyCode::Char('x'), KeyModifiers::NONE);
    settle_until(&mut visual, &mut tui, &|s: &State| s.current_source().is_some_and(|d| d.copy_range(0..d.len_bytes()).unwrap() == "xsaved\n"));
    // Each adapter's own highlighted row, which its command hands to the core.
    let highlight = |visual: &mut VisualTestContext, tui: &mut App, row: &str| {
        gui.update(visual, |v, cx| { v.dispatch_for_test(Command::RevealInTree(row.into()), cx); v.tree_sel = Some(row.into()); });
        tui.dispatch(Command::RevealInTree(row.into()));
        tui.tree_at = tui.session.state().tree_rows().position(|r| &*r.path == row).expect("the row is listed");
    };
    let type_both = |visual: &mut VisualTestContext, tui: &mut App, erase: usize, text: &str| {
        for _ in 0..erase {
            visual.simulate_keystrokes("backspace");
            key(tui, KeyCode::Backspace, KeyModifiers::NONE);
        }
        for c in text.chars() {
            visual.simulate_keystrokes(&c.to_string());
            key(tui, KeyCode::Char(c), KeyModifiers::NONE);
        }
    };
    // Rename: each dialog starts with the current name; a taken name is refused in the dialog itself.
    highlight(&mut visual, &mut tui, "src/a.txt");
    gui.update_in(&mut visual, |v, w, cx| v.run(CommandId::RenameExplorerEntry, w, cx));
    tui.run(CommandId::RenameExplorerEntry);
    assert_eq!(tui.modal.as_ref().map(|m| m.input.as_str()), Some("a.txt"));
    assert!(gui.read_with(&visual, |v, _| matches!(&v.modal, Some(code_gui::Modal::Relocate { input, .. }) if input == "a.txt")));
    type_both(&mut visual, &mut tui, 5, "b.txt");
    visual.simulate_keystrokes("enter");
    key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(tui.modal.as_ref().and_then(|m| m.error.as_deref()), Some("src/b.txt already exists"));
    assert!(gui.read_with(&visual, |v, _| matches!(&v.modal, Some(code_gui::Modal::Relocate { error: Some(e), .. }) if e == "src/b.txt already exists")));
    type_both(&mut visual, &mut tui, 5, "c.md");
    visual.simulate_keystrokes("enter");
    key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    let renamed = |s: &State| s.active().is_some_and(|t| t.title() == "c.md");
    settle_until(&mut visual, &mut tui, &renamed);
    assert!(tui.modal.is_none() && gui.read_with(&visual, |v, _| v.modal.is_none()));
    assert_eq!(snapshot(tui.session.state()), gui.read_with(&visual, |v, _| snapshot(v.session().state())));
    for root in &roots {
        assert!(!root.join("src/a.txt").exists());
        assert_eq!(std::fs::read(root.join("src/c.md")).unwrap(), b"saved\n", "a move never writes the draft");
        assert_eq!(std::fs::read(root.join("src/b.txt")).unwrap(), b"other\n");
    }
    // Move: the dialog starts with the current folder; the draft follows into lib.
    highlight(&mut visual, &mut tui, "src/c.md");
    gui.update_in(&mut visual, |v, w, cx| v.run(CommandId::MoveExplorerEntry, w, cx));
    tui.run(CommandId::MoveExplorerEntry);
    assert_eq!(tui.modal.as_ref().map(|m| m.input.as_str()), Some("src"));
    type_both(&mut visual, &mut tui, 3, "lib");
    visual.simulate_keystrokes("enter");
    key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    settle_until(&mut visual, &mut tui, &|s: &State| s.active().is_some_and(|t| t.source().ends_with("lib/c.md")));
    assert_eq!(snapshot(tui.session.state()), gui.read_with(&visual, |v, _| snapshot(v.session().state())));
    assert!(tui.session.state().tab_dirty(tui.session.state().active().unwrap().id()));
    // A whole folder: its open file follows too, and Escape in a dialog moves nothing.
    highlight(&mut visual, &mut tui, "lib");
    gui.update_in(&mut visual, |v, w, cx| v.run(CommandId::RenameExplorerEntry, w, cx));
    tui.run(CommandId::RenameExplorerEntry);
    visual.simulate_keystrokes("escape");
    key(&mut tui, KeyCode::Esc, KeyModifiers::NONE);
    for root in &roots { assert!(root.join("lib/c.md").exists()); }
    gui.update_in(&mut visual, |v, w, cx| v.run(CommandId::RenameExplorerEntry, w, cx));
    tui.run(CommandId::RenameExplorerEntry);
    type_both(&mut visual, &mut tui, 3, "docs");
    visual.simulate_keystrokes("enter");
    key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    settle_until(&mut visual, &mut tui, &|s: &State| s.active().is_some_and(|t| t.source().ends_with("docs/c.md")));
    assert_eq!(snapshot(tui.session.state()), gui.read_with(&visual, |v, _| snapshot(v.session().state())));
    for root in &roots {
        assert_eq!(std::fs::read(root.join("docs/c.md")).unwrap(), b"saved\n");
        assert_eq!(std::fs::read(root.join("docs/keep.txt")).unwrap(), b"keep\n");
        assert!(!root.join("lib").exists());
    }
    // Save writes the moved document where it is now, in both.
    visual.simulate_keystrokes("ctrl-s");
    key(&mut tui, KeyCode::Char('s'), KeyModifiers::CONTROL);
    settle_until(&mut visual, &mut tui, &|s: &State| s.active().is_some_and(|t| !s.tab_dirty(t.id())));
    for root in &roots { assert_eq!(std::fs::read(root.join("docs/c.md")).unwrap(), b"xsaved\n"); }
}

#[gpui::test]
fn explorer_delete_refuses_drafts_trashes_restores_and_deletes_permanently_alike(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let roots = [dir.path().join("gui"), dir.path().join("tui")];
    let trashes = [dir.path().join("gui-trash"), dir.path().join("tui-trash")];
    for root in &roots {
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("lib")).unwrap();
        std::fs::write(root.join("src/a.txt"), b"saved\n").unwrap();
        std::fs::write(root.join("lib/keep.txt"), b"keep\n").unwrap();
    }
    let gui = cx.new(|cx| ReaderView::new(Session::new(config(&dir.path().join("gui-state"))), None, cx));
    let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| gui.clone())).unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let focus = gui.read_with(&visual, |v, _| v.focus_handle());
    visual.update(|w, cx| focus.focus(w, cx));
    gui.update(&mut visual, |v, cx| { v.set_trash_home_for_test(trashes[0].clone()); v.open_root(roots[0].clone(), cx); });
    let mut tui = App::new(config(&dir.path().join("tui-state")), Some(roots[1].clone()));
    tui.session.set_trash_home_for_test(trashes[1].clone());
    gui.update(&mut visual, |v, _| v.enable_draft_input_for_test());
    tui.enable_draft_input_for_test();
    let key = |tui: &mut App, code, modifiers| { tui.handle(Event::Key(KeyEvent::new(code, modifiers))); };
    let settle_until = |visual: &mut VisualTestContext, tui: &mut App, done: &dyn Fn(&State) -> bool| {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !gui.read_with(visual, |v, _| done(v.session().state())) || !done(tui.session.state()) {
            gui.update(visual, |v, cx| v.poll(cx));
            while let Ok(c) = tui.session.completions().try_recv() { tui.apply(c); }
            assert!(Instant::now() < deadline, "timed out");
            std::thread::yield_now();
        }
    };
    let scanned = |s: &State| s.project().is_some_and(|p| matches!(p.coverage(), code_core::project::Coverage::Complete(_)) && !p.refreshing());
    settle_until(&mut visual, &mut tui, &scanned);
    for (i, root) in roots.iter().enumerate() {
        let open = Command::OpenRetained(root.join("src/a.txt"));
        if i == 0 { gui.update(&mut visual, |v, cx| v.dispatch_for_test(open, cx)); } else { tui.dispatch(open); }
    }
    settle_until(&mut visual, &mut tui, &|s: &State| s.current_source().is_some_and(|d| d.source().ends_with("src/a.txt")));
    visual.simulate_keystrokes("x");
    key(&mut tui, KeyCode::Char('x'), KeyModifiers::NONE);
    let highlight = |visual: &mut VisualTestContext, tui: &mut App, row: &str| {
        gui.update(visual, |v, cx| { v.dispatch_for_test(Command::RevealInTree(row.into()), cx); v.tree_sel = Some(row.into()); });
        tui.dispatch(Command::RevealInTree(row.into()));
        tui.tree_at = tui.session.state().tree_rows().position(|r| &*r.path == row).expect("the row is listed");
    };
    let run = |visual: &mut VisualTestContext, tui: &mut App, id: CommandId| {
        gui.update_in(visual, |v, w, cx| v.run(id, w, cx));
        tui.run(id);
    };
    let errors = |visual: &VisualTestContext, tui: &App| {
        let gui_error = gui.read_with(visual, |v, _| match &v.modal { Some(code_gui::Modal::ConfirmDelete { error, .. }) => error.clone(), _ => None });
        (gui_error, tui.modal.as_ref().and_then(|m| m.error.clone()))
    };
    // A draft refuses the delete in both dialogs; Enter and typing do nothing, Escape closes.
    highlight(&mut visual, &mut tui, "src/a.txt");
    run(&mut visual, &mut tui, CommandId::DeleteExplorerEntry);
    let refused = Some("a.txt has unsaved changes: save or discard them first".to_string());
    assert_eq!(errors(&visual, &tui), (refused.clone(), refused));
    visual.simulate_keystrokes("enter y");
    key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    key(&mut tui, KeyCode::Char('y'), KeyModifiers::NONE);
    visual.simulate_keystrokes("escape");
    key(&mut tui, KeyCode::Esc, KeyModifiers::NONE);
    assert!(tui.modal.is_none() && gui.read_with(&visual, |v, _| v.modal.is_none()));
    assert_eq!(snapshot(tui.session.state()), gui.read_with(&visual, |v, _| snapshot(v.session().state())));
    for root in &roots { assert_eq!(std::fs::read(root.join("src/a.txt")).unwrap(), b"saved\n"); }
    // Undone to the saved text, the file goes to each trash and its tab closes.
    visual.simulate_keystrokes("ctrl-z");
    key(&mut tui, KeyCode::Char('z'), KeyModifiers::CONTROL);
    settle_until(&mut visual, &mut tui, &|s: &State| s.active().is_some_and(|t| !s.tab_dirty(t.id())));
    highlight(&mut visual, &mut tui, "src/a.txt");
    run(&mut visual, &mut tui, CommandId::DeleteExplorerEntry);
    assert_eq!(errors(&visual, &tui), (None, None));
    visual.simulate_keystrokes("enter");
    key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    settle_until(&mut visual, &mut tui, &|s: &State| s.tabs().is_empty());
    assert_eq!(snapshot(tui.session.state()), gui.read_with(&visual, |v, _| snapshot(v.session().state())));
    for (root, trash) in roots.iter().zip(&trashes) {
        assert!(!root.join("src/a.txt").exists());
        assert_eq!(std::fs::read(trash.join("files/a.txt")).unwrap(), b"saved\n");
    }
    // Restore puts it back in both.
    run(&mut visual, &mut tui, CommandId::RestoreDeleted);
    settle_until(&mut visual, &mut tui, &|s: &State| s.project().is_some_and(|p| p.rows().iter().any(|r| &*r.path == "src/a.txt")));
    for (root, trash) in roots.iter().zip(&trashes) {
        assert_eq!(std::fs::read(root.join("src/a.txt")).unwrap(), b"saved\n");
        assert!(!trash.join("info/a.txt.trashinfo").exists());
    }
    // Permanent deletion is its own command and dialog; Escape keeps the folder, Enter removes it with no trash entry.
    highlight(&mut visual, &mut tui, "lib");
    run(&mut visual, &mut tui, CommandId::DeleteExplorerPermanently);
    assert!(tui.modal_title().contains("cannot be restored"), "{}", tui.modal_title());
    visual.simulate_keystrokes("escape");
    key(&mut tui, KeyCode::Esc, KeyModifiers::NONE);
    for root in &roots { assert!(root.join("lib/keep.txt").exists()); }
    run(&mut visual, &mut tui, CommandId::DeleteExplorerPermanently);
    visual.simulate_keystrokes("enter");
    key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    for (root, trash) in roots.iter().zip(&trashes) {
        assert!(!root.join("lib").exists());
        assert!(!trash.join("files/lib").exists());
    }
    assert_eq!(snapshot(tui.session.state()), gui.read_with(&visual, |v, _| snapshot(v.session().state())));
}

#[gpui::test]
fn gui_and_terminal_mark_the_same_changes_against_head_through_edit_save_and_undo(cx: &mut TestAppContext) {
    use lenz_content::git::testing::{commit_all, repo, write};
    let make = || {
        let r = repo();
        write(r.path(), "a.txt", "one\ntwo\nthree\n");
        commit_all(r.path(), "base");
        write(r.path(), "other.txt", "untracked\n");
        r
    };
    let (gr, tr) = (make(), make());
    let state = tempfile::tempdir().unwrap();
    let mut core = Session::new(config(&state.path().join("gui")));
    core.dispatch(Command::OpenRoot(gr.path().to_path_buf()));
    let gui = cx.new(|cx| ReaderView::new(core, Some(gr.path().join("a.txt")), cx));
    let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| gui.clone())).unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let focus = gui.read_with(&visual, |v, _| v.focus_handle());
    visual.update(|w, cx| focus.focus(w, cx));
    let mut tui = App::new(config(&state.path().join("tui")), Some(tr.path().to_path_buf()));
    tui.session.dispatch(Command::OpenRetained(tr.path().join("a.txt")));
    // What both adapters' gutter letters show for the active tab, and the baseline they name.
    let markers = |s: &State| {
        let markers = s.active().and_then(|tab| s.markers(tab.id()))?;
        Some((
            markers.baseline().map(code_core::markers::Baseline::kind),
            // The transient catch-up suffix is a timing detail, not a different marker.
            markers.summary().map(|summary| summary.replace(" (updating…)", "")),
            (0..3).map(|line| format!("{}{}", markers.change(line).map_or(" ", code_core::markers::LineChange::letter), if markers.deleted_above(line) > 0 { "D" } else { " " })).collect::<Vec<_>>(),
        ))
    };
    let settle = |visual: &mut VisualTestContext, tui: &mut App, done: &dyn Fn(&State) -> bool| {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            gui.update(visual, |v, cx| v.poll(cx));
            while let Ok(c) = tui.session.completions().try_recv() { tui.apply(c); }
            if gui.read_with(visual, |v, _| done(v.session().state())) && done(tui.session.state()) { break; }
            assert!(Instant::now() < deadline, "adapters did not settle: {:?} / {:?}", gui.read_with(visual, |v, _| markers(v.session().state())), markers(tui.session.state()));
            std::thread::sleep(Duration::from_millis(2));
        }
    };
    settle(&mut visual, &mut tui, &|s| s.markers(s.active().map(|t| t.id()).unwrap_or(code_core::tabs::TabId(0))).is_some_and(|m| !m.pending()));
    assert_eq!(markers(tui.session.state()), gui.read_with(&visual, |v, _| markers(v.session().state())));
    assert_eq!(markers(tui.session.state()).map(|m| (m.0, m.2.clone())), Some((Some(code_core::markers::BaselineKind::Head), vec!["  ".into(), "  ".into(), "  ".into()])));
    gui.update(&mut visual, |v, _| v.enable_draft_input_for_test());
    tui.enable_draft_input_for_test();
    // A real keystroke in each adapter, then the same settled change against HEAD.
    visual.simulate_keystrokes("x");
    tui.handle(Event::Key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE)));
    let changed = |s: &State| markers(s).is_some_and(|m| !s.active().and_then(|t| s.markers(t.id())).unwrap().pending() && m.2[0] == "M ");
    settle(&mut visual, &mut tui, &changed);
    let (g, t) = (gui.read_with(&visual, |v, _| markers(v.session().state())), markers(tui.session.state()));
    assert_eq!(g, t);
    assert_eq!(t.map(|m| m.2), Some(vec!["M ".into(), "  ".into(), "  ".into()]));
    assert_eq!(gui.read_with(&visual, |v, _| v.marker_letters().len()), 1);
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 24)).unwrap();
    terminal.draw(|f| tui.draw(f)).unwrap();
    let rows = (0..24).map(|y| (0..100).map(|x| terminal.backend().buffer()[(x, y)].symbol()).collect::<String>()).collect::<Vec<_>>();
    assert!(rows.iter().any(|r| r.contains(" M")), "{rows:#?}");
    // Save keeps the repository markers in both, and writes the same bytes.
    visual.simulate_keystrokes("ctrl-s");
    tui.handle(Event::Key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)));
    let saved = |s: &State| s.tabs().iter().all(|t| !s.tab_dirty(t.id())) && markers(s).is_some_and(|m| m.2[0] == "M ");
    settle(&mut visual, &mut tui, &saved);
    assert_eq!(gui.read_with(&visual, |v, _| markers(v.session().state())), markers(tui.session.state()));
    for r in [&gr, &tr] { assert_eq!(std::fs::read_to_string(r.path().join("a.txt")).unwrap(), "xone\ntwo\nthree\n"); }
    // Undo returns to the committed text: neither adapter shows a change any more.
    visual.simulate_keystrokes("ctrl-z");
    tui.handle(Event::Key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL)));
    let undone = |s: &State| s.active().and_then(|t| s.markers(t.id())).is_some_and(|m| !m.pending() && m.change(0).is_none());
    settle(&mut visual, &mut tui, &undone);
    assert_eq!(gui.read_with(&visual, |v, _| markers(v.session().state())), markers(tui.session.state()));
    assert_eq!(gui.read_with(&visual, |v, _| v.marker_letters()), Vec::<(char, u32)>::new());
    for r in [&gr, &tr] { assert_eq!(std::fs::read_to_string(r.path().join("a.txt")).unwrap(), "xone\ntwo\nthree\n", "undo changes no disk bytes"); }
}

#[gpui::test]
fn structural_edits_replay_the_same_text_and_selections_in_both_adapters(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("edit.rs");
    let original = "é😀\r\nλ two\n三 three\r\nfour";
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
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 20)).unwrap();
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
    // Tab indents with the file's step at the caret in both adapters; Shift+Tab takes it back.
    visual.simulate_keystrokes("tab");
    key(&mut tui, KeyCode::Tab, KeyModifiers::NONE);
    check(&visual, &tui, "    é😀\r\nλ two\n三 three\r\nfour");
    visual.simulate_keystrokes("shift-tab");
    key(&mut tui, KeyCode::BackTab, KeyModifiers::SHIFT);
    check(&visual, &tui, original);
    // Ctrl+Shift+Down moves the caret's line over the next one, keeping every ending where it stands.
    visual.simulate_keystrokes("ctrl-shift-down");
    key(&mut tui, KeyCode::Down, KeyModifiers::CONTROL | KeyModifiers::SHIFT);
    check(&visual, &tui, "λ two\r\né😀\n三 three\r\nfour");
    // A whole-line selection comments, then Alt+Shift+Down duplicates it.
    visual.simulate_keystrokes("home ctrl-a");
    for key_press in [KeyCode::Home, KeyCode::Char('a')] { key(&mut tui, key_press, if key_press == KeyCode::Home { KeyModifiers::NONE } else { KeyModifiers::CONTROL }); }
    visual.simulate_keystrokes("alt-/");
    key(&mut tui, KeyCode::Char('/'), KeyModifiers::ALT);
    check(&visual, &tui, "// λ two\r\n// é😀\n// 三 three\r\n// four");
    visual.simulate_keystrokes("alt-shift-down");
    key(&mut tui, KeyCode::Down, KeyModifiers::ALT | KeyModifiers::SHIFT);
    let duplicated = gui.read_with(&visual, |view, _| { let source = view.session().state().current_source().unwrap(); source.copy_range(0..source.len_bytes()).unwrap() });
    check(&visual, &tui, &duplicated);
    // Undo takes every action back in both adapters.
    for expected in ["// λ two\r\n// é😀\n// 三 three\r\n// four", "λ two\r\né😀\n三 three\r\nfour", original] {
        visual.simulate_keystrokes("ctrl-z");
        key(&mut tui, KeyCode::Char('z'), KeyModifiers::CONTROL);
        check(&visual, &tui, expected);
    }
    // A multiline selection over Unicode lines moves as one block, with the same selection in both adapters.
    visual.simulate_keystrokes("down shift-down");
    for key_press in [(KeyCode::Down, KeyModifiers::NONE), (KeyCode::Down, KeyModifiers::SHIFT)] { key(&mut tui, key_press.0, key_press.1); }
    visual.simulate_keystrokes("ctrl-g 1 enter down shift-down");
    for key_press in [(KeyCode::Char('g'), KeyModifiers::CONTROL), (KeyCode::Char('1'), KeyModifiers::NONE), (KeyCode::Enter, KeyModifiers::NONE), (KeyCode::Down, KeyModifiers::NONE), (KeyCode::Down, KeyModifiers::SHIFT)] {
        key(&mut tui, key_press.0, key_press.1);
    }
    visual.simulate_keystrokes("ctrl-shift-down");
    key(&mut tui, KeyCode::Down, KeyModifiers::CONTROL | KeyModifiers::SHIFT);
    check(&visual, &tui, "é😀\r\n三 three\nλ two\r\nfour");
    draw(&mut visual, &mut tui, &mut terminal);
    assert_eq!(std::fs::read(&path).unwrap(), original.as_bytes(), "neither adapter writes the file");
}

/// A draft whose file changed on disk is announced at once, and Ctrl+S opens the reconcile dialog in both adapters
/// instead of a refused write.
#[gpui::test]
fn gui_and_terminal_announce_an_external_change_and_save_opens_the_dialog(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("conflict.txt");
    std::fs::write(&path, "saved\r\n").unwrap();
    let gui = cx.new(|cx| ReaderView::new(Session::new(config(&dir.path().join("gui"))), Some(path.clone()), cx));
    let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| gui.clone())).unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let focus = gui.read_with(&visual, |v, _| v.focus_handle());
    visual.update(|w, cx| focus.focus(w, cx));
    let mut tui = App::new(config(&dir.path().join("tui")), Some(path.clone()));
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
    settle(&mut visual, &mut tui, &|s| s.state().current_source().is_some());
    gui.update(&mut visual, |v, _| v.enable_draft_input_for_test());
    tui.enable_draft_input_for_test();
    visual.simulate_keystrokes("x");
    tui.handle(Event::Key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE)));
    std::fs::write(&path, "external replacement\r\n").unwrap();
    gui.update(&mut visual, |v, cx| v.dispatch_for_test(Command::Recheck, cx));
    tui.dispatch(Command::Recheck);
    settle(&mut visual, &mut tui, &|s| s.status_message().starts_with("File changed on disk"));
    visual.simulate_keystrokes("ctrl-s");
    tui.handle(Event::Key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)));
    settle(&mut visual, &mut tui, &|s| s.reconcile_prompt().is_some_and(|p| p.disk.is_some()));
    assert!(matches!(tui.modal.as_ref().map(|m| &m.kind), Some(crate::app::Prompt::Details { kind: crate::app::Details::Reconcile, .. })));
    assert_eq!(std::fs::read(&path).unwrap(), b"external replacement\r\n", "nothing was written");
}

/// Both adapters reach the external-change dialog through their own menus, offer the same sides and decisions,
/// and taking the disk version leaves the same draft, history and file bytes.
#[gpui::test]
fn gui_and_terminal_reconcile_the_same_external_change_through_their_dialogs(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("conflict.txt");
    std::fs::write(&path, "saved\r\n").unwrap();
    let gui = cx.new(|cx| ReaderView::new(Session::new(config(&dir.path().join("gui"))), Some(path.clone()), cx));
    let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| gui.clone())).unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let focus = gui.read_with(&visual, |v, _| v.focus_handle());
    visual.update(|w, cx| focus.focus(w, cx));
    let mut tui = App::new(config(&dir.path().join("tui")), Some(path.clone()));
    let settle = |visual: &mut VisualTestContext, tui: &mut App, done: &dyn Fn(&State) -> bool| {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            gui.update(visual, |v, cx| v.poll(cx));
            while let Ok(c) = tui.session.completions().try_recv() { tui.apply(c); }
            if gui.read_with(visual, |v, _| done(v.session().state())) && done(tui.session.state()) { break; }
            assert!(Instant::now() < deadline, "adapters did not settle");
            std::thread::sleep(Duration::from_millis(2));
        }
    };
    settle(&mut visual, &mut tui, &|s| s.current_source().is_some());
    gui.update(&mut visual, |v, _| v.enable_draft_input_for_test());
    tui.enable_draft_input_for_test();
    visual.update(|_, cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string("draft λ\r\n".into())));
    visual.simulate_keystrokes("ctrl-a ctrl-v");
    tui.handle(Event::Key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL)));
    tui.handle(Event::Paste("draft λ\r\n".into()));
    std::fs::write(&path, "external replacement\r\n").unwrap();
    // Save in both: the writer refuses the version that is no longer on disk, which is the recorded conflict.
    visual.simulate_keystrokes("ctrl-s");
    tui.handle(Event::Key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)));
    let rejected = |s: &State| s.active().is_some_and(|t| matches!(s.view(t.id()).unwrap().save_status(), code_core::save::Status::Rejected(_)));
    settle(&mut visual, &mut tui, &rejected);
    // The same menu entry opens the dialog in each adapter.
    let menu = |visual: &mut VisualTestContext, tui: &mut App| {
        visual.simulate_keystrokes("f10");
        let row = gui.read_with(visual, |v, _| v.modal_rows().iter().position(|r| r.0 == CommandId::ResolveExternalChange.label()).unwrap());
        for _ in 0..row { visual.simulate_keystrokes("down"); }
        visual.simulate_keystrokes("enter");
        tui.handle(Event::Key(KeyEvent::new(KeyCode::F(10), KeyModifiers::NONE)));
        let row = tui.choices().iter().position(|row| matches!(&row.target, crate::app::ChoiceTarget::Action(id) if *id == CommandId::ResolveExternalChange)).unwrap();
        for _ in 0..row { tui.handle(Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))); }
        tui.handle(Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)));
    };
    menu(&mut visual, &mut tui);
    let ready = |session: &Session| session.reconcile_prompt().is_some_and(|prompt| prompt.disk.is_some());
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        gui.update(&mut visual, |v, cx| v.poll(cx));
        while let Ok(c) = tui.session.completions().try_recv() { tui.apply(c); }
        if gui.read_with(&visual, |v, _| ready(v.session())) && ready(&tui.session) { break; }
        assert!(Instant::now() < deadline, "the dialog never read the file");
        std::thread::sleep(Duration::from_millis(2));
    }
    let (g, t) = (gui.read_with(&visual, |v, _| v.session().reconcile_prompt().unwrap()), tui.session.reconcile_prompt().unwrap());
    assert_eq!(g.text(), t.text(), "both adapters present the same three sides");
    let labels = |prompt: &code_core::reconcile::Prompt| code_core::reconcile::Choice::ALL.map(|choice| prompt.disabled(choice)).to_vec();
    assert_eq!(labels(&g), labels(&t));
    for word in ["Disk:", "Current draft:", "Confirmed baseline"] { assert!(t.text().contains(word), "{}", t.text()); }
    // Typing into either dialog changes no draft and no bytes.
    visual.write_to_clipboard(gpui::ClipboardItem::new_string("must not paste".into()));
    visual.simulate_keystrokes("ctrl-v x");
    tui.handle(Event::Paste("must not paste".into()));
    tui.handle(Event::Key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE)));
    assert_eq!(gui.read_with(&visual, |v, _| snapshot(v.session().state())), snapshot(tui.session.state()));
    assert_eq!(std::fs::read(&path).unwrap(), "external replacement\r\n".as_bytes());
    // D takes the disk version in both; the file is not written.
    visual.simulate_keystrokes("d");
    tui.handle(Event::Key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE)));
    let taken = |s: &State| s.current_source().is_some_and(|text| text.copy_range(0..text.len_bytes()).unwrap() == "external replacement\r\n");
    settle(&mut visual, &mut tui, &taken);
    assert_eq!(gui.read_with(&visual, |v, _| snapshot(v.session().state())), snapshot(tui.session.state()));
    assert!(!tui.session.state().view(tui.session.state().active().unwrap().id()).unwrap().dirty());
    assert_eq!(std::fs::read(&path).unwrap(), "external replacement\r\n".as_bytes());
    // Undo restores the same dirty draft and its baseline in both.
    visual.simulate_keystrokes("ctrl-z");
    tui.handle(Event::Key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL)));
    let undone = |s: &State| s.current_source().is_some_and(|text| text.copy_range(0..text.len_bytes()).unwrap() == "draft λ\r\n");
    settle(&mut visual, &mut tui, &undone);
    assert_eq!(gui.read_with(&visual, |v, _| snapshot(v.session().state())), snapshot(tui.session.state()));
    assert!(tui.session.state().view(tui.session.state().active().unwrap().id()).unwrap().dirty());
    assert_eq!(std::fs::read(&path).unwrap(), "external replacement\r\n".as_bytes());
}

/// A tool written at test time (from another process, so no descriptor of this one keeps it open for writing).
fn shell_tool(path: &Path, script: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let ok = std::process::Command::new("/bin/sh").args(["-c", "printf '%s' \"$1\" > \"$2\" && chmod +x \"$2\"", "fixture"]).arg(script).arg(path).status().unwrap();
    assert!(ok.success());
}

/// ShellCheck (a stand-in) through both adapters, with bash-language-server and shfmt missing: the same problem,
/// marked after `😀` in each; F8 to it, Alt+Enter listing ShellCheck's fix and Enter applying it, Ctrl+Z; Ctrl+Alt+L
/// saying shfmt is missing; Server Details (Find Action) naming the shell and ShellCheck alike; typing and Ctrl+S
/// writing the same bytes.
#[gpui::test]
fn shellcheck_problems_fixes_and_a_missing_shfmt_match_in_both_adapters(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let report = r#"{"comments":[{"file":"-","line":2,"endLine":2,"column":12,"endColumn":14,"level":"info","code":2086,"message":"Double quote to prevent globbing and word splitting.","fix":{"replacements":[{"line":2,"endLine":2,"column":12,"endColumn":12,"insertionPoint":"afterEnd","precedence":7,"replacement":"\""},{"line":2,"endLine":2,"column":14,"endColumn":14,"insertionPoint":"beforeStart","precedence":7,"replacement":"\""}]}}]}"#;
    let shellcheck = dir.path().join("tools/shellcheck");
    shell_tool(&shellcheck, &format!("#!/bin/sh\nif [ \"$1\" = --version ]; then printf 'version: 0.0.1-test\\n'; exit 0; fi\ncat > /dev/null\nprintf '%s' '{report}'\nexit 1\n"));
    let text = "greet() {\n  echo \"😀\" $1\n}\n";
    let make = |name: &str| {
        let root = dir.path().join(name).join("project");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join(".shellcheckrc"), "shell=sh\n").unwrap();
        std::fs::write(root.join("run.sh"), text).unwrap();
        let c = config(&dir.path().join(name).join("config"));
        let mut settings: serde_json::Value = serde_json::from_slice(&std::fs::read(&c.providers).unwrap()).unwrap();
        settings["providers"]["bash-language-server"]["settings"] = serde_json::json!({ "shellcheckPath": shellcheck, "shfmt": { "path": "/nonexistent/shfmt" } });
        std::fs::write(&c.providers, settings.to_string()).unwrap(); (c, root)
    };
    let ((gc, groot), (tc, troot)) = (make("gui"), make("tui"));
    let file = |root: &Path| root.join("run.sh");
    let mut core = Session::new(gc); core.dispatch(Command::OpenRoot(groot.clone()));
    let gui = cx.new(|cx| ReaderView::new(core, Some(file(&groot)), cx));
    let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| gui.clone())).unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let focus = gui.read_with(&visual, |view, _| view.focus_handle()); visual.update(|window, cx| focus.focus(window, cx));
    let mut tui = App::new(tc, Some(troot.clone())); tui.session.dispatch(Command::Open(file(&troot)));
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 40)).unwrap();
    let wait = |visual: &mut VisualTestContext, tui: &mut App, terminal: &mut ratatui::Terminal<ratatui::backend::TestBackend>, done: &dyn Fn(&State) -> bool| {
        let until = Instant::now() + Duration::from_secs(20);
        loop {
            gui.update(visual, |view, cx| view.poll(cx)); while let Ok(completion) = tui.session.completions().try_recv() { tui.apply(completion); }
            visual.update(|window, cx| { let _ = window.draw(cx); }); terminal.draw(|frame| tui.draw(frame)).unwrap();
            if gui.read_with(visual, |view, _| done(view.session().state())) && done(tui.session.state()) { break; }
            assert!(Instant::now() < until, "GUI {:?}; TUI {:?} {:?}", gui.read_with(visual, |view, _| view.session().state().notices()), tui.session.state().notices(), tui.modal_title());
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    let problems = |state: &State| state.active().and_then(|t| state.diagnostics(t.id())).map(|d| d.items.iter().map(|p| (p.message.clone(), p.range.clone())).collect::<Vec<_>>()).unwrap_or_default();
    wait(&mut visual, &mut tui, &mut terminal, &|state| !problems(state).is_empty());
    let at = text.find("$1").unwrap();
    let expected = vec![("Double quote to prevent globbing and word splitting.".to_owned(), at..at + 2)];
    assert_eq!(problems(tui.session.state()), expected);
    assert_eq!(gui.read_with(&visual, |view, _| problems(view.session().state())), expected);
    // Each marks it: the GUI's problem marks, the terminal's underline.
    assert_eq!(gui.read_with(&visual, |view, _| view.problem_marks()).into_iter().map(|(t, _)| t).collect::<Vec<_>>(), ["$1"]);
    let screen = terminal.backend().buffer().clone();
    let cell = tui.layout.panes[0].hits.iter().find(|h| h.line == 1 && h.byte == at - text.find('\n').unwrap() - 1).map(|h| screen[(h.x, h.y)].style()).unwrap();
    assert!(cell.add_modifier.contains(ratatui::style::Modifier::UNDERLINED));
    gui.update(&mut visual, |view, _| view.enable_draft_input_for_test()); tui.enable_draft_input_for_test();
    let key = |tui: &mut App, code, modifiers| { tui.handle(Event::Key(KeyEvent::new(code, modifiers))); };
    let current = |state: &State| state.current_source().map(|d| d.copy_range(0..d.len_bytes()).unwrap());
    let both = |visual: &mut VisualTestContext, tui: &App, expected: &str| {
        assert_eq!(current(tui.session.state()).as_deref(), Some(expected));
        assert_eq!(gui.read_with(visual, |view, _| current(view.session().state())).as_deref(), Some(expected));
    };
    // F8, then Alt+Enter: ShellCheck's fix first, drawn in each; Enter applies it; Ctrl+Z takes it back.
    visual.simulate_keystrokes("f8"); key(&mut tui, KeyCode::F(8), KeyModifiers::NONE);
    let caret = |state: &State| state.selection().map(|s| s.head);
    assert_eq!((caret(tui.session.state()), gui.read_with(&visual, |view, _| caret(view.session().state()))), (Some(at), Some(at)));
    assert_eq!(tui.session.state().problem_at_caret().as_deref(), Some("Info: Double quote to prevent globbing and word splitting. (shellcheck SC2086)"));
    let fixes = |state: &State| state.code_actions().map(|c| c.actions.iter().map(|a| a.title.clone()).collect::<Vec<_>>());
    visual.simulate_keystrokes("alt-enter"); key(&mut tui, KeyCode::Enter, KeyModifiers::ALT);
    wait(&mut visual, &mut tui, &mut terminal, &|state| state.code_actions().is_some());
    let listed = fixes(tui.session.state()).unwrap();
    assert_eq!(listed, ["Apply fix for SC2086: Double quote to prevent globbing and word splitting.", "Disable ShellCheck rule SC2086 for the entire file"]);
    assert_eq!(gui.read_with(&visual, |view, _| fixes(view.session().state())), Some(listed));
    assert!(gui.read_with(&visual, |view, _| view.code_actions_bounds().is_some()) && tui.layout.code_actions.width > 0, "both draw the list");
    visual.simulate_keystrokes("enter"); key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    both(&mut visual, &tui, &text.replace("$1", "\"$1\""));
    visual.simulate_keystrokes("ctrl-z"); key(&mut tui, KeyCode::Char('z'), KeyModifiers::CONTROL);
    both(&mut visual, &tui, text);
    // Ctrl+Alt+L: shfmt is missing, said alike; nothing changes.
    visual.simulate_keystrokes("ctrl-alt-l"); key(&mut tui, KeyCode::Char('l'), KeyModifiers::CONTROL | KeyModifiers::ALT);
    let refused = |state: &State| state.notices().iter().any(|n| n.starts_with("Format Document: shfmt is missing (/nonexistent/shfmt does not exist). Install it"));
    wait(&mut visual, &mut tui, &mut terminal, &refused);
    both(&mut visual, &tui, text);
    // Server Details (Find Action) names the shell and ShellCheck alike.
    visual.simulate_keystrokes("ctrl-shift-a s e r v e r space d e t a i l s");
    key(&mut tui, KeyCode::Char('a'), KeyModifiers::CONTROL | KeyModifiers::SHIFT);
    tui.handle(Event::Paste("server details".into()));
    wait(&mut visual, &mut tui, &mut terminal, &|state| state.navigation().is_some_and(|n| !n.pending));
    visual.simulate_keystrokes("enter"); key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    let shown = gui.read_with(&visual, |v, _| v.modal_rows()).into_iter().map(|(row, _)| row).collect::<Vec<_>>();
    let Some(crate::app::Prompt::Details { body, .. }) = tui.modal.as_ref().map(|m| &m.kind) else { panic!("no terminal details: {:?}", tui.modal_title()) };
    let body = body.clone();
    visual.simulate_keystrokes("escape"); key(&mut tui, KeyCode::Esc, KeyModifiers::NONE);
    assert!(gui.read_with(&visual, |v, _| v.modal_rows()).is_empty() && tui.modal_title().is_empty());
    let problems_row = shown.iter().find(|r| r.starts_with("Problems: ")).unwrap_or_else(|| panic!("{shown:?}"));
    assert_eq!(problems_row, &format!("Problems: ShellCheck 0.0.1-test ({}), checking as sh; it reads the text shown on standard input, never runs or sources the script, and runs again when edits pause", shellcheck.display()));
    assert!(body.contains(problems_row.as_str()), "{body}");
    assert!(shown.iter().any(|r| r.starts_with(&format!("Shell: sh, from `shell=` in {}.", groot.join(".shellcheckrc").display()))), "{shown:?}");
    assert!(body.contains(&format!("Shell: sh, from `shell=` in {}.", troot.join(".shellcheckrc").display())), "{body}");
    // Typing and Ctrl+S write the same bytes in both.
    for root in [&groot, &troot] { assert_eq!(std::fs::read_to_string(file(root)).unwrap(), text); }
    gui.update(&mut visual, |view, cx| view.dispatch_for_test(Command::Select(code_core::session::Selection { anchor: 0, head: 0 }), cx));
    tui.session.dispatch(Command::Select(code_core::session::Selection { anchor: 0, head: 0 }));
    visual.simulate_keystrokes("# space x enter"); for c in "# x".chars() { key(&mut tui, KeyCode::Char(c), KeyModifiers::NONE); } key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    let edited = format!("# x\n{text}");
    both(&mut visual, &tui, &edited);
    visual.simulate_keystrokes("ctrl-s"); key(&mut tui, KeyCode::Char('s'), KeyModifiers::CONTROL);
    wait(&mut visual, &mut tui, &mut terminal, &|_| [&groot, &troot].iter().all(|root| std::fs::read_to_string(file(root)).is_ok_and(|t| t == edited)));
}

/// Real bash-language-server, ShellCheck and shfmt behind both adapters on a Bash script: the same problem after
/// `😀`; Ctrl+Space after a typed `$gree` listing the script's variable, drawn in both, Enter accepting it; Alt+Enter
/// listing ShellCheck's fix and Enter applying it; Ctrl+Alt+L formatting with `.editorconfig`'s size and
/// `switch_case_indent`; each one Ctrl+Z; Server Details (Find Action) naming the shell and each tool alike. The same
/// draft in both and the files untouched. Needs `bash-language-server`, `shellcheck` and `shfmt` on PATH.
#[gpui::test]
#[ignore = "needs bash-language-server (Node.js), ShellCheck and shfmt; run alone"]
fn real_shell_assistance_matches_in_both_adapters(cx: &mut TestAppContext) {
    let find = |name: &str| code_core::semantics::provider::find_executable(Path::new(name), None);
    let (Some(server), Some(_), Some(_)) = (find("bash-language-server"), find("shellcheck"), find("shfmt")) else {
        println!("SKIPPED: bash-language-server, shellcheck or shfmt is not installed");
        return;
    };
    let text = "#!/usr/bin/env bash\n# 😀 deploy\ngreeting=\"héllo\"\ngreet() {\n  if [ -n \"$1\" ]; then\n    echo \"😀 $greeting\" $1\n  fi\n  case $1 in\n  w*) echo w ;;\n  esac\n}\ngreet \"wörld\"\n";
    let formatted = "#!/usr/bin/env bash\n# 😀 deploy\ngreeting=\"héllo\"\ngreet() {\n   if [ -n \"$1\" ]; then\n      echo \"😀 $greeting\" $1\n   fi\n   case $1 in\n      w*) echo w ;;\n   esac\n}\ngreet \"wörld\"\n";
    let dir = tempfile::tempdir().unwrap();
    let make = |name: &str| {
        let root = dir.path().join(name).join("project");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join(".editorconfig"), "root = true\n[*.sh]\nindent_style = space\nindent_size = 3\nswitch_case_indent = true\n").unwrap();
        std::fs::write(root.join("deploy.sh"), text).unwrap();
        let c = config(&dir.path().join(name).join("config"));
        let mut settings: serde_json::Value = serde_json::from_slice(&std::fs::read(&c.providers).unwrap()).unwrap();
        settings["providers"]["bash-language-server"] = serde_json::json!({ "executable": server });
        std::fs::write(&c.providers, settings.to_string()).unwrap(); (c, root)
    };
    let ((gc, groot), (tc, troot)) = (make("gui"), make("tui"));
    let file = |root: &Path| root.join("deploy.sh");
    let mut core = Session::new(gc); core.dispatch(Command::OpenRoot(groot.clone()));
    let gui = cx.new(|cx| ReaderView::new(core, Some(file(&groot)), cx));
    let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| gui.clone())).unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let focus = gui.read_with(&visual, |view, _| view.focus_handle()); visual.update(|window, cx| focus.focus(window, cx));
    let mut tui = App::new(tc, Some(troot.clone())); tui.session.dispatch(Command::Open(file(&troot)));
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 40)).unwrap();
    let wait = |visual: &mut VisualTestContext, tui: &mut App, terminal: &mut ratatui::Terminal<ratatui::backend::TestBackend>, limit: u64, done: &dyn Fn(&State) -> bool| {
        let until = Instant::now() + Duration::from_secs(limit);
        loop {
            gui.update(visual, |view, cx| view.poll(cx)); while let Ok(completion) = tui.session.completions().try_recv() { tui.apply(completion); }
            visual.update(|window, cx| { let _ = window.draw(cx); }); terminal.draw(|frame| tui.draw(frame)).unwrap();
            if gui.read_with(visual, |view, _| done(view.session().state())) && done(tui.session.state()) { break; }
            assert!(Instant::now() < until, "GUI {:?}; TUI {:?} {:?}", gui.read_with(visual, |view, _| view.session().state().notices()), tui.session.state().notices(), tui.modal_title());
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    let screen = |terminal: &ratatui::Terminal<ratatui::backend::TestBackend>| terminal.backend().buffer().content().chunks(120).map(|r| r.iter().map(|c| c.symbol()).collect::<String>() + "\n").collect::<String>();
    let ready = |state: &State| state.provider().is_some_and(|p| p.status == code_core::semantics::Status::Ready);
    let problems = |state: &State| state.active().and_then(|t| state.diagnostics(t.id())).map(|d| d.items.iter().map(|p| (p.message.clone(), p.range.clone())).collect::<Vec<_>>()).unwrap_or_default();
    wait(&mut visual, &mut tui, &mut terminal, 120, &|state| ready(state) && !problems(state).is_empty());
    let at = text.find("\" $1").unwrap() + 2;
    let expected = vec![("Double quote to prevent globbing and word splitting.".to_owned(), at..at + 2)];
    assert_eq!(problems(tui.session.state()), expected);
    assert_eq!(gui.read_with(&visual, |view, _| problems(view.session().state())), expected);
    assert_eq!(gui.read_with(&visual, |view, _| view.problem_marks()).into_iter().map(|(t, _)| t).collect::<Vec<_>>(), ["$1"]);
    gui.update(&mut visual, |view, _| view.enable_draft_input_for_test()); tui.enable_draft_input_for_test();
    let key = |tui: &mut App, code, modifiers| { tui.handle(Event::Key(KeyEvent::new(code, modifiers))); };
    let current = |state: &State| state.current_source().map(|d| d.copy_range(0..d.len_bytes()).unwrap());
    let both = |visual: &mut VisualTestContext, tui: &App, expected: &str| {
        assert_eq!(current(tui.session.state()).as_deref(), Some(expected));
        assert_eq!(gui.read_with(visual, |view, _| current(view.session().state())).as_deref(), Some(expected));
    };
    let ctrl_z = |visual: &mut VisualTestContext, tui: &mut App| { visual.simulate_keystrokes("ctrl-z"); key(tui, KeyCode::Char('z'), KeyModifiers::CONTROL); };
    let undo_to = |visual: &mut VisualTestContext, tui: &mut App, expected: &str| {
        for _ in 0..4 { if current(tui.session.state()).as_deref() != Some(expected) { ctrl_z(visual, tui); } }
        both(visual, tui, expected);
    };
    let select = |visual: &mut VisualTestContext, tui: &mut App, at: usize| {
        gui.update(visual, |view, cx| view.dispatch_for_test(Command::Select(code_core::session::Selection { anchor: at, head: at }), cx));
        tui.session.dispatch(Command::Select(code_core::session::Selection { anchor: at, head: at }));
    };
    println!("step: variable completion");
    select(&mut visual, &mut tui, text.len());
    visual.simulate_keystrokes("e c h o space $ g r e e"); for c in "echo $gree".chars() { key(&mut tui, KeyCode::Char(c), KeyModifiers::NONE); }
    both(&mut visual, &tui, &format!("{text}echo $gree"));
    let rows = |state: &State| state.completion().map(|c| c.rows().map(|r| r.label.clone()).collect::<Vec<_>>());
    let offered = |state: &State| rows(state).is_some_and(|r| r.iter().any(|l| l == "greeting"));
    // (bash-language-server 5.8.1 sometimes answers its first request after an edit with nothing; asked again, it lists them.)
    for _ in 0..20 {
        visual.simulate_keystrokes("ctrl-space"); key(&mut tui, KeyCode::Char(' '), KeyModifiers::CONTROL);
        wait(&mut visual, &mut tui, &mut terminal, 30, &|state| state.completion().is_some() || state.notices().last().is_some_and(|n| n.starts_with("Code completion")));
        if offered(tui.session.state()) && gui.read_with(&visual, |view, _| offered(view.session().state())) { break; }
        visual.simulate_keystrokes("escape"); key(&mut tui, KeyCode::Esc, KeyModifiers::NONE);
        std::thread::sleep(Duration::from_millis(100));
    }
    let listed = rows(tui.session.state()).unwrap();
    println!("listed: {listed:?}");
    assert_eq!(gui.read_with(&visual, |view, _| rows(view.session().state())), Some(listed.clone()), "the same suggestions in both");
    assert!(gui.read_with(&visual, |view, _| view.completion_bounds().is_some()), "the GUI draws its suggestions");
    assert!(screen(&terminal).contains("greeting"), "{}", screen(&terminal));
    let row = listed.iter().position(|l| l == "greeting").unwrap();
    for _ in 0..row { visual.simulate_keystrokes("down"); key(&mut tui, KeyCode::Down, KeyModifiers::NONE); }
    visual.simulate_keystrokes("enter"); key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    let completed = format!("{text}echo $greeting");
    wait(&mut visual, &mut tui, &mut terminal, 30, &|state| current(state).as_deref() == Some(completed.as_str()));
    undo_to(&mut visual, &mut tui, text);
    println!("step: quick fix");
    wait(&mut visual, &mut tui, &mut terminal, 30, &|state| !problems(state).is_empty());
    select(&mut visual, &mut tui, at);
    let fixes = |state: &State| state.code_actions().map(|c| c.actions.iter().map(|a| a.title.clone()).collect::<Vec<_>>());
    visual.simulate_keystrokes("alt-enter"); key(&mut tui, KeyCode::Enter, KeyModifiers::ALT);
    wait(&mut visual, &mut tui, &mut terminal, 30, &|state| state.code_actions().is_some());
    let listed = fixes(tui.session.state()).unwrap();
    assert_eq!(listed, ["Apply fix for SC2086: Double quote to prevent globbing and word splitting.", "Disable ShellCheck rule SC2086 for the entire file"]);
    assert_eq!(gui.read_with(&visual, |view, _| fixes(view.session().state())), Some(listed));
    assert!(gui.read_with(&visual, |view, _| view.code_actions_bounds().is_some()) && tui.layout.code_actions.width > 0, "both draw the list");
    visual.simulate_keystrokes("enter"); key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    both(&mut visual, &tui, &text.replace("\" $1", "\" \"$1\""));
    ctrl_z(&mut visual, &mut tui);
    both(&mut visual, &tui, text);
    println!("step: formatting");
    visual.simulate_keystrokes("ctrl-alt-l"); key(&mut tui, KeyCode::Char('l'), KeyModifiers::CONTROL | KeyModifiers::ALT);
    wait(&mut visual, &mut tui, &mut terminal, 60, &|state| current(state).as_deref() == Some(formatted));
    ctrl_z(&mut visual, &mut tui);
    both(&mut visual, &tui, text);
    println!("step: server details");
    visual.simulate_keystrokes("ctrl-shift-a s e r v e r space d e t a i l s");
    key(&mut tui, KeyCode::Char('a'), KeyModifiers::CONTROL | KeyModifiers::SHIFT);
    tui.handle(Event::Paste("server details".into()));
    wait(&mut visual, &mut tui, &mut terminal, 10, &|state| state.navigation().is_some_and(|n| !n.pending));
    visual.simulate_keystrokes("enter"); key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    let shown = gui.read_with(&visual, |v, _| v.modal_rows()).into_iter().map(|(row, _)| row).collect::<Vec<_>>();
    let Some(crate::app::Prompt::Details { body, .. }) = tui.modal.as_ref().map(|m| &m.kind) else { panic!("no terminal details: {:?}", tui.modal_title()) };
    let body = body.clone();
    for prefix in ["Server version: ", "Shell: ", "Problems: ", "Format Document: "] {
        let row = shown.iter().find(|r| r.starts_with(prefix)).unwrap_or_else(|| panic!("{prefix}: {shown:?}"));
        println!("{row}");
        assert!(body.contains(row.as_str()), "both adapters show the same row: {row}\n{body}");
    }
    assert!(shown.iter().any(|r| r == "Server version: bash-language-server 5.8.1"), "{shown:?}");
    assert!(shown.iter().any(|r| r.starts_with("Shell: Bash, from the shebang `#!/usr/bin/env bash`.")), "{shown:?}");
    assert!(shown.iter().any(|r| r.starts_with("Problems: ShellCheck 0.9.0 (") && r.contains("checking as bash")), "{shown:?}");
    assert!(shown.iter().any(|r| r.starts_with("Format Document: shfmt 3.8.0 (") && r.contains("`-ln=bash`")), "{shown:?}");
    visual.simulate_keystrokes("escape"); key(&mut tui, KeyCode::Esc, KeyModifiers::NONE);
    assert!(gui.read_with(&visual, |v, _| v.modal_rows()).is_empty() && tui.modal_title().is_empty());
    both(&mut visual, &tui, text);
    for root in [&groot, &troot] { assert_eq!(std::fs::read_to_string(file(root)).unwrap(), text); }
}

/// An environment file through both adapters, with no language server: the same key/value colours drawn in each
/// (comment, `export`, key, `=`, quoted and unquoted values, an escape, a multi-line value, `$(…)` as value text);
/// typed `"` pairs only after `=` and `${X}` stays literal; Alt+/ comments with `#`; Ctrl+Z; a paste with CRLF and
/// Unicode; Ctrl+S writes exactly the draft's bytes; Language Server Status and its details name no value. Nothing in
/// the file is run.
#[gpui::test]
fn environment_files_highlight_edit_and_save_alike_in_both_adapters(cx: &mut TestAppContext) {
    use lenz_content::highlight::Capture;
    const SECRET: &str = "s3cr$t_SENTINEL";
    let text = format!("# config 𝄞\r\nexport DB_USER=admin # owner\r\nDB_PASSWORD='{SECRET}'\nGREETING=\"hi\\n ${{NAME}}\"\nCERT=\"BEGIN\nMIIB==\nEND\"\nCMD=$(touch ran)\n");
    let dir = tempfile::tempdir().unwrap();
    let make = |name: &str| {
        let root = dir.path().join(name).join("project");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join(".env"), &text).unwrap();
        (config(&dir.path().join(name).join("config")), root)
    };
    let ((gc, groot), (tc, troot)) = (make("gui"), make("tui"));
    let file = |root: &Path| root.join(".env");
    let mut core = Session::new(gc); core.dispatch(Command::OpenRoot(groot.clone()));
    let gui = cx.new(|cx| ReaderView::new(core, Some(file(&groot)), cx));
    let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| gui.clone())).unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let focus = gui.read_with(&visual, |view, _| view.focus_handle()); visual.update(|window, cx| focus.focus(window, cx));
    let mut tui = App::new(tc, Some(troot.clone())); tui.session.dispatch(Command::Open(file(&troot)));
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
    let current = |state: &State| state.current_source().map(|d| d.copy_range(0..d.len_bytes()).unwrap());
    let draw = |visual: &mut VisualTestContext, tui: &mut App, terminal: &mut ratatui::Terminal<ratatui::backend::TestBackend>| {
        visual.update(|window, cx| { let _ = window.draw(cx); }); terminal.draw(|frame| tui.draw(frame)).unwrap();
    };
    let settle = |visual: &mut VisualTestContext, tui: &mut App, terminal: &mut ratatui::Terminal<ratatui::backend::TestBackend>, done: &dyn Fn(&State) -> bool| {
        let until = Instant::now() + Duration::from_secs(20);
        loop {
            gui.update(visual, |view, cx| view.poll(cx)); while let Ok(completion) = tui.session.completions().try_recv() { tui.apply(completion); }
            draw(visual, tui, terminal);
            if gui.read_with(visual, |view, _| done(view.session().state())) && done(tui.session.state()) { break; }
            assert!(Instant::now() < until, "both adapters never settled");
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    let highlighted = |state: &State| state.highlights().is_some() && state.current_source().is_some_and(|d| d.language() == Some(lenz_content::highlight::Language::Env));
    settle(&mut visual, &mut tui, &mut terminal, &highlighted);
    // The same tokens in the same syntax colours: the GUI's painted rows and the terminal's cells.
    let tokens = [
        ("config", Capture::Comment), ("export", Capture::Keyword), ("DB_USER", Capture::Property), ("admin", Capture::String),
        ("# owner", Capture::Comment), ("DB_PASSWORD", Capture::Property), (SECRET, Capture::String), ("\\n", Capture::Escape),
        ("${NAME}", Capture::String), ("MIIB==", Capture::String), ("END\"", Capture::String), ("$(touch ran)", Capture::String),
    ];
    let rows = gui.read_with(&visual, |view, _| view.visible_rows());
    let palette = gui.read_with(&visual, |view, _| **view.session().state().palette());
    for (token, capture) in tokens {
        let row = rows.iter().find(|r| r.text.contains(token)).unwrap_or_else(|| panic!("GUI: no row shows {token:?}"));
        let at = row.text.find(token).unwrap();
        let want = palette.syntax_color(capture);
        assert!(row.fg.iter().any(|(r, c)| r.start <= at && at + token.len() <= r.end && *c == want), "GUI: {token:?} is not drawn as {capture:?}: {:?}", row.fg);
    }
    let buffer = terminal.backend().buffer().clone();
    let tpalette = **tui.session.state().palette();
    let width = buffer.area.width as usize;
    for (token, capture) in tokens {
        let want = crate::render::color(tpalette.syntax_color(capture));
        let found = buffer.content.chunks(width).find_map(|cells| (0..cells.len().saturating_sub(token.len())).find(|&x| {
            token.chars().enumerate().all(|(i, c)| cells[x + i].symbol() == c.to_string())
        }).map(|x| cells[x..x + token.len()].iter().all(|cell| cell.fg == want)));
        assert_eq!(found, Some(true), "terminal: {token:?} is not drawn as {capture:?}");
    }
    gui.update(&mut visual, |view, _| view.enable_draft_input_for_test()); tui.enable_draft_input_for_test();
    let key = |tui: &mut App, code, modifiers| { tui.handle(Event::Key(KeyEvent::new(code, modifiers))); };
    let type_both = |visual: &mut VisualTestContext, tui: &mut App, typed: &str| {
        for c in typed.chars() {
            let stroke = if c == ' ' { "space".to_owned() } else { c.to_string() };
            visual.simulate_keystrokes(&stroke);
            tui.handle(Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)));
        }
    };
    let both = |visual: &mut VisualTestContext, tui: &App, expected: &str| {
        assert_eq!(current(tui.session.state()).as_deref(), Some(expected));
        assert_eq!(gui.read_with(visual, |view, _| current(view.session().state())).as_deref(), Some(expected));
    };
    let ctrl_z = |visual: &mut VisualTestContext, tui: &mut App| { visual.simulate_keystrokes("ctrl-z"); key(tui, KeyCode::Char('z'), KeyModifiers::CONTROL); };
    // A new entry at the end: `"` after `=` opens a pair, `${X}` and `$(id)` are typed as they are, the closer steps over.
    let end = text.len();
    gui.update(&mut visual, |view, cx| view.dispatch_for_test(Command::Select(code_core::session::Selection { anchor: end, head: end }), cx));
    tui.session.dispatch(Command::Select(code_core::session::Selection { anchor: end, head: end }));
    type_both(&mut visual, &mut tui, "NEW=\"");
    both(&mut visual, &tui, &format!("{text}NEW=\"\""));
    type_both(&mut visual, &mut tui, "${X} $(id) [a] `b`\"");
    let entry = format!("{text}NEW=\"${{X}} $(id) [a] `b`\"");
    both(&mut visual, &tui, &entry);
    // Alt+/ comments the entry with `#` in both, and Ctrl+Z takes it back.
    visual.simulate_keystrokes("alt-/"); key(&mut tui, KeyCode::Char('/'), KeyModifiers::ALT);
    let commented = format!("{text}# NEW=\"${{X}} $(id) [a] `b`\"");
    both(&mut visual, &tui, &commented);
    ctrl_z(&mut visual, &mut tui);
    both(&mut visual, &tui, &entry);
    // A paste keeps its CRLF, LF and Unicode byte for byte.
    let pasted = "\r\nTOKEN=\"é😀\\t$HOME\"\nRAW=`id`";
    visual.update(|_, cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string(pasted.into()))); visual.simulate_keystrokes("ctrl-v");
    tui.handle(Event::Paste(pasted.into()));
    let draft = format!("{entry}{pasted}");
    both(&mut visual, &tui, &draft);
    draw(&mut visual, &mut tui, &mut terminal);
    // Language Server Status's details explain the file's handling and show none of its text.
    visual.update(|window, cx| gui.update(cx, |view, cx| view.run(CommandId::ServerDetails, window, cx)));
    tui.run(CommandId::ServerDetails);
    let shown = gui.read_with(&visual, |v, _| v.modal_rows()).into_iter().map(|(row, _)| row).collect::<Vec<_>>().join("\n");
    let Some(crate::app::Prompt::Details { body, .. }) = tui.modal.as_ref().map(|m| &m.kind) else { panic!("no terminal details") };
    let body = body.clone();
    for details in [shown.as_str(), body.as_str()] {
        assert!(details.contains("Environment file: its keys and values are edited as literal text."), "{details}");
        for private in [SECRET, "admin", "MIIB", "TOKEN", "touch ran"] { assert!(!details.contains(private), "{private} in {details}"); }
    }
    visual.simulate_keystrokes("escape"); key(&mut tui, KeyCode::Esc, KeyModifiers::NONE);
    assert!(gui.read_with(&visual, |v, _| v.modal_rows()).is_empty() && tui.modal_title().is_empty());
    // Ctrl+S writes the draft's exact bytes in both; nothing in a value ran.
    visual.simulate_keystrokes("ctrl-s"); key(&mut tui, KeyCode::Char('s'), KeyModifiers::CONTROL);
    let saved = |state: &State| state.active().is_some_and(|t| !state.tab_dirty(t.id()));
    settle(&mut visual, &mut tui, &mut terminal, &saved);
    for root in [&groot, &troot] {
        assert_eq!(std::fs::read(file(root)).unwrap(), draft.as_bytes());
        assert!(!root.join("ran").exists());
    }
}

/// Markdown authoring through each adapter's own keys and pointer: Enter continues and ends a list, `](` and `#`
/// suggest project files and headings (accepted with Enter, dismissed with Escape), each one Undo step; Open
/// Markdown Preview to the Side shows a preview that follows the typing without blanking; Ctrl+click on its links
/// goes to the draft's (unsaved) heading and opens another file at its heading; its context menu offers Show in
/// Source and Follow Link; narrow and tiny sizes draw; Save writes the same bytes in both.
#[gpui::test]
fn markdown_lists_link_completion_and_a_live_preview_match_in_both_adapters(cx: &mut TestAppContext) {
    use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    let dir = tempfile::tempdir().unwrap();
    let make = |name: &str| {
        let root = dir.path().join(name).join("project");
        std::fs::create_dir_all(root.join("docs")).unwrap();
        std::fs::write(root.join("docs/guide.md"), "# Guide\n\n## Install it\n").unwrap();
        std::fs::write(root.join("notes.md"), "# Notes\n\n").unwrap();
        (config(&dir.path().join(name).join("config")), root)
    };
    let ((gc, groot), (tc, troot)) = (make("gui"), make("tui"));
    let notes = |root: &Path| root.join("notes.md");
    let mut core = Session::new(gc); core.dispatch(Command::OpenRoot(groot.clone()));
    let gui = cx.new(|cx| ReaderView::new(core, Some(notes(&groot)), cx));
    let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| gui.clone())).unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    visual.simulate_resize(gpui::size(gpui::px(1200.0), gpui::px(700.0)));
    let focus = gui.read_with(&visual, |view, _| view.focus_handle()); visual.update(|window, cx| focus.focus(window, cx));
    let mut tui = App::new(tc, Some(troot.clone())); tui.session.dispatch(Command::Open(notes(&troot)));
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 30)).unwrap();
    let current = |state: &State| state.current_source().map(|d| d.copy_range(0..d.len_bytes()).unwrap());
    let draw = |visual: &mut VisualTestContext, tui: &mut App, terminal: &mut ratatui::Terminal<ratatui::backend::TestBackend>| {
        visual.update(|window, cx| { let _ = window.draw(cx); }); terminal.draw(|frame| tui.draw(frame)).unwrap();
    };
    let settle = |visual: &mut VisualTestContext, tui: &mut App, terminal: &mut ratatui::Terminal<ratatui::backend::TestBackend>, done: &dyn Fn(&State) -> bool| {
        let until = Instant::now() + Duration::from_secs(20);
        loop {
            gui.update(visual, |view, cx| view.poll(cx)); while let Ok(completion) = tui.session.completions().try_recv() { tui.apply(completion); }
            draw(visual, tui, terminal);
            if gui.read_with(visual, |view, _| done(view.session().state())) && done(tui.session.state()) { break; }
            assert!(Instant::now() < until, "both adapters never settled");
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    // Link suggestions list the project's files: its listing (made off the UI thread) must be complete first.
    let opened = |state: &State| state.current_source().is_some_and(|d| d.source().ends_with("notes.md")) && state.active().is_some_and(|t| t.loaded())
        && state.project().is_some_and(|p| !p.refreshing() && matches!(p.coverage(), code_core::project::Coverage::Complete(_)));
    settle(&mut visual, &mut tui, &mut terminal, &opened);
    gui.update(&mut visual, |view, _| view.enable_draft_input_for_test()); tui.enable_draft_input_for_test();
    let key = |tui: &mut App, code, modifiers| { tui.handle(Event::Key(KeyEvent::new(code, modifiers))); };
    let type_both = |visual: &mut VisualTestContext, tui: &mut App, typed: &str| {
        for c in typed.chars() {
            let stroke = if c == ' ' { "space".to_owned() } else { c.to_string() };
            visual.simulate_keystrokes(&stroke);
            tui.handle(Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)));
        }
    };
    let enter = |visual: &mut VisualTestContext, tui: &mut App| { visual.simulate_keystrokes("enter"); key(tui, KeyCode::Enter, KeyModifiers::NONE); };
    let ctrl_z = |visual: &mut VisualTestContext, tui: &mut App| { visual.simulate_keystrokes("ctrl-z"); key(tui, KeyCode::Char('z'), KeyModifiers::CONTROL); };
    let both = |visual: &mut VisualTestContext, tui: &App, expected: &str| {
        assert_eq!(current(tui.session.state()).as_deref(), Some(expected));
        assert_eq!(gui.read_with(visual, |view, _| current(view.session().state())).as_deref(), Some(expected));
    };
    let labels = |state: &State| state.completion().map(|c| c.rows().map(|r| r.label.clone()).collect::<Vec<_>>()).unwrap_or_default();
    let popup = |visual: &mut VisualTestContext, tui: &mut App, terminal: &mut ratatui::Terminal<ratatui::backend::TestBackend>, expected: &[&str]| {
        draw(visual, tui, terminal);
        assert_eq!(labels(tui.session.state()), expected);
        assert_eq!(gui.read_with(visual, |view, _| labels(view.session().state())), expected);
        assert!(gui.read_with(visual, |view, _| view.completion_bounds().is_some()), "the GUI draws the popup");
        let shown = terminal.backend().buffer().content.iter().map(|c| c.symbol()).collect::<String>();
        assert!(shown.contains(expected[0]), "the terminal draws {:?}", expected[0]);
    };
    let end = "# Notes\n\n".len();
    gui.update(&mut visual, |view, cx| view.dispatch_for_test(Command::Select(code_core::session::Selection { anchor: end, head: end }), cx));
    tui.session.dispatch(Command::Select(code_core::session::Selection { anchor: end, head: end }));
    // Enter continues the list, Enter on its empty item ends it, and Ctrl+Z takes back one step.
    type_both(&mut visual, &mut tui, "- one");
    enter(&mut visual, &mut tui);
    both(&mut visual, &tui, "# Notes\n\n- one\n- ");
    type_both(&mut visual, &mut tui, "two");
    enter(&mut visual, &mut tui);
    enter(&mut visual, &mut tui);
    both(&mut visual, &tui, "# Notes\n\n- one\n- two\n");
    ctrl_z(&mut visual, &mut tui);
    both(&mut visual, &tui, "# Notes\n\n- one\n- two\n- ");
    enter(&mut visual, &mut tui);
    // `](` suggests the folder's entries (folders first); Enter takes a folder and goes on into it.
    type_both(&mut visual, &mut tui, "See [guide](");
    popup(&mut visual, &mut tui, &mut terminal, &["docs/", "notes.md"]);
    enter(&mut visual, &mut tui);
    both(&mut visual, &tui, "# Notes\n\n- one\n- two\nSee [guide](docs/");
    popup(&mut visual, &mut tui, &mut terminal, &["guide.md"]);
    enter(&mut visual, &mut tui);
    // `#` suggests that file's headings, read off the UI thread; typing filters them.
    type_both(&mut visual, &mut tui, "#");
    settle(&mut visual, &mut tui, &mut terminal, &|s: &State| s.completion().is_some());
    popup(&mut visual, &mut tui, &mut terminal, &["guide", "install-it"]);
    type_both(&mut visual, &mut tui, "in");
    popup(&mut visual, &mut tui, &mut terminal, &["install-it"]);
    enter(&mut visual, &mut tui);
    let linked = "# Notes\n\n- one\n- two\nSee [guide](docs/guide.md#install-it";
    both(&mut visual, &tui, linked);
    // Ctrl+Z takes back the accepted heading alone.
    ctrl_z(&mut visual, &mut tui);
    both(&mut visual, &tui, "# Notes\n\n- one\n- two\nSee [guide](docs/guide.md#in");
    for _ in 0..2 { visual.simulate_keystrokes("backspace"); key(&mut tui, KeyCode::Backspace, KeyModifiers::NONE); }
    type_both(&mut visual, &mut tui, "install-it) and [x](");
    // Escape dismisses a popup without touching the text; Backspace leaves it closed.
    popup(&mut visual, &mut tui, &mut terminal, &["docs/", "notes.md"]);
    visual.simulate_keystrokes("escape"); key(&mut tui, KeyCode::Esc, KeyModifiers::NONE);
    draw(&mut visual, &mut tui, &mut terminal);
    assert!(tui.session.state().completion().is_none() && gui.read_with(&visual, |view, _| view.session().state().completion().is_none() && view.completion_bounds().is_none()));
    for _ in 0.." and [x](".len() { visual.simulate_keystrokes("backspace"); key(&mut tui, KeyCode::Backspace, KeyModifiers::NONE); }
    both(&mut visual, &tui, &format!("{linked})"));
    enter(&mut visual, &mut tui);
    enter(&mut visual, &mut tui);
    type_both(&mut visual, &mut tui, "## Later");
    enter(&mut visual, &mut tui);
    enter(&mut visual, &mut tui);
    type_both(&mut visual, &mut tui, "Go [up](#later)");
    visual.simulate_keystrokes("escape"); key(&mut tui, KeyCode::Esc, KeyModifiers::NONE);
    let draft = format!("{linked})\n\n## Later\n\nGo [up](#later)");
    both(&mut visual, &tui, &draft);
    // Open Markdown Preview to the Side through each adapter's command route: the source keeps focus.
    visual.update(|window, cx| gui.update(cx, |view, cx| view.run(CommandId::PreviewBeside, window, cx)));
    tui.run(CommandId::PreviewBeside);
    let beside = |state: &State| state.shown(1).and_then(|t| state.view(t.id())).is_some_and(|v| v.preview_current());
    settle(&mut visual, &mut tui, &mut terminal, &beside);
    let terminal_text = |terminal: &ratatui::Terminal<ratatui::backend::TestBackend>| {
        let buffer = terminal.backend().buffer();
        buffer.content.chunks(buffer.area.width as usize).map(|row| row.iter().map(|c| c.symbol()).collect::<String>()).collect::<Vec<_>>().join("\n")
    };
    draw(&mut visual, &mut tui, &mut terminal);
    assert_eq!((tui.session.state().focused_pane(), gui.read_with(&visual, |view, _| view.session().state().focused_pane())), (0, 0), "the source keeps focus");
    // The GUI shapes its layout off the UI thread: wait for the pane to show it.
    let gui_shows = |visual: &mut VisualTestContext, tui: &mut App, terminal: &mut ratatui::Terminal<ratatui::backend::TestBackend>, text: &str| {
        let until = Instant::now() + Duration::from_secs(20);
        while !gui.read_with(visual, |view, _| view.preview_rows_in(1)).iter().any(|(_, row)| row.contains(text)) {
            assert!(Instant::now() < until, "the GUI preview never showed {text:?}: {:?} / {:?}", gui.read_with(visual, |view, _| view.preview_rows_in(1)), gui.read_with(visual, |view, _| (view.preview_rows_in(0), view.panes.iter().map(|g| (g.pane, g.rows)).collect::<Vec<_>>(), view.session().state().shown(1).and_then(|t| view.session().state().view(t.id())).map(|v| format!("{:?}", v.rendering().map(|r| matches!(r, code_core::session::Rendering::Preview(_))))))));
            gui.update(visual, |view, cx| view.poll(cx));
            draw(visual, tui, terminal);
            visual.run_until_parked();
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    gui_shows(&mut visual, &mut tui, &mut terminal, "Go up");
    assert!(terminal_text(&terminal).contains("Go up"));
    // Typing in the source: the preview beside keeps showing (never blank) and then shows the new text.
    type_both(&mut visual, &mut tui, " now");
    draw(&mut visual, &mut tui, &mut terminal);
    assert!(!gui.read_with(&visual, |view, _| view.preview_rows_in(1)).is_empty(), "the GUI preview stays while the next is made");
    assert!(!terminal_text(&terminal).contains("Preparing Markdown preview"), "the terminal preview stays while the next is made");
    settle(&mut visual, &mut tui, &mut terminal, &|s: &State| beside(s) && s.shown(1).and_then(|t| s.view(t.id())).is_some_and(|v| matches!(v.rendering(), Some(code_core::session::Rendering::Preview(p)) if p.doc.text.contains("Go up now"))));
    // The new version's preview has landed but the GUI has not shaped it yet: the last layout stands in.
    assert!(gui.read_with(&visual, |view, _| view.preview_rows_in(1)).iter().any(|(_, row)| row.contains("Go up")), "the GUI keeps the last layout while shaping the next");
    gui_shows(&mut visual, &mut tui, &mut terminal, "Go up now");
    assert!(terminal_text(&terminal).contains("Go up now"));
    let draft = format!("{draft} now");
    // The preview's context menu offers Show in Source and Follow Link where it was opened.
    let (gx, gy) = gui.read_with(&visual, |view, _| view.preview_point_in(1, "up").unwrap());
    visual.simulate_event(gpui::MouseDownEvent { position: gpui::point(gpui::px(gx), gpui::px(gy)), button: gpui::MouseButton::Right, modifiers: gpui::Modifiers::default(), click_count: 1, first_mouse: false });
    let cell = |terminal: &ratatui::Terminal<ratatui::backend::TestBackend>, tui: &App, word: &str| {
        let area = tui.layout.panes.iter().find(|p| p.pane == 1).unwrap().text;
        let text = terminal_text(terminal);
        let rows: Vec<&str> = text.lines().collect();
        (area.y..area.bottom()).find_map(|y| {
            let row: Vec<char> = rows[y as usize].chars().collect();
            let line: String = row[area.x as usize..area.right() as usize].iter().collect();
            line.find(word).map(|at| (area.x + line[..at].chars().count() as u16 + 1, y))
        }).unwrap_or_else(|| panic!("{word} not in the terminal preview"))
    };
    let (tx, ty) = cell(&terminal, &tui, "up");
    tui.handle(Event::Mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Right), column: tx, row: ty, modifiers: KeyModifiers::NONE }));
    let gui_menu = gui.read_with(&visual, |view, _| view.modal_rows());
    let tui_menu: Vec<_> = tui.choices().into_iter().map(|c| (c.label, c.disabled)).collect();
    for menu in [gui_menu, tui_menu] {
        assert!(menu.contains(&("Show in Source".to_string(), None)) && menu.contains(&("Follow Link".to_string(), None)), "{menu:?}");
    }
    visual.simulate_keystrokes("escape"); key(&mut tui, KeyCode::Esc, KeyModifiers::NONE);
    draw(&mut visual, &mut tui, &mut terminal);
    assert!(gui.read_with(&visual, |v, _| v.modal_rows()).is_empty() && tui.modal_title().is_empty());
    both(&mut visual, &tui, &draft);
    // The rendered preview itself stays read-only: typing and Enter in its pane change nothing.
    assert_eq!((tui.session.state().focused_pane(), gui.read_with(&visual, |view, _| view.session().state().focused_pane())), (1, 1));
    type_both(&mut visual, &mut tui, "x");
    enter(&mut visual, &mut tui);
    both(&mut visual, &tui, &draft);
    // Ctrl+click on `#later`: the caret goes to the draft's unsaved heading, in the source pane beside.
    let (gx, gy) = gui.read_with(&visual, |view, _| view.preview_point_in(1, "up").unwrap());
    visual.simulate_click(gpui::point(gpui::px(gx), gpui::px(gy)), gpui::Modifiers::control());
    let (tx, ty) = cell(&terminal, &tui, "up");
    tui.handle(Event::Mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: tx, row: ty, modifiers: KeyModifiers::CONTROL }));
    tui.handle(Event::Mouse(MouseEvent { kind: MouseEventKind::Up(MouseButton::Left), column: tx, row: ty, modifiers: KeyModifiers::CONTROL }));
    let heading = draft.find("## Later").unwrap();
    let at_heading = move |s: &State| s.focused_pane() == 0 && s.selection().is_some_and(|c| c.head == heading);
    settle(&mut visual, &mut tui, &mut terminal, &at_heading);
    // Ctrl+click on the other link opens that file at its heading, in the preview's pane.
    let (gx, gy) = gui.read_with(&visual, |view, _| view.preview_point_in(1, "guide").unwrap());
    visual.simulate_click(gpui::point(gpui::px(gx), gpui::px(gy)), gpui::Modifiers::control());
    let (tx, ty) = cell(&terminal, &tui, "guide");
    tui.handle(Event::Mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: tx, row: ty, modifiers: KeyModifiers::CONTROL }));
    tui.handle(Event::Mouse(MouseEvent { kind: MouseEventKind::Up(MouseButton::Left), column: tx, row: ty, modifiers: KeyModifiers::CONTROL }));
    let at_guide = |s: &State| s.current_source().is_some_and(|d| d.source().ends_with("docs/guide.md")) && s.selection().is_some_and(|c| c.head == "# Guide\n\n".len());
    settle(&mut visual, &mut tui, &mut terminal, &at_guide);
    // Narrow, tiny and End-then-Up on the remaining panes draw without losing anything.
    for (w, h) in [(30, 8), (1, 1), (120, 30)] {
        terminal.backend_mut().resize(w, h);
        terminal.draw(|frame| tui.draw(frame)).unwrap();
    }
    for (w, h) in [(160.0, 90.0), (1.0, 1.0), (1200.0, 700.0)] {
        visual.simulate_resize(gpui::size(gpui::px(w), gpui::px(h)));
        draw(&mut visual, &mut tui, &mut terminal);
    }
    visual.simulate_keystrokes("end up"); key(&mut tui, KeyCode::End, KeyModifiers::NONE); key(&mut tui, KeyCode::Up, KeyModifiers::NONE);
    draw(&mut visual, &mut tui, &mut terminal);
    // Back in the notes' pane, Ctrl+S writes the same bytes in both.
    gui.update(&mut visual, |view, cx| view.dispatch_for_test(Command::FocusPane(0), cx));
    tui.session.dispatch(Command::FocusPane(0));
    both(&mut visual, &tui, &draft);
    visual.simulate_keystrokes("ctrl-s"); key(&mut tui, KeyCode::Char('s'), KeyModifiers::CONTROL);
    let saved = |state: &State| state.active().is_some_and(|t| !state.tab_dirty(t.id()));
    settle(&mut visual, &mut tui, &mut terminal, &saved);
    for root in [&groot, &troot] {
        assert_eq!(std::fs::read_to_string(notes(root)).unwrap(), draft);
        assert_eq!(std::fs::read_to_string(root.join("docs/guide.md")).unwrap(), "# Guide\n\n## Install it\n");
    }
}

/// Haml markup is edited as Haml through both adapters, even without Ruby LSP: suggestions replace
/// only the tag/attribute name, Enter follows Haml indentation, and each change is one Undo step.
#[gpui::test]
fn haml_markup_and_indentation_match_in_both_adapters(cx: &mut TestAppContext) { haml_replay(cx, false); }

#[gpui::test]
#[ignore = "needs installed ruby-lsp, rubocop, haml and haml_lint; run alone"]
fn real_haml_assistance_matches_in_both_adapters(cx: &mut TestAppContext) { haml_replay(cx, true); }

fn haml_replay(cx: &mut TestAppContext, real: bool) {
    let text = if real { "%di\r\n  %a(hre) 𝄞\r\n  %p= Greeter.gre\r\n  %p= Greeter.greet(\"𝄞\", excited: true)\r\n  %p= 1+2" } else { "%di\r\n  %a(hre) 𝄞\r\n" };
    let dir = tempfile::tempdir().unwrap();
    let make = |name: &str| {
        let root = dir.path().join(name).join("project"); std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("page.html.haml"), text).unwrap();
        std::fs::write(root.join(".editorconfig"), "root = true\n[*.haml]\nindent_size = 2\n").unwrap();
        let c = config(&dir.path().join(name).join("config"));
        if real {
            let server = std::env::var_os("READER_RUBY_LSP").map(std::path::PathBuf::from)
                .or_else(|| code_core::semantics::provider::find_executable(Path::new("ruby-lsp"), None)).expect("install Ruby LSP for this opt-in test");
            std::fs::create_dir_all(root.join("lib")).unwrap();
            std::fs::write(root.join("lib/greeter.rb"), "module Greeter\n  def self.greet(name, excited: false)\n    name\n  end\nend\n").unwrap();
            std::fs::write(root.join("Gemfile"), "source \"https://rubygems.org\"\ngem \"rubocop\"\n").unwrap();
            std::fs::write(root.join(".haml-lint.yml"), "linters:\n  FinalNewline:\n    enabled: false\n").unwrap();
            std::fs::write(root.join(".rubocop.yml"), "AllCops:\n  NewCops: disable\n  SuggestExtensions: false\nStyle/StringLiterals:\n  EnforcedStyle: double_quotes\n").unwrap();
            assert!(std::process::Command::new("bundle").args(["lock", "--local"]).current_dir(&root).output().unwrap().status.success());
            let mut settings: serde_json::Value = serde_json::from_slice(&std::fs::read(&c.providers).unwrap()).unwrap();
            settings["providers"]["ruby-lsp"] = serde_json::json!({"executable": server});
            std::fs::write(&c.providers, settings.to_string()).unwrap();
        }
        (c, root)
    };
    let ((gc, groot), (tc, troot)) = (make("gui"), make("tui"));
    let mut core = Session::new(gc); core.dispatch(Command::OpenRoot(groot.clone()));
    let gui = cx.new(|cx| ReaderView::new(core, Some(groot.join("page.html.haml")), cx));
    let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| gui.clone())).unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let focus = gui.read_with(&visual, |view, _| view.focus_handle()); visual.update(|window, cx| focus.focus(window, cx));
    let mut tui = App::new(tc, Some(troot.clone())); tui.session.dispatch(Command::Open(troot.join("page.html.haml")));
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
    let wait = |visual: &mut VisualTestContext, tui: &mut App, terminal: &mut ratatui::Terminal<ratatui::backend::TestBackend>, done: &dyn Fn(&State) -> bool| {
        let until = Instant::now() + Duration::from_secs(if real { 60 } else { 15 });
        loop {
            gui.update(visual, |view, cx| view.poll(cx)); while let Ok(c) = tui.session.completions().try_recv() { tui.apply(c); }
            visual.update(|window, cx| { let _ = window.draw(cx); }); terminal.draw(|frame| tui.draw(frame)).unwrap();
            if gui.read_with(visual, |view, _| done(view.session().state())) && done(tui.session.state()) { break; }
            assert!(Instant::now() < until, "GUI {:?} {:?}; TUI {:?} {:?}", gui.read_with(visual, |v, _| v.session().state().notices()), gui.read_with(visual, |v, _| v.session().state().servers().to_vec()), tui.session.state().notices(), tui.session.state().servers());
            std::thread::yield_now();
        }
    };
    wait(&mut visual, &mut tui, &mut terminal, &|s| s.current_source().is_some());
    gui.update(&mut visual, |v, _| v.enable_draft_input_for_test()); tui.enable_draft_input_for_test();
    let key = |t: &mut App, code, modifiers| { t.handle(Event::Key(KeyEvent::new(code, modifiers))); };
    let select = |v: &mut VisualTestContext, t: &mut App, at| {
        let c = Command::Select(code_core::session::Selection { anchor: at, head: at });
        gui.update(v, |view, cx| view.dispatch_for_test(c, cx));
        t.session.dispatch(Command::Select(code_core::session::Selection { anchor: at, head: at }));
    };
    let current = |s: &State| s.current_source().map(|d| d.copy_range(0..d.len_bytes()).unwrap());
    let same = |v: &mut VisualTestContext, t: &App, expected: &str| {
        assert_eq!(current(t.session.state()).as_deref(), Some(expected));
        assert_eq!(gui.read_with(v, |view, _| current(view.session().state())).as_deref(), Some(expected));
    };
    select(&mut visual, &mut tui, 3);
    visual.simulate_keystrokes("ctrl-space"); key(&mut tui, KeyCode::Char(' '), KeyModifiers::CONTROL);
    wait(&mut visual, &mut tui, &mut terminal, &|s| s.completion().is_some());
    assert!(gui.read_with(&visual, |v, _| v.completion_bounds().is_some()));
    assert!(terminal.backend().buffer().content().iter().map(|c| c.symbol()).collect::<String>().contains("Haml tag"));
    visual.simulate_keystrokes("enter"); key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    same(&mut visual, &tui, &text.replacen("%di", "%div", 1));
    visual.simulate_keystrokes("ctrl-z"); key(&mut tui, KeyCode::Char('z'), KeyModifiers::CONTROL);
    same(&mut visual, &tui, text);
    select(&mut visual, &mut tui, text.find("hre").unwrap() + 3);
    visual.simulate_keystrokes("ctrl-space"); key(&mut tui, KeyCode::Char(' '), KeyModifiers::CONTROL);
    wait(&mut visual, &mut tui, &mut terminal, &|s| s.completion().is_some());
    visual.simulate_keystrokes("enter"); key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    same(&mut visual, &tui, &text.replace("(hre)", "(href=\"\")"));
    visual.simulate_keystrokes("ctrl-z"); key(&mut tui, KeyCode::Char('z'), KeyModifiers::CONTROL);
    same(&mut visual, &tui, text);
    select(&mut visual, &mut tui, 3);
    visual.simulate_keystrokes("enter"); key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    same(&mut visual, &tui, &text.replacen("%di", "%di\r\n  ", 1));
    visual.simulate_keystrokes("ctrl-z"); key(&mut tui, KeyCode::Char('z'), KeyModifiers::CONTROL);
    same(&mut visual, &tui, text);
    // An old popup cannot apply after changing the draft.
    select(&mut visual, &mut tui, 3);
    visual.simulate_keystrokes("ctrl-space"); key(&mut tui, KeyCode::Char(' '), KeyModifiers::CONTROL);
    visual.simulate_keystrokes("escape"); key(&mut tui, KeyCode::Esc, KeyModifiers::NONE);
    same(&mut visual, &tui, text);
    if real {
        println!("Haml replay: waiting for servers");
        wait(&mut visual, &mut tui, &mut terminal, &|s| s.provider().is_some_and(|p| matches!(p.status, code_core::semantics::Status::Ready)));
        println!("Haml replay: completion");
        // Real Ruby completion, exactly inside the script after non-BMP text and CRLF.
        let at = text.find("Greeter.gre\r").unwrap() + "Greeter.gre".len();
        select(&mut visual, &mut tui, at);
        visual.simulate_keystrokes("ctrl-space"); key(&mut tui, KeyCode::Char(' '), KeyModifiers::CONTROL);
        wait(&mut visual, &mut tui, &mut terminal, &|s| s.completion().is_some_and(|c| c.rows().any(|r| r.label == "greet")));
        visual.simulate_keystrokes("enter"); key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
        let completed = text.replacen("Greeter.gre\r", "Greeter.greet\r", 1);
        wait(&mut visual, &mut tui, &mut terminal, &|s| current(s).as_deref() == Some(completed.as_str()));
        same(&mut visual, &tui, &completed);
        visual.simulate_keystrokes("ctrl-z"); key(&mut tui, KeyCode::Char('z'), KeyModifiers::CONTROL);
        same(&mut visual, &tui, text);
        println!("Haml replay: signature");
        // Real parameter help is placed inside the template's interpolation/call, not scaffolding.
        let at = text.find("excited:").unwrap();
        select(&mut visual, &mut tui, at);
        // Terminals do not reliably distinguish Ctrl+Shift+Space; use their Parameter Info menu action.
        for _ in 0..20 {
            if gui.read_with(&visual, |v, _| v.session().state().signature_help().is_none()) { visual.simulate_keystrokes("ctrl-shift-space"); }
            if tui.session.state().signature_help().is_none() { tui.run(CommandId::ParameterInfo); }
            let until = Instant::now() + Duration::from_secs(2);
            while Instant::now() < until {
                gui.update(&mut visual, |v, cx| v.poll(cx)); while let Ok(c) = tui.session.completions().try_recv() { tui.apply(c); }
                visual.update(|window, cx| { let _ = window.draw(cx); }); terminal.draw(|f| tui.draw(f)).unwrap();
                if gui.read_with(&visual, |v, _| v.session().state().signature_help().is_some()) && tui.session.state().signature_help().is_some() { break; }
                std::thread::sleep(Duration::from_millis(5));
            }
            if gui.read_with(&visual, |v, _| v.session().state().signature_help().is_some()) && tui.session.state().signature_help().is_some() { break; }
        }
        wait(&mut visual, &mut tui, &mut terminal, &|s| s.signature_help().is_some());
        let labels = |s: &State| s.signature_help().map(|h| h.signatures.iter().map(|s| s.label.clone()).collect::<Vec<_>>());
        assert_eq!(labels(tui.session.state()), Some(vec!["greet(name, excited: <default>)".into()]));
        assert_eq!(gui.read_with(&visual, |v, _| labels(v.session().state())), labels(tui.session.state()));
        visual.simulate_keystrokes("escape"); key(&mut tui, KeyCode::Esc, KeyModifiers::NONE);
        println!("Haml replay: format");
        // Format Document uses the project configuration and one Undo restores all original bytes.
        visual.simulate_keystrokes("ctrl-alt-l"); key(&mut tui, KeyCode::Char('l'), KeyModifiers::CONTROL | KeyModifiers::ALT);
        let formatted = text.replace("1+2", "1 + 2");
        wait(&mut visual, &mut tui, &mut terminal, &|s| s.notices().iter().any(|n| n.starts_with("Format Document:")) && s.asking().is_none());
        same(&mut visual, &tui, &formatted);
        visual.simulate_keystrokes("ctrl-z"); key(&mut tui, KeyCode::Char('z'), KeyModifiers::CONTROL);
        same(&mut visual, &tui, text);
    }
    for (width, height) in [(40, 12), (12, 5), (100, 30)] { terminal.backend_mut().resize(width, height); terminal.draw(|f| tui.draw(f)).unwrap(); }
    for root in [&groot, &troot] { assert_eq!(std::fs::read_to_string(root.join("page.html.haml")).unwrap(), text); }
}

/// The real in-process PostgreSQL route behind both adapters: exact native syntax ranges,
/// query-local completion, builtin parameter help and guarded formatting, with one Undo each.
#[gpui::test]
fn postgres_offline_assistance_matches_in_both_adapters(cx: &mut TestAppContext) { sql_offline_replay(cx, false); }

#[gpui::test]
#[ignore = "requires explicit LENZ_TEST_DUCKDB executable; no automatic installation"]
fn native_duckdb_offline_assistance_matches_in_both_adapters(cx: &mut TestAppContext) { sql_offline_replay(cx, true); }

fn sql_offline_replay(cx: &mut TestAppContext, duck: bool) {
    let dialect = if duck { code_core::sql_dialect::Dialect::DuckDb } else { code_core::sql_dialect::Dialect::PostgreSql };
    let text = "-- 𝄞\r\nWITH q AS (SELECT 1 AS id) SELECT q. FROM q;\r\nSELECT date_trunc('day', now());\r\nSELECT 'λ', )";
    let text = if duck { "-- 𝄞\r\nWITH q AS (SELECT * EXCLUDE(hidden) FROM (SELECT 1 AS id, 2 AS hidden) x GROUP BY ALL) SELECT q. FROM q;\r\nSELECT date_trunc('day', now());\r\nSELECT 'λ', )" } else { text };
    let format_text = if duck { "from (select 1 as id, 2 as hidden) q select * exclude(hidden) group by all;" } else { "select 1 as id, $$𝄞; -- data$$ as note;" };
    let dir = tempfile::tempdir().unwrap();
    let make = |name: &str| {
        let root = dir.path().join(name).join("project"); std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("query.sql"), text).unwrap(); std::fs::write(root.join("format.sql"), format_text).unwrap();
        (config(&dir.path().join(name).join("config")), root)
    };
    let ((gc, groot), (tc, troot)) = (make("gui"), make("tui"));
    let mut core = Session::new(gc); core.dispatch(Command::OpenRoot(groot.clone()));
    let gui = cx.new(|cx| ReaderView::new(core, Some(groot.join("query.sql")), cx));
    let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| gui.clone())).unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let focus = gui.read_with(&visual, |v, _| v.focus_handle()); visual.update(|w, cx| focus.focus(w, cx));
    let mut tui = App::new(tc, Some(troot.clone())); tui.session.dispatch(Command::Open(troot.join("query.sql")));
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(110, 32)).unwrap();
    let wait = |visual: &mut VisualTestContext, tui: &mut App, terminal: &mut ratatui::Terminal<ratatui::backend::TestBackend>, done: &dyn Fn(&State) -> bool| {
        let until = Instant::now() + Duration::from_secs(15);
        loop {
            gui.update(visual, |v, cx| v.poll(cx)); while let Ok(c) = tui.session.completions().try_recv() { tui.apply(c); }
            visual.update(|w, cx| { let _ = w.draw(cx); }); terminal.draw(|f| tui.draw(f)).unwrap();
            if gui.read_with(visual, |v, _| done(v.session().state())) && done(tui.session.state()) { break; }
            assert!(Instant::now() < until, "GUI {:?}; TUI {:?}", gui.read_with(visual, |v, _| v.session().state().notices()), tui.session.state().notices());
            std::thread::yield_now();
        }
    };
    wait(&mut visual, &mut tui, &mut terminal, &|s| s.current_source().is_some());
    gui.update(&mut visual, |v, _| v.enable_draft_input_for_test()); tui.enable_draft_input_for_test();
    if duck {
        let path = std::path::PathBuf::from(std::env::var_os("LENZ_TEST_DUCKDB").expect("select an installed native DuckDB"));
        gui.update(&mut visual, |v, cx| v.dispatch_for_test(Command::SetDuckDbExecutable(Some(path.clone())), cx));
        tui.session.dispatch(Command::SetDuckDbExecutable(Some(path)));
    }
    gui.update(&mut visual, |v, cx| v.dispatch_for_test(Command::SetProjectSqlDialect(Some(dialect)), cx));
    tui.run(if duck { CommandId::ProjectSqlDialectDuckDb } else { CommandId::ProjectSqlDialectPostgreSql });
    let current = |s: &State| s.current_source().map(|d| d.copy_range(0..d.len_bytes()).unwrap());
    let both = |visual: &mut VisualTestContext, tui: &App, expected: &str| {
        assert_eq!(current(tui.session.state()).as_deref(), Some(expected));
        assert_eq!(gui.read_with(visual, |v, _| current(v.session().state())).as_deref(), Some(expected));
    };
    let select = |visual: &mut VisualTestContext, tui: &mut App, at| {
        gui.update(visual, |v, cx| v.dispatch_for_test(Command::Select(code_core::session::Selection { anchor: at, head: at }), cx));
        tui.session.dispatch(Command::Select(code_core::session::Selection { anchor: at, head: at }));
    };
    let key = |t: &mut App, code, modifiers| { t.handle(Event::Key(KeyEvent::new(code, modifiers))); };
    let problems = |s: &State| s.active().and_then(|t| s.diagnostics(t.id()));
    wait(&mut visual, &mut tui, &mut terminal, &|s| problems(s).is_some());
    let initial = problems(tui.session.state()).unwrap();
    let expected = if duck {
        let native = code_core::duckdb_assistance::DuckDb::new(std::env::var_os("LENZ_TEST_DUCKDB").unwrap());
        let error = native.diagnose(text).unwrap_err(); let at = error.byte_offset.unwrap(); at..at + error.byte_length.unwrap_or(1)
    } else { text.rfind(')').unwrap()..text.len() };
    assert_eq!(initial.items[0].range, expected);
    assert_eq!(gui.read_with(&visual, |v, _| v.problem_marks()).into_iter().map(|(text, _)| text).collect::<Vec<_>>(), [&text[expected]]);
    assert_eq!(gui.read_with(&visual, |v, _| problems(v.session().state()).unwrap().items.clone()), initial.items);
    select(&mut visual, &mut tui, text.find("q. FROM").unwrap() + 2);
    visual.simulate_keystrokes("ctrl-space"); key(&mut tui, KeyCode::Char(' '), KeyModifiers::CONTROL);
    wait(&mut visual, &mut tui, &mut terminal, &|s| s.completion().is_some());
    assert_eq!(tui.session.state().completion().unwrap().rows().map(|r| r.label.as_str()).collect::<Vec<_>>(), ["id"]);
    assert!(gui.read_with(&visual, |v, _| v.completion_bounds().is_some()));
    assert!(terminal.backend().buffer().content().iter().map(|c| c.symbol()).collect::<String>().contains(&format!("{} offline", dialect.label())));
    visual.simulate_keystrokes("enter"); key(&mut tui, KeyCode::Enter, KeyModifiers::NONE);
    both(&mut visual, &tui, &text.replace("q. FROM", "q.id FROM"));
    visual.simulate_keystrokes("ctrl-z"); key(&mut tui, KeyCode::Char('z'), KeyModifiers::CONTROL);
    both(&mut visual, &tui, text);
    select(&mut visual, &mut tui, text.find("now()").unwrap());
    visual.simulate_keystrokes("ctrl-shift-space"); tui.run(CommandId::ParameterInfo);
    wait(&mut visual, &mut tui, &mut terminal, &|s| s.signature_help().is_some());
    assert!(tui.session.state().signature_help().unwrap().active_signature().unwrap().label.starts_with("date_trunc("));
    visual.simulate_keystrokes("escape"); key(&mut tui, KeyCode::Esc, KeyModifiers::NONE);
    gui.update(&mut visual, |v, cx| v.dispatch_for_test(Command::Open(groot.join("format.sql")), cx));
    tui.session.dispatch(Command::Open(troot.join("format.sql")));
    wait(&mut visual, &mut tui, &mut terminal, &|s| current(s).as_deref() == Some(format_text));
    visual.simulate_keystrokes("ctrl-alt-l"); key(&mut tui, KeyCode::Char('l'), KeyModifiers::CONTROL | KeyModifiers::ALT);
    wait(&mut visual, &mut tui, &mut terminal, &|s| current(s).as_deref() != Some(format_text) && s.asking().is_none());
    let expected = if duck { code_core::duckdb_assistance::DuckDb::new(std::env::var_os("LENZ_TEST_DUCKDB").unwrap()).format(format_text).unwrap() } else { code_core::postgres_assistance::format(format_text).unwrap() };
    both(&mut visual, &tui, &expected);
    visual.simulate_keystrokes("ctrl-z"); key(&mut tui, KeyCode::Char('z'), KeyModifiers::CONTROL);
    both(&mut visual, &tui, format_text);
    select(&mut visual, &mut tui, format_text.len());
    visual.simulate_keystrokes("ctrl-space"); key(&mut tui, KeyCode::Char(' '), KeyModifiers::CONTROL);
    wait(&mut visual, &mut tui, &mut terminal, &|s| s.notices().iter().any(|n| n == "Code completion: no names match at this position"));
    assert!(tui.session.state().completion().is_none());
    assert!(gui.read_with(&visual, |v, _| v.session().state().completion().is_none() && v.completion_bounds().is_none()));
    both(&mut visual, &tui, format_text);
    for (width, height) in [(40, 12), (12, 5), (110, 32)] { terminal.backend_mut().resize(width, height); terminal.draw(|f| tui.draw(f)).unwrap(); }
    for root in [&groot, &troot] {
        assert_eq!(std::fs::read_to_string(root.join("query.sql")).unwrap(), text);
        assert_eq!(std::fs::read_to_string(root.join("format.sql")).unwrap(), format_text);
    }
}

#[gpui::test]
#[ignore = "requires LENZ_TEST_PG_SERVICE and a disposable PostgreSQL catalog fixture"]
fn native_postgres_metadata_controls_match_in_both_adapters(cx: &mut TestAppContext) {
    metadata_replay(cx, false);
}

fn metadata_replay(cx: &mut TestAppContext, duck: bool) {
    let source = if duck { std::env::var("LENZ_TEST_DUCKDB_FILE").unwrap() } else { std::env::var("LENZ_TEST_PG_SERVICE").unwrap() };
    let relation = if duck { "main.lenz_metadata_fixture" } else { "public.lenz_metadata_fixture" };
    let text = format!("-- DROP TABLE {relation}; UPDATE {relation} SET id=0;\r\nSELECT t. FROM {relation} t");
    let dir = tempfile::tempdir().unwrap();
    let make = |name: &str| { let root = dir.path().join(name).join("project"); std::fs::create_dir_all(&root).unwrap(); std::fs::write(root.join("query.sql"), &text).unwrap(); (config(&dir.path().join(name).join("config")),root) };
    let ((gc,groot),(tc,troot)) = (make("gui"),make("tui"));
    let mut core = Session::new(gc); core.dispatch(Command::OpenRoot(groot.clone()));
    let gui = cx.new(|cx| ReaderView::new(core,Some(groot.join("query.sql")),cx));
    let window = cx.update(|cx| cx.open_window(Default::default(),|_,_|gui.clone())).unwrap();
    let mut visual = VisualTestContext::from_window(window.into(),cx);
    let focus = gui.read_with(&visual, |v,_|v.focus_handle()); visual.update(|w,cx|focus.focus(w,cx));
    let mut tui = App::new(tc,Some(troot.clone())); tui.session.dispatch(Command::Open(troot.join("query.sql")));
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(110,32)).unwrap();
    let pump = |visual: &mut VisualTestContext,tui: &mut App,terminal: &mut ratatui::Terminal<ratatui::backend::TestBackend>| {
        gui.update(visual,|v,cx|v.poll(cx)); while let Ok(c)=tui.session.completions().try_recv(){tui.apply(c);}
        visual.update(|w,cx|{let _=w.draw(cx);}); terminal.draw(|f|tui.draw(f)).unwrap();
    };
    let until = Instant::now()+Duration::from_secs(20);
    loop { pump(&mut visual,&mut tui,&mut terminal); if tui.session.state().current_source().is_some() && gui.read_with(&visual,|v,_|v.session().state().current_source().is_some()){break;} assert!(Instant::now()<until); }
    gui.update(&mut visual,|v,_|v.enable_draft_input_for_test()); tui.enable_draft_input_for_test();
    let dialect = if duck {CommandId::ProjectSqlDialectDuckDb}else{CommandId::ProjectSqlDialectPostgreSql};
    let run = |visual: &mut VisualTestContext,tui: &mut App,id|{gui.update_in(visual,|v,w,cx|v.run(id,w,cx));tui.run(id);};
    run(&mut visual,&mut tui,dialect);
    if duck {
        let tool=std::path::PathBuf::from(std::env::var_os("LENZ_TEST_DUCKDB").unwrap());
        gui.update(&mut visual,|v,cx|v.dispatch_for_test(Command::SetDuckDbExecutable(Some(tool.clone())),cx));tui.session.dispatch(Command::SetDuckDbExecutable(Some(tool)));
    }
    run(&mut visual,&mut tui,CommandId::ConfigureSqlMetadata);
    pump(&mut visual,&mut tui,&mut terminal);
    if duck {
        assert!(cx.did_prompt_for_paths());
        cx.simulate_path_prompt_response(|options| { assert!(options.files && !options.directories); Some(vec![std::path::PathBuf::from(&source)]) });
        cx.run_until_parked();
    } else {
        assert!(gui.read_with(&visual,|v,_|v.modal_rows()).iter().any(|(s,_)|s.contains("configures only")));
        visual.simulate_keystrokes("ctrl-s");
        for c in source.chars() { visual.simulate_keystrokes(&c.to_string()); }
        visual.simulate_keystrokes("enter");
    }
    assert!(tui.modal_title().contains("configures only"));
    tui.handle(Event::Key(KeyEvent::new(KeyCode::Char('s'),KeyModifiers::CONTROL)));
    for c in source.chars() { tui.handle(Event::Key(KeyEvent::new(KeyCode::Char(c),KeyModifiers::NONE))); }
    tui.handle(Event::Key(KeyEvent::new(KeyCode::Enter,KeyModifiers::NONE)));
    let until=Instant::now()+Duration::from_secs(15);
    while gui.read_with(&visual,|v,_|v.session().sql_metadata_source().to_owned()) != source { pump(&mut visual,&mut tui,&mut terminal); assert!(Instant::now()<until); }
    assert_eq!(tui.session.sql_metadata_source(),source);
    assert_eq!(gui.read_with(&visual,|v,_|v.session().sql_metadata_source().to_owned()),source);
    assert!(tui.session.sql_metadata_details().contains("configured; disconnected"));
    for id in [CommandId::ConnectSqlMetadata,CommandId::RefreshSqlMetadata] {
        run(&mut visual,&mut tui,id);
        let until=Instant::now()+Duration::from_secs(15);
        loop {pump(&mut visual,&mut tui,&mut terminal);if tui.session.sql_metadata_details().contains("snapshot loaded") && gui.read_with(&visual,|v,_|v.session().sql_metadata_details().contains("snapshot loaded")){break;}assert!(Instant::now()<until,"{}",tui.session.sql_metadata_details());}
    }
    if duck { visual.simulate_keystrokes("escape escape"); }
    let at=text.rfind("t. FROM").unwrap()+2;
    gui.update(&mut visual,|v,cx|v.dispatch_for_test(Command::Select(code_core::session::Selection{anchor:at,head:at}),cx));tui.session.dispatch(Command::Select(code_core::session::Selection{anchor:at,head:at}));
    visual.simulate_keystrokes("ctrl-space");tui.handle(Event::Key(KeyEvent::new(KeyCode::Char(' '),KeyModifiers::CONTROL)));
    let until=Instant::now()+Duration::from_secs(15);
    loop {pump(&mut visual,&mut tui,&mut terminal);if tui.session.state().completion().is_some()&&gui.read_with(&visual,|v,_|v.session().state().completion().is_some()){break;}assert!(Instant::now()<until);}
    assert_eq!(tui.session.state().completion().unwrap().rows().map(|r|r.label.as_str()).collect::<Vec<_>>(),["id","note"]);
    visual.simulate_keystrokes("enter");tui.handle(Event::Key(KeyEvent::new(KeyCode::Enter,KeyModifiers::NONE)));
    let expected=text.replace("t. FROM","t.id FROM");
    assert_eq!(tui.session.state().current_source().unwrap().copy_range(0..expected.len()).unwrap(),expected);
    assert_eq!(gui.read_with(&visual,|v,_|v.session().state().current_source().unwrap().copy_range(0..expected.len()).unwrap()),expected);
    visual.simulate_keystrokes("ctrl-z");tui.handle(Event::Key(KeyEvent::new(KeyCode::Char('z'),KeyModifiers::CONTROL)));
    run(&mut visual,&mut tui,CommandId::DisconnectSqlMetadata);
    run(&mut visual,&mut tui,CommandId::SqlDialect);
    pump(&mut visual,&mut tui,&mut terminal);
    assert!(gui.read_with(&visual,|v,_|v.modal_rows()).iter().any(|(s,_)|s.contains("disconnected")));
    assert!(terminal.backend().buffer().content().iter().map(|c|c.symbol()).collect::<String>().contains("disconnected"));
    for (w,h) in [(12,5),(40,12),(110,32)]{terminal.backend_mut().resize(w,h);pump(&mut visual,&mut tui,&mut terminal);}
    assert_eq!(std::fs::read_to_string(groot.join("query.sql")).unwrap(),text);assert_eq!(std::fs::read_to_string(troot.join("query.sql")).unwrap(),text);
    assert_eq!(tui.session.state().current_source().unwrap().copy_range(0..text.len()).unwrap(),text);
}

#[gpui::test]
#[ignore = "requires LENZ_TEST_DUCKDB and LENZ_TEST_DUCKDB_FILE disposable fixture"]
fn native_duckdb_metadata_controls_match_in_both_adapters(cx: &mut TestAppContext) {
    metadata_replay(cx, true);
}

#[gpui::test]
fn inspect_revert_saved_changes_matches_in_both_adapters(cx:&mut TestAppContext){change_inspector_replay(cx,false);}
#[gpui::test]
fn inspect_revert_head_changes_matches_in_both_adapters(cx:&mut TestAppContext){change_inspector_replay(cx,true);}

fn change_inspector_replay(cx:&mut TestAppContext,tracked:bool){
    use crossterm::event::{MouseButton,MouseEvent,MouseEventKind};
    let original="one\r\nλ two\r\nthree\r\n";let draft="ONE\r\nλ two\r\nTHREE\r\n";
    let dir=tempfile::tempdir().unwrap();
    let make=|name:&str|{
        let root=dir.path().join(name).join("project");std::fs::create_dir_all(&root).unwrap();std::fs::write(root.join("a.txt"),original).unwrap();
        if tracked {lenz_content::git::testing::git(&root,&["init","-b","main"]);lenz_content::git::testing::git(&root,&["config","user.name","Fixture"]);lenz_content::git::testing::git(&root,&["config","user.email","fixture@example.invalid"]);lenz_content::git::testing::commit_all(&root,"base");}
        (config(&dir.path().join(name).join("config")),root)
    };
    let ((gc,groot),(tc,troot))=(make("gui"),make("tui"));
    let index=|root:&Path|std::fs::read(root.join(".git/index")).ok();let gi=index(&groot);let ti=index(&troot);
    let mut core=Session::new(gc);core.dispatch(Command::OpenRoot(groot.clone()));
    let gui=cx.new(|cx|ReaderView::new(core,Some(groot.join("a.txt")),cx));let window=cx.update(|cx|cx.open_window(Default::default(),|_,_|gui.clone())).unwrap();let mut visual=VisualTestContext::from_window(window.into(),cx);
    let focus=gui.read_with(&visual,|v,_|v.focus_handle());visual.update(|w,cx|focus.focus(w,cx));
    let mut tui=App::new(tc,Some(troot.clone()));tui.session.dispatch(Command::OpenRetained(troot.join("a.txt")));
    let mut terminal=ratatui::Terminal::new(ratatui::backend::TestBackend::new(110,32)).unwrap();
    let pump=|visual:&mut VisualTestContext,tui:&mut App,terminal:&mut ratatui::Terminal<ratatui::backend::TestBackend>|{
        gui.update(visual,|v,cx|{v.poll(cx);});
        tui.session.dispatch(code_core::session::Completion::MarkersDue);while let Ok(c)=tui.session.completions().try_recv(){tui.apply(c);}
        visual.update(|w,cx|{let _=w.draw(cx);});terminal.draw(|f|tui.draw(f)).unwrap();
    };
    let wait_step=std::cell::Cell::new(0);
    let wait=|visual:&mut VisualTestContext,tui:&mut App,terminal:&mut ratatui::Terminal<ratatui::backend::TestBackend>,done:&dyn Fn(&State)->bool|{
        wait_step.set(wait_step.get()+1);let until=Instant::now()+Duration::from_secs(15);loop{pump(visual,tui,terminal);if done(tui.session.state())&&gui.read_with(visual,|v,_|done(v.session().state())){break;}assert!(Instant::now()<until,"wait {} GUI {} {:?}; TUI {} {:?} {:?}",wait_step.get(),gui.read_with(visual,|v,_|v.session().status_details()),gui.read_with(visual,|v,_|v.modal_rows()),tui.session.status_details(),tui.modal_title(),tui.session.inspected_change_text());std::thread::yield_now();}
    };
    let settled=|s:&State|s.active().and_then(|t|s.markers(t.id())).is_some_and(|m|!m.pending());
    wait(&mut visual,&mut tui,&mut terminal,&settled);gui.update(&mut visual,|v,_|v.enable_draft_input_for_test());tui.enable_draft_input_for_test();
    let edit=|s:&State,text:&str|{let tab=s.active().unwrap().id();let d=s.current_source().unwrap();Command::Edit{tab,expected:d.key(),edits:vec![code_core::text::TextEdit{range:0..d.len_bytes(),text:text.into()}],kind:code_core::text::EditKind::Replace}};
    let g=gui.read_with(&visual,|v,_|edit(v.session().state(),draft));gui.update(&mut visual,|v,cx|v.dispatch_for_test(g,cx));tui.session.dispatch(edit(tui.session.state(),draft));
    wait(&mut visual,&mut tui,&mut terminal,&settled);
    let both=|visual:&mut VisualTestContext,tui:&App,text:&str|{let current=|s:&State|s.current_source().map(|d|d.copy_range(0..d.len_bytes()).unwrap());assert_eq!(current(tui.session.state()).as_deref(),Some(text));assert_eq!(gui.read_with(visual,|v,_|current(v.session().state())).as_deref(),Some(text));};
    let key=|t:&mut App,c,m|t.handle(Event::Key(KeyEvent::new(c,m)));
    gui.update(&mut visual,|v,cx|{v.dispatch_for_test(Command::Select(code_core::session::Selection{anchor:0,head:0}),cx);v.dispatch_for_test(Command::Scroll(0),cx);});tui.session.dispatch(Command::Select(code_core::session::Selection{anchor:0,head:0}));tui.session.dispatch(Command::Scroll(0));
    let until=Instant::now()+Duration::from_secs(10);
    loop {pump(&mut visual,&mut tui,&mut terminal);if tui.layout.panes[0].rows.first().is_some_and(|(_,line,_)|*line==0) && gui.read_with(&visual,|v,_|v.visible_rows().first().is_some_and(|r|r.number==1)){break;}assert!(Instant::now()<until);}
    // Real gutter clicks open the inspector without changing the caret or document.
    let bounds=gui.read_with(&visual,|v,_|v.panes[0].text);visual.simulate_click(gpui::point(bounds.origin.x+gpui::px(4.),bounds.origin.y+gpui::px(4.)),gpui::Modifiers::default());
    let pane=&tui.layout.panes[0];let (x,y)=(pane.gutter_x+pane.ann,pane.text.y);tui.handle(Event::Mouse(MouseEvent{kind:MouseEventKind::Down(MouseButton::Left),column:x,row:y,modifiers:KeyModifiers::NONE}));
    pump(&mut visual,&mut tui,&mut terminal);
    let label=if tracked{"HEAD ("}else{"last saved"};assert!(tui.session.inspected_change_text().unwrap_or_else(||panic!("No TUI inspector: rows {:?}; {}",tui.layout.panes[0].rows,tui.session.status_details())).contains(label));assert!(gui.read_with(&visual,|v,_|v.modal_rows()).iter().any(|(s,_)|s.contains(label)));
    visual.simulate_keystrokes("ctrl-s");key(&mut tui,KeyCode::Char('s'),KeyModifiers::CONTROL);both(&mut visual,&tui,draft);
    for root in [&groot,&troot]{assert_eq!(std::fs::read_to_string(root.join("a.txt")).unwrap(),original);}
    visual.simulate_keystrokes("escape");key(&mut tui,KeyCode::Esc,KeyModifiers::NONE);
    // Keyboard command, then Show Diff: the comparison edits the existing draft/history.
    visual.simulate_keystrokes("alt-shift-c");key(&mut tui,KeyCode::Char('c'),KeyModifiers::ALT|KeyModifiers::SHIFT);
    visual.simulate_keystrokes("d");key(&mut tui,KeyCode::Char('d'),KeyModifiers::NONE);
    wait(&mut visual,&mut tui,&mut terminal,&|s|s.saved_comparison().is_some_and(|c|!c.pending()));
    assert!(tui.session.state().saved_comparison().unwrap().baseline_label().contains(label));
    visual.simulate_keystrokes("right right right x");for _ in 0..3{key(&mut tui,KeyCode::Right,KeyModifiers::NONE);}key(&mut tui,KeyCode::Char('x'),KeyModifiers::NONE);both(&mut visual,&tui,"ONEx\r\nλ two\r\nTHREE\r\n");
    visual.simulate_keystrokes("ctrl-z");key(&mut tui,KeyCode::Char('z'),KeyModifiers::CONTROL);both(&mut visual,&tui,draft);
    gui.update_in(&mut visual,|v,w,cx|v.run(CommandId::CloseSavedComparison,w,cx));tui.run(CommandId::CloseSavedComparison);
    wait(&mut visual,&mut tui,&mut terminal,&settled);
    gui.update(&mut visual,|v,cx|v.dispatch_for_test(Command::Select(code_core::session::Selection{anchor:0,head:0}),cx));tui.session.dispatch(Command::Select(code_core::session::Selection{anchor:0,head:0}));
    visual.simulate_keystrokes("alt-shift-c");key(&mut tui,KeyCode::Char('c'),KeyModifiers::ALT|KeyModifiers::SHIFT);
    visual.simulate_keystrokes("r");key(&mut tui,KeyCode::Char('r'),KeyModifiers::NONE);wait(&mut visual,&mut tui,&mut terminal,&|s|s.current_source().is_some_and(|d|d.copy_range(0..d.len_bytes()).as_deref()==Some("one\r\nλ two\r\nTHREE\r\n")));both(&mut visual,&tui,"one\r\nλ two\r\nTHREE\r\n");
    visual.simulate_keystrokes("ctrl-z");key(&mut tui,KeyCode::Char('z'),KeyModifiers::CONTROL);both(&mut visual,&tui,draft);
    wait(&mut visual,&mut tui,&mut terminal,&settled);
    gui.update_in(&mut visual,|v,w,cx|v.run(CommandId::InspectChange,w,cx));tui.run(CommandId::InspectChange);
    for (w,h) in [(12,5),(40,12),(110,32)]{terminal.backend_mut().resize(w,h);pump(&mut visual,&mut tui,&mut terminal);}
    visual.simulate_keystrokes("end up escape");key(&mut tui,KeyCode::End,KeyModifiers::NONE);key(&mut tui,KeyCode::Up,KeyModifiers::NONE);key(&mut tui,KeyCode::Esc,KeyModifiers::NONE);
    visual.simulate_keystrokes("ctrl-s");key(&mut tui,KeyCode::Char('s'),KeyModifiers::CONTROL);
    wait(&mut visual,&mut tui,&mut terminal,&|s|s.tabs().iter().all(|t|!s.tab_dirty(t.id())));
    for root in [&groot,&troot]{assert_eq!(std::fs::read_to_string(root.join("a.txt")).unwrap(),draft);}assert_eq!(index(&groot),gi);assert_eq!(index(&troot),ti);
}

#[gpui::test]
fn home_end_and_document_bounds_edit_and_select_alike_in_both_adapters(cx:&mut TestAppContext) {
    let dir=tempfile::tempdir().unwrap();let text="α😀 first\r\n  λ second\nlast";
    let gp=dir.path().join("gui.txt");let tp=dir.path().join("tui.txt");for p in [&gp,&tp]{std::fs::write(p,text).unwrap();}
    let gui=cx.new(|cx|ReaderView::new(Session::new(config(&dir.path().join("g"))),Some(gp.clone()),cx));
    let window=cx.update(|cx|cx.open_window(Default::default(),|_,_|gui.clone())).unwrap();let mut visual=VisualTestContext::from_window(window.into(),cx);
    let focus=gui.read_with(&visual,|v,_|v.focus_handle());visual.update(|w,cx|focus.focus(w,cx));
    let mut tui=App::new(config(&dir.path().join("t")),None);tui.session.dispatch(Command::OpenRetained(tp.clone()));
    let mut terminal=ratatui::Terminal::new(ratatui::backend::TestBackend::new(90,24)).unwrap();
    let pump=|visual:&mut VisualTestContext,tui:&mut App,terminal:&mut ratatui::Terminal<ratatui::backend::TestBackend>|{gui.update(visual,|v,cx|v.poll(cx));while let Ok(c)=tui.session.completions().try_recv(){tui.apply(c);}visual.update(|w,cx|{let _=w.draw(cx);});terminal.draw(|f|tui.draw(f)).unwrap();};
    let until=Instant::now()+Duration::from_secs(15);loop{pump(&mut visual,&mut tui,&mut terminal);if tui.session.state().current_source().is_some()&&gui.read_with(&visual,|v,_|v.session().state().current_source().is_some()){break;}assert!(Instant::now()<until);}
    gui.update(&mut visual,|v,_|v.enable_draft_input_for_test());tui.enable_draft_input_for_test();
    let start=text.find("  λ").unwrap();let end=text.find("\nlast").unwrap();
    let select=Command::Select(code_core::session::Selection{anchor:start+4,head:start+4});gui.update(&mut visual,|v,cx|v.dispatch_for_test(select.clone(),cx));tui.session.dispatch(select);
    for (keys,code,modifiers,anchor,head) in [
        ("home",KeyCode::Home,KeyModifiers::NONE,start,start),
        ("shift-end",KeyCode::End,KeyModifiers::SHIFT,start,end),
        ("ctrl-home",KeyCode::Home,KeyModifiers::CONTROL,0,0),
        ("ctrl-shift-end",KeyCode::End,KeyModifiers::CONTROL|KeyModifiers::SHIFT,0,text.len()),
    ] {
        visual.simulate_keystrokes(keys);tui.handle(Event::Key(KeyEvent::new(code,modifiers)));pump(&mut visual,&mut tui,&mut terminal);
        let expected=Some(code_core::session::Selection{anchor,head});assert_eq!(tui.session.state().selection(),expected);assert_eq!(gui.read_with(&visual,|v,_|v.session().state().selection()),expected);
    }
    assert_eq!(copied_selection(tui.session.state()).as_deref(),Some(text));
    visual.simulate_keystrokes("x");tui.handle(Event::Key(KeyEvent::new(KeyCode::Char('x'),KeyModifiers::NONE)));
    assert_eq!(snapshot(tui.session.state()).text.as_deref(),Some("x"));assert_eq!(gui.read_with(&visual,|v,_|snapshot(v.session().state())).text.as_deref(),Some("x"));
    visual.simulate_keystrokes("ctrl-z");tui.handle(Event::Key(KeyEvent::new(KeyCode::Char('z'),KeyModifiers::CONTROL)));
    assert_eq!(snapshot(tui.session.state()).text.as_deref(),Some(text));assert_eq!(gui.read_with(&visual,|v,_|snapshot(v.session().state())).text.as_deref(),Some(text));
    assert_eq!(std::fs::read_to_string(gp).unwrap(),text);assert_eq!(std::fs::read_to_string(tp).unwrap(),text);
}

#[gpui::test]
fn bulk_tab_closure_cancel_and_discard_match_in_both_adapters(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let gui = cx.new(|cx| ReaderView::new(Session::new(config(&dir.path().join("gui"))), None, cx));
    let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| gui.clone())).unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let focus = gui.read_with(&visual, |v, _| v.focus_handle());
    visual.update(|w, cx| focus.focus(w, cx));
    let mut tui = App::new(config(&dir.path().join("tui")), None);
    gui.update(&mut visual, |v, _| v.enable_draft_input_for_test());
    tui.enable_draft_input_for_test();
    let key = |tui: &mut App, code, modifiers| { tui.handle(Event::Key(KeyEvent::new(code, modifiers))); };
    let both = |visual: &VisualTestContext, tui: &App| {
        let state = snapshot(tui.session.state());
        assert_eq!(state, gui.read_with(visual, |v, _| snapshot(v.session().state())));
        state
    };
    for text in ["first draft", "second draft"] {
        visual.simulate_keystrokes("ctrl-n"); key(&mut tui, KeyCode::Char('n'), KeyModifiers::CONTROL);
        visual.update(|_, cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string(text.into())));
        visual.simulate_keystrokes("ctrl-v"); tui.handle(Event::Paste(text.into()));
    }
    let close_all = |visual: &mut VisualTestContext, tui: &mut App| {
        visual.simulate_keystrokes("f10");
        let at = gui.read_with(visual, |v, _| v.modal_rows().iter().position(|r| r.0 == CommandId::CloseAllTabs.label()).unwrap());
        for _ in 0..at { visual.simulate_keystrokes("down"); }
        visual.simulate_keystrokes("enter");
        key(tui, KeyCode::F(10), KeyModifiers::NONE); tui.handle(Event::Paste("Close All Tabs".into()));
        key(tui, KeyCode::Enter, KeyModifiers::NONE);
    };
    both(&visual, &tui);
    close_all(&mut visual, &mut tui);
    visual.simulate_keystrokes("d"); key(&mut tui, KeyCode::Char('d'), KeyModifiers::NONE);
    assert_eq!(tui.session.state().tabs().len(), 2, "early Discard closes no tab");
    visual.simulate_keystrokes("escape"); key(&mut tui, KeyCode::Esc, KeyModifiers::NONE);
    assert_eq!(both(&visual, &tui).text.as_deref(), Some("second draft"));
    close_all(&mut visual, &mut tui);
    for _ in 0..2 { visual.simulate_keystrokes("d"); key(&mut tui, KeyCode::Char('d'), KeyModifiers::NONE); }
    both(&visual, &tui);
    assert!(tui.session.state().tabs().is_empty());
}

#[gpui::test]
fn results_panel_hide_show_stepping_and_history_match_through_keys(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("proj");
    std::fs::create_dir_all(root.join("src")).unwrap();
    for (name, text) in [("src/a.rs", "needle\n"), ("src/b.rs", "x\nneedle\n"), ("c.rs", "x\n")] { std::fs::write(root.join(name), text).unwrap(); }
    let gui = cx.new(|cx| ReaderView::new(Session::new(config(&dir.path().join("gui"))), Some(root.clone()), cx));
    let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| gui.clone())).unwrap();
    let mut visual = VisualTestContext::from_window(window.into(), cx); let focus = gui.read_with(&visual, |v, _| v.focus_handle()); visual.update(|w, cx| focus.focus(w, cx));
    let mut tui = App::new(config(&dir.path().join("tui")), Some(root.clone()));
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 32)).unwrap();
    let mut settle = |visual: &mut VisualTestContext, tui: &mut App, done: &dyn Fn(&State) -> bool| {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !(gui.read_with(visual, |v, _| done(v.session().state())) && done(tui.session.state())) {
            gui.update(visual, |v, cx| v.poll(cx)); while let Ok(c) = tui.session.completions().try_recv() { tui.apply(c); }
            assert!(Instant::now() < deadline, "did not settle"); std::thread::sleep(Duration::from_millis(2));
        }
        visual.update(|w, cx| { let _ = w.draw(cx); }); terminal.draw(|f| tui.draw(f)).unwrap();
    };
    // What both adapters must agree on: the list shown, its selection, the history around it and the open file.
    let state = |s: &State| (s.results().map(|r| (r.query().to_owned(), r.selected(), r.hits().len())), s.active().map(|t| t.title().to_owned()));
    let same = |visual: &VisualTestContext, tui: &App| { let t = state(tui.session.state()); assert_eq!(t, gui.read_with(visual, |v, _| state(v.session().state()))); t };
    let key = |tui: &mut App, code, modifiers| { tui.handle(Event::Key(KeyEvent::new(code, modifiers))); };
    settle(&mut visual, &mut tui, &|s| s.project().is_some_and(|p| matches!(p.coverage(), code_core::project::Coverage::Complete(_))));
    let search = |visual: &mut VisualTestContext, tui: &mut App, q: &str| {
        gui.update(visual, |v, cx| { v.dispatch_for_test(Command::Search { query: q.into(), scope: code_core::session::SearchIn::Project }, cx); v.results_focus = true; });
        tui.dispatch(Command::Search { query: q.into(), scope: code_core::session::SearchIn::Project }); tui.focus = crate::app::Focus::Results;
    };
    search(&mut visual, &mut tui, "needle");
    settle(&mut visual, &mut tui, &|s| s.results().is_some_and(|r| r.done() && r.preview().is_some()));
    visual.simulate_keystrokes("down"); key(&mut tui, KeyCode::Down, KeyModifiers::NONE);
    while tui.session.state().results().unwrap().selected() == Some(0) { key(&mut tui, KeyCode::Down, KeyModifiers::NONE); }
    assert_eq!(same(&visual, &tui).0, Some(("needle".into(), Some(1), 2)));
    // Escape hides, Ctrl+J shows the same list and selection again.
    visual.simulate_keystrokes("escape"); key(&mut tui, KeyCode::Esc, KeyModifiers::NONE);
    assert_eq!(same(&visual, &tui).0, None);
    visual.simulate_keystrokes("ctrl-j"); key(&mut tui, KeyCode::Char('j'), KeyModifiers::CONTROL);
    assert_eq!(same(&visual, &tui).0, Some(("needle".into(), Some(1), 2)));
    // The terminal's highlight is back on that result, so Enter opens it rather than folding a folder.
    let r = tui.session.state().results().unwrap();
    assert_eq!(r.row_of_hit(1), Some(tui.results_at));
    // F4 opens the next result (wrapping), Shift+F4 the one before.
    visual.simulate_keystrokes("f4"); key(&mut tui, KeyCode::F(4), KeyModifiers::NONE);
    settle(&mut visual, &mut tui, &|s| s.active().is_some_and(|t| t.title() == "a.rs"));
    visual.simulate_keystrokes("shift-f4"); key(&mut tui, KeyCode::F(4), KeyModifiers::SHIFT);
    settle(&mut visual, &mut tui, &|s| s.active().is_some_and(|t| t.title() == "b.rs"));
    same(&visual, &tui);
    // Another search keeps this list: Alt+Left in the panel brings it back with its selection, Alt+Right the later one.
    search(&mut visual, &mut tui, "x");
    settle(&mut visual, &mut tui, &|s| s.results().is_some_and(|r| r.done() && r.query() == "x"));
    visual.simulate_keystrokes("alt-left"); key(&mut tui, KeyCode::Left, KeyModifiers::ALT);
    assert_eq!(same(&visual, &tui).0, Some(("needle".into(), Some(1), 2)));
    visual.simulate_keystrokes("alt-right"); key(&mut tui, KeyCode::Right, KeyModifiers::ALT);
    assert_eq!(same(&visual, &tui).0.map(|r| r.0), Some("x".to_owned()));
    for (name, text) in [("src/a.rs", "needle\n"), ("src/b.rs", "x\nneedle\n"), ("c.rs", "x\n")] { assert_eq!(std::fs::read(root.join(name)).unwrap(), text.as_bytes()); }
}
