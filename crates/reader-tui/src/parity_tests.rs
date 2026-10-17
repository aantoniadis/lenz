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
        text: s.document().map(|d| d.text().to_string()),
        selection: s.selection(),
        selected: s.selected_text().map(str::to_owned),
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
                    && s.document().is_some()
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
    assert_eq!(tui.session.state().selected_text(), Some(original));

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
    ] {
        gui.update_in(&mut visual, |v, window, cx| v.run(id, window, cx));
        tui.run(id);
        settle(&mut visual, &mut tui);
    }
    // A pending navigation can be cancelled in either adapter without changing the read-only document.
    tui.dispatch(Command::CancelSemantic);
    assert_eq!(std::fs::read(&path).unwrap(), original.as_bytes());
}
