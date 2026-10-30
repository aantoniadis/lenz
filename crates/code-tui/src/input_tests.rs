use super::*;
#[path = "input_tests/terminal.rs"]
#[cfg(target_os = "linux")]
mod terminal;
#[path = "input_tests/tab_wheel.rs"]
mod tab_wheel;
#[path = "input_tests/create.rs"]
mod create;
#[path = "input_tests/activity.rs"]
mod activity;
#[path = "input_tests/server_install.rs"]
mod server_install;
#[path = "input_tests/external.rs"]
mod external;
#[path = "input_tests/find_replace.rs"]
mod find_replace;
#[path = "input_tests/save_as.rs"]
mod save_as;
#[path = "input_tests/recovery.rs"]
mod recovery;
#[path = "input_tests/reconcile.rs"]
mod reconcile;
#[path = "input_tests/keymap.rs"]
mod keymap;
#[path = "input_tests/sql_dialect.rs"]
mod sql_dialect;
#[path = "input_tests/annotation_width.rs"]
mod annotation_width;
#[path = "input_tests/csv.rs"]
mod csv;
#[path = "input_tests/problems.rs"]
mod problems;
#[path = "input_tests/project_syntax.rs"]
mod project_syntax;
use crossterm::event::{KeyEvent, KeyModifiers};
use lenz_content::theme::{ThemeOverrides, ThemePaths};
use std::{
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

fn copied_selection(state: &code_core::session::State) -> Option<String> {
    state.view(state.active()?.id())?.copy_selection()
}

pub(crate) fn config(dir: &Path) -> Config {
    Config {
        themes: ThemePaths::under(&dir.join("config"), &dir.join("state"), "lenz-code"),
        recent: dir.join("state/recent.json"),
        settings: dir.join("config/settings.json"),
        providers: dir.join("config/providers.json"),
        cache: dir.join("cache"),
        idle: Duration::ZERO,
        clock: Arc::new(Instant::now),
        overrides: ThemeOverrides::default(),
        dark: true,
    }
}
fn app(dir: &Path) -> App {
    // Reader-shortcut scenarios explicitly model a backend without validated text input.
    App::with_text_input(config(dir), Some(dir.to_owned()), false)
}

fn key(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
    app.handle(Event::Key(KeyEvent::new(code, modifiers)));
}
pub(crate) fn settle(app: &mut App, done: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done(app) {
        assert!(Instant::now() < deadline, "background work timed out; loading {:?}; tabs {:?}; recovery offers {}; prompt {:?}; modal {}", app.session.state().loading(), app.session.state().tabs().iter().map(|t| (t.title(), t.loaded())).collect::<Vec<_>>(), app.session.recovered_drafts().len(), app.session.recovery_prompt(), app.modal_title());
        if let Ok(c) = app.session.completions().try_recv() {
            app.apply(c);
        } else {
            std::thread::yield_now();
        }
    }
}

#[cfg(target_os = "linux")]
#[test]
fn linux_default_constructor_edits_and_saves_without_test_opt_in() {
    let dir=tempfile::tempdir().unwrap();let path=dir.path().join("default.txt");
    std::fs::write(&path,b"saved\r\n").unwrap();
    let mut a=App::new(config(dir.path()),Some(path.clone()));
    settle(&mut a,|a|a.session.state().current_source().is_some());draw(&mut a,80,16);
    assert_eq!(a.session.disabled(CommandId::Paste),None,"Linux ordinary constructor enables validated text input");
    key(&mut a,KeyCode::Char('a'),KeyModifiers::CONTROL);
    key(&mut a,KeyCode::Char('q'),KeyModifiers::NONE);
    a.handle(Event::Paste("λ😀\r\npaste".into()));draw(&mut a,80,16);
    let source=a.session.state().current_source().unwrap();
    assert_eq!(source.copy_range(0..source.len_bytes()).unwrap(),"qλ😀\r\npaste");
    assert!(!a.quit && a.modal.is_none());
    assert_eq!(std::fs::read(&path).unwrap(),b"saved\r\n");
    key(&mut a,KeyCode::Char('s'),KeyModifiers::CONTROL);
    settle(&mut a,|a|!a.session.state().tab_dirty(a.session.state().active().unwrap().id()));
    assert_eq!(std::fs::read(&path).unwrap(),"qλ😀\r\npaste".as_bytes());
    assert_eq!(a.session.disabled(CommandId::Undo),None);
}

#[test]
fn explicit_read_only_editor_input_cannot_fall_through_to_reading_shortcuts() {
    let dir = tempfile::tempdir().unwrap(); let path = dir.path().join("readonly.txt"); let original = "e\u{301}👩🏽‍💻\r\n"; std::fs::write(&path, original).unwrap();
    let mut a = app(dir.path()); a.dispatch(Command::OpenRetained(path.clone())); settle(&mut a, |a| a.session.state().current_source().is_some()); a.enable_draft_input_for_test(); draw(&mut a, 80, 16);
    a.run(CommandId::ReadOnlyMode);
    for c in ['q', 'j', 'k', '/', ' '] { key(&mut a, KeyCode::Char(c), KeyModifiers::NONE); }
    assert!(!a.quit, "read-only editor input must not become Quit");
    assert!(a.modal.is_none(), "slash must not become Find");
    let source = a.session.state().current_source().unwrap(); assert_eq!(source.copy_range(0..source.len_bytes()).unwrap(), original);
    assert!(a.session.status_details().contains("Read-only mode is on"));
    assert_eq!(std::fs::read(path).unwrap(), original.as_bytes());
}

#[test]
fn unicode_delete_cut_and_bracketed_paste_use_shared_transactions() {
    let dir = tempfile::tempdir().unwrap(); let path = dir.path().join("typing.txt"); let original = "é😀\r\nsecond"; std::fs::write(&path, original).unwrap();
    let mut a = app(dir.path()); a.dispatch(Command::OpenRetained(path.clone())); settle(&mut a, |a| a.session.state().current_source().is_some()); draw(&mut a, 80, 16);
    let text = |a: &App| { let source = a.session.state().current_source().unwrap(); source.copy_range(0..source.len_bytes()).unwrap() };
    key(&mut a, KeyCode::Right, KeyModifiers::NONE); key(&mut a, KeyCode::Delete, KeyModifiers::NONE); assert_eq!(text(&a), original);
    a.enable_draft_input_for_test(); key(&mut a, KeyCode::Delete, KeyModifiers::NONE); assert_eq!(text(&a), "é\r\nsecond");
    key(&mut a, KeyCode::Char('z'), KeyModifiers::CONTROL); assert_eq!(text(&a), original);
    key(&mut a, KeyCode::Backspace, KeyModifiers::NONE); assert_eq!(text(&a), "é\r\nsecond");
    key(&mut a, KeyCode::Char('a'), KeyModifiers::CONTROL); key(&mut a, KeyCode::Char('x'), KeyModifiers::CONTROL); assert_eq!(text(&a), ""); assert_eq!(a.copy.as_deref(), Some("é\r\nsecond"));
    a.handle(Event::Paste(a.copy.clone().unwrap())); assert_eq!(text(&a), "é\r\nsecond");
    key(&mut a, KeyCode::Char('z'), KeyModifiers::CONTROL); assert_eq!(text(&a), "");
    key(&mut a, KeyCode::Char('z'), KeyModifiers::CONTROL | KeyModifiers::SHIFT); assert_eq!(text(&a), "é\r\nsecond");
    assert_eq!(std::fs::read(path).unwrap(), original.as_bytes());
}

#[test]
fn draft_refresh_and_reload_use_real_focus_keys_and_preserve_modal_precedence() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("refresh.txt");
    std::fs::write(&path, b"saved\r\n").unwrap();
    let mut a = app(d.path());
    a.dispatch(Command::OpenRetained(path.clone()));
    settle(&mut a, |a| a.session.state().current_source().is_some());
    a.enable_draft_input_for_test();
    draw(&mut a, 80, 12);
    key(&mut a, KeyCode::Char('a'), KeyModifiers::CONTROL);
    a.handle(Event::Paste("draft λ\r\n".into()));
    let position = a.session.state().selection();
    let ordinary = a.session.state().active().unwrap().id();
    key(
        &mut a,
        KeyCode::Char('d'),
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    );
    settle(&mut a, |a| {
        a.session
            .state()
            .saved_comparison()
            .is_some_and(|c| !c.pending())
    });
    draw(&mut a, 80, 12);
    key(&mut a, KeyCode::F(5), KeyModifiers::NONE);
    assert!(a.session.close_prompt().is_some() && a.session.state().saved_comparison().is_none());
    assert_eq!(a.session.state().active().unwrap().id(), ordinary);
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    assert_eq!(a.session.state().selection(), position);
    std::fs::write(&path, b"external\r\n").unwrap();
    a.handle(Event::FocusGained);
    settle(&mut a, |a| {
        a.session.state().active().unwrap().stale_on_disk()
    });
    assert!(draw_screen(&mut a, 100, 12).contains("changed on disk"));
    assert_eq!(a.session.state().selection(), position);
    key(&mut a, KeyCode::F(5), KeyModifiers::NONE);
    assert!(draw_screen(&mut a, 80, 12).contains("Unsaved changes"));
    key(&mut a, KeyCode::Char('s'), KeyModifiers::CONTROL);
    a.handle(Event::Paste("ignored".into()));
    key(&mut a, KeyCode::End, KeyModifiers::NONE);
    key(&mut a, KeyCode::Up, KeyModifiers::NONE);
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    assert_eq!(a.session.state().selection(), position);
    assert_eq!(std::fs::read(&path).unwrap(), b"external\r\n");
    for (w, h) in [(1, 1), (24, 6)] {
        draw(&mut a, w, h);
        key(&mut a, KeyCode::F(5), KeyModifiers::NONE);
        key(&mut a, KeyCode::End, KeyModifiers::NONE);
        key(&mut a, KeyCode::Up, KeyModifiers::NONE);
        draw(&mut a, w, h);
        key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    }
    draw(&mut a, 80, 12);
    key(&mut a, KeyCode::F(5), KeyModifiers::NONE);
    key(&mut a, KeyCode::Char('d'), KeyModifiers::NONE);
    settle(&mut a, |a| a.session.state().loading().is_none());
    key(&mut a, KeyCode::Up, KeyModifiers::NONE);
    let screen = draw_screen(&mut a, 80, 12);
    assert!(
        screen.contains("external"),
        "{screen}\nstatus: {}",
        a.session.status_details()
    );
    let s = a.session.state().current_source().unwrap();
    assert_eq!(
        s.copy_range(0..s.len_bytes()).as_deref(),
        Some("external\r\n")
    );
    assert!(a.modal.is_none());
    assert_eq!(std::fs::read(path).unwrap(), b"external\r\n");
}

#[test]
fn draft_workspace_changes_use_real_folder_branch_and_new_branch_controls() {
    use lenz_content::git::testing::git;
    let (r, wt, mut a) = branches_app();
    let path = r.path().join("a.txt");
    a.dispatch(Command::OpenRetained(path.clone()));
    settle(&mut a, |a| a.session.state().current_source().is_some());
    a.enable_draft_input_for_test();
    draw(&mut a, 100, 20);
    key(&mut a, KeyCode::Char('a'), KeyModifiers::CONTROL);
    a.handle(Event::Paste("draft λ\r\n".into()));
    let position = a.session.state().selection();
    key(&mut a, KeyCode::F(10), KeyModifiers::NONE);
    draw(&mut a, 100, 20);
    let row = a
        .choices()
        .iter()
        .position(|r| matches!(r.target, ChoiceTarget::Action(CommandId::OpenFolder)))
        .unwrap();
    for _ in 0..row {
        key(&mut a, KeyCode::Down, KeyModifiers::NONE);
    }
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    key(&mut a, KeyCode::Char('a'), KeyModifiers::CONTROL);
    a.handle(Event::Paste(wt.path().join("second").display().to_string()));
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    assert!(draw_screen(&mut a, 100, 20).contains("Unsaved changes"));
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    assert_eq!(a.session.state().project().unwrap().root(), r.path());
    assert_eq!(a.session.state().selection(), position);
    for new_branch in [false, true] {
        key(
            &mut a,
            KeyCode::Char('`'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        );
        settle(&mut a, |a| {
            a.session.state().branches().is_some_and(|b| !b.loading)
        });
        draw(&mut a, 100, 20);
        if new_branch {
            typed(&mut a, "new");
            key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
            typed(&mut a, "draft-guard");
            key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
        } else {
            typed(&mut a, "other");
            key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
        }
        assert!(draw_screen(&mut a, 100, 20).contains("Unsaved changes"));
        assert_eq!(git(r.path(), &["branch", "--show-current"]).trim(), "main");
        key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
        assert_eq!(a.session.state().selection(), position);
        assert_eq!(std::fs::read(&path).unwrap(), b"main\n");
    }
    key(
        &mut a,
        KeyCode::Char('`'),
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    );
    settle(&mut a, |a| {
        a.session.state().branches().is_some_and(|b| !b.loading)
    });
    draw(&mut a, 100, 20);
    typed(&mut a, "other");
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    key(&mut a, KeyCode::Char('d'), KeyModifiers::NONE);
    settle(&mut a, |a| {
        !a.session.state().checking_out()
            && a.session
                .state()
                .current_source()
                .is_some_and(|s| s.copy_range(0..s.len_bytes()).as_deref() == Some("other\n"))
    });
    assert!(draw_screen(&mut a, 100, 20).contains("other"));
    assert_eq!(git(r.path(), &["branch", "--show-current"]).trim(), "other");
    assert_eq!(std::fs::read(path).unwrap(), b"other\n");
    assert!(a.modal.is_none());
}

#[test]
fn saved_comparison_uses_real_keys_shared_history_and_read_only_rendering() {
    let dir = tempfile::tempdir().unwrap(); let path = dir.path().join("comparison.txt");
    let original = "α saved\r\nsecond\n"; std::fs::write(&path, original).unwrap();
    let mut a = app(dir.path()); a.dispatch(Command::OpenRetained(path.clone()));
    settle(&mut a, |a| a.session.state().current_source().is_some()); a.enable_draft_input_for_test(); draw(&mut a, 100, 16);
    let ordinary = a.session.state().active().unwrap().id();
    key(&mut a, KeyCode::Char('a'), KeyModifiers::CONTROL); a.handle(Event::Paste("α draft\r\nsecond\n".into()));
    key(&mut a, KeyCode::F(2), KeyModifiers::SHIFT);
    key(&mut a, KeyCode::Char('d'), KeyModifiers::CONTROL | KeyModifiers::SHIFT);
    assert!(a.session.state().saved_comparison().is_none()); key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    key(&mut a, KeyCode::Char('d'), KeyModifiers::CONTROL | KeyModifiers::SHIFT);
    settle(&mut a, |a| a.session.state().saved_comparison().is_some_and(|c| !c.pending()));
    let (saved, current) = { let c = a.session.state().saved_comparison().unwrap(); (c.saved_tab(), c.current_tab()) };
    key(&mut a, KeyCode::Char('g'), KeyModifiers::CONTROL); key(&mut a, KeyCode::Char('1'), KeyModifiers::NONE); key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    let screen = draw_screen(&mut a, 100, 16); assert!(screen.contains("Last saved"), "{screen}"); assert!(screen.contains("Current draft"), "{screen}"); assert!(screen.contains('▌'), "{screen}");
    for theme in ["github-light", "darcula"] {
        a.dispatch(Command::Theme(lenz_content::theme::ThemeCommand::Select(lenz_content::theme::ThemeSelection::from_id(theme).unwrap())));
        assert!(draw_screen(&mut a, 100, 16).contains('▌'));
    }
    key(&mut a, KeyCode::Char('z'), KeyModifiers::CONTROL); key(&mut a, KeyCode::Char('z'), KeyModifiers::CONTROL | KeyModifiers::SHIFT);
    let rect = a.layout.panes.iter().flat_map(|pane| pane.tabs.iter()).find(|(id, _)| *id == saved).unwrap().1;
    click(&mut a, rect.x + rect.width / 2, rect.y);
    assert!(a.session.state().saved_comparison().is_some() && a.session.state().active().unwrap().id() == saved);
    key(&mut a, KeyCode::Char('a'), KeyModifiers::CONTROL); a.handle(Event::Paste("forbidden".into())); key(&mut a, KeyCode::Char('z'), KeyModifiers::CONTROL); key(&mut a, KeyCode::Char('s'), KeyModifiers::CONTROL);
    let snapshot = a.session.state().view(saved).unwrap().snapshot(); assert_eq!(snapshot.copy_range(0..snapshot.len_bytes()).unwrap(), original);
    a.dispatch(Command::FocusPane(1)); key(&mut a, KeyCode::Char('s'), KeyModifiers::CONTROL);
    settle(&mut a, |a| !a.session.state().view(current).unwrap().dirty() && !a.session.state().saved_comparison().unwrap().pending()); assert_eq!(std::fs::read(&path).unwrap(), "α draft\r\nsecond\n".as_bytes());
    draw(&mut a, 28, 6); draw(&mut a, 1, 1);
    key(&mut a, KeyCode::F(4), KeyModifiers::CONTROL);
    assert!(a.session.state().saved_comparison().is_none()); assert_eq!(a.session.state().active().unwrap().id(), ordinary);
    key(&mut a, KeyCode::Char('z'), KeyModifiers::CONTROL); assert!(a.session.state().tab_dirty(ordinary));
    assert_eq!(std::fs::read(path).unwrap(), "α draft\r\nsecond\n".as_bytes());
}

#[test]
fn drawing_drops_a_close_dialog_the_core_settled_outside_the_apps_dispatch() {
    use code_core::close::Choice;
    let d = tempfile::tempdir().unwrap(); let path = d.path().join("settled.txt");
    std::fs::write(&path, b"saved").unwrap();
    let mut a = app(d.path()); a.dispatch(Command::OpenRetained(path.clone()));
    settle(&mut a, |a| a.session.state().current_source().is_some());
    a.enable_draft_input_for_test(); draw(&mut a, 80, 12);
    key(&mut a, KeyCode::Char('a'), KeyModifiers::CONTROL); a.handle(Event::Paste("draft".into()));
    key(&mut a, KeyCode::F(4), KeyModifiers::CONTROL); let p = a.session.close_prompt().unwrap();
    assert!(a.modal.is_some());
    // Draw-time layout commands reach the core directly, and any core dispatch can settle a prompt.
    a.session.dispatch(Command::ChooseClose { ticket: p.ticket, buffer: p.buffer, choice: Choice::Cancel });
    draw(&mut a, 80, 12);
    assert!(a.session.close_prompt().is_none() && a.modal.is_none());
    key(&mut a, KeyCode::F(4), KeyModifiers::CONTROL);
    assert!(a.session.close_prompt().is_some_and(|fresh| fresh.ticket > p.ticket) && a.modal.is_some());
    assert_eq!(std::fs::read(path).unwrap(), b"saved");
}

#[test]
fn fresh_discard_and_close_after_save_fault_keeps_disk_and_rejects_old_ticket() {
    use lenz_content::write::readiness_fault::{at_path, Fault};
    use code_core::{close::Choice, save::Status};
    for fault in [Fault::BeforeReplace, Fault::AfterReplace] {
        let d = tempfile::tempdir().unwrap(); let path = d.path().join("fresh-close.txt");
        std::fs::write(&path, b"saved").unwrap();
        let mut a = app(d.path()); a.dispatch(Command::OpenRetained(path.clone()));
        settle(&mut a, |a| a.session.state().current_source().is_some());
        a.enable_draft_input_for_test(); draw(&mut a, 80, 12);
        key(&mut a, KeyCode::Char('a'), KeyModifiers::CONTROL); a.handle(Event::Paste("draft λ".into()));
        let tab = a.session.state().active().unwrap().id();
        key(&mut a, KeyCode::F(4), KeyModifiers::CONTROL); let old = a.session.close_prompt().unwrap();
        let guard = at_path(&path, fault); key(&mut a, KeyCode::Char('s'), KeyModifiers::NONE);
        settle(&mut a, |a| matches!(a.session.state().view(tab).unwrap().save_status(), Status::Rejected(_) | Status::Uncertain(_)));
        drop(guard); assert!(a.session.close_prompt().is_none());
        let disk = std::fs::read(&path).unwrap();
        assert_eq!(disk, if fault == Fault::AfterReplace { "draft λ".as_bytes() } else { b"saved" });
        key(&mut a, KeyCode::F(4), KeyModifiers::CONTROL);
        let fresh = a.session.close_prompt().unwrap();
        assert!(fresh.ticket > old.ticket && fresh.discard_disabled.is_none());
        if fault == Fault::AfterReplace { assert!(draw_screen(&mut a, 80, 12).contains("Discard Draft and Close")); }
        a.dispatch(Command::ChooseClose { ticket: old.ticket, buffer: old.buffer, choice: Choice::Discard });
        assert_eq!(a.session.close_prompt().unwrap().ticket, fresh.ticket);
        assert!(a.session.state().view(tab).unwrap().dirty() && a.session.state().view(tab).unwrap().can_undo());
        key(&mut a, KeyCode::Char('d'), KeyModifiers::NONE);
        assert!(a.session.state().active().is_none() && a.modal.is_none());
        assert_eq!(std::fs::read(path).unwrap(), disk);
    }
}

#[test]
fn checked_save_allows_fresh_discard_after_current_recheck_without_changing_disk() {
    use lenz_content::write::readiness_fault::{at_path,Fault};
    use code_core::save::{DiskMatch,Status};
    for changed in [false,true] {
        let d=tempfile::tempdir().unwrap();let path=d.path().join("close.txt");std::fs::write(&path,b"saved").unwrap();let mut a=app(d.path());a.dispatch(Command::OpenRetained(path.clone()));settle(&mut a,|a|a.session.state().current_source().is_some());a.enable_draft_input_for_test();draw(&mut a,80,12);
        key(&mut a,KeyCode::Char('a'),KeyModifiers::CONTROL);a.handle(Event::Paste("draft λ".into()));let tab=a.session.state().active().unwrap().id();let guard=at_path(&path,Fault::AfterReplace);key(&mut a,KeyCode::Char('s'),KeyModifiers::CONTROL);settle(&mut a,|a|matches!(a.session.state().view(tab).unwrap().save_status(),Status::Uncertain(_)));
        key(&mut a,KeyCode::F(4),KeyModifiers::CONTROL);assert!(a.session.state().active().is_some());assert!(a.session.close_prompt().unwrap().discard_disabled.is_none());key(&mut a,KeyCode::Esc,KeyModifiers::NONE);drop(guard);
        if changed {std::fs::write(&path,b"external content").unwrap();}let disk=std::fs::read(&path).unwrap();
        key(&mut a,KeyCode::F(10),KeyModifiers::NONE);draw(&mut a,80,12);let row=a.choices().iter().position(|row|matches!(&row.target,ChoiceTarget::Action(CommandId::RecheckSave))).unwrap();for _ in 0..row {key(&mut a,KeyCode::Down,KeyModifiers::NONE);}key(&mut a,KeyCode::Enter,KeyModifiers::NONE);
        settle(&mut a,|a|matches!(a.session.state().view(tab).unwrap().save_status(),Status::Checked(_)));assert_eq!(a.session.state().view(tab).unwrap().save_status(),&Status::Checked(if changed {DiskMatch::Changed}else{DiskMatch::Attempted}));assert!(a.session.state().tab_dirty(tab));
        key(&mut a,KeyCode::F(4),KeyModifiers::CONTROL);let p=a.session.close_prompt().unwrap();assert!(p.save_disabled.is_some() && p.discard_disabled.is_none());assert!(draw_screen(&mut a,80,12).contains("Unsaved changes"));key(&mut a,KeyCode::Char('d'),KeyModifiers::NONE);
        assert!(a.session.state().active().is_none());assert!(a.modal.is_none());assert_eq!(std::fs::read(path).unwrap(),disk);
    }
}

#[test]
fn save_rejection_uncertainty_and_explicit_recheck_are_visible_and_modal_safe() {
    use lenz_content::write::readiness_fault::{at_path, Fault};
    use code_core::save::{Status, DiskMatch};
    for fault in [Fault::BeforeReplace, Fault::AfterReplace] {
        let dir = tempfile::tempdir().unwrap(); let path = dir.path().join("fault.txt"); std::fs::write(&path, "old").unwrap();
        let mut a = app(dir.path()); a.dispatch(Command::OpenRetained(path.clone()));
        settle(&mut a, |a| a.session.state().current_source().is_some());
        a.enable_draft_input_for_test(); draw(&mut a, 80, 12);
        key(&mut a, KeyCode::Char('a'), KeyModifiers::CONTROL); a.handle(Event::Paste("new".into()));
        let tab = a.session.state().active().unwrap().id(); let guard = at_path(&path, fault);
        key(&mut a, KeyCode::Char('s'), KeyModifiers::CONTROL);
        settle(&mut a, |a| matches!(a.session.state().view(tab).unwrap().save_status(), Status::Rejected(_) | Status::Uncertain(_)));
        let disk = std::fs::read(&path).unwrap();
        assert_eq!(disk, if fault == Fault::AfterReplace { b"new" } else { b"old" });
        let message = a.session.status_message(); assert!(message.contains(if fault == Fault::AfterReplace { "uncertain" } else { "rejected" }), "{message}");
        assert!(draw_screen(&mut a, 80, 12).contains("Save"));
        key(&mut a, KeyCode::F(2), KeyModifiers::SHIFT);
        assert!(a.modal.is_some()); key(&mut a, KeyCode::Char('s'), KeyModifiers::CONTROL);
        assert_eq!(std::fs::read(&path).unwrap(), disk); key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
        if fault==Fault::AfterReplace {
            let before=a.session.state().current_source().unwrap().key();key(&mut a,KeyCode::F(4),KeyModifiers::CONTROL);
            let p=a.session.close_prompt().unwrap();assert!(p.save_disabled.is_some() && p.discard_disabled.is_none());
            key(&mut a,KeyCode::Char('s'),KeyModifiers::NONE);
            assert_eq!(a.session.state().current_source().unwrap().key(),before);assert!(a.session.close_prompt().is_some());key(&mut a,KeyCode::Esc,KeyModifiers::NONE);
        }
        drop(guard);
        a.run(CommandId::RecheckSave);
        settle(&mut a, |a| matches!(a.session.state().view(tab).unwrap().save_status(), Status::Checked(_)));
        let expected = if fault == Fault::AfterReplace { DiskMatch::Attempted } else { DiskMatch::Baseline };
        assert_eq!(a.session.state().view(tab).unwrap().save_status(), &Status::Checked(expected));
        assert!(a.session.state().tab_dirty(tab));
        assert!(draw_screen(&mut a, 28, 6).contains("Disk"));
        if fault==Fault::AfterReplace {
            key(&mut a,KeyCode::F(4),KeyModifiers::CONTROL);assert!(a.session.close_prompt().unwrap().discard_disabled.is_none());
            assert!(a.session.state().active().is_some());key(&mut a,KeyCode::Esc,KeyModifiers::NONE);
        }
        a.run(CommandId::ConfirmRecheckedSave);
        assert_eq!(a.session.state().tab_dirty(tab), fault != Fault::AfterReplace);
        assert_eq!(std::fs::read(path).unwrap(), disk);
        if fault==Fault::AfterReplace {key(&mut a,KeyCode::F(4),KeyModifiers::CONTROL);assert!(a.session.state().active().is_none());}
    }
}

#[test]
fn save_shortcut_keeps_history_after_actual_cache_eviction() {
    let dir = tempfile::tempdir().unwrap(); let path = dir.path().join("save.txt");
    let original = "\u{feff}α😀\r\nbeta\n";
    std::fs::write(&path, original).unwrap();
    let mut a = app(dir.path()); a.dispatch(Command::OpenRetained(path.clone()));
    settle(&mut a, |a| a.session.state().current_source().is_some());
    a.enable_draft_input_for_test(); draw(&mut a, 80, 12);
    let tab = a.session.state().active().unwrap().id();
    key(&mut a, KeyCode::Home, KeyModifiers::NONE);
    key(&mut a, KeyCode::Right, KeyModifiers::SHIFT);
    a.handle(Event::Paste("é".into()));
    assert!(a.session.state().tab_dirty(tab));
    key(&mut a, KeyCode::Char('s'), KeyModifiers::CONTROL);
    settle(&mut a, |a| !a.session.state().tab_dirty(tab));
    assert_eq!(std::fs::read(&path).unwrap(), "\u{feff}é😀\r\nbeta\n".as_bytes());
    assert!(!draw_screen(&mut a, 80, 12).contains('●'));
    let clean = dir.path().join("evicted.txt"); let other = dir.path().join("pressure.txt");
    std::fs::write(&clean, vec![b'a'; 17 << 20]).unwrap(); std::fs::write(&other, vec![b'b'; 17 << 20]).unwrap();
    a.dispatch(Command::OpenRetained(clean.clone()));
    settle(&mut a, |a| a.session.state().current_source().is_some_and(|s| s.source() == clean));
    draw(&mut a, 80, 12);
    let weak = Arc::downgrade(a.session.state().current_source().unwrap());
    a.dispatch(Command::OpenRetained(other.clone()));
    settle(&mut a, |a| a.session.state().current_source().is_some_and(|s| s.source() == other));
    draw(&mut a, 80, 12);
    a.dispatch(Command::OpenRetained(path.clone()));
    settle(&mut a, |a| a.session.state().current_source().is_some_and(|s| s.source() == path));
    draw(&mut a, 80, 12);
    assert!(weak.upgrade().is_none(), "an unrelated reloadable source really left the bounded cache");
    key(&mut a, KeyCode::Char('z'), KeyModifiers::CONTROL);
    assert_eq!(a.session.state().current_source().unwrap().copy_range(0..13).as_deref(), Some("α😀\r\nbeta\n"));
    assert!(a.session.state().tab_dirty(tab));
    assert!(draw_screen(&mut a, 28, 6).contains('●'));
    assert_eq!(std::fs::read(path).unwrap(), "\u{feff}é😀\r\nbeta\n".as_bytes());
}

#[test]
fn opted_in_draft_input_replaces_unicode_and_replays_shared_undo_without_saving() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("draft.txt");
    let original = "é界\r\nline two\r\n";
    std::fs::write(&path, original).unwrap();
    let mut a = app(dir.path());
    a.dispatch(Command::OpenRetained(path.clone()));
    settle(&mut a, |a| a.session.state().current_source().is_some());
    let text = |a: &App| { let source = a.session.state().current_source().unwrap(); source.copy_range(0..source.len_bytes()).unwrap() };
    draw(&mut a, 100, 20);
    key(&mut a, KeyCode::Char('λ'), KeyModifiers::NONE);
    a.handle(Event::Paste("ignored".into()));
    assert_eq!(text(&a), original, "an unsupported input capability keeps its reader shortcuts");
    a.enable_draft_input_for_test();
    key(&mut a, KeyCode::Right, KeyModifiers::NONE);
    key(&mut a, KeyCode::Right, KeyModifiers::SHIFT);
    assert_eq!(a.session.state().selection().unwrap().range(), 2..5);
    key(&mut a, KeyCode::Char('λ'), KeyModifiers::NONE);
    assert_eq!(text(&a), "éλ\r\nline two\r\n");
    assert_eq!(a.session.state().selection().unwrap(), Selection { anchor: 4, head: 4 });
    let typed = draw_screen(&mut a, 100, 20);
    assert!(typed.contains("éλ"));
    assert!(typed.contains('●'), "dirty source is marked in the tab bar");
    let id = a.session.state().active().unwrap().id();
    assert!(a.session.state().view(id).unwrap().dirty());
    for theme in ["github-light", "darcula"] {
        a.dispatch(Command::Theme(ThemeCommand::Select(ThemeSelection::from_id(theme).unwrap())));
        assert!(draw_screen(&mut a, 100, 20).contains('●'), "dirty indicator is visible in {theme}");
    }
    a.handle(Event::Paste("👩🏽‍💻".into()));
    assert_eq!(text(&a), "éλ👩🏽‍💻\r\nline two\r\n");
    assert_eq!(a.session.state().selection().unwrap().head, 19);
    key(&mut a, KeyCode::Char('z'), KeyModifiers::CONTROL);
    assert_eq!(text(&a), "éλ\r\nline two\r\n");
    assert_eq!(a.session.state().selection().unwrap().head, 4);
    key(&mut a, KeyCode::Char('z'), KeyModifiers::CONTROL);
    assert_eq!(text(&a), original);
    assert_eq!(a.session.state().selection().unwrap().head, 5);
    assert!(!a.session.state().view(id).unwrap().dirty());
    assert!(!draw_screen(&mut a, 100, 20).contains('●'), "undo to clean text removes the dirty indicator");
    key(&mut a, KeyCode::Char('z'), KeyModifiers::CONTROL | KeyModifiers::SHIFT);
    key(&mut a, KeyCode::Char('z'), KeyModifiers::CONTROL | KeyModifiers::SHIFT);
    assert_eq!(text(&a), "éλ👩🏽‍💻\r\nline two\r\n");
    let other = dir.path().join("other.txt");
    std::fs::write(&other, "other text\n").unwrap();
    a.dispatch(Command::OpenRetained(other.clone()));
    settle(&mut a, |a| a.session.state().current_source().is_some_and(|source| source.source() == other));
    assert!(a.session.state().view(id).is_none() && a.session.state().tab_dirty(id));
    assert!(draw_screen(&mut a, 100, 20).contains('●'), "inactive dirty tab retains its indicator");
    key(&mut a, KeyCode::BackTab, KeyModifiers::CONTROL | KeyModifiers::SHIFT);
    settle(&mut a, |a| a.session.state().active().is_some_and(|tab| tab.id() == id && tab.loaded()));
    draw(&mut a, 100, 20);
    assert_eq!(text(&a), "éλ👩🏽‍💻\r\nline two\r\n");
    let before_modal = a.session.state().current_source().unwrap().key();
    key(&mut a, KeyCode::F(1), KeyModifiers::NONE);
    key(&mut a, KeyCode::Char('x'), KeyModifiers::NONE);
    a.handle(Event::Paste("blocked by modal".into()));
    key(&mut a, KeyCode::Char('z'), KeyModifiers::CONTROL);
    draw(&mut a, 100, 20);
    assert_eq!(a.session.state().current_source().unwrap().key(), before_modal);
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    assert_eq!(std::fs::read_to_string(path).unwrap(), original);
}

#[test]
fn declaration_search_follows_a_coarse_checkout_through_keys() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    std::fs::create_dir_all(root.join("app")).unwrap();
    let path = root.join("app/a.rb");
    std::fs::write(&path, "class WatchAlpha; end\n").unwrap();
    let mut a = app(dir.path());
    a.dispatch(Command::OpenRoot(root.clone()));
    settle(&mut a, |a| a.session.state().index_coverage().indexed == 1 && a.session.state().index_coverage().complete());
    draw(&mut a, 100, 26);
    key(&mut a, KeyCode::Char('t'), KeyModifiers::CONTROL);
    for c in "watch".chars() { key(&mut a, KeyCode::Char(c), KeyModifiers::NONE); }
    settle(&mut a, |a| a.session.state().results().is_some_and(|r| r.done() && r.hits().len() == 1));
    std::fs::write(&path, "class WatchOmega; end\n").unwrap();
    a.apply(Completion::Changed(code_core::watch::Change { overflow: true, ..Default::default() }));
    settle(&mut a, |a| a.session.state().results().is_some_and(|r| r.done() && r.hits().first().is_some_and(|h| &*h.text == "WatchOmega")));
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    for (w, h) in [(26, 8), (100, 26)] { draw(&mut a, w, h); }
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 26)).unwrap();
    terminal.draw(|f| a.draw(f)).unwrap();
    let text: String = terminal.backend().buffer().content.iter().map(|c| c.symbol()).collect();
    assert!(text.contains("WatchOmega"), "{text}");
    assert!(!text.contains("WatchAlpha"));
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    settle(&mut a, |a| copied_selection(a.session.state()).as_deref() == Some("WatchOmega"));
    assert_eq!(std::fs::read_to_string(path).unwrap(), "class WatchOmega; end\n");
}
#[test]
fn right_click_uses_the_shared_context_actions_and_left_click_selects_text() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sample.txt");
    std::fs::write(&path, "hello world\nsecond\n").unwrap();
    let mut a = app(dir.path());
    a.dispatch(Command::Open(path.clone()));
    settle(&mut a, |a| a.session.state().current_source().is_some());
    draw(&mut a, 100, 26);
    let p = &a.layout.panes[0];
    let (x, y) = (p.text.x, p.text.y);
    click(&mut a, x + 2, y);
    assert!(a.modal.is_none());
    assert_eq!(a.session.state().selection().unwrap().head, 2);
    a.dispatch(Command::Select(Selection { anchor: 0, head: 5 }));
    a.handle(Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Right),
        column: x + 2,
        row: y,
        modifiers: KeyModifiers::NONE,
    }));
    let actions: Vec<_> = a
        .choices()
        .iter()
        .filter_map(|c| {
            if let ChoiceTarget::Action(id) = c.target {
                Some(id)
            } else {
                None
            }
        })
        .collect();
    let expected: Vec<_> = a
        .session
        .context_menu()
        .iter()
        .filter_map(|e| {
            if let code_core::session::MenuEntry::Command(id) = e {
                Some(*id)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(actions, expected);
    assert!(!actions.contains(&CommandId::OpenFolder));
    assert_eq!(copied_selection(a.session.state()).as_deref(), Some("hello"));
    draw(&mut a, 40, 12);
    let before = a.session.state().selection();
    click(&mut a, 0, 11);
    assert!(a.modal.is_none());
    assert_eq!(a.session.state().selection(), before);
    assert_eq!(
        std::fs::read_to_string(path).unwrap(),
        "hello world\nsecond\n"
    );
}

#[test]
fn top_bar_back_control_navigates_instead_of_opening_the_palette() {
    let dir = tempfile::tempdir().unwrap();
    let one = dir.path().join("one.txt");
    let two = dir.path().join("two.txt");
    std::fs::write(&one, "one\n").unwrap();
    std::fs::write(&two, "two\n").unwrap();
    let mut a = app(dir.path());
    a.dispatch(Command::OpenRetained(one.clone()));
    settle(&mut a, |a| a.session.state().current_source().is_some());
    a.dispatch(Command::OpenRetained(two));
    settle(&mut a, |a| {
        a.session
            .state()
            .active()
            .is_some_and(|t| t.source() == dir.path().join("two.txt"))
            && a.session.state().loading().is_none()
    });
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 26)).unwrap();
    terminal.draw(|f| a.draw(f)).unwrap();
    let top: String = (0..100)
        .map(|x| terminal.backend().buffer()[(x, 0)].symbol())
        .collect();
    let byte = top.find('←').expect("Back is visible beside the tabs");
    let x = top[..byte].chars().count() as u16;
    click(&mut a, x, 0);
    settle(&mut a, |a| {
        a.session
            .state()
            .active()
            .is_some_and(|t| t.source() == one)
            && a.session.state().loading().is_none()
    });
    assert!(a.modal.is_none());
}

#[test]
fn context_menu_follows_the_clicked_split_and_preview_and_scrolls_in_tiny_frames() {
    let dir = tempfile::tempdir().unwrap();
    let text = dir.path().join("a.txt");
    let markdown = dir.path().join("b.md");
    std::fs::write(&text, "α界 name\n").unwrap();
    std::fs::write(&markdown, "# Heading\n\nBody\n").unwrap();
    let mut a = app(dir.path());
    a.dispatch(Command::OpenRetained(text.clone()));
    settle(&mut a, |a| a.session.state().current_source().is_some());
    a.run(CommandId::SplitRight);
    a.dispatch(Command::OpenRetained(markdown));
    settle(&mut a, |a| {
        a.session
            .state()
            .active()
            .is_some_and(|t| t.source().ends_with("b.md"))
            && a.session.state().current_source().is_some()
    });
    a.run(CommandId::MarkdownSource);
    settle(&mut a, |a| {
        matches!(a.session.state().rendering(), Some(Rendering::Preview(_)))
    });
    draw(&mut a, 120, 30);
    let panes = a
        .layout
        .panes
        .iter()
        .map(|p| (p.pane, p.text, p.preview))
        .collect::<Vec<_>>();
    for (pane, bounds, preview) in panes {
        a.handle(Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Right),
            column: bounds.x + 1,
            row: bounds.y,
            modifiers: KeyModifiers::NONE,
        }));
        assert_eq!(a.session.state().focused_pane(), pane);
        assert_eq!(
            a.choices()
                .iter()
                .any(|c| matches!(c.target, ChoiceTarget::Action(CommandId::Definition))),
            !preview
        );
        for (w, h) in [(24, 8), (1, 1), (120, 30)] {
            draw(&mut a, w, h);
            key(&mut a, KeyCode::End, KeyModifiers::NONE);
            key(&mut a, KeyCode::Up, KeyModifiers::NONE);
            draw(&mut a, w, h);
            assert!(a.modal.as_ref().unwrap().at < a.choices().len());
        }
        key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    }
    // A right-click on an explorer row opens the explorer's own menu: creation and the row's rename or move, never a
    // document command.
    a.handle(Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Right),
        column: 0,
        row: 2,
        modifiers: KeyModifiers::NONE,
    }));
    assert!(matches!(a.modal.as_ref().map(|m| &m.kind), Some(Prompt::Context { .. })));
    assert!(a.choices().iter().all(|c| matches!(c.target, ChoiceTarget::Action(CommandId::NewExplorerFile | CommandId::NewExplorerFolder | CommandId::RenameExplorerEntry | CommandId::MoveExplorerEntry
        | CommandId::CopyExplorerName | CommandId::CopyExplorerFullPath | CommandId::CopyExplorerRelativePath | CommandId::CopyExplorerEntry
        | CommandId::PasteExplorerEntry | CommandId::CancelExplorerCopy | CommandId::ExplorerHistory | CommandId::PasteTerminalPath
        | CommandId::DeleteExplorerEntry | CommandId::DeleteExplorerPermanently | CommandId::RestoreDeleted))));
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    assert!(a.modal.is_none());
    assert_eq!(std::fs::read_to_string(text).unwrap(), "α界 name\n");
}

#[test]
fn modal_input_is_isolated_and_reader_never_writes_sources() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sample.txt");
    std::fs::write(&path, "hello\nworld\n").unwrap();
    let mut a = app(dir.path());
    a.dispatch(Command::Open(path.clone()));
    settle(&mut a, |a| a.session.state().current_source().is_some());
    a.dispatch(Command::Select(Selection { anchor: 0, head: 5 }));
    a.run(CommandId::GoToLine);
    key(&mut a, KeyCode::Char('a'), KeyModifiers::CONTROL);
    key(&mut a, KeyCode::Down, KeyModifiers::NONE);
    assert_eq!(
        a.session.state().selection(),
        Some(Selection { anchor: 0, head: 5 })
    );
    a.handle(Event::Paste("2:2".into()));
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    assert!(a.modal.is_none());
    assert_eq!(a.session.state().selection().unwrap().head, 7);
    assert_eq!(std::fs::read_to_string(path).unwrap(), "hello\nworld\n");
}
#[test]
fn palette_exposes_entire_core_catalog_and_filters() {
    let dir = tempfile::tempdir().unwrap();
    let mut a = app(dir.path());
    key(&mut a, KeyCode::F(10), KeyModifiers::NONE);
    assert_eq!(a.choices().len(), CommandId::ALL.len());
    a.handle(Event::Paste("split".into()));
    assert_eq!(a.choices().len(), 3);
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    assert!(a.modal.is_none());
    key(&mut a, KeyCode::F(10), KeyModifiers::NONE);
    a.handle(Event::Paste("language servers:".into()));
    let choices = a.choices();
    assert!(!choices.is_empty());
    assert!(choices.iter().all(|c| matches!(c.target, ChoiceTarget::Action(id) if id.group() == code_core::actions::CommandGroup::LanguageServers)));
}

#[test]
fn theme_chooser_previews_before_enter_and_escape_restores_without_saving() {
    let dir = tempfile::tempdir().unwrap();
    let selection_file = config(dir.path()).themes.selection;
    let mut a = app(dir.path());
    let original = a.session.state().palette().clone();
    let selected = a.session.theme_id();
    a.run(CommandId::ChooseTheme);
    draw(&mut a, 100, 26);
    let (index, preview) = a
        .session
        .theme_entries()
        .iter()
        .enumerate()
        .find(|(_, entry)| entry.palette.bg != original.bg)
        .map(|(index, entry)| (index + 1, entry.palette.clone()))
        .unwrap();
    key(&mut a, KeyCode::Home, KeyModifiers::NONE);
    for _ in 0..index {
        key(&mut a, KeyCode::Down, KeyModifiers::NONE);
    }
    assert_eq!(a.session.state().palette().bg, preview.bg);
    assert_eq!(a.session.theme_id(), selected);
    assert!(!selection_file.exists());
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    assert!(a.modal.is_none());
    assert_eq!(a.session.state().palette().bg, original.bg);
    assert_eq!(a.session.theme_id(), selected);
    assert!(!selection_file.exists());
}

#[test]
fn theme_chooser_current_choice_navigation_and_enter_or_click_persist() {
    for mouse in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let selection_file = config(dir.path()).themes.selection;
        std::fs::create_dir_all(selection_file.parent().unwrap()).unwrap();
        std::fs::write(
            &selection_file,
            r#"{"version":1,"theme":"light","keep":true}"#,
        )
        .unwrap();
        let mut a = app(dir.path());
        a.run(CommandId::ChooseTheme);
        assert_eq!(a.highlighted_theme().as_deref(), Some("light"));
        key(&mut a, KeyCode::Home, KeyModifiers::NONE);
        assert_eq!(a.modal.as_ref().unwrap().at, 0);
        key(&mut a, KeyCode::PageDown, KeyModifiers::NONE);
        assert_eq!(a.modal.as_ref().unwrap().at, 8);
        key(&mut a, KeyCode::PageUp, KeyModifiers::NONE);
        assert_eq!(a.modal.as_ref().unwrap().at, 0);
        key(&mut a, KeyCode::End, KeyModifiers::NONE);
        assert_eq!(a.modal.as_ref().unwrap().at, a.choices().len() - 1);
        key(&mut a, KeyCode::Up, KeyModifiers::NONE);
        let chosen = a.highlighted_theme().unwrap();
        let palette = a.session.state().palette().clone();
        draw(&mut a, 100, 26);
        if mouse {
            let selected = a.modal.as_ref().unwrap().at;
            let row = a
                .layout
                .modal_items
                .iter()
                .position(|i| *i == selected)
                .unwrap();
            let (x, y) = (a.layout.modal_rows.x, a.layout.modal_rows.y + row as u16);
            click(&mut a, x, y);
        } else {
            key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
        }
        assert!(a.modal.is_none());
        assert_eq!(a.session.theme_id(), chosen);
        assert_eq!(a.session.state().palette().bg, palette.bg);
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let saved: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&selection_file).unwrap()).unwrap();
            if saved["theme"] == chosen {
                assert_eq!(saved["keep"], true);
                break;
            }
            assert!(Instant::now() < deadline, "theme selection was not saved");
            std::thread::yield_now();
        }
        a.run(CommandId::ChooseTheme);
        assert_eq!(a.highlighted_theme().as_deref(), Some(chosen.as_str()));
        key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    }
}

#[test]
fn theme_chooser_wheel_and_header_keep_preview_and_modal_blocks_source_input() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sample.txt");
    let source = "hello\r\nworld\r\n";
    std::fs::write(&path, source).unwrap();
    let mut a = app(dir.path());
    a.dispatch(Command::Open(path.clone()));
    settle(&mut a, |a| a.session.state().current_source().is_some());
    a.dispatch(Command::Select(Selection { anchor: 0, head: 5 }));
    let selection = a.session.state().selection();
    let original = a.session.state().palette().clone();
    a.run(CommandId::ChooseTheme);
    key(&mut a, KeyCode::Down, KeyModifiers::NONE);
    key(&mut a, KeyCode::Down, KeyModifiers::NONE);
    let preview = a.session.state().palette().clone();
    assert_ne!(preview.bg, original.bg);
    let at = a.modal.as_ref().unwrap().at;
    draw(&mut a, 100, 26);
    let (x, y) = (a.layout.modal_rows.x, a.layout.modal_rows.y);
    click(&mut a, x, y);
    assert!(
        a.modal.is_some(),
        "help/current source are not theme choices"
    );
    for (w, h) in [(100, 26), (24, 10), (8, 5), (1, 1), (0, 0), (100, 26)] {
        draw(&mut a, w, h);
        for kind in [MouseEventKind::ScrollDown, MouseEventKind::ScrollUp] {
            a.handle(Event::Mouse(MouseEvent {
                kind,
                column: x,
                row: y,
                modifiers: KeyModifiers::NONE,
            }));
        }
        a.handle(Event::Paste("ignored filter".into()));
        key(&mut a, KeyCode::Char('x'), KeyModifiers::NONE);
        for code in [
            KeyCode::Char('s'),
            KeyCode::Char('z'),
            KeyCode::Char('o'),
            KeyCode::Char('p'),
        ] {
            key(&mut a, code, KeyModifiers::CONTROL);
        }
        key(&mut a, KeyCode::F(5), KeyModifiers::NONE);
        assert_eq!(a.modal.as_ref().unwrap().at, at);
        assert!(a.modal.as_ref().unwrap().input.is_empty());
        assert_eq!(a.session.state().palette().bg, preview.bg);
        assert_eq!(a.session.state().selection(), selection);
    }
    click(&mut a, 0, 0);
    assert!(a.modal.is_none());
    assert_eq!(a.session.state().palette().bg, original.bg);
    assert_eq!(std::fs::read_to_string(path).unwrap(), source);
}

#[test]
fn theme_chooser_refresh_retains_highlighted_id_when_registry_rows_move() {
    use lenz_content::theme::{ThemeEntry, ThemeUpdate};
    let dir = tempfile::tempdir().unwrap();
    let mut a = app(dir.path());
    key(&mut a, KeyCode::F(10), KeyModifiers::NONE);
    a.handle(Event::Paste("Change theme".into()));
    assert_eq!(a.choices().len(), 1);
    assert_eq!(a.choices()[0].label, "Application: Change theme");
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    let entry = |id: &str| ThemeEntry {
        id: id.into(),
        label: id.into(),
        palette: a.session.state().palette().clone(),
    };
    let one = entry("user:one");
    let two = entry("user:two");
    a.apply(Completion::Theme(Box::new(ThemeUpdate::Registry {
        request: 1,
        entries: vec![one.clone(), two.clone()],
        notices: Vec::new(),
    })));
    key(&mut a, KeyCode::End, KeyModifiers::NONE);
    assert_eq!(a.highlighted_theme().as_deref(), Some("user:two"));
    a.apply(Completion::Theme(Box::new(ThemeUpdate::Registry {
        request: 1,
        entries: vec![two, one],
        notices: Vec::new(),
    })));
    assert_eq!(a.highlighted_theme().as_deref(), Some("user:two"));
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
}

#[test]
fn server_status_reports_missing_setup_and_offers_restart_or_enable_without_touching_source() {
    use code_core::semantics::Status;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("main.rs");
    let source = "fn main() {}\n";
    std::fs::write(&path, source).unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname='status'\nversion='0.1.0'\n",
    )
    .unwrap();
    let cfg = config(dir.path());
    std::fs::create_dir_all(cfg.providers.parent().unwrap()).unwrap();
    std::fs::write(
        &cfg.providers,
        r#"{"providers":{"rust-analyzer":{"executable":"/nonexistent/lenz-status-test"}}}"#,
    )
    .unwrap();
    let mut a = App::new(cfg, Some(dir.path().into()));
    settle(&mut a, |a| a.session.state().project().is_some());
    a.dispatch(Command::Open(path.clone()));
    settle(&mut a, |a| {
        a.session
            .state()
            .provider()
            .is_some_and(|v| matches!(v.status, Status::Missing(_)))
    });
    a.dispatch(Command::Select(Selection { anchor: 0, head: 2 }));
    let selected = a.session.state().selection();
    draw(&mut a, 100, 26);
    let control = a
        .layout
        .controls
        .iter()
        .find(|(id, _)| *id == CommandId::LanguageServer)
        .unwrap()
        .1;
    click(&mut a, control.x, control.y);
    assert_eq!(a.modal.as_ref().unwrap().kind, Prompt::LanguageServer);
    draw(&mut a, 100, 26);
    assert_eq!(a.choices().len(), 10);
    assert!(a.choices().iter().all(|c| matches!(c.target, ChoiceTarget::Action(_))));
    let (x, y) = (a.layout.modal_rows.x, a.layout.modal_rows.y);
    click(&mut a, x, y);
    let details = match &a.modal.as_ref().unwrap().kind {
        Prompt::Details { title: "Server details", body, .. } => body.clone(),
        other => panic!("unexpected prompt: {other:?}"),
    };
    assert!(details.contains("was not found") && details.contains("rustup component add rust-analyzer") && details.contains("providers.json"));
    key(&mut a, KeyCode::Char('a'), KeyModifiers::CONTROL);
    key(&mut a, KeyCode::F(5), KeyModifiers::NONE);
    assert_eq!(a.session.state().selection(), selected);
    for (w, h) in [(26, 8), (1, 1), (80, 24)] {
        draw(&mut a, w, h);
        key(&mut a, KeyCode::End, KeyModifiers::NONE);
        key(&mut a, KeyCode::Up, KeyModifiers::NONE);
        draw(&mut a, w, h);
        assert!(a.modal.as_ref().unwrap().top <= a.layout.modal_content_rows.saturating_sub(a.layout.modal_rows.height as usize));
    }
    key(&mut a, KeyCode::Char('c'), KeyModifiers::CONTROL);
    let copied = a
        .copy
        .take()
        .expect("Ctrl+C copies the LSP error from the status dialog");
    assert!(
        copied.contains("was not found")
            && copied.contains("rustup component add rust-analyzer")
            && copied.contains("providers.json"),
        "{copied}"
    );
    assert!(!copied.contains("\nRestart Language Server\n"));
    assert_eq!(a.session.state().selection(), selected);
    assert!(matches!(a.modal.as_ref().unwrap().kind, Prompt::Details { .. }));
    draw(&mut a, 80, 24);
    let copy = a.layout.details_copy;
    click(&mut a, copy.x, copy.y);
    assert_eq!(a.copy.take().as_deref(), Some(copied.as_str()));
    let close = a.layout.details_close;
    click(&mut a, close.x, close.y);
    assert!(a.modal.is_none());
    key(&mut a, KeyCode::F(2), KeyModifiers::NONE);
    draw(&mut a, 80, 24);
    let activate = |a: &mut App, id| {
        a.modal.as_mut().unwrap().at = a
            .choices()
            .iter()
            .position(|c| matches!(c.target, ChoiceTarget::Action(action) if action == id))
            .unwrap();
        key(a, KeyCode::Enter, KeyModifiers::NONE);
        assert!(a.modal.is_none());
    };
    activate(&mut a, CommandId::RestartServer);
    settle(&mut a, |a| {
        a.session
            .state()
            .provider()
            .is_some_and(|v| matches!(v.status, Status::Missing(_)))
    });
    key(&mut a, KeyCode::F(2), KeyModifiers::NONE);
    draw(&mut a, 80, 24);
    activate(&mut a, CommandId::DisableServer);
    assert_eq!(a.session.state().language_server_label(), "LSP off");
    settle(&mut a, |a| a.session.disabled(CommandId::EnableServer).is_none());
    key(&mut a, KeyCode::F(2), KeyModifiers::NONE);
    draw(&mut a, 80, 24);
    assert!(a.choices().iter().any(|c| matches!(
        c.target,
        ChoiceTarget::Action(CommandId::EnableServer)
    ) && c.disabled.is_none()));
    activate(&mut a, CommandId::EnableServer);
    settle(&mut a, |a| {
        a.session
            .state()
            .provider()
            .is_some_and(|v| matches!(v.status, Status::Missing(_)))
    });
    assert_eq!(std::fs::read_to_string(path).unwrap(), source);
    assert_eq!(a.session.state().selection(), selected);
}
#[test]
fn finder_and_find_use_real_core_workers() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notes.txt");
    std::fs::write(&path, "needle\nneedle\n").unwrap();
    let mut a = app(dir.path());
    settle(&mut a, |a| {
        a.session.state().project().is_some_and(|p| p.files() > 0)
    });
    a.run(CommandId::FindFile);
    a.handle(Event::Paste("notes.txt:2".into()));
    settle(&mut a, |a| {
        a.session
            .state()
            .navigation()
            .is_some_and(|s| !s.pending && !s.rows.is_empty())
    });
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    settle(&mut a, |a| a.session.state().current_source().is_some());
    assert_eq!(a.session.state().active().unwrap().source(), path);
    assert_eq!(a.session.state().selection().unwrap().head, 7);
    a.run(CommandId::FindInFile);
    a.handle(Event::Paste("needle".into()));
    settle(&mut a, |a| {
        a.session
            .state()
            .active()
            .and_then(|t| t.find())
            .is_some_and(|f| f.matches().len() == 2)
    });
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    assert!(a.modal.is_none());
}
#[test]
fn empty_real_constructor_and_tiny_frames_are_safe() {
    let dir = tempfile::tempdir().unwrap();
    let mut a = app(dir.path());
    for command in [
        CommandId::Reload,
        CommandId::Close,
        CommandId::Copy,
        CommandId::GoToLine,
    ] {
        a.run(command);
    }
    a.modal = None;
    let backend = ratatui::backend::TestBackend::new(1, 1);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();
    terminal.draw(|f| a.draw(f)).unwrap();
    key(&mut a, KeyCode::F(1), KeyModifiers::NONE);
    key(&mut a, KeyCode::End, KeyModifiers::NONE);
    key(&mut a, KeyCode::Up, KeyModifiers::NONE);
    terminal.draw(|f| a.draw(f)).unwrap();
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    assert!(a.modal.is_none());
}

pub(crate) fn draw(app: &mut App, width: u16, height: u16) {
    let mut terminal =
        ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
    terminal.draw(|f| app.draw(f)).unwrap();
}
fn click(app: &mut App, x: u16, y: u16) {
    app.handle(Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: x,
        row: y,
        modifiers: KeyModifiers::NONE,
    }));
}
#[test]
fn mouse_selection_keeps_utf8_bytes_and_modal_blocks_underlying_clicks() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("unicode.txt");
    std::fs::write(&path, "α界 z\nsecond\n").unwrap();
    let mut a = app(dir.path());
    a.dispatch(Command::Open(path.clone()));
    settle(&mut a, |a| a.session.state().current_source().is_some());
    draw(&mut a, 80, 20);
    let (x, y) = {
        let p = &a.layout.panes[0];
        (p.text.x, p.text.y)
    };
    click(&mut a, x, y);
    a.handle(Event::Mouse(MouseEvent {
        kind: MouseEventKind::Drag(MouseButton::Left),
        column: x + 3,
        row: y,
        modifiers: KeyModifiers::NONE,
    }));
    assert_eq!(copied_selection(a.session.state()).as_deref(), Some("α界"));
    key(&mut a, KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert_eq!(a.copy.take().as_deref(), Some("α界"));
    let before = a.session.state().selection();
    key(&mut a, KeyCode::F(10), KeyModifiers::NONE);
    draw(&mut a, 80, 20);
    click(&mut a, x, y);
    assert_eq!(a.session.state().selection(), before);
    assert_eq!(std::fs::read_to_string(path).unwrap(), "α界 z\nsecond\n");
}
#[test]
fn horizontal_scroll_is_clamped_to_current_terminal_extents_after_input_and_resize() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("wide.txt");
    let source = "α界 ".repeat(100) + "end\n";
    std::fs::write(&path, &source).unwrap();
    let mut a = app(dir.path());
    a.dispatch(Command::Open(path.clone()));
    settle(&mut a, |a| a.session.state().current_source().is_some());
    a.dispatch(Command::SetSoftWrap(false));
    a.dispatch(Command::SetStickyScroll(false));
    draw(&mut a, 40, 10);
    a.dispatch(Command::ScrollColumn(usize::MAX));
    draw(&mut a, 40, 10);
    let width = a.layout.panes[0].text.width as usize;
    assert_eq!(a.session.state().column(), 404usize.saturating_sub(width));
    assert!(a.next_deadline().is_some(), "a render-time clamp must schedule a fresh frame");
    assert!(a.tick(Instant::now()));
    draw(&mut a, 80, 10);
    let width = a.layout.panes[0].text.width as usize;
    assert_eq!(a.session.state().column(), 404usize.saturating_sub(width));
    assert!(a.tick(Instant::now()));
    draw(&mut a, 80, 10);
    assert!(a.next_deadline().is_none(), "settled source geometry must not schedule idle redraws");
    assert!(a.layout.panes[0].hits.iter().any(|hit| hit.line == 0 && hit.byte == 603), "the clamped viewport includes the EOF caret cell");
    key(&mut a, KeyCode::End, KeyModifiers::NONE);
    key(&mut a, KeyCode::Up, KeyModifiers::NONE);
    draw(&mut a, 1, 1);
    draw(&mut a, 80, 10);
    key(&mut a, KeyCode::F(1), KeyModifiers::NONE);
    key(&mut a, KeyCode::End, KeyModifiers::NONE);
    key(&mut a, KeyCode::Up, KeyModifiers::NONE);
    draw(&mut a, 1, 1);
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    assert!(a.modal.is_none());
    assert_eq!(std::fs::read_to_string(path).unwrap(), source);
}
#[test]
fn long_unicode_line_uses_exact_grapheme_cells_and_source_hits_across_projection_windows() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("families.txt");
    let family = "👨‍👩‍👧‍👦";
    let source = family.repeat(2_000) + " END" + &" ".repeat(100) + "\n";
    std::fs::write(&path, &source).unwrap();
    let mut a = app(dir.path());
    a.dispatch(Command::Open(path.clone()));
    settle(&mut a, |a| a.session.state().current_source().is_some());
    a.dispatch(Command::SetSoftWrap(false));
    a.dispatch(Command::SetStickyScroll(false));
    let view = a.session.state().view(a.session.state().active().unwrap().id()).unwrap();
    let mut cursor = view.display_start(0).unwrap();
    let window = view.display_window(&mut cursor, code_core::projection::WINDOW_WORK).unwrap();
    assert_ne!(window.source.end % 25, 0, "fixture must split a family grapheme across chunks");
    draw(&mut a, 80, 20);
    a.dispatch(Command::ScrollColumn(3_998));
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 20)).unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        terminal.draw(|f| a.draw(f)).unwrap();
        if a.layout.panes[0].hits.iter().any(|hit| hit.byte == 50_001) { break; }
        assert!(Instant::now() < deadline, "the tail of the line never became usable");
        while let Ok(completion) = a.session.completions().try_recv() { a.apply(completion); }
        std::thread::yield_now();
    }
    let pane = &a.layout.panes[0];
    let (x, y) = (pane.text.x, pane.text.y);
    assert_eq!(terminal.backend().buffer()[(x, y)].symbol(), family);
    assert_eq!(pane.hits.iter().find(|hit| hit.x == x).map(|hit| (hit.width, hit.byte)), Some((2, 49_975)));
    assert_eq!(terminal.backend().buffer()[(x + 3, y)].symbol(), "E");
    click(&mut a, x + 3, y);
    a.handle(Event::Mouse(MouseEvent { kind: MouseEventKind::Drag(MouseButton::Left), column: x + 6, row: y, modifiers: KeyModifiers::NONE }));
    assert_eq!(copied_selection(a.session.state()).as_deref(), Some("END"));
    key(&mut a, KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert_eq!(a.copy.take().as_deref(), Some("END"));
    assert_eq!(std::fs::read_to_string(path).unwrap(), source);
}
#[test]
fn leading_zero_width_source_keeps_its_first_hit_after_resizing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("leading-marks.txt");
    let source = "\u{301}\u{302}\n";
    std::fs::write(&path, source).unwrap();
    let mut a = app(dir.path());
    a.dispatch(Command::Open(path.clone()));
    settle(&mut a, |a| a.session.state().current_source().is_some());
    a.dispatch(Command::SetSoftWrap(false));
    for width in [80, 70, 80] {
        draw(&mut a, width, 20);
        assert_eq!(a.layout.panes[0].hits.first().map(|hit| hit.byte), Some(0));
    }
    assert_eq!(std::fs::read_to_string(path).unwrap(), source);
}
#[test]
fn unwrapped_vertical_arrows_preserve_unicode_display_columns_and_reveal_the_caret() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vertical-unicode.txt");
    let source = "éx\r\nabcdef\n終端\n";
    std::fs::write(&path, source).unwrap();
    let mut a = app(dir.path());
    a.dispatch(Command::Open(path.clone()));
    settle(&mut a, |a| a.session.state().active().is_some_and(|tab| tab.loaded()));
    a.dispatch(Command::SetSoftWrap(false));
    a.dispatch(Command::SetStickyScroll(false));
    draw(&mut a, 8, 10);
    assert_eq!(a.layout.panes[0].text.width, 2);
    key(&mut a, KeyCode::Right, KeyModifiers::NONE);
    draw(&mut a, 8, 10);
    assert_eq!(a.session.state().selection().unwrap().head, 2);
    key(&mut a, KeyCode::Down, KeyModifiers::NONE);
    draw(&mut a, 8, 10);
    assert_eq!(a.session.state().selection().unwrap().head, 6);
    key(&mut a, KeyCode::Right, KeyModifiers::NONE);
    draw(&mut a, 8, 10);
    draw(&mut a, 8, 10);
    assert_eq!(a.session.state().selection().unwrap().head, 7);
    assert!(a.session.state().column() > 0);
    assert!(a.layout.panes[0].hits.iter().any(|hit| hit.line == 1 && hit.byte == 2));
    assert_eq!(std::fs::read_to_string(path).unwrap(), source);
}
#[test]
fn unicode_go_to_caret_repaints_after_native_overscroll_clamping() {
    for (source, column, expected, wrapped, width) in [
        ("👨‍👩‍👧‍👦".repeat(2_000) + " END" + &" ".repeat(100) + "\n", 14_002, 50_001, false, 120),
        ("x".repeat(20_000) + " END" + &" ".repeat(100) + "\n", 20_002, 20_001, false, 120),
        ("1\u{FE0F}\u{20E3}".repeat(500) + " END" + &" ".repeat(100) + "\n", 1_502, 3_501, true, 80),
        ("x".repeat(20_000), 20_001, 20_000, false, 80),
    ] {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("go-to-families.txt");
    std::fs::write(&path, &source).unwrap();
    let mut a = app(dir.path());
    a.dispatch(Command::Open(path.clone()));
    settle(&mut a, |a| a.session.state().current_source().is_some());
    a.dispatch(Command::SetSoftWrap(wrapped));
    a.dispatch(Command::SetStickyScroll(false));
    draw(&mut a, width, 36);
    a.dispatch(Command::GoToLine(lenz_content::goto::LineColumn { line: 1, column: Some(column) }));
    draw(&mut a, width, 36);
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut trace = Vec::new();
    while !a.layout.panes[0].hits.iter().any(|hit| hit.byte == expected) {
        assert!(Instant::now() < deadline, "no END after GoTo, state={trace:?}");
        let mut dirty = false;
        while let Ok(completion) = a.session.completions().try_recv() { dirty |= a.apply(completion); }
        if a.next_deadline().is_some_and(|at| at <= Instant::now()) { dirty |= a.tick(Instant::now()); }
        if dirty {
            draw(&mut a, width, 36);
            let pane = &a.layout.panes[0];
            trace.push((a.session.state().column(), pane.hits.len(), pane.hits.first().map(|hit| hit.byte), pane.hits.last().map(|hit| hit.byte), a.next_deadline().is_some()));
            if trace.len() > 12 { trace.remove(0); }
        } else { std::thread::yield_now(); }
    }
    assert_eq!(a.session.state().selection().unwrap().head, expected);
    assert!(a.session.native_reveal(0).is_none());
    if expected + 3 <= source.len() && &source[expected..expected + 3] == "END" {
        key(&mut a, KeyCode::Right, KeyModifiers::SHIFT);
        key(&mut a, KeyCode::Right, KeyModifiers::SHIFT);
        key(&mut a, KeyCode::Right, KeyModifiers::SHIFT);
        key(&mut a, KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(a.copy.take().as_deref(), Some("END"));
    }
    assert_eq!(std::fs::read_to_string(path).unwrap(), source);
    }
}
#[test]
fn resizing_a_pending_wrapped_reveal_keeps_its_exact_caret_row_visible() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("resize-keycaps.txt");
    let source = "1\u{FE0F}\u{20E3}".repeat(500) + " END";
    std::fs::write(&path, &source).unwrap();
    let mut a = app(dir.path());
    a.dispatch(Command::Open(path.clone()));
    settle(&mut a, |a| a.session.state().active().is_some_and(|tab| tab.loaded()));
    a.dispatch(Command::SetSoftWrap(true));
    a.dispatch(Command::SetStickyScroll(false));
    draw(&mut a, 80, 10);
    a.dispatch(Command::GoToLine(lenz_content::goto::LineColumn { line: 1, column: Some(1_502) }));
    settle(&mut a, |a| a.session.native_reveal(0).is_some());
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        draw(&mut a, 28, 10);
        if a.layout.panes[0].hits.iter().any(|hit| hit.byte == 3_501) { break; }
        assert!(Instant::now() < deadline, "resize lost the pending caret's logical row");
        while let Ok(done) = a.session.completions().try_recv() { a.apply(done); }
        std::thread::yield_now();
    }
    assert_eq!(a.session.state().selection().unwrap().head, 3_501);
    assert_eq!(std::fs::read_to_string(path).unwrap(), source);
}
#[test]
fn wrapped_keycap_in_one_text_cell_has_an_exact_source_indicator() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("keycap.txt");
    let source = "1\u{FE0F}\u{20E3}";
    std::fs::write(&path, source).unwrap();
    let mut a = app(dir.path());
    a.dispatch(Command::Open(path.clone()));
    settle(&mut a, |a| a.session.state().active().is_some_and(|tab| tab.loaded()));
    a.dispatch(Command::SetSoftWrap(true));
    a.dispatch(Command::SetStickyScroll(false));
    let mut checked = false;
    for width in 4..10 {
        a.dispatch(Command::GoToLine(lenz_content::goto::LineColumn { line: 1, column: Some(1) }));
        let rendered = draw_screen(&mut a, width, 10);
        let pane = &a.layout.panes[0];
        if pane.text.width == 1 {
            let hit = pane.hits.iter().find(|hit| hit.byte == 0).expect("overwide source remains reachable");
            assert_eq!(hit.width, 1);
            assert!(rendered.contains('▏'), "a one-cell indicator replaces the unfit glyph");
            checked = true;
        }
    }
    assert!(checked, "fixture must exercise one native text cell");
    assert_eq!(std::fs::read_to_string(path).unwrap(), source);
}
#[test]
fn find_reveals_a_match_after_the_long_unwrapped_prefix() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("find-tail.txt");
    let source = "x".repeat(20_000) + " END" + &" ".repeat(100);
    std::fs::write(&path, &source).unwrap();
    let mut a = app(dir.path());
    a.dispatch(Command::Open(path.clone()));
    settle(&mut a, |a| a.session.state().active().is_some_and(|tab| tab.loaded()));
    a.dispatch(Command::SetSoftWrap(false));
    draw(&mut a, 80, 20);
    a.run(CommandId::FindInFile);
    a.handle(Event::Paste("END".into()));
    settle(&mut a, |a| a.session.state().active().and_then(|tab| tab.find()).is_some_and(|find| find.matches().len() == 1));
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        draw(&mut a, 80, 20);
        if a.layout.panes[0].hits.iter().any(|hit| hit.byte == 20_001) { break; }
        assert!(Instant::now() < deadline, "found match stays outside the viewport");
        while let Ok(done) = a.session.completions().try_recv() { a.apply(done); }
        std::thread::yield_now();
    }
    key(&mut a, KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert_eq!(a.copy.take().as_deref(), Some("END"));
    assert_eq!(std::fs::read_to_string(path).unwrap(), source);
}
#[test]
fn suppressed_zero_width_windows_keep_bounded_hits_and_exact_source_end() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("zero-width-tail.txt");
    let source = "\u{200B}".repeat(200_000);
    std::fs::write(&path, &source).unwrap();
    let mut a = app(dir.path());
    a.dispatch(Command::Open(path.clone()));
    settle(&mut a, |a| a.session.state().current_source().is_some());
    a.dispatch(Command::SetSoftWrap(false));
    a.dispatch(Command::SetStickyScroll(false));
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        draw(&mut a, 80, 20);
        let pane = &a.layout.panes[0];
        assert!(pane.hits.iter().filter(|hit| hit.y == pane.text.y).count() <= lenz_content::display::MAX_MARKS + 2);
        if pane.hits.iter().any(|hit| hit.byte == source.len()) { break; }
        assert!(Instant::now() < deadline, "suppressed source tail never finished");
        while let Ok(completion) = a.session.completions().try_recv() { a.apply(completion); }
        std::thread::yield_now();
    }
    assert_eq!(a.layout.panes[0].hits.first().map(|hit| hit.byte), Some(0));
    draw(&mut a, 1, 1);
    draw(&mut a, 80, 20);
    assert_eq!(a.layout.panes[0].hits.first().map(|hit| hit.byte), Some(0));
    a.dispatch(Command::SelectAll);
    key(&mut a, KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert_eq!(a.copy.take().as_deref(), Some(source.as_str()));
    assert_eq!(std::fs::read_to_string(path).unwrap(), source);
}
#[test]
fn wrapped_continuation_whitespace_keeps_the_caret_on_its_clicked_row() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("row-end.txt");
    let source = "abcdef ".repeat(20);
    std::fs::write(&path, &source).unwrap();
    let mut a = app(dir.path());
    a.dispatch(Command::Open(path.clone()));
    settle(&mut a, |a| a.session.state().current_source().is_some());
    a.dispatch(Command::SetSoftWrap(true));
    a.dispatch(Command::SetStickyScroll(false));
    draw(&mut a, 30, 10);
    let pane = &a.layout.panes[0];
    let end = pane.hits.iter().rfind(|hit| hit.y == pane.text.y).unwrap();
    assert!(end.width > 1, "fixture needs whitespace after the continued row");
    let (x, y) = (end.x + end.width - 1, end.y);
    click(&mut a, x, y);
    assert_eq!(a.session.state().selection().unwrap().head, 20);
    assert_eq!(std::fs::read_to_string(path).unwrap(), source);
}
#[test]
fn unwrapped_end_then_up_starts_the_caret_at_the_visible_source_tail() {
    for shift in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("end-up.txt");
        let source: String = (0..100).map(|line| format!("line{line:03}\n")).collect();
        std::fs::write(&path, &source).unwrap();
        let mut a = app(dir.path());
        a.dispatch(Command::Open(path.clone()));
        settle(&mut a, |a| a.session.state().current_source().is_some());
        a.dispatch(Command::SetSoftWrap(false));
        draw(&mut a, 80, 20);
        key(&mut a, KeyCode::End, KeyModifiers::NONE);
        draw(&mut a, 80, 20);
        key(&mut a, KeyCode::Up, if shift { KeyModifiers::SHIFT } else { KeyModifiers::NONE });
        draw(&mut a, 80, 20);
        let state = a.session.state();
        let view = state.view(state.active().unwrap().id()).unwrap();
        assert_eq!(view.snapshot().line_of(state.selection().unwrap().head), 99);
        if shift { assert_eq!(copied_selection(state).as_deref(), Some("line099\n")); }
        assert_eq!(std::fs::read_to_string(path).unwrap(), source);
    }
}
#[test]
fn wrapped_end_then_up_reaches_unicode_tail_and_copies_absolute_source_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("wrapped-families.txt");
    let family = "👨‍👩‍👧‍👦";
    let source = family.repeat(2_000) + " END";
    std::fs::write(&path, &source).unwrap();
    let mut a = app(dir.path());
    a.dispatch(Command::Open(path.clone()));
    settle(&mut a, |a| a.session.state().current_source().is_some());
    a.dispatch(Command::SetSoftWrap(true));
    a.dispatch(Command::SetStickyScroll(false));
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(28, 10)).unwrap();
    terminal.draw(|f| a.draw(f)).unwrap();
    key(&mut a, KeyCode::End, KeyModifiers::NONE);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        terminal.draw(|f| a.draw(f)).unwrap();
        if a.layout.panes[0].hits.iter().any(|hit| hit.byte == 50_001) { break; }
        assert!(Instant::now() < deadline, "wrapped End never exposed the uncapped source tail");
        while let Ok(completion) = a.session.completions().try_recv() { a.apply(completion); }
        std::thread::yield_now();
    }
    let end_row = a.session.state().active().unwrap().top_row();
    assert!(end_row > 500);
    key(&mut a, KeyCode::Up, KeyModifiers::NONE);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        terminal.draw(|f| a.draw(f)).unwrap();
        let selection = a.session.state().selection();
        if a.session.state().active().unwrap().top_row() < end_row && selection.is_some_and(|selection| selection.head > 49_000 && selection.head < 50_001 && selection.head % 25 == 0)
            && a.layout.panes[0].hits.iter().any(|hit| selection.is_some_and(|selection| hit.byte == selection.head)) { break; }
        assert!(Instant::now() < deadline, "Up lost the exact wrapped source position");
        while let Ok(completion) = a.session.completions().try_recv() { a.apply(completion); }
    }
    key(&mut a, KeyCode::End, KeyModifiers::NONE);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        terminal.draw(|f| a.draw(f)).unwrap();
        if a.layout.panes[0].hits.iter().any(|hit| hit.byte == 50_001) { break; }
        assert!(Instant::now() < deadline);
        while let Ok(completion) = a.session.completions().try_recv() { a.apply(completion); }
    }
    let pane = &a.layout.panes[0];
    for hit in &pane.hits {
        if terminal.backend().buffer()[(hit.x, hit.y)].symbol() == family {
            assert_eq!(hit.width, 2);
            assert_eq!(hit.byte % 25, 0);
        }
    }
    let begin = pane.hits.iter().find(|hit| hit.byte == 50_001).unwrap();
    let end = pane.hits.iter().find(|hit| hit.byte == 50_004).unwrap();
    let (x, y, end_x, end_y) = (begin.x, begin.y, end.x, end.y);
    click(&mut a, x, y);
    a.handle(Event::Mouse(MouseEvent { kind: MouseEventKind::Drag(MouseButton::Left), column: end_x, row: end_y, modifiers: KeyModifiers::NONE }));
    key(&mut a, KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert_eq!(a.copy.take().as_deref(), Some("END"));
    draw(&mut a, 1, 1);
    key(&mut a, KeyCode::F(1), KeyModifiers::NONE);
    key(&mut a, KeyCode::End, KeyModifiers::NONE);
    key(&mut a, KeyCode::Up, KeyModifiers::NONE);
    draw(&mut a, 1, 1);
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    assert!(a.modal.is_none());
    assert_eq!(std::fs::read_to_string(path).unwrap(), source);
}
#[test]
fn cold_horizontal_seek_completes_every_visible_long_line_without_starving_later_rows() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("many-long-lines.txt");
    let source: String = (0..40).map(|line| format!("{}TAIL_{line:02}{}\n", "x".repeat(12_000), " ".repeat(100))).collect();
    std::fs::write(&path, &source).unwrap();
    let mut a = app(dir.path());
    a.dispatch(Command::Open(path.clone()));
    settle(&mut a, |a| a.session.state().current_source().is_some());
    a.dispatch(Command::SetSoftWrap(false));
    a.dispatch(Command::SetStickyScroll(false));
    draw(&mut a, 100, 50);
    a.dispatch(Command::ScrollColumn(12_000));
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        draw(&mut a, 100, 50);
        let pane = &a.layout.panes[0];
        if (0..40).all(|line| pane.hits.iter().any(|hit| hit.line == line && hit.byte == 12_000)) { break; }
        assert!(Instant::now() < deadline, "visible cold lines did not finish");
        while let Ok(completion) = a.session.completions().try_recv() { a.apply(completion); }
        std::thread::yield_now();
    }
    assert!(a.session.projection(0).is_none(), "finished viewports must release the raw batch");
    key(&mut a, KeyCode::Left, KeyModifiers::CONTROL | KeyModifiers::ALT);
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        draw(&mut a, 100, 50);
        if a.layout.panes[0].hits.iter().any(|hit| hit.line == 0 && hit.byte == 11_996) { break; }
        assert!(Instant::now() < deadline);
        while let Ok(completion) = a.session.completions().try_recv() { a.apply(completion); }
    }
    assert_eq!(a.session.state().column(), 11_996);
    assert_eq!(std::fs::read_to_string(path).unwrap(), source);
}
#[test]
fn oversized_joined_grapheme_escapes_keep_original_source_hit_and_copy_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("joined.txt");
    let source = "👩\u{200D}".repeat(1_000) + "💻 END\n";
    std::fs::write(&path, &source).unwrap();
    let mut a = app(dir.path());
    a.dispatch(Command::Open(path.clone()));
    settle(&mut a, |a| a.session.state().current_source().is_some());
    a.dispatch(Command::SetSoftWrap(false));
    a.dispatch(Command::SetStickyScroll(false));
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 20)).unwrap();
    terminal.draw(|f| a.draw(f)).unwrap();
    let pane = &a.layout.panes[0];
    let escaped = pane.hits.iter().find(|hit| hit.byte == 256).unwrap();
    let next = pane.hits.iter().find(|hit| hit.byte == 259).unwrap();
    let (x, y, end) = (escaped.x, escaped.y, next.x);
    assert_eq!(terminal.backend().buffer()[(x, y)].symbol(), "[");
    let escape: String = (x..x + 8).map(|x| terminal.backend().buffer()[(x, y)].symbol()).collect();
    assert_eq!(escape, "[U+200D]");
    click(&mut a, x, y);
    a.handle(Event::Mouse(MouseEvent { kind: MouseEventKind::Drag(MouseButton::Left), column: end, row: y, modifiers: KeyModifiers::NONE }));
    assert_eq!(copied_selection(a.session.state()).as_deref(), Some("\u{200D}"));
    key(&mut a, KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert_eq!(a.copy.take().as_deref(), Some("\u{200D}"));
    assert_eq!(std::fs::read_to_string(path).unwrap(), source);
}
#[test]
fn double_and_triple_click_select_word_and_group() {
    let dir = tempfile::tempdir().unwrap();
    let src = "fn f() { g(\"hello world\", 1); }\n";
    let path = dir.path().join("a.rs");
    std::fs::write(&path, src).unwrap();
    let mut a = app(dir.path());
    a.dispatch(Command::Open(path.clone()));
    settle(&mut a, |a| a.session.state().current_source().is_some());
    draw(&mut a, 80, 20);
    let (x, y) = {
        let p = &a.layout.panes[0];
        (p.text.x + src.find("world").unwrap() as u16 + 2, p.text.y)
    };
    let sel = |a: &App| copied_selection(a.session.state()).unwrap_or_default();
    click(&mut a, x, y);
    click(&mut a, x, y);
    assert_eq!(sel(&a), "world");
    click(&mut a, x, y);
    settle(&mut a, |a| copied_selection(a.session.state()).as_deref().is_some_and(|t| t.len() > 5));
    assert_eq!(sel(&a), "hello world");
    // Async selection/declaration replies can retire hit geometry. Render as the event loop does before another click.
    draw(&mut a, 80, 20);
    click(&mut a, x, y);
    assert_eq!(sel(&a), "", "a fourth click starts over");
    // A second click after the pause, or on another cell, is a new first click.
    std::thread::sleep(Duration::from_millis(450));
    click(&mut a, x, y);
    assert_eq!(sel(&a), "");
    click(&mut a, x + 1, y);
    assert_eq!(sel(&a), "");
    assert_eq!(std::fs::read_to_string(path).unwrap(), src);
}
#[test]
fn split_tab_and_bookmark_shortcuts_reach_shared_state() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("one.txt");
    let second = dir.path().join("two.txt");
    std::fs::write(&first, "one\ntwo\nthree\n").unwrap();
    std::fs::write(&second, "other\n").unwrap();
    let mut a = app(dir.path());
    a.dispatch(Command::OpenRetained(first.clone()));
    settle(&mut a, |a| a.session.state().current_source().is_some());
    key(&mut a, KeyCode::Char('\\'), KeyModifiers::CONTROL);
    assert_eq!(
        a.session.state().split(),
        Some(code_core::session::Split::Columns)
    );
    assert_eq!(a.session.state().focused_pane(), 1);
    a.dispatch(Command::OpenRetained(second));
    settle(&mut a, |a| {
        a.session
            .state()
            .active()
            .is_some_and(|t| t.source().ends_with("two.txt") && t.loaded())
    });
    key(&mut a, KeyCode::Char('1'), KeyModifiers::CONTROL);
    assert_eq!(a.session.state().active().unwrap().source(), first);
    key(&mut a, KeyCode::F(11), KeyModifiers::CONTROL);
    assert_eq!(a.modal.as_ref().unwrap().kind, Prompt::Mnemonic);
    key(&mut a, KeyCode::Char('a'), KeyModifiers::NONE);
    assert!(a.modal.is_none());
    assert_eq!(
        a.session.state().places().bookmarks()[0].mnemonic,
        Some('A')
    );
    a.dispatch(Command::GoToLine(
        lenz_content::goto::parse_line_column("3").unwrap(),
    ));
    a.dispatch(Command::JumpMnemonic('A'));
    assert_eq!(a.session.state().selection().unwrap().head, 0);
    key(&mut a, KeyCode::F(11), KeyModifiers::SHIFT);
    assert_eq!(a.choices().len(), 1);
    key(&mut a, KeyCode::Delete, KeyModifiers::NONE);
    assert!(a.session.state().places().bookmarks().is_empty());
}
#[test]
fn project_search_results_preview_then_open_through_input() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("file.txt");
    std::fs::write(&path, "first\nneedle\nlast\n").unwrap();
    let mut a = app(dir.path());
    settle(&mut a, |a| {
        a.session
            .state()
            .project()
            .is_some_and(|p| matches!(p.coverage(), code_core::project::Coverage::Complete(_)))
    });
    key(
        &mut a,
        KeyCode::Char('f'),
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    );
    a.handle(Event::Paste("needle".into()));
    key(&mut a, KeyCode::Tab, KeyModifiers::NONE);
    a.handle(Event::Paste("*.txt".into()));
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    settle(&mut a, |a| {
        a.session.state().results().is_some_and(|r| r.done())
    });
    assert_eq!(a.session.state().results().unwrap().hits().len(), 1);
    assert!(a.session.state().active().is_none());
    draw(&mut a, 100, 30);
    key(&mut a, KeyCode::End, KeyModifiers::NONE);
    settle(&mut a, |a| {
        a.session
            .state()
            .results()
            .is_some_and(|r| r.preview().is_some())
    });
    assert!(a.session.state().active().is_none());
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    settle(&mut a, |a| a.session.state().current_source().is_some());
    assert_eq!(a.session.state().active().unwrap().source(), path);
    assert_eq!(a.session.state().selection().unwrap().range(), 6..12);
}
#[test]
fn path_validation_and_empty_independent_view_preserve_open_document() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("one.txt");
    std::fs::write(&path, "one").unwrap();
    let mut a = app(dir.path());
    a.dispatch(Command::Open(path.clone()));
    settle(&mut a, |a| a.session.state().current_source().is_some());
    a.run(CommandId::OpenFolder);
    a.handle(Event::Paste("one.txt".into()));
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    assert!(a.modal.as_ref().unwrap().error.is_some());
    assert_eq!(a.session.state().active().unwrap().source(), path);
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    a.run(CommandId::NewWindow);
    assert!(a.new_view);
    assert_eq!(a.session.state().active().unwrap().source(), path);
    let empty = App::new(config(dir.path()), None);
    assert!(empty.session.state().active().is_none());
    key(&mut a, KeyCode::F(1), KeyModifiers::NONE);
    key(&mut a, KeyCode::Char('q'), KeyModifiers::CONTROL);
    assert!(!a.quit, "a modal consumes close input before the runtime");
    assert!(a.modal.is_some());
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    key(&mut a, KeyCode::Char('q'), KeyModifiers::CONTROL);
    assert!(a.quit);
}

#[test]
fn open_in_editor_starts_the_launch_and_compare_with_asks_for_the_other_side() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("one.txt");
    std::fs::write(&path, "one").unwrap();
    std::fs::create_dir(dir.path().join("sub")).unwrap();
    std::fs::write(dir.path().join("sub/two.txt"), "two").unwrap();
    let mut a = app(dir.path());
    a.dispatch(Command::Open(path.clone()));
    settle(&mut a, |a| a.session.state().current_source().is_some() && a.session.state().tree_rows().any(|r| &*r.name == "sub"));
    a.run(CommandId::OpenInEditor);
    a.run(CommandId::OpenWithDefaultApp);
    assert_eq!(a.launched.len(), 2);
    assert_eq!(a.launched[1].args.last().map(String::as_str), Some(&*path.to_string_lossy()));
    assert!(a.modal.is_none());
    // From the text: the active file; the other side must exist, and Lenz Diff must sit beside this executable.
    a.run(CommandId::CompareWith);
    assert_eq!(a.modal.as_ref().unwrap().kind, Prompt::Compare(path.clone()));
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(a.modal.as_ref().unwrap().error.as_deref(), Some("Enter a file or folder path"));
    a.handle(Event::Paste("gone.txt".into()));
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    assert!(a.modal.as_ref().unwrap().error.as_deref().unwrap().contains("no such file"));
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    // From the explorer: its highlighted row, a folder here.
    a.focus = Focus::Tree;
    a.tree_at = a.session.state().tree_rows().position(|r| &*r.name == "sub").unwrap();
    a.run(CommandId::CompareWith);
    assert_eq!(a.modal.as_ref().unwrap().kind, Prompt::Compare(dir.path().join("sub")));
    a.handle(Event::Paste("one.txt".into()));
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    assert!(a.modal.as_ref().unwrap().error.as_deref().unwrap().contains("a folder with a folder"));
    assert!(a.foreground.is_none());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "one");
}

#[test]
fn markdown_preview_selects_and_copies_rendered_text_without_source_selection() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notes.md");
    std::fs::write(&path, "# Heading\n\nSome **bold** text.\n").unwrap();
    let mut a = app(dir.path());
    a.dispatch(Command::Open(path.clone()));
    settle(&mut a, |a| a.session.state().current_source().is_some());
    a.run(CommandId::MarkdownSource);
    settle(&mut a, |a| {
        matches!(a.session.state().rendering(), Some(Rendering::Preview(_)))
    });
    draw(&mut a, 100, 30);
    let rendered = match a.session.state().rendering() {
        Some(Rendering::Preview(p)) => p.doc.text.clone(),
        _ => unreachable!(),
    };
    let source_selection = a.session.state().selection();
    key(&mut a, KeyCode::Char('a'), KeyModifiers::CONTROL);
    key(&mut a, KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert_eq!(a.copy.take().as_deref(), Some(rendered.as_str()));
    assert!(!rendered.contains("**"));
    assert_eq!(a.session.state().selection(), source_selection);
    let (start, end) = {
        let p = a
            .layout
            .panes
            .iter()
            .find(|p| p.pane == a.session.state().focused_pane())
            .unwrap();
        assert!(p.preview);
        let first = p.hits.first().unwrap();
        let third = p.hits.iter().filter(|h| h.y == first.y).nth(2).unwrap();
        ((first.x, first.y), (third.x, third.y))
    };
    click(&mut a, start.0, start.1);
    a.handle(Event::Mouse(MouseEvent {
        kind: MouseEventKind::Drag(MouseButton::Left),
        column: end.0,
        row: end.1,
        modifiers: KeyModifiers::NONE,
    }));
    key(&mut a, KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert_eq!(a.copy.take().as_deref(), Some("He"));
    a.run(CommandId::MarkdownSource);
    assert!(a.preview_selection.is_none());
    assert_eq!(
        std::fs::read_to_string(path).unwrap(),
        "# Heading\n\nSome **bold** text.\n"
    );
}

#[test]
fn terminal_shortcuts_wrap_toggle_preview_reorder_tabs_and_reverse_focus() {
    let dir = tempfile::tempdir().unwrap();
    let one = dir.path().join("one.txt");
    let two = dir.path().join("two.txt");
    std::fs::write(&one, "one\n").unwrap();
    std::fs::write(&two, "two\n").unwrap();
    let mut a = app(dir.path());
    a.dispatch(Command::OpenRetained(one));
    settle(&mut a, |a| a.session.state().current_source().is_some());
    a.dispatch(Command::OpenRetained(two));
    settle(&mut a, |a| {
        a.session
            .state()
            .active()
            .is_some_and(|t| t.loaded() && t.source().ends_with("two.txt"))
    });
    let id = a.session.state().active().unwrap().id();
    key(
        &mut a,
        KeyCode::Left,
        KeyModifiers::ALT | KeyModifiers::SHIFT,
    );
    assert_eq!(a.session.state().pane_tabs(0).next().unwrap().id(), id);
    let wrap = a.session.state().soft_wrap();
    key(&mut a, KeyCode::Char('z'), KeyModifiers::ALT);
    assert_eq!(a.session.state().soft_wrap(), !wrap);
    key(&mut a, KeyCode::Tab, KeyModifiers::NONE);
    assert_eq!(a.focus, Focus::Tree);
    key(&mut a, KeyCode::BackTab, KeyModifiers::SHIFT);
    assert_eq!(a.focus, Focus::Text);
}
#[test]
fn long_single_line_markdown_preview_scrolls_within_block_and_clamps() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("long.md");
    std::fs::write(&path, "word ".repeat(2_000)).unwrap();
    let mut a = app(dir.path());
    a.dispatch(Command::Open(path));
    settle(&mut a, |a| a.session.state().current_source().is_some());
    key(
        &mut a,
        KeyCode::Char('v'),
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    );
    settle(&mut a, |a| {
        matches!(a.session.state().rendering(), Some(Rendering::Preview(_)))
    });
    draw(&mut a, 60, 15);
    let first = a.layout.panes[0].hits.first().unwrap().byte;
    key(&mut a, KeyCode::PageDown, KeyModifiers::NONE);
    draw(&mut a, 60, 15);
    assert!(a.layout.panes[0].hits.first().unwrap().byte > first);
    key(&mut a, KeyCode::End, KeyModifiers::NONE);
    draw(&mut a, 60, 15);
    let end = a.layout.panes[0].preview_top;
    assert!(end > 0);
    key(&mut a, KeyCode::Up, KeyModifiers::NONE);
    draw(&mut a, 60, 15);
    assert_eq!(a.layout.panes[0].preview_top, end - 1);
    draw(&mut a, 25, 8);
    assert!(a.layout.panes[0].preview_top < a.layout.panes[0].preview_total);
    key(&mut a, KeyCode::Home, KeyModifiers::NONE);
    draw(&mut a, 25, 8);
    assert_eq!(a.layout.panes[0].preview_top, 0);
}

#[test]
fn markdown_preview_keeps_its_scroll_when_a_retained_tab_is_released_and_reopened() {
    let dir = tempfile::tempdir().unwrap();
    let markdown = dir.path().join("cached-preview.md");
    let other = dir.path().join("other.txt");
    let source = "word ".repeat(2_000);
    std::fs::write(&markdown, &source).unwrap();
    std::fs::write(&other, "other source\n").unwrap();
    let mut a = app(dir.path());
    a.dispatch(Command::OpenRetained(markdown.clone()));
    settle(&mut a, |a| a.session.state().current_source().is_some());
    key(&mut a, KeyCode::Char('v'), KeyModifiers::CONTROL | KeyModifiers::SHIFT);
    settle(&mut a, |a| matches!(a.session.state().rendering(), Some(Rendering::Preview(_))));
    draw(&mut a, 60, 15);
    key(&mut a, KeyCode::PageDown, KeyModifiers::NONE);
    draw(&mut a, 60, 15);
    let id = a.session.state().active().unwrap().id();
    let pane = a.session.state().focused_pane();
    let top = a.layout.panes[pane as usize].preview_top;
    let first_byte = a.layout.panes[pane as usize].hits.first().unwrap().byte;
    assert!(top > 0 && first_byte > 0, "fixture must be scrolled inside the long Markdown block");

    a.dispatch(Command::OpenRetained(other.clone()));
    settle(&mut a, |a| a.session.state().active().is_some_and(|tab| tab.source() == other && tab.loaded()));
    draw(&mut a, 60, 15);
    assert!(a.session.state().tabs().iter().any(|tab| tab.id() == id && !tab.loaded()), "Markdown remains retained as an inactive released tab");
    assert!(a.session.state().view(id).is_none(), "released text must not be exposed as a loaded view");

    key(&mut a, KeyCode::BackTab, KeyModifiers::CONTROL | KeyModifiers::SHIFT);
    settle(&mut a, |a| a.session.state().active().is_some_and(|tab| tab.id() == id && tab.loaded())
        && matches!(a.session.state().rendering(), Some(Rendering::Preview(_))));
    draw(&mut a, 60, 15);
    assert_eq!(a.session.state().focused_pane(), pane);
    assert_eq!(a.layout.panes[pane as usize].preview_top, top, "cached Markdown resumes the same preview row");
    assert_eq!(a.layout.panes[pane as usize].hits.first().unwrap().byte, first_byte, "the same rendered source position is visible");
    assert_eq!(std::fs::read_to_string(markdown).unwrap(), source);
    assert_eq!(std::fs::read_to_string(other).unwrap(), "other source\n");
}

#[test]
fn tree_selection_follows_path_when_streaming_inserts_an_earlier_row() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("z.txt"), "z").unwrap();
    let mut a = app(dir.path());
    settle(&mut a, |a| {
        a.session
            .state()
            .project()
            .is_some_and(|p| matches!(p.coverage(), code_core::project::Coverage::Complete(_)))
    });
    let p = a.session.state().project().unwrap();
    a.tree_at = p.rows().iter().position(|r| &*r.path == "z.txt").unwrap();
    let generation = p.generation();
    a.apply(Completion::Scanned {
        generation,
        batch: code_core::project::Batch {
            paths: vec!["a.txt".into()],
            done: true,
            skipped: 0,
            error: None,
        },
    });
    assert_eq!(
        &*a.session.state().project().unwrap().rows()[a.tree_at].path,
        "z.txt"
    );
}
#[test]
fn hover_is_debounced_cancelled_and_uses_real_core_documentation_request() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"hover-fixture\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    let path = root.join("main.rs");
    let source = "fn main() { helper(); }\nfn helper() {}\n";
    std::fs::write(&path, source).unwrap();
    let profile = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_owned();
    let mut build =
        std::process::Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    build
        .args(["build", "-q", "-p", "code-core", "--example", "fake_lsp"])
        .current_dir(env!("CARGO_MANIFEST_DIR"));
    if profile.ends_with("release") {
        build.arg("--release");
    }
    assert!(build.status().unwrap().success());
    let fake = profile.join(format!("examples/fake_lsp{}", std::env::consts::EXE_SUFFIX));
    let log = dir.path().join("hover.jsonl");
    let mut cfg = config(dir.path());
    cfg.idle = code_core::semantics::IDLE;
    std::fs::create_dir_all(cfg.providers.parent().unwrap()).unwrap();
    std::fs::write(&cfg.providers, serde_json::json!({ "providers": { "rust-analyzer": { "executable": fake, "env": { "FAKE_LSP_LOG": log, "FAKE_LSP_NO_DECOR": "1" } } } }).to_string()).unwrap();
    let mut a = App::new(cfg, Some(root));
    a.dispatch(Command::Open(path.clone()));
    settle(&mut a, |a| {
        a.session
            .state()
            .provider()
            .is_some_and(|p| p.status == code_core::semantics::Status::Ready)
            && a.session.state().view(a.session.state().active().unwrap().id()).unwrap().highlights().is_some()
    });
    draw(&mut a, 100, 30);
    let at = source.find("helper").unwrap();
    let (x, y) = {
        let h = a.layout.panes[0]
            .hits
            .iter()
            .find(|h| h.line == 0 && h.byte == at)
            .unwrap();
        (h.x, h.y)
    };
    let moved = Event::Mouse(MouseEvent {
        kind: MouseEventKind::Moved,
        column: x,
        row: y,
        modifiers: KeyModifiers::NONE,
    });
    a.handle(moved.clone());
    let due = a.next_deadline().unwrap();
    assert!(!a.tick(due - Duration::from_millis(1)));
    a.handle(moved.clone());
    assert_eq!(a.next_deadline(), Some(due));
    let selection = a.session.state().selection();
    assert!(a.tick(due));
    settle(&mut a, |a| a.session.state().documentation().is_some());
    assert_eq!(
        a.session.state().documentation().unwrap().trigger,
        code_core::session::DocTrigger::Pointer
    );
    assert!(matches!(a.modal.as_ref().unwrap().kind, Prompt::Details { .. }));
    assert_eq!(a.session.state().selection(), selection);
    let messages = std::fs::read_to_string(&log).unwrap();
    assert_eq!(
        messages
            .lines()
            .filter(
                |line| serde_json::from_str::<serde_json::Value>(line).unwrap()["method"]
                    == "textDocument/hover"
            )
            .count(),
        1
    );
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    assert!(a.session.state().documentation().is_none());
    a.run(CommandId::DocumentationOnHover);
    a.handle(moved.clone());
    assert!(a.next_deadline().is_none());
    a.run(CommandId::DocumentationOnHover);
    a.handle(moved);
    assert!(a.next_deadline().is_some());
    key(&mut a, KeyCode::F(10), KeyModifiers::NONE);
    assert!(a.next_deadline().is_none());
    assert_eq!(std::fs::read_to_string(path).unwrap(), source);
}

#[test]
fn shifted_selection_head_stays_in_real_terminal_viewport() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("lines.txt");
    let source = (0..100)
        .map(|line| format!("line {line}\n"))
        .collect::<String>();
    std::fs::write(&path, &source).unwrap();
    let mut a = app(dir.path());
    a.dispatch(Command::Open(path.clone()));
    settle(&mut a, |a| a.session.state().current_source().is_some());
    draw(&mut a, 80, 12);
    for _ in 0..30 {
        key(&mut a, KeyCode::Down, KeyModifiers::SHIFT);
        draw(&mut a, 80, 12);
    }
    let head = a.session.state().selection().unwrap().head;
    let line = a.session.state().current_source().unwrap().line_of(head);
    assert_eq!(line, 30);
    assert!(a.layout.panes[0].rows.iter().any(|(_, l, _)| *l == line));
    assert_eq!(std::fs::read_to_string(path).unwrap(), source);
}

#[test]
fn followed_rename_invalidates_source_context_hits_without_changing_text_version() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("before.txt");
    let moved = dir.path().join("after.txt");
    std::fs::write(&path, "relative-link target\n").unwrap();
    let mut a = app(dir.path());
    a.dispatch(Command::OpenRetained(path.clone()));
    settle(&mut a, |a| a.session.state().current_source().is_some());
    draw(&mut a, 80, 20);
    let (x, y) = (a.layout.panes[0].text.x + 4, a.layout.panes[0].text.y);
    let tab = a.session.state().active().unwrap().id();
    let key = a.session.state().view(tab).unwrap().snapshot().key();
    std::fs::rename(&path, &moved).unwrap();
    settle(&mut a, |a| a.session.state().active().is_some_and(|t| t.source() == moved));
    assert_eq!(a.session.state().view(tab).unwrap().snapshot().key(), key);
    a.handle(Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Right), column: x, row: y, modifiers: KeyModifiers::NONE,
    }));
    assert!(a.modal.is_none(), "old source context must wait for a new frame");
    assert_eq!(a.session.state().selection(), None);
    assert_eq!(std::fs::read_to_string(moved).unwrap(), "relative-link target\n");
}

#[test]
fn hover_identifier_lookup_uses_bounded_current_snapshot_windows() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("long.rs");
    let source = format!("{}\n{}target(); // hidden\n{}", "header".repeat(1000), " ".repeat(12_000), "a".repeat(10_000));
    std::fs::write(&path, &source).unwrap();
    let mut a = app(dir.path());
    a.dispatch(Command::Open(path.clone()));
    settle(&mut a, |a| a.session.state().active().is_some_and(|t| a.session.state().view(t.id()).is_some_and(|view| view.highlights().is_some())));
    let state = a.session.state();
    let view = state.view(state.active().unwrap().id()).unwrap();
    assert_eq!(view.linkable(1, 12_002), Some(12_000..12_006));
    assert_eq!(view.linkable(1, 12_015), None, "comments are not hover targets");
    assert_eq!(view.linkable(2, 5_000), None, "do not fabricate a truncated long identifier");
    assert_eq!(std::fs::read_to_string(path).unwrap(), source);
}

#[test]
fn changed_inlay_hints_invalidate_source_hits_until_the_next_frame() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"hint-geometry\"\nversion = \"0.1.0\"\n").unwrap();
    let path = root.join("main.rs");
    let source = "fn main() { let value = helper(); }\nfn helper() {}\n";
    std::fs::write(&path, source).unwrap();
    let profile = std::env::current_exe().unwrap().parent().unwrap().parent().unwrap().to_owned();
    let mut build = std::process::Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    build.args(["build", "-q", "-p", "code-core", "--example", "fake_lsp"])
        .current_dir(env!("CARGO_MANIFEST_DIR"));
    if profile.ends_with("release") { build.arg("--release"); }
    assert!(build.status().unwrap().success());
    let fake = profile.join(format!("examples/fake_lsp{}", std::env::consts::EXE_SUFFIX));
    let cfg = config(dir.path());
    std::fs::create_dir_all(cfg.providers.parent().unwrap()).unwrap();
    std::fs::write(&cfg.providers, serde_json::json!({"providers":{"rust-analyzer":{"executable":fake,"env":{"FAKE_LSP_SLOW_DECOR":"1"}}}}).to_string()).unwrap();
    let mut a = App::new(cfg, Some(root));
    a.dispatch(Command::Open(path.clone()));
    settle(&mut a, |a| a.session.state().provider().is_some_and(|p| p.status == code_core::semantics::Status::Ready)
        && a.session.state().active().is_some_and(|t| a.session.state().view(t.id()).is_some_and(|view| view.highlights().is_some())));
    draw(&mut a, 80, 20);
    let (x, y) = (a.layout.panes[0].text.x + 25, a.layout.panes[0].text.y);
    a.handle(Event::Mouse(MouseEvent { kind: MouseEventKind::Moved, column: x, row: y, modifiers: KeyModifiers::NONE }));
    assert!(a.next_deadline().is_some());
    settle(&mut a, |a| a.session.state().active().is_some_and(|t| a.session.state().view(t.id()).is_some_and(|view| view.inlay_hints().is_some())));
    assert!(a.next_deadline().is_none(), "late hint layout invalidates the pending hover target");
    draw(&mut a, 80, 20);
    let old_hints = Arc::downgrade(a.session.state().view(a.session.state().active().unwrap().id()).unwrap().inlay_hints().unwrap());
    let (x, y) = (a.layout.panes[0].text.x + 24, a.layout.panes[0].text.y);
    a.dispatch(Command::SetInlayTypes(false));
    let before = a.session.state().selection();
    click(&mut a, x, y);
    assert_eq!(a.session.state().selection(), before);
    draw(&mut a, 80, 20);
    assert!(old_hints.upgrade().is_none(), "obsolete native layout must release replaced hint labels");
    assert_eq!(std::fs::read_to_string(path).unwrap(), source);
}

#[test]
fn ctrl_shift_v_switches_markdown_source_and_preview_without_pasting() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("note.md");
    std::fs::write(&path, "# Header\n").unwrap();
    let mut a = app(dir.path());
    a.enable_draft_input_for_test();
    a.dispatch(Command::Open(path.clone()));
    settle(&mut a, |a| a.session.state().current_source().is_some());
    let preview = |a: &App| a.session.state().active().and_then(|t| a.session.state().view(t.id())).is_some_and(|v| matches!(v.rendering(), Some(code_core::session::Rendering::Preview(_) | code_core::session::Rendering::Pending)));
    let shown = preview(&a);
    // Terminals that report the chord send the letter with Shift, lower or upper case.
    for code in [KeyCode::Char('v'), KeyCode::Char('V')] {
        key(&mut a, code, KeyModifiers::CONTROL | KeyModifiers::SHIFT);
        assert_ne!(preview(&a), shown);
        key(&mut a, code, KeyModifiers::CONTROL | KeyModifiers::SHIFT);
        assert_eq!(preview(&a), shown);
    }
    assert!(!a.session.state().notices().iter().any(|n| n.contains("terminal Paste")));
    assert_eq!(std::fs::read_to_string(path).unwrap(), "# Header\n");
}

#[test]
fn source_hits_do_not_select_source_after_switching_to_markdown_preview() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("note.md");
    std::fs::write(&path, "# Header\n\nBody\n").unwrap();
    let mut a = app(dir.path());
    a.dispatch(Command::Open(path.clone()));
    settle(&mut a, |a| a.session.state().current_source().is_some());
    draw(&mut a, 80, 20);
    let (x, y) = (a.layout.panes[0].text.x + 2, a.layout.panes[0].text.y);
    a.run(CommandId::MarkdownSource);
    let before = a.session.state().selection();
    click(&mut a, x, y);
    assert_eq!(a.session.state().selection(), before);
    assert_eq!(std::fs::read_to_string(path).unwrap(), "# Header\n\nBody\n");
}

#[test]
fn source_hits_are_invalidated_by_header_split_and_explorer_layout_changes() {
    for change in 0..3 {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.txt");
        std::fs::write(&path, "alpha\nbeta\ngamma\n").unwrap();
        let mut a = app(dir.path());
        a.dispatch(Command::OpenRoot(dir.path().to_path_buf()));
        a.dispatch(Command::Open(path.clone()));
        settle(&mut a, |a| a.session.state().current_source().is_some());
        draw(&mut a, 100, 24);
        let (x, y) = (a.layout.panes[0].text.x + 2, a.layout.panes[0].text.y);
        match change {
            0 => a.dispatch(Command::SetBreadcrumbs(false)),
            1 => a.run(CommandId::SplitRight),
            _ => a.run(CommandId::ToggleExplorer),
        }
        let before = a.session.state().selection();
        click(&mut a, x, y);
        assert_eq!(a.session.state().selection(), before, "layout change={change}");
        assert_eq!(std::fs::read_to_string(path).unwrap(), "alpha\nbeta\ngamma\n");
    }
}

#[test]
fn folded_source_does_not_accept_hits_from_the_old_projection() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fold.rs");
    let source = "fn main() {\n  let first = 1;\n  let second = 2;\n}\ntail\n";
    std::fs::write(&path, source).unwrap();
    let mut a = app(dir.path());
    a.dispatch(Command::Open(path.clone()));
    settle(&mut a, |a| a.session.state().active().is_some_and(|t| a.session.state().view(t.id()).is_some_and(|view| view.folds().is_some())));
    draw(&mut a, 80, 20);
    let (x, y) = (a.layout.panes[0].text.x + 4, a.layout.panes[0].text.y + 1);
    a.dispatch(Command::ToggleFold(0));
    let before = a.session.state().selection();
    click(&mut a, x, y);
    assert_eq!(a.session.state().selection(), before);
    assert!(a.session.state().view(a.session.state().active().unwrap().id()).unwrap().visible().hidden(1));
    assert_eq!(std::fs::read_to_string(path).unwrap(), source);
}

#[test]
fn source_hits_wait_for_render_after_scrolling_or_resizing() {
    for mode in 0..3 {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lines.txt");
        let source = (0..100).map(|i| format!("line{i:03}\n")).collect::<String>();
        std::fs::write(&path, &source).unwrap();
        let mut a = app(dir.path());
        a.dispatch(Command::Open(path.clone()));
        settle(&mut a, |a| a.session.state().current_source().is_some());
        draw(&mut a, 80, 20);
        let (x, y) = (a.layout.panes[0].text.x + 4, a.layout.panes[0].text.y);
        match mode {
            0 => a.dispatch(Command::ScrollRows(3)),
            1 => { a.handle(Event::Resize(40, 10)); }
            _ => a.dispatch(Command::SetTextSize { pane: 0, cols: 10, rows: 5 }),
        }
        click(&mut a, x, y);
        assert_eq!(a.session.state().selection(), None, "mode={mode}");
        draw(&mut a, if mode > 0 { 40 } else { 80 }, if mode > 0 { 10 } else { 20 });
        let (x, y) = (a.layout.panes[0].text.x + 4, a.layout.panes[0].text.y);
        click(&mut a, x, y);
        let expected = if mode > 0 { 4 } else { 28 };
        assert_eq!(a.session.state().selection(), Some(Selection { anchor: expected, head: expected }));
        assert_eq!(std::fs::read_to_string(path).unwrap(), source);
    }
}

#[test]
fn stale_source_geometry_cannot_target_reloaded_text() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("changing.txt");
    std::fs::write(&path, "original content\n").unwrap();
    let mut a = app(dir.path());
    a.dispatch(Command::Open(path.clone()));
    settle(&mut a, |a| a.session.state().current_source().is_some());
    draw(&mut a, 80, 20);
    let (x, y) = {
        let pane = &a.layout.panes[0];
        (pane.text.x + 4, pane.text.y)
    };
    let version = a.session.state().current_source().unwrap().key().version;
    std::fs::write(&path, "replacement text\n").unwrap();
    a.run(CommandId::Reload);
    settle(&mut a, |a| a.session.state().current_source().is_some_and(|d| d.key().version != version));
    let before = a.session.state().selection();
    click(&mut a, x, y);
    assert_eq!(a.session.state().selection(), before);
    assert_eq!(std::fs::read_to_string(path).unwrap(), "replacement text\n");
}

#[test]
fn a_drag_started_on_an_old_document_version_cannot_select_reloaded_text() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("changing.txt");
    std::fs::write(&path, "original content\n").unwrap();
    let mut a = app(dir.path());
    a.dispatch(Command::Open(path.clone()));
    settle(&mut a, |a| a.session.state().current_source().is_some());
    draw(&mut a, 80, 20);
    let (x, y) = {
        let h = &a.layout.panes[0].hits[0];
        (h.x, h.y)
    };
    click(&mut a, x, y);
    let version = a.session.state().current_source().unwrap().key().version;
    std::fs::write(&path, "replaced\n").unwrap();
    a.run(CommandId::Reload);
    settle(&mut a, |a| {
        a.session
            .state()
            .current_source()
            .is_some_and(|d| d.key().version != version)
    });
    draw(&mut a, 80, 20);
    let selection = a.session.state().selection();
    a.handle(Event::Mouse(MouseEvent {
        kind: MouseEventKind::Drag(MouseButton::Left),
        column: x + 4,
        row: y,
        modifiers: KeyModifiers::NONE,
    }));
    assert_eq!(a.session.state().selection(), selection);
    assert_eq!(std::fs::read_to_string(path).unwrap(), "replaced\n");
}

#[test]
#[cfg(unix)]
fn server_runtime_dialog_validates_saves_and_preserves_the_document() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let source = "class Main {}\n";
    let path = dir.path().join("Main.java");
    std::fs::write(&path, source).unwrap();
    let java = dir.path().join("java with spaces");
    std::fs::write(&java, "#!/bin/sh\ntest \"$1\" = -version\n").unwrap();
    std::fs::set_permissions(&java, std::fs::Permissions::from_mode(0o755)).unwrap();
    let cfg = config(dir.path());
    let settings = cfg.providers.clone();
    std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
    let original = r#"{"providers":{"jdtls":{"executable":"/nonexistent/jdtls"}},"keep":true}"#;
    std::fs::write(&settings, original).unwrap();
    let mut a = App::new(cfg, Some(dir.path().into()));
    a.dispatch(Command::Open(path.clone()));
    settle(&mut a, |a| a.session.state().current_source().is_some());
    key(&mut a, KeyCode::Char('a'), KeyModifiers::CONTROL);
    let selected = a.session.state().selection();
    key(&mut a, KeyCode::F(2), KeyModifiers::NONE);
    let at = a
        .choices()
        .iter()
        .position(|c| matches!(c.target, ChoiceTarget::Action(CommandId::ServerRuntime)))
        .unwrap();
    for _ in 0..at {
        key(&mut a, KeyCode::Down, KeyModifiers::NONE);
    }
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(a.modal.as_ref().unwrap().kind, Prompt::ServerRuntime);
    a.handle(Event::Paste("/nonexistent/java".into()));
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    settle(&mut a, |a| {
        a.session
            .state()
            .server_runtime()
            .unwrap()
            .message
            .contains("not changed")
    });
    assert_eq!(std::fs::read_to_string(&settings).unwrap(), original);
    for (w, h) in [(26, 8), (1, 1), (100, 26)] {
        draw(&mut a, w, h);
        key(&mut a, KeyCode::End, KeyModifiers::NONE);
        key(&mut a, KeyCode::Up, KeyModifiers::NONE);
        draw(&mut a, w, h);
        assert!(a.modal.as_ref().unwrap().at < a.choices().len());
    }
    for _ in 0.."/nonexistent/java".len() {
        key(&mut a, KeyCode::Backspace, KeyModifiers::NONE);
    }
    a.handle(Event::Paste(java.to_str().unwrap().into()));
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    settle(&mut a, |a| {
        a.session
            .state()
            .server_runtime()
            .unwrap()
            .message
            .contains("saved.")
    });
    assert_eq!(
        a.session
            .state()
            .server_runtime()
            .unwrap()
            .configured
            .as_ref(),
        Some(&java)
    );
    key(&mut a, KeyCode::F(5), KeyModifiers::NONE);
    assert_eq!(a.session.state().selection(), selected);
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    assert!(a.modal.is_none());
    assert!(a.session.state().server_runtime().is_none());
    let saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&settings).unwrap()).unwrap();
    assert_eq!(saved["keep"], true);
    assert_eq!(
        saved["providers"]["jdtls"]["args"],
        serde_json::json!([format!("--java-executable={}", java.display())])
    );
    assert_eq!(std::fs::read_to_string(path).unwrap(), source);
}

fn navigation_ready(a: &mut App) {
    settle(a, |a| {
        a.session.state().navigation().is_some_and(|s| !s.pending)
    });
}

#[test]
fn navigation_shortcuts_cycle_scopes_and_keep_modal_input_isolated() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sample.txt");
    let source = "hello\nworld\n";
    std::fs::write(&path, source).unwrap();
    let mut a = app(dir.path());
    a.dispatch(Command::Open(path.clone()));
    settle(&mut a, |a| a.session.state().current_source().is_some());
    a.dispatch(Command::Select(Selection { anchor: 0, head: 5 }));
    let selection = a.session.state().selection();
    let ctrl_shift = KeyModifiers::CONTROL | KeyModifiers::SHIFT;
    for (code, modifiers, scope) in [
        ('p', KeyModifiers::CONTROL, Scope::Files),
        ('N', ctrl_shift, Scope::Files),
        ('N', ctrl_shift | KeyModifiers::ALT, Scope::Symbols),
        ('A', ctrl_shift, Scope::Actions),
        ('P', ctrl_shift, Scope::Actions),
        ('k', KeyModifiers::CONTROL, Scope::All),
    ] {
        key(&mut a, KeyCode::Char(code), modifiers);
        assert_eq!(a.modal.as_ref().unwrap().kind, Prompt::Navigation);
        assert_eq!(a.session.state().navigation().unwrap().scope, scope);
        key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
        assert!(a.session.state().navigation().is_none());
    }
    key(&mut a, KeyCode::Char('k'), KeyModifiers::CONTROL);
    a.handle(Event::Paste("α界\nquery".into()));
    assert_eq!(a.modal.as_ref().unwrap().input, "α界query");
    key(&mut a, KeyCode::Tab, KeyModifiers::NONE);
    assert_eq!(a.session.state().navigation().unwrap().scope, Scope::Files);
    key(&mut a, KeyCode::BackTab, KeyModifiers::SHIFT);
    assert_eq!(a.session.state().navigation().unwrap().scope, Scope::All);
    assert_eq!(a.session.state().navigation().unwrap().query, "α界query");
    key(&mut a, KeyCode::Char('s'), KeyModifiers::CONTROL);
    key(&mut a, KeyCode::Backspace, KeyModifiers::NONE);
    assert_eq!(a.modal.as_ref().unwrap().input, "α界quer");
    assert_eq!(a.session.state().selection(), selection);
    assert_eq!(std::fs::read_to_string(path).unwrap(), source);
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
}

#[test]
fn navigation_opens_syntactic_symbols_and_toggles_actions_through_events() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sample.rs");
    let source = "// heading\nfn uniquely_named_symbol() {}\n";
    std::fs::write(&path, source).unwrap();
    std::fs::create_dir_all(dir.path().join("config")).unwrap();
    std::fs::write(
        dir.path().join("config/providers.json"),
        serde_json::json!({"providers":{"rust-analyzer":{"disabled":[dir.path()]}}}).to_string(),
    )
    .unwrap();
    let mut a = app(dir.path());
    settle(&mut a, |a| {
        a.session
            .state()
            .project()
            .is_some_and(|p| matches!(p.coverage(), code_core::project::Coverage::Complete(_)))
    });
    a.run(CommandId::FindSymbol);
    a.handle(Event::Paste("uniquely_named_symbol".into()));
    settle(&mut a, |a| {
        a.session.state().navigation().is_some_and(|s| {
            !s.pending
                && s.rows
                    .iter()
                    .any(|r| r.kind == code_core::navigation::Kind::Symbol)
        })
    });
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    settle(&mut a, |a| a.session.state().current_source().is_some());
    assert_eq!(a.session.state().active().unwrap().source(), path);
    assert_eq!(
        a.session.state().selection().unwrap().head,
        source.find("fn uniquely_named_symbol").unwrap()
    );
    assert_eq!(a.session.state().language_server_label(), "LSP off");
    let before = a.session.state().soft_wrap();
    a.run(CommandId::FindAction);
    a.handle(Event::Paste("soft wrap".into()));
    navigation_ready(&mut a);
    let index = a
        .session
        .state()
        .navigation()
        .unwrap()
        .rows
        .iter()
        .position(|r| r.label == CommandId::SoftWrap.label())
        .unwrap();
    a.modal.as_mut().unwrap().at = index;
    draw(&mut a, 100, 26);
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    assert!(a.modal.is_none());
    assert!(a.session.state().navigation().is_none());
    assert_ne!(a.session.state().soft_wrap(), before);
    assert_eq!(std::fs::read_to_string(path).unwrap(), source);
}

#[test]
fn navigation_mouse_scopes_and_rows_reject_stale_displayed_revisions() {
    let dir = tempfile::tempdir().unwrap();
    let mut a = app(dir.path());
    a.run(CommandId::FindAction);
    a.handle(Event::Paste("soft wrap".into()));
    navigation_ready(&mut a);
    draw(&mut a, 100, 26);
    let old_revision = a.layout.navigation_revision.unwrap();
    let (x, y) = (a.layout.modal_rows.x, a.layout.modal_rows.y);
    let before = a.session.state().soft_wrap();
    a.handle(Event::Paste("xxxxxxxx".into()));
    navigation_ready(&mut a);
    assert_ne!(
        a.session.state().navigation().unwrap().revision,
        old_revision
    );
    click(&mut a, x, y);
    assert!(a.modal.is_some());
    assert_eq!(a.session.state().soft_wrap(), before);
    a.accept_navigation(a.session.state().navigation().unwrap().revision, usize::MAX);
    assert!(a.modal.is_some());
    key(&mut a, KeyCode::Char('u'), KeyModifiers::CONTROL);
    navigation_ready(&mut a);
    draw(&mut a, 100, 26);
    let (_, rect) = a
        .layout
        .navigation_scopes
        .iter()
        .find(|(s, _)| *s == Scope::Files)
        .unwrap();
    let (x, y) = (rect.x, rect.y);
    click(&mut a, x, y);
    assert_eq!(a.session.state().navigation().unwrap().scope, Scope::Files);
    assert!(a.modal.as_ref().unwrap().input.is_empty());
    a.navigation_scope(Scope::Actions);
    a.handle(Event::Paste("soft wrap".into()));
    navigation_ready(&mut a);
    draw(&mut a, 100, 26);
    let (x, y) = (a.layout.modal_rows.x, a.layout.modal_rows.y);
    click(&mut a, x, y);
    assert!(a.modal.is_none());
    assert_ne!(a.session.state().soft_wrap(), before);
}

fn shift_event(a: &mut App, kind: KeyEventKind) -> bool {
    a.handle(Event::Key(KeyEvent::new_with_kind(
        KeyCode::Modifier(ModifierKeyCode::LeftShift),
        if kind == KeyEventKind::Release {
            KeyModifiers::NONE
        } else {
            KeyModifiers::SHIFT
        },
        kind,
    )))
}

#[test]
fn navigation_double_shift_requires_modifier_reports_and_uninterrupted_taps() {
    let dir = tempfile::tempdir().unwrap();
    let mut a = app(dir.path());
    assert!(!shift_event(&mut a, KeyEventKind::Press));
    assert!(!shift_event(&mut a, KeyEventKind::Release));
    assert!(!shift_event(&mut a, KeyEventKind::Press));
    assert!(!shift_event(&mut a, KeyEventKind::Repeat));
    assert!(!shift_event(&mut a, KeyEventKind::Release));
    assert!(a.modal.is_none(), "a repeated hold cannot complete a double tap");
    assert!(!shift_event(&mut a, KeyEventKind::Press));
    assert!(!shift_event(&mut a, KeyEventKind::Release));
    assert!(!shift_event(&mut a, KeyEventKind::Press));
    assert!(shift_event(&mut a, KeyEventKind::Release));
    assert_eq!(a.session.state().navigation().unwrap().scope, Scope::All);
    assert!(!a.handle(Event::Key(KeyEvent::new(
        KeyCode::Modifier(ModifierKeyCode::LeftControl),
        KeyModifiers::CONTROL,
    ))));
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    shift_event(&mut a, KeyEventKind::Press);
    key(&mut a, KeyCode::Char('X'), KeyModifiers::SHIFT);
    shift_event(&mut a, KeyEventKind::Release);
    shift_event(&mut a, KeyEventKind::Press);
    shift_event(&mut a, KeyEventKind::Release);
    assert!(a.modal.is_none(), "shifted text cannot count as a tap");
    a.handle(Event::FocusLost);
    shift_event(&mut a, KeyEventKind::Press);
    shift_event(&mut a, KeyEventKind::Release);
    assert!(a.modal.is_none(), "focus changes clear the previous tap");
    a.handle(Event::FocusLost);
    for _ in 0..2 {
        key(&mut a, KeyCode::Char('X'), KeyModifiers::SHIFT);
    }
    assert!(a.modal.is_none(), "uppercase text is never a double Shift");
    a.handle(Event::Key(KeyEvent::new_with_kind(
        KeyCode::Char('k'),
        KeyModifiers::CONTROL,
        KeyEventKind::Repeat,
    )));
    assert!(a.modal.is_none(), "search openers reject repeat events");
    a.handle(Event::Key(KeyEvent::new_with_kind(
        KeyCode::Char('k'),
        KeyModifiers::CONTROL,
        KeyEventKind::Release,
    )));
    assert!(a.modal.is_none(), "ordinary commands reject release events");
}

#[test]
fn navigation_double_shift_from_either_key_preserves_documents_and_other_modals() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("keep.txt");
    let source = b"keep\r\nbytes\r\n";
    std::fs::write(&path, source).unwrap();
    let mut a = App::new(config(dir.path()), Some(path.clone()));
    settle(&mut a, |a| a.session.state().current_source().is_some());
    a.enable_draft_input_for_test();
    draw(&mut a, 90, 30);
    let before = a.session.state().selection();
    let tap = |a: &mut App, code| {
        for kind in [KeyEventKind::Press, KeyEventKind::Release] {
            a.handle(Event::Key(KeyEvent::new_with_kind(KeyCode::Modifier(code), KeyModifiers::SHIFT, kind)));
        }
    };
    tap(&mut a, ModifierKeyCode::LeftShift);
    tap(&mut a, ModifierKeyCode::RightShift);
    assert!(matches!(a.modal.as_ref().map(|m| &m.kind), Some(Prompt::Navigation)));
    assert!(draw_screen(&mut a, 90, 30).contains("Search Everywhere"));
    a.handle(Event::Paste("é終 query".into()));
    assert_eq!(a.session.state().navigation().unwrap().query, "é終 query");
    for (w, h) in [(18, 6), (1, 1), (90, 30)] { draw(&mut a, w, h); }
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    key(&mut a, KeyCode::F(10), KeyModifiers::NONE);
    tap(&mut a, ModifierKeyCode::RightShift);
    tap(&mut a, ModifierKeyCode::RightShift);
    assert!(matches!(a.modal.as_ref().map(|m| &m.kind), Some(Prompt::Commands)));
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    assert_eq!(a.session.state().selection(), before);
    let draft = a.session.state().current_source().unwrap();
    assert_eq!(draft.copy_range(0..draft.len_bytes()).unwrap().as_bytes(), source);
    assert_eq!(std::fs::read(path).unwrap(), source);
}

#[test]
fn double_shift_opens_search_everywhere_from_release_only_reports() {
    // foot reports a bare Shift's release only (`CSI 57441;1:3u`), never its press.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("keep.txt");
    std::fs::write(&path, "keep\n").unwrap();
    let mut a = App::new(config(dir.path()), Some(path.clone()));
    settle(&mut a, |a| a.session.state().current_source().is_some());
    a.enable_draft_input_for_test();
    draw(&mut a, 90, 30);
    let release = |a: &mut App| { shift_event(a, KeyEventKind::Release); };
    let opened = |a: &App| matches!(a.modal.as_ref().map(|m| &m.kind), Some(Prompt::Navigation));
    // The pointer moving first does not spend the first tap.
    a.handle(Event::Mouse(MouseEvent { kind: MouseEventKind::Moved, column: 20, row: 5, modifiers: KeyModifiers::NONE }));
    release(&mut a);
    release(&mut a);
    assert!(opened(&a));
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    // A capital typed with Shift held is not a tap; neither is one tap and a plain key.
    a.handle(Event::Key(KeyEvent::new(KeyCode::Char('K'), KeyModifiers::SHIFT)));
    release(&mut a);
    release(&mut a);
    assert!(!opened(&a));
    key(&mut a, KeyCode::Char('x'), KeyModifiers::NONE);
    release(&mut a);
    assert!(!opened(&a));
    release(&mut a);
    assert!(opened(&a));
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    let draft = a.session.state().current_source().unwrap();
    assert_eq!(draft.copy_range(0..draft.len_bytes()).unwrap(), "Kxkeep\n");
    assert_eq!(std::fs::read(path).unwrap(), b"keep\n");
}

#[test]
fn double_shift_opens_when_foot_reports_the_release_as_caps_lock() {
    // Recorded from foot with an us,gr layout: Shift's press is Shift, its release Caps Lock, with Num Lock on.
    use crossterm::event::KeyEventState;
    let dir = tempfile::tempdir().unwrap();
    let mut a = app(dir.path());
    draw(&mut a, 90, 30);
    let event = |a: &mut App, code, modifiers, kind| {
        a.handle(Event::Key(KeyEvent::new_with_kind_and_state(code, modifiers, kind, KeyEventState::NUM_LOCK)));
    };
    event(&mut a, KeyCode::Enter, KeyModifiers::NONE, KeyEventKind::Release);
    for _ in 0..2 {
        event(&mut a, KeyCode::Modifier(ModifierKeyCode::LeftShift), KeyModifiers::SHIFT, KeyEventKind::Press);
        event(&mut a, KeyCode::CapsLock, KeyModifiers::NONE, KeyEventKind::Release);
    }
    assert!(matches!(a.modal.as_ref().map(|m| &m.kind), Some(Prompt::Navigation)));
    // A real Caps Lock tap, with no Shift press before it, is not a Shift tap.
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    for _ in 0..2 {
        event(&mut a, KeyCode::CapsLock, KeyModifiers::NONE, KeyEventKind::Press);
        event(&mut a, KeyCode::CapsLock, KeyModifiers::NONE, KeyEventKind::Release);
    }
    assert!(a.modal.is_none());
}

#[test]
fn navigation_enhanced_keyboard_repeats_move_caret_and_modal_selection() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("repeated.txt");
    std::fs::write(&path, "line\n".repeat(100)).unwrap();
    let mut a = app(dir.path());
    a.dispatch(Command::Open(path));
    settle(&mut a, |a| a.session.state().current_source().is_some());
    draw(&mut a, 100, 26);
    let repeat = |a: &mut App, code| {
        a.handle(Event::Key(KeyEvent::new_with_kind(
            code,
            KeyModifiers::NONE,
            KeyEventKind::Repeat,
        )));
    };
    repeat(&mut a, KeyCode::Down);
    assert_eq!(
        a.session.state().selection(),
        Some(Selection { anchor: 5, head: 5 })
    );
    assert_eq!(a.session.state().active().unwrap().top_line(), 0);
    a.run(CommandId::FindAction);
    navigation_ready(&mut a);
    draw(&mut a, 100, 26);
    key(&mut a, KeyCode::Down, KeyModifiers::NONE);
    repeat(&mut a, KeyCode::Down);
    assert_eq!(a.modal.as_ref().unwrap().at, 2);
    repeat(&mut a, KeyCode::Char('s'));
    assert_eq!(a.modal.as_ref().unwrap().input, "s");
    assert_eq!(
        a.session.state().selection(),
        Some(Selection { anchor: 5, head: 5 })
    );
    assert_eq!(a.session.state().active().unwrap().top_line(), 0);
}

#[test]
fn navigation_scope_shortcuts_preserve_query_inside_modal_and_plain_ctrl_a_clears() {
    let dir = tempfile::tempdir().unwrap();
    let mut a = app(dir.path());
    a.run(CommandId::SearchEverywhere);
    a.handle(Event::Paste("α界 typed query".into()));
    let ctrl_shift = KeyModifiers::CONTROL | KeyModifiers::SHIFT;
    for (code, modifiers, scope) in [
        ('A', ctrl_shift, Scope::Actions),
        ('p', KeyModifiers::CONTROL, Scope::Files),
        ('e', KeyModifiers::CONTROL, Scope::Recent),
        ('N', ctrl_shift | KeyModifiers::ALT, Scope::Symbols),
        ('P', ctrl_shift, Scope::Actions),
        ('N', ctrl_shift, Scope::Files),
        ('k', KeyModifiers::CONTROL, Scope::All),
    ] {
        a.modal.as_mut().unwrap().at = 5;
        a.modal.as_mut().unwrap().top = 4;
        key(&mut a, KeyCode::Char(code), modifiers);
        let modal = a.modal.as_ref().unwrap();
        assert_eq!(modal.kind, Prompt::Navigation);
        assert_eq!(modal.input, "α界 typed query");
        assert_eq!((modal.at, modal.top), (0, 0));
        let search = a.session.state().navigation().unwrap();
        assert_eq!(search.scope, scope);
        assert_eq!(search.query, "α界 typed query");
    }
    key(&mut a, KeyCode::Char('a'), KeyModifiers::CONTROL);
    assert!(a.modal.as_ref().unwrap().input.is_empty());
    assert!(a.session.state().navigation().unwrap().query.is_empty());
    assert_eq!(a.session.state().navigation().unwrap().scope, Scope::All);
}

#[test]
fn popup_wheel_moves_visible_rows_without_changing_enter_target_or_document() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("source.txt");
    std::fs::write(&path, "hello\nworld\n").unwrap();
    let mut a = app(dir.path());
    a.dispatch(Command::Open(path.clone()));
    settle(&mut a, |a| a.session.state().current_source().is_some());
    a.dispatch(Command::Select(Selection { anchor: 0, head: 5 }));
    let selection = a.session.state().selection();
    a.prompt(Prompt::Commands);
    draw(&mut a, 40, 10);
    let selected = a.modal.as_ref().unwrap().at;
    let first_target = a.choices()[selected].label.clone();
    for _ in 0..4 {
        a.handle(Event::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        }));
        draw(&mut a, 40, 10);
    }
    assert_eq!(a.modal.as_ref().unwrap().at, selected);
    assert!(a.modal.as_ref().unwrap().top > 0);
    assert!(!a.layout.modal_items.contains(&selected));
    assert_eq!(
        a.choices()[a.modal.as_ref().unwrap().at].label,
        first_target
    );
    assert_eq!(a.session.state().selection(), selection);
    assert_eq!(a.session.state().active().unwrap().top_line(), 0);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello\nworld\n");
    key(&mut a, KeyCode::Up, KeyModifiers::NONE);
    draw(&mut a, 40, 10);
    assert_eq!(a.modal.as_ref().unwrap().top, 0);
}

#[test]
fn plain_arrow_keys_move_utf8_caret_and_keep_both_axes_visible() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("caret.txt");
    let source = format!("é界z{}\n{}", "x".repeat(120), "é界z\n".repeat(60));
    std::fs::write(&path, &source).unwrap();
    let mut a = app(dir.path());
    a.dispatch(Command::Open(path.clone()));
    settle(&mut a, |a| a.session.state().current_source().is_some());
    draw(&mut a, 40, 12);
    for (code, head) in [
        (KeyCode::Right, 2),
        (KeyCode::Right, 5),
        (KeyCode::Left, 2),
        (KeyCode::Down, 129),
        (KeyCode::Up, 2),
    ] {
        key(&mut a, code, KeyModifiers::NONE);
        assert_eq!(
            a.session.state().selection(),
            Some(Selection { anchor: head, head }),
            "{code:?} lines {:?} {:?}",
            a.session.state().current_source().unwrap().line_range(0),
            a.session.state().current_source().unwrap().line_range(1)
        );
    }
    for _ in 0..50 {
        a.handle(Event::Key(KeyEvent::new_with_kind(
            KeyCode::Right,
            KeyModifiers::NONE,
            KeyEventKind::Repeat,
        )));
    }
    draw(&mut a, 40, 12);
    assert!(a.session.state().column() > 0);
    assert!(a.next_deadline().is_some(), "native reveal feedback schedules the translated viewport");
    assert!(a.tick(Instant::now()));
    draw(&mut a, 40, 12);
    let head = a.session.state().selection().unwrap().head;
    assert!(a.layout.panes[0].hits.iter().any(|h| h.byte == head));
    for _ in 0..30 {
        key(&mut a, KeyCode::Down, KeyModifiers::NONE);
        draw(&mut a, 40, 12);
    }
    let selection = a.session.state().selection().unwrap();
    assert_eq!(selection.anchor, selection.head);
    let line = a
        .session
        .state()
        .current_source()
        .unwrap()
        .line_of(selection.head);
    assert_eq!(line, 30);
    assert!(a.layout.panes[0].rows.iter().any(|(_, l, _)| *l == line));
    key(&mut a, KeyCode::Right, KeyModifiers::SHIFT);
    assert_ne!(
        a.session.state().selection().unwrap().anchor,
        a.session.state().selection().unwrap().head
    );
    assert_eq!(std::fs::read_to_string(path).unwrap(), source);
}

#[test]
fn ctrl_e_opens_the_same_searchable_file_popup_as_find_files() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("recent.txt");
    std::fs::write(&path, "recent\n").unwrap();
    let mut a = app(dir.path());
    a.dispatch(Command::Open(path));
    settle(&mut a, |a| a.session.state().current_source().is_some());
    key(&mut a, KeyCode::Char('e'), KeyModifiers::CONTROL);
    assert_eq!(a.modal.as_ref().unwrap().kind, Prompt::Navigation);
    navigation_ready(&mut a);
    draw(&mut a, 80, 20);
    assert!(a.layout.navigation_revision.is_some());
}

#[test]
fn navigation_wheel_keeps_hidden_selection_for_enter() {
    let dir = tempfile::tempdir().unwrap();
    let mut a = app(dir.path());
    a.run(CommandId::FindAction);
    navigation_ready(&mut a);
    let index = a
        .session
        .state()
        .navigation()
        .unwrap()
        .rows
        .iter()
        .position(|r| r.label == CommandId::SoftWrap.label())
        .unwrap();
    a.modal.as_mut().unwrap().at = index;
    draw(&mut a, 40, 10);
    let before = a.session.state().soft_wrap();
    let scrolls = a.session.state().navigation().unwrap().rows.len();
    for _ in 0..scrolls {
        a.handle(Event::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        }));
        draw(&mut a, 40, 10);
    }
    assert_eq!(a.modal.as_ref().unwrap().at, index);
    assert_eq!(a.modal.as_ref().unwrap().top, 0);
    assert!(!a.layout.modal_items.contains(&index));
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    assert!(a.modal.is_none());
    assert_ne!(a.session.state().soft_wrap(), before);
}

#[test]
fn help_and_wrapped_details_wheel_scroll_without_selection_and_clamp_on_resize() {
    let dir = tempfile::tempdir().unwrap();
    let mut a = app(dir.path());
    for prompt in [
        Prompt::Help,
        Prompt::Details { title: "Details", body: "long wrapped detail ".repeat(200), logo: false, kind: Details::Text },
    ] {
        a.prompt(prompt);
        draw(&mut a, 24, 10);
        let selected = a.modal.as_ref().unwrap().at;
        for _ in 0..12 {
            a.handle(Event::Mouse(MouseEvent {
                kind: MouseEventKind::ScrollDown,
                column: 0,
                row: 0,
                modifiers: KeyModifiers::NONE,
            }));
            draw(&mut a, 24, 10);
        }
        assert_eq!(a.modal.as_ref().unwrap().at, selected);
        assert_eq!(a.modal.as_ref().unwrap().top, 12);
        key(&mut a, KeyCode::End, KeyModifiers::NONE);
        draw(&mut a, 24, 10);
        let end = a.modal.as_ref().unwrap().top;
        key(&mut a, KeyCode::Up, KeyModifiers::NONE);
        draw(&mut a, 24, 10);
        assert_eq!(a.modal.as_ref().unwrap().top, end - 1);
        draw(&mut a, 100, 26);
        assert!(
            a.modal.as_ref().unwrap().top
                <= a.layout
                    .modal_content_rows
                    .saturating_sub(a.layout.modal_rows.height as usize)
        );
        draw(&mut a, 1, 1);
        key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
        assert!(a.modal.is_none());
    }
}

#[test]
fn plain_arrows_follow_visual_wrap_rows_and_skip_collapsed_source_lines() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fold.rs");
    let source = "fn main() {\n  let x = 1;\n  let y = 2;\n}\ntrailing\n";
    std::fs::write(&path, source).unwrap();
    let mut a = app(dir.path());
    a.dispatch(Command::Open(path.clone()));
    settle(&mut a, |a| {
        a.session
            .state()
            .active()
            .is_some_and(|t| a.session.state().view(t.id()).is_some_and(|view| view.folds().is_some()))
    });
    a.dispatch(Command::ToggleFold(0));
    draw(&mut a, 70, 12);
    key(&mut a, KeyCode::Down, KeyModifiers::NONE);
    let selection = a.session.state().selection().unwrap();
    assert_eq!(selection.anchor, selection.head);
    assert_eq!(
        a.session
            .state()
            .current_source()
            .unwrap()
            .line_of(selection.head),
        4
    );
    key(&mut a, KeyCode::Up, KeyModifiers::NONE);
    assert_eq!(
        a.session.state().selection(),
        Some(Selection { anchor: 0, head: 0 })
    );

    let wrapped = dir.path().join("wrapped.txt");
    std::fs::write(&wrapped, "abcdef ".repeat(30)).unwrap();
    a.dispatch(Command::Open(wrapped.clone()));
    settle(&mut a, |a| {
        a.session
            .state()
            .active()
            .is_some_and(|t| t.loaded() && t.source() == wrapped)
    });
    a.run(CommandId::SoftWrap);
    draw(&mut a, 30, 10);
    let second_row = a.layout.panes[0].rows[1].1;
    assert_eq!(second_row, 0);
    key(&mut a, KeyCode::Down, KeyModifiers::NONE);
    let selection = a.session.state().selection().unwrap();
    assert!(selection.head > 0 && selection.head < 210);
    assert_eq!(selection.anchor, selection.head);
    assert_eq!(
        a.session
            .state()
            .current_source()
            .unwrap()
            .line_of(selection.head),
        0
    );
    key(&mut a, KeyCode::Up, KeyModifiers::NONE);
    assert_eq!(
        a.session.state().selection(),
        Some(Selection { anchor: 0, head: 0 })
    );
    assert_eq!(std::fs::read_to_string(path).unwrap(), source);
}

#[test]
fn recent_popup_fuzzy_filters_forgets_and_opens_scrolled_paths_at_saved_caret() {
    let dir = tempfile::tempdir().unwrap();
    let mut a = app(dir.path());
    let mut paths = Vec::new();
    for i in 0..10 {
        let path = dir.path().join(format!("folder-{i}/same_name.txt"));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "first\nsecond\nthird\n").unwrap();
        a.dispatch(Command::OpenRetained(path.clone()));
        settle(&mut a, |a| {
            a.session
                .state()
                .active()
                .is_some_and(|t| t.loaded() && t.source() == path)
        });
        a.dispatch(Command::Select(Selection { anchor: 8, head: 8 }));
        paths.push(path);
    }
    key(&mut a, KeyCode::Char('e'), KeyModifiers::CONTROL);
    navigation_ready(&mut a);
    assert_eq!(a.session.state().navigation().unwrap().scope, Scope::Recent);
    assert!(
        a.session.state().navigation().unwrap().rows[0]
            .detail
            .contains("folder-8")
    );
    key(&mut a, KeyCode::Tab, KeyModifiers::NONE);
    assert_eq!(a.session.state().navigation().unwrap().scope, Scope::Recent);
    draw(&mut a, 40, 10);
    for _ in 0..4 {
        a.handle(Event::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        }));
        draw(&mut a, 40, 10);
    }
    assert_eq!(a.modal.as_ref().unwrap().at, 0);
    let visible = a.layout.modal_items[0];
    assert!(visible > 0);
    assert_eq!(visible, 4);
    assert!(
        a.session.state().navigation().unwrap().rows[visible]
            .detail
            .contains("folder-5")
    );
    let target = paths[5].clone();
    let (x, y) = (a.layout.modal_rows.x, a.layout.modal_rows.y);
    click(&mut a, x, y);
    settle(&mut a, |a| {
        a.session
            .state()
            .active()
            .is_some_and(|t| t.loaded() && t.source() == target)
    });
    assert_eq!(
        a.session.state().selection(),
        Some(Selection { anchor: 8, head: 8 })
    );
    key(&mut a, KeyCode::Char('e'), KeyModifiers::CONTROL);
    a.handle(Event::Paste("fdr3/smnm".into()));
    navigation_ready(&mut a);
    assert_eq!(a.session.state().navigation().unwrap().rows.len(), 1);
    assert!(
        a.session.state().navigation().unwrap().rows[0]
            .detail
            .contains("folder-3")
    );
    key(&mut a, KeyCode::Delete, KeyModifiers::NONE);
    assert_eq!(
        a.session.state().navigation().unwrap().rows.len(),
        1,
        "Delete must reject a stale displayed revision"
    );
    draw(&mut a, 40, 10);
    key(&mut a, KeyCode::Delete, KeyModifiers::NONE);
    navigation_ready(&mut a);
    assert!(a.session.state().navigation().unwrap().rows.is_empty());
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    for path in paths {
        assert_eq!(
            std::fs::read_to_string(path).unwrap(),
            "first\nsecond\nthird\n"
        );
    }
}

#[test]
fn chrome_status_click_opens_copyable_snapshot_and_keeps_document_input_modal() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("status.txt");
    let bytes = "hello 界e\u{301}\r\nsecond\r\n";
    std::fs::write(&path, bytes).unwrap();
    let mut a = app(dir.path());
    a.dispatch(Command::Open(path.clone()));
    settle(&mut a, |a| a.session.state().current_source().is_some());
    let notice = "Adapter warning 界e\u{301} ".repeat(20) + "\nsecond diagnostic line";
    a.dispatch(Command::Notice(notice.clone()));
    draw(&mut a, 100, 26);
    let before = a.session.state().selection();
    click(&mut a, 30, 25);
    let snapshot = match &a.modal.as_ref().expect("status click opens details").kind {
        Prompt::Details { body, .. } => body.clone(),
        other => panic!("unexpected prompt: {other:?}"),
    };
    assert!(snapshot.contains(&notice));
    a.dispatch(Command::Notice("new background notice".into()));
    key(&mut a, KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert_eq!(a.copy.as_deref(), Some(snapshot.as_str()));
    for (w, h) in [(28, 10), (8, 5), (1, 1), (0, 0), (100, 26)] {
        draw(&mut a, w, h);
        key(&mut a, KeyCode::End, KeyModifiers::NONE);
        draw(&mut a, w, h);
        key(&mut a, KeyCode::Up, KeyModifiers::NONE);
        key(&mut a, KeyCode::Right, KeyModifiers::SHIFT);
        key(&mut a, KeyCode::Char('s'), KeyModifiers::CONTROL);
        a.handle(Event::Paste("should not edit".into()));
    }
    assert_eq!(a.session.state().selection(), before);
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    assert!(a.modal.is_none());
    assert_eq!(std::fs::read_to_string(path).unwrap(), bytes);
}

#[test]
fn chrome_menu_button_opens_the_application_menu_like_the_gui_without_moving_caret() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("note.txt");
    let bytes = "first\nsecond\n";
    std::fs::write(&path, bytes).unwrap();
    let mut a = App::new(config(dir.path()), Some(path.clone()));
    settle(&mut a, |a| a.session.state().current_source().is_some());
    let caret = a.session.state().selection();
    let kind = |a: &App| a.modal.as_ref().map(|m| m.kind.clone());
    let labels = |a: &App| a.choices().into_iter().map(|c| c.label).collect::<Vec<_>>();
    draw(&mut a, 100, 26);
    click(&mut a, 2, 0);
    assert_eq!(kind(&a), Some(Prompt::Menu(None)));
    assert_eq!(labels(&a)[0], "File ›");
    // The task menus and every command, a line, then the application's own commands; the keys pass over the line.
    let all = labels(&a).iter().position(|l| l == "All commands…").unwrap();
    assert_eq!(labels(&a)[all + 1..], ["─".repeat(24).as_str(), "Change theme", "About Lenz Code", "Keyboard Shortcuts ›", "Close Window", "Quit All Windows"]);
    a.modal.as_mut().unwrap().at = all + 2;
    key(&mut a, KeyCode::Up, KeyModifiers::NONE);
    assert_eq!(a.modal.as_ref().unwrap().at, all);
    key(&mut a, KeyCode::Down, KeyModifiers::NONE);
    assert_eq!(a.modal.as_ref().unwrap().at, all + 2);
    a.modal.as_mut().unwrap().at = 0;
    // Typing does not filter the menu.
    key(&mut a, KeyCode::Char('x'), KeyModifiers::NONE);
    // Each group but Application, All commands, the line, then Application's three commands, its shortcuts and two
    // ways to close.
    assert_eq!(labels(&a).len(), CommandGroup::ALL.iter().filter(|g| g.parent().is_none() && **g != CommandGroup::Application).count() + 7);
    // Down Enter opens View; its first row goes back, then the commands with their states.
    key(&mut a, KeyCode::Down, KeyModifiers::NONE);
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(kind(&a), Some(Prompt::Menu(Some(CommandGroup::View))));
    assert_eq!(labels(&a)[0], "‹ All menus — View");
    assert!(labels(&a).iter().any(|l| l == "Editor Appearance ›"));
    // Left and Escape go back to the groups, at the group left.
    key(&mut a, KeyCode::Left, KeyModifiers::NONE);
    assert_eq!(kind(&a), Some(Prompt::Menu(None)));
    assert_eq!(a.modal.as_ref().unwrap().at, 1);
    key(&mut a, KeyCode::Right, KeyModifiers::NONE);
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    assert_eq!(kind(&a), Some(Prompt::Menu(None)));
    // A click on the heading row goes back too; a click on a command runs it.
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    draw(&mut a, 100, 26);
    let rows = a.layout.modal_rows;
    click(&mut a, rows.x + 1, rows.y);
    assert_eq!(kind(&a), Some(Prompt::Menu(None)));
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    draw(&mut a, 100, 26);
    let appearance = a.choices().iter().position(|c| c.label == "Editor Appearance ›").unwrap();
    a.modal.as_mut().unwrap().at = appearance;
    key(&mut a, KeyCode::Right, KeyModifiers::NONE);
    assert_eq!(kind(&a), Some(Prompt::Menu(Some(CommandGroup::Appearance))));
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    assert_eq!(kind(&a), Some(Prompt::Menu(Some(CommandGroup::View))));
    key(&mut a, KeyCode::Right, KeyModifiers::NONE);
    draw(&mut a, 100, 26);
    let rows = a.layout.modal_rows;
    let wrap = a.choices().iter().position(|c| c.label == "Soft Wrap (off)").unwrap();
    let row = a.layout.modal_items.iter().position(|i| *i == wrap).unwrap();
    click(&mut a, rows.x + 1, rows.y + row as u16);
    assert!(a.modal.is_none() && a.session.state().soft_wrap());
    click(&mut a, 2, 0);
    let servers = CommandGroup::ALL.iter().filter(|g| g.parent().is_none()).position(|g| *g == CommandGroup::LanguageServers).unwrap();
    for _ in 0..servers { key(&mut a, KeyCode::Down, KeyModifiers::NONE); }
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(kind(&a), Some(Prompt::Menu(Some(CommandGroup::LanguageServers))));
    draw(&mut a, 100, 26);
    let rows = a.layout.modal_rows;
    click(&mut a, rows.x + 1, rows.y + 1);
    assert!(matches!(kind(&a), Some(Prompt::Details { title: "Server details", .. })));
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    // All commands opens the shared action search; a click outside closes the menu.
    click(&mut a, 2, 0);
    a.modal.as_mut().unwrap().at = labels(&a).iter().position(|l| l == "All commands…").unwrap();
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(kind(&a), Some(Prompt::Navigation));
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    draw(&mut a, 100, 26);
    click(&mut a, 2, 0);
    draw(&mut a, 100, 26);
    click(&mut a, 80, 20);
    assert!(a.modal.is_none());
    assert_eq!(a.session.state().selection(), caret);
    // F10 keeps the full command list, as in the GUI.
    key(&mut a, KeyCode::F(10), KeyModifiers::NONE);
    assert_eq!(kind(&a), Some(Prompt::Commands));
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    // Tiny terminals keep the dropdown inside the frame.
    for (w, h) in [(30, 8), (12, 4), (2, 2), (1, 1), (0, 0)] {
        a.handle(Event::Resize(w, h));
        a.open_menu(Some(CommandGroup::View), 1);
        draw(&mut a, w, h);
        let m = a.layout.modal;
        assert!(m.right() <= w && m.bottom() <= h, "{w}x{h}: {m:?}");
        key(&mut a, KeyCode::End, KeyModifiers::NONE);
        key(&mut a, KeyCode::Up, KeyModifiers::NONE);
        draw(&mut a, w, h);
        key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
        key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
        assert!(a.modal.is_none());
    }
    assert_eq!(std::fs::read_to_string(&path).unwrap(), bytes);
}

#[test]
fn an_empty_window_starts_on_open_file_open_folder_and_recents() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("proj");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("a.rs"), "fn a() {}\n").unwrap();
    std::fs::create_dir_all(dir.path().join("state")).unwrap();
    std::fs::write(dir.path().join("state/recent.json"), serde_json::to_string(&[&root]).unwrap()).unwrap();
    let lone = dir.path().join("lone.txt");
    std::fs::write(&lone, "alone\n").unwrap();
    std::fs::write(dir.path().join("state/recent-files.json"), serde_json::to_string(&[&lone]).unwrap()).unwrap();
    let mut a = App::with_text_input(config(dir.path()), None, false);
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 24)).unwrap();
    terminal.draw(|f| a.draw(f)).unwrap();
    let screen: String = terminal.backend().buffer().content().iter().map(|c| c.symbol()).collect();
    for shown in ["Open File…  Ctrl+O", "Open Folder…", "Recent folders", &root.display().to_string(), "Recent files", &lone.display().to_string()] {
        assert!(screen.contains(shown), "{shown} on the start screen");
    }
    let start = a.layout.panes[0].start.clone();
    assert_eq!(start.iter().map(|(i, _)| *i).collect::<Vec<_>>(), [0, 1, 2, 3]);
    // Open File… and Open Folder… ask for a path; a recent folder opens.
    click(&mut a, start[0].1.x, start[0].1.y);
    assert_eq!(a.modal.as_ref().unwrap().kind, Prompt::Open);
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    click(&mut a, start[1].1.x, start[1].1.y);
    assert_eq!(a.modal.as_ref().unwrap().kind, Prompt::OpenFolder);
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    // A recent file opens; closing it returns to the start screen.
    click(&mut a, start[3].1.x, start[3].1.y);
    settle(&mut a, |a| a.session.state().active().is_some_and(|t| t.source() == lone && t.loaded()));
    draw(&mut a, 100, 24);
    assert!(a.layout.panes[0].start.is_empty());
    key(&mut a, KeyCode::F(4), KeyModifiers::CONTROL);
    draw(&mut a, 100, 24);
    assert_eq!(a.layout.panes[0].start.len(), 4);
    click(&mut a, start[2].1.x, start[2].1.y);
    settle(&mut a, |a| a.session.state().project().is_some_and(|p| p.root() == root));
    draw(&mut a, 100, 24);
    assert!(a.layout.panes[0].start.is_empty(), "the open folder replaces the start screen");
    // A tiny terminal clips the rows it cannot show and keeps no target for them.
    let mut b = App::with_text_input(config(dir.path()), None, false);
    draw(&mut b, 20, 4);
    assert!(b.layout.panes.iter().flat_map(|p| &p.start).all(|(_, r)| r.height == 1 && r.right() <= 20 && r.bottom() <= 4));
}

#[test]
fn chrome_toolbar_controls_and_footer_commands_run_catalog_actions() {
    let dir = tempfile::tempdir().unwrap();
    let mut a = app(dir.path());
    settle(&mut a, |a| a.session.state().project().is_some());
    draw(&mut a, 120, 26);
    assert!(!a.layout.controls.iter().any(|(control, _)| *control == CommandId::Open), "Open is in the File menu and on Ctrl+O");
    key(&mut a, KeyCode::Char('o'), KeyModifiers::CONTROL);
    assert_eq!(a.modal.as_ref().unwrap().kind, Prompt::Open);
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    for id in [CommandId::FindFile, CommandId::SearchProject, CommandId::CompareSaved] {
        assert!(!a.layout.controls.iter().any(|(control, _)| *control == id));
    }
    draw(&mut a, 120, 26);
    let (_, rect) = a.layout.controls.iter().find(|(id, _)| *id == CommandId::ToggleExplorer).unwrap();
    let (x, y) = (rect.x, rect.y);
    let visible = a.tree_shown;
    click(&mut a, x, y);
    assert_eq!(a.tree_shown, !visible);
    draw(&mut a, 120, 26);
    let rect = *a.layout.command_menus.last().unwrap();
    click(&mut a, rect.x, rect.y);
    assert_eq!(a.modal.as_ref().unwrap().kind, Prompt::Commands);
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    key(&mut a, KeyCode::F(2), KeyModifiers::SHIFT);
    assert!(matches!(a.modal.as_ref().unwrap().kind, Prompt::Details { .. }));
    draw(&mut a, 120, 26);
    // A toolbar click dismisses the dialog, without opening its underlying command.
    click(&mut a, 2, 0);
    assert!(a.modal.is_none());
}

#[test]
fn a_reopened_search_selects_its_old_query_so_typing_replaces_it() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.rs"), "Foo foo\n").unwrap();
    let mut a = app(dir.path());
    settle(&mut a, |a| a.session.state().project().is_some());
    let field = |a: &App| a.modal.as_ref().filter(|m| m.kind == Prompt::Search).map(|m| (m.input.clone(), m.selected));
    let ctrl = KeyModifiers::CONTROL;
    key(&mut a, KeyCode::Char('F'), ctrl | KeyModifiers::SHIFT);
    a.handle(Event::Paste("foo".into()));
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    assert!(a.session.state().results().is_some());
    key(&mut a, KeyCode::Char('F'), ctrl | KeyModifiers::SHIFT);
    assert_eq!(field(&a), Some(("foo".into(), true)));
    key(&mut a, KeyCode::Char('x'), KeyModifiers::NONE);
    assert_eq!(field(&a), Some(("x".into(), false)), "typing replaces the selected query");
    a.handle(Event::Paste(" y".into()));
    key(&mut a, KeyCode::Char('w'), ctrl);
    assert_eq!(field(&a), Some(("x ".into(), false)), "Ctrl+W drops the last word");
    key(&mut a, KeyCode::Char('a'), ctrl);
    assert_eq!(field(&a), Some(("x ".into(), true)));
    key(&mut a, KeyCode::Backspace, KeyModifiers::NONE);
    assert_eq!(field(&a), Some((String::new(), false)), "Backspace clears a selection");
}

#[test]
fn search_dialog_fields_and_toggles_take_clicks_and_a_bad_expression_does_not_search() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.rs"), "Foo foo\n").unwrap();
    let mut a = app(dir.path());
    settle(&mut a, |a| a.session.state().project().is_some());
    a.run(CommandId::SearchProject);
    a.handle(Event::Paste("(".into()));
    draw(&mut a, 100, 30);
    let files = a.layout.search_fields[1];
    click(&mut a, files.x + 1, files.y);
    a.handle(Event::Paste("*.rs".into()));
    let m = a.modal.as_ref().unwrap();
    assert!(m.secondary_focus && m.input == "(" && m.secondary == "*.rs");
    let (id, toggle) = a.layout.search_toggles[2];
    assert_eq!(id, CommandId::RegularExpression);
    click(&mut a, toggle.x + 1, toggle.y);
    assert!(a.session.find_options().regex);
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(a.modal.as_ref().map(|m| &m.kind), Some(&Prompt::Search), "a bad expression does not search");
    assert!(a.session.state().results().is_none());
    draw(&mut a, 100, 30);
    let modal = a.layout.modal;
    click(&mut a, modal.x + 1, modal.y + 1);
    assert_eq!(a.modal.as_ref().map(|m| &m.kind), Some(&Prompt::Search), "padding keeps it");
    let find = a.layout.search_fields[0];
    click(&mut a, find.x + 1, find.y);
    assert!(!a.modal.as_ref().unwrap().secondary_focus);
    click(&mut a, toggle.x + 1, toggle.y);
    assert!(!a.session.find_options().regex);
    click(&mut a, 0, 15);
    assert!(a.modal.is_none(), "a click outside closes it");
    a.run(CommandId::SearchProject);
    a.handle(Event::Paste("foo".into()));
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    assert!(a.modal.is_none() && a.session.state().results().is_some());
    assert_eq!(std::fs::read_to_string(dir.path().join("a.rs")).unwrap(), "Foo foo\n");
}

/// About from ☰: the shared text with the half-block logo when wide enough (text only when narrow), modal,
/// bounded End then Up, Esc back to the unchanged document; tiny terminals still draw.
#[test]
fn about_opens_from_the_application_menu_with_its_logo_and_closes_to_the_document() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.txt");
    std::fs::write(&path, "first\n").unwrap();
    let mut a = App::new(config(dir.path()), Some(path.clone()));
    settle(&mut a, |a| a.session.state().current_source().is_some());
    let screen = |a: &mut App, w: u16, h: u16| {
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(w, h)).unwrap();
        terminal.draw(|f| a.draw(f)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let text: String = buffer.content.iter().map(|c| c.symbol()).collect();
        (text, buffer.content.iter().filter(|c| c.symbol() == "▀").count())
    };
    draw(&mut a, 100, 30);
    click(&mut a, 2, 0);
    a.modal.as_mut().unwrap().at = a.choices().iter().position(|row| row.label == "About Lenz Code").unwrap();
    assert_eq!(a.choices()[a.modal.as_ref().unwrap().at].label, "About Lenz Code");
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    assert!(matches!(a.modal.as_ref().map(|m| &m.kind), Some(Prompt::Details { title: "About Lenz Code", logo: true, .. })));
    let (text, logo) = screen(&mut a, 100, 30);
    assert!(text.contains("About Lenz Code") && text.contains(&format!("Version: {}", env!("CARGO_PKG_VERSION"))), "{text}");
    assert_eq!(logo, 24 * 12, "the whole logo");
    let (text, logo) = screen(&mut a, 40, 12);
    assert_eq!(logo, 0, "no room: text only");
    assert!(text.contains("Version"));
    key(&mut a, KeyCode::End, KeyModifiers::NONE);
    key(&mut a, KeyCode::Up, KeyModifiers::NONE);
    assert!(screen(&mut a, 40, 12).0.contains("Licence"), "End then Up stays within the text");
    for (w, h) in [(1, 1), (0, 0), (20, 5)] {
        draw(&mut a, w, h);
    }
    key(&mut a, KeyCode::Char('x'), KeyModifiers::NONE);
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    assert!(a.modal.is_none());
    assert_eq!(std::fs::read(&path).unwrap(), b"first\n");
    assert!(a.session.state().current_source().is_some_and(|d| d.copy_range(0..d.len_bytes()).unwrap() == "first\n"));
}

/// An app on a git repository with `a.txt` (two lines) committed as "First line", annotated with Alt+B.
fn annotated_app() -> (tempfile::TempDir, App, String) {
    use lenz_content::git::testing::{commit_all, git, repo, write};
    let r = repo();
    write(r.path(), "a.txt", "one\ntwo\n");
    commit_all(r.path(), "First line");
    let sha = git(r.path(), &["rev-parse", "HEAD"]).trim().to_string();
    let mut a = app(r.path());
    a.dispatch(Command::Open(r.path().join("a.txt")));
    settle(&mut a, |a| a.session.state().current_source().is_some());
    key(&mut a, KeyCode::Char('b'), KeyModifiers::ALT);
    settle(&mut a, |a| a.session.state().active().is_some_and(|t| t.annotation(1).is_some()));
    draw(&mut a, 100, 20);
    (r, a, sha)
}

#[test]
fn edited_blame_renders_pending_current_lines_undo_and_failure_without_saving() {
    use lenz_content::git::testing::git;
    let (r, mut a, sha) = annotated_app();
    let gutter = |a: &mut App| {
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 20)).unwrap();
        terminal.draw(|f| a.draw(f)).unwrap();
        let g = &a.layout.panes[0];
        (g.gutter_x..g.gutter_x + g.ann).map(|x| terminal.backend().buffer()[(x, g.text.y)].symbol()).collect::<String>()
    };
    a.enable_draft_input_for_test();
    key(&mut a, KeyCode::Home, KeyModifiers::NONE);
    a.handle(Event::Paste("new\n".into()));
    key(&mut a, KeyCode::Home, KeyModifiers::NONE);
    // Keep the caret on the next line while making the newly inserted first line visible for its gutter check.
    a.dispatch(Command::Scroll(0));
    let tab = a.session.state().active().unwrap().id();
    assert_eq!(a.session.disabled(CommandId::CopyRevision), Some("Blame pending"));
    assert!(a.session.annotation(tab, 0).is_none());
    assert!(gutter(&mut a).contains("Blame pending…"));
    settle(&mut a, |a| matches!(a.session.state().active().unwrap().blame_status(), Some(code_core::blame::Status::Ready)));
    assert_eq!(a.session.annotation(tab, 0).unwrap().label, "Not committed");
    for line in [1, 2] { assert_eq!(a.session.annotation(tab, line).unwrap().commit.sha, sha); }
    let shown = draw_screen(&mut a, 100, 20);
    assert!(shown.contains("Not committed") && shown.contains("new"), "top {:?}, selection {:?}:\n{shown}", a.session.state().active().map(|t| t.top_line()), a.session.state().selection());
    for _ in 0..4 { key(&mut a, KeyCode::Right, KeyModifiers::SHIFT); }
    assert_eq!(a.session.state().selection().unwrap().range(), 4..8);
    a.handle(Event::Paste("λ".into()));
    assert!(a.session.annotation(tab, 0).is_none());
    assert!(gutter(&mut a).contains("Blame pending…"));
    settle(&mut a, |a| matches!(a.session.state().active().unwrap().blame_status(), Some(code_core::blame::Status::Ready)));
    let source = a.session.state().current_source().unwrap();
    assert_eq!(source.copy_range(0..source.len_bytes()).unwrap(), "new\nλtwo\n");
    assert_eq!(a.session.annotation(tab, 0).unwrap().label, "Not committed");
    assert_eq!(a.session.annotation(tab, 1).unwrap().label, "Not committed");
    key(&mut a, KeyCode::Char('z'), KeyModifiers::CONTROL);
    settle(&mut a, |a| matches!(a.session.state().active().unwrap().blame_status(), Some(code_core::blame::Status::Ready)));
    let source = a.session.state().current_source().unwrap();
    assert_eq!(source.copy_range(0..source.len_bytes()).unwrap(), "new\none\ntwo\n");
    key(&mut a, KeyCode::Char('z'), KeyModifiers::CONTROL);
    assert!(a.session.annotation(tab, 0).is_none());
    assert!(gutter(&mut a).contains("Blame pending…"));
    settle(&mut a, |a| matches!(a.session.state().active().unwrap().blame_status(), Some(code_core::blame::Status::Ready)));
    assert_eq!(a.session.annotation(tab, 0).unwrap().commit.sha, sha);
    let source = a.session.state().current_source().unwrap();
    assert_eq!(source.copy_range(0..source.len_bytes()).unwrap(), "one\ntwo\n");
    std::fs::write(r.path().join("bad-revisions"), "not-a-revision\n").unwrap();
    git(r.path(), &["config", "blame.ignoreRevsFile", "bad-revisions"]);
    for _ in 0..2 { key(&mut a, KeyCode::Char('b'), KeyModifiers::ALT); }
    settle(&mut a, |a| matches!(a.session.state().active().unwrap().blame_status(), Some(code_core::blame::Status::Failed(_))));
    assert!(gutter(&mut a).contains("Blame failed"));
    assert!(a.session.annotation(tab, 0).is_none());
    assert_eq!(a.session.disabled(CommandId::CopyRevision), Some("Blame failed"));
    assert_eq!(std::fs::read_to_string(r.path().join("a.txt")).unwrap(), "one\ntwo\n");
}
struct Pane {
    gutter_x: u16,
    ann: u16,
    text: ratatui::layout::Rect,
}
fn mouse(a: &mut App, button: MouseButton, x: u16, y: u16) {
    a.handle(Event::Mouse(MouseEvent { kind: MouseEventKind::Down(button), column: x, row: y, modifiers: KeyModifiers::NONE }));
}
fn commit_body(a: &App) -> Option<String> {
    match &a.modal.as_ref()?.kind {
        Prompt::Details { kind: Details::Commit, body, .. } => Some(body.clone()),
        _ => None,
    }
}

#[test]
fn annotate_click_opens_the_commit_and_its_buttons_run() {
    let (r, mut a, sha) = annotated_app();
    let p = { let g = &a.layout.panes[0]; (g.gutter_x, g.ann, g.text) };
    let p = Pane { gutter_x: p.0, ann: p.1, text: p.2 };
    assert_eq!(p.ann as usize, code_core::blame::ANNOTATION_COLS + 1);
    // The second line's annotation: the caret goes there and the commit opens.
    mouse(&mut a, MouseButton::Left, p.gutter_x + 2, p.text.y + 1);
    assert_eq!(a.session.state().selection().unwrap().range(), 4..4);
    settle(&mut a, |a| a.session.state().commit_popup().is_some_and(|c| c.details.is_some()));
    draw(&mut a, 100, 20);
    assert!(commit_body(&a).unwrap().starts_with(&format!("commit {sha}\nAuthor: t <t@t>")));
    key(&mut a, KeyCode::Char('c'), KeyModifiers::NONE);
    assert_eq!(a.copy.take().as_deref(), Some(sha.as_str()));
    // Show Diff: Lenz Diff is not beside the test binary, so it says so; the popup closes.
    key(&mut a, KeyCode::Char('d'), KeyModifiers::NONE);
    draw(&mut a, 100, 20);
    assert!(a.modal.is_none() && a.session.state().commit_popup().is_none());
    assert!(a.session.status_message().starts_with("Show Diff: Lenz Diff is not beside Lenz Code"), "{}", a.session.status_message());
    // Escape closes a reopened popup; the file is unchanged.
    mouse(&mut a, MouseButton::Left, p.gutter_x + 2, p.text.y);
    draw(&mut a, 100, 20);
    assert!(commit_body(&a).is_some());
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    draw(&mut a, 100, 20);
    assert!(a.modal.is_none() && a.session.state().commit_popup().is_none());
    assert_eq!(std::fs::read_to_string(r.path().join("a.txt")).unwrap(), "one\ntwo\n");
}

#[test]
fn right_click_on_the_gutter_opens_the_context_menu_at_that_line() {
    let (_r, mut a, _) = annotated_app();
    let p = { let g = &a.layout.panes[0]; (g.gutter_x, g.ann, g.text) };
    let p = Pane { gutter_x: p.0, ann: p.1, text: p.2 };
    for x in [p.gutter_x, p.text.x - 2] {
        a.dispatch(Command::Select(Selection { anchor: 0, head: 0 }));
        mouse(&mut a, MouseButton::Right, x, p.text.y + 1);
        assert_eq!(a.session.state().selection().unwrap().range(), 4..4, "x {x}");
        let entries = match &a.modal.as_ref().unwrap().kind { Prompt::Context { entries, .. } => entries.clone(), k => panic!("{k:?}") };
        assert!(entries.contains(&MenuEntry::Command(CommandId::ShowCommit)) && entries.contains(&MenuEntry::Command(CommandId::CloseAnnotations)));
        key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    }
}

#[test]
fn annotation_column_takes_text_columns_and_hides_in_a_tiny_terminal() {
    let (_r, mut a, _) = annotated_app();
    let with = a.layout.panes[0].text.width;
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 20)).unwrap();
    terminal.draw(|f| a.draw(f)).unwrap();
    let screen: String = terminal.backend().buffer().content.iter().map(|c| c.symbol()).collect();
    let today = lenz_content::blame::date(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64);
    assert!(screen.contains(&format!("{today} t")), "{screen}");
    key(&mut a, KeyCode::Char('b'), KeyModifiers::ALT);
    draw(&mut a, 100, 20);
    assert_eq!(a.layout.panes[0].text.width - with, code_core::blame::ANNOTATION_COLS as u16 + 1, "off: the text takes the columns back");
    key(&mut a, KeyCode::Char('b'), KeyModifiers::ALT);
    settle(&mut a, |a| a.session.state().active().is_some_and(|t| t.annotation(1).is_some()));
    for (w, h) in [(20, 6), (12, 3)] {
        draw(&mut a, w, h);
        assert_eq!(a.layout.panes[0].ann, 0, "{w}×{h}: no room");
        key(&mut a, KeyCode::End, KeyModifiers::CONTROL);
        key(&mut a, KeyCode::Up, KeyModifiers::NONE);
        draw(&mut a, w, h);
    }
}

#[test]
fn right_click_on_the_gutter_inside_a_selection_still_moves_the_caret() {
    let (_r, mut a, _) = annotated_app();
    let p = { let g = &a.layout.panes[0]; (g.gutter_x, g.text.y) };
    a.dispatch(Command::Select(Selection { anchor: 0, head: 8 }));
    mouse(&mut a, MouseButton::Right, p.0 + 2, p.1 + 1);
    assert_eq!(a.session.state().selection().unwrap().range(), 4..4, "the clicked line, not the selection's head");
}

/// An app on a git repository: `main` with `a.txt`, branch `other` changing it, tag `v1`, worktree `second` on `side`.
fn branches_app() -> (tempfile::TempDir, tempfile::TempDir, App) {
    use lenz_content::git::testing::{commit_all, git, repo, write};
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
    let wt = tempfile::tempdir().unwrap();
    git(r.path(), &["worktree", "add", "-q", "-b", "side", &wt.path().join("second").to_string_lossy()]);
    let state = tempfile::tempdir().unwrap();
    let mut a = App::new(config(state.path()), Some(r.path().to_path_buf()));
    std::mem::forget(state);
    settle(&mut a, |a| a.session.branch_label().is_some());
    draw(&mut a, 100, 20);
    (r, wt, a)
}
fn typed(a: &mut App, text: &str) {
    for c in text.chars() {
        key(a, KeyCode::Char(c), KeyModifiers::NONE);
    }
}
fn prompt_kind(a: &App) -> Option<Prompt> {
    a.modal.as_ref().map(|m| m.kind.clone())
}
fn open_popup(a: &mut App) {
    a.run(CommandId::GitBranches);
    settle(a, |a| a.session.state().branches().is_some_and(|b| !b.loading));
    draw(a, 100, 20);
}

#[test]
fn the_toolbar_branch_opens_the_popup_and_enter_checks_out() {
    let (r, _wt, mut a) = branches_app();
    let (_, rect) = *a.layout.controls.iter().find(|(id, _)| *id == CommandId::GitBranches).expect("a branch cell");
    assert_eq!(rect.y, 0);
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 20)).unwrap();
    terminal.draw(|f| a.draw(f)).unwrap();
    let text: String = (rect.x..rect.right()).map(|x| terminal.backend().buffer()[(x, rect.y)].symbol().to_string()).collect();
    assert_eq!(text.trim(), "⎇ main");
    click(&mut a, rect.x + 1, rect.y);
    settle(&mut a, |a| a.session.state().branches().is_some_and(|b| !b.loading));
    draw(&mut a, 100, 20);
    assert_eq!(prompt_kind(&a), Some(Prompt::Branches));
    assert!(a.choices().iter().any(|c| c.label.contains("● main")));
    typed(&mut a, "oth");
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    settle(&mut a, |a| !a.session.state().checking_out());
    draw(&mut a, 100, 20);
    assert_eq!(a.session.branch_label().as_deref(), Some("⎇ other"));
    assert!(a.modal.is_none());
    assert_eq!(std::fs::read_to_string(r.path().join("a.txt")).unwrap(), "other\n");
}

#[test]
fn a_rows_menu_makes_a_branch_and_the_revision_prompt_checks_out() {
    let (r, _wt, mut a) = branches_app();
    open_popup(&mut a);
    typed(&mut a, "v1");
    key(&mut a, KeyCode::Right, KeyModifiers::NONE);
    assert!(matches!(prompt_kind(&a), Some(Prompt::BranchMenu { .. })));
    assert_eq!(a.choices().into_iter().map(|c| c.label).collect::<Vec<_>>(), ["Checkout", "New Branch from Here…", "Compare with Working Tree"]);
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    assert_eq!((prompt_kind(&a), a.modal.as_ref().unwrap().input.as_str()), (Some(Prompt::Branches), "v1"), "back to the list");
    key(&mut a, KeyCode::Right, KeyModifiers::NONE);
    key(&mut a, KeyCode::Down, KeyModifiers::NONE);
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(prompt_kind(&a), Some(Prompt::NewBranch { from: Some("refs/tags/v1".into()), checkout: true }));
    assert!(a.modal_title().starts_with("New branch from v1 "), "{}", a.modal_title());
    key(&mut a, KeyCode::Tab, KeyModifiers::NONE);
    assert_eq!(prompt_kind(&a), Some(Prompt::NewBranch { from: Some("refs/tags/v1".into()), checkout: false }));
    key(&mut a, KeyCode::Tab, KeyModifiers::NONE);
    typed(&mut a, "other");
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(a.modal.as_ref().unwrap().error.as_deref(), Some("A branch named \"other\" already exists"));
    for _ in 0..5 { key(&mut a, KeyCode::Backspace, KeyModifiers::NONE); }
    typed(&mut a, "from-v1");
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    settle(&mut a, |a| !a.session.state().checking_out());
    assert_eq!(a.session.branch_label().as_deref(), Some("⎇ from-v1"));
    a.run(CommandId::CheckoutRevision);
    assert_eq!(prompt_kind(&a), Some(Prompt::Revision));
    typed(&mut a, "nope");
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(a.modal.as_ref().unwrap().error.as_deref(), Some("No commit named \"nope\""));
    for _ in 0..4 { key(&mut a, KeyCode::Backspace, KeyModifiers::NONE); }
    typed(&mut a, "v1");
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    settle(&mut a, |a| !a.session.state().checking_out());
    let sha = lenz_content::git::testing::git(r.path(), &["rev-parse", "v1"]).trim().to_string();
    assert_eq!(a.session.branch_label(), Some(format!("⎇ {}", &sha[..7])));
}

#[test]
fn checkout_problems_offer_smart_and_a_confirmed_force_by_key_and_click() {
    let (r, _wt, mut a) = branches_app();
    std::fs::write(r.path().join("a.txt"), "mine\n").unwrap();
    a.dispatch(Command::Checkout(lenz_content::branches::Target::Branch("other".into())));
    settle(&mut a, |a| a.session.state().checkout_problem().is_some());
    draw(&mut a, 100, 20);
    assert!(matches!(prompt_kind(&a), Some(Prompt::Details { kind: Details::Checkout, .. })));
    assert_eq!(a.layout.details_choices.len(), 3);
    key(&mut a, KeyCode::Char('f'), KeyModifiers::NONE);
    draw(&mut a, 100, 20);
    assert_eq!(a.layout.details_choices.len(), 2, "the confirmation step");
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    key(&mut a, KeyCode::Char('f'), KeyModifiers::NONE);
    draw(&mut a, 100, 20);
    assert_eq!(std::fs::read_to_string(r.path().join("a.txt")).unwrap(), "mine\n", "neither Enter nor a second f discards");
    assert_eq!(a.layout.details_choices.len(), 2);
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    draw(&mut a, 100, 20);
    assert!(a.modal.is_none() && a.session.state().checkout_problem().is_none());
    a.dispatch(Command::Checkout(lenz_content::branches::Target::Branch("other".into())));
    settle(&mut a, |a| a.session.state().checkout_problem().is_some());
    draw(&mut a, 100, 20);
    let force = a.layout.details_choices[1];
    click(&mut a, force.x + 1, force.y);
    draw(&mut a, 100, 20);
    let confirm = a.layout.details_choices[0];
    click(&mut a, confirm.x + 1, confirm.y);
    settle(&mut a, |a| !a.session.state().checking_out());
    assert_eq!(a.session.branch_label().as_deref(), Some("⎇ other"));
    assert_eq!(std::fs::read_to_string(r.path().join("a.txt")).unwrap(), "other\n");
    // By keys: f lists, y confirms.
    std::fs::write(r.path().join("a.txt"), "mine again\n").unwrap();
    a.dispatch(Command::Checkout(lenz_content::branches::Target::Branch("main".into())));
    settle(&mut a, |a| a.session.state().checkout_problem().is_some());
    draw(&mut a, 100, 20);
    key(&mut a, KeyCode::Char('f'), KeyModifiers::NONE);
    key(&mut a, KeyCode::Char('y'), KeyModifiers::NONE);
    settle(&mut a, |a| !a.session.state().checking_out());
    assert_eq!(a.session.branch_label().as_deref(), Some("⎇ main"));
    assert_eq!(std::fs::read_to_string(r.path().join("a.txt")).unwrap(), "main\n");
}

#[test]
fn a_worktree_opens_here_or_in_a_new_view_and_a_tiny_terminal_copes() {
    let (_r, wt, mut a) = branches_app();
    open_popup(&mut a);
    for (w, h) in [(20, 6), (12, 3)] {
        draw(&mut a, w, h);
        key(&mut a, KeyCode::End, KeyModifiers::NONE);
        key(&mut a, KeyCode::Up, KeyModifiers::NONE);
        draw(&mut a, w, h);
    }
    draw(&mut a, 100, 20);
    a.modal.as_mut().unwrap().input = "second".into();
    a.modal.as_mut().unwrap().at = 0;
    key(&mut a, KeyCode::Right, KeyModifiers::NONE);
    key(&mut a, KeyCode::Down, KeyModifiers::NONE);
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    assert!(a.new_view);
    assert_eq!(a.new_view_root.as_deref().map(|p| std::fs::canonicalize(p).unwrap()), Some(std::fs::canonicalize(wt.path().join("second")).unwrap()));
    open_popup(&mut a);
    typed(&mut a, "second");
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    settle(&mut a, |a| a.session.branch_label().as_deref() == Some("⎇ side"));
}

#[test]
fn enter_on_the_current_branch_or_worktree_does_nothing_and_keeps_the_popup() {
    let (_r, _wt, mut a) = branches_app();
    open_popup(&mut a);
    for current in ["● main", "Worktrees"] {
        let at = a.choices().iter().position(|c| c.label.contains(current) && c.label.contains("●")).unwrap();
        a.modal.as_mut().unwrap().at = at;
        key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
        assert!(!a.session.state().checking_out(), "{current}: no checkout");
        assert_eq!(prompt_kind(&a), Some(Prompt::Branches), "{current}: the popup stays");
    }
    assert!(!a.session.status_details().contains("Checking out"));
}

/// A Lenz Diff terminal app beside where the app thinks Lenz Code is.
fn lenz_diff_beside(a: &mut App) -> tempfile::TempDir {
    let bin = tempfile::tempdir().unwrap();
    std::fs::write(bin.path().join("lenz-diff-tui"), "").unwrap();
    a.exe = Some(bin.path().join("lenz-code"));
    bin
}
fn foreground_args(a: &mut App) -> Option<Vec<String>> {
    a.foreground.take().map(|l| l.args)
}

#[test]
fn the_git_menu_compares_the_working_tree_in_this_terminal() {
    let (r, _wt, mut a) = branches_app();
    let _bin = lenz_diff_beside(&mut a);
    a.run(CommandId::ShowLocalChanges);
    let launch = a.foreground.take().unwrap();
    assert!(launch.program.ends_with("lenz-diff-tui"), "{}", launch.program);
    assert_eq!(launch.args, ["HEAD"]);
    a.run(CommandId::ShowBranchChanges);
    assert!(a.foreground.is_none());
    assert!(a.session.status_message().contains("Show Changes Since Branch Start: main is the base branch"), "{}", a.session.status_message());
    a.run(CommandId::CompareWithBase);
    assert_eq!(foreground_args(&mut a), Some(vec!["refs/heads/main".to_string()]));
    // Compare with Branch, Tag or Revision…: a pick list, or what is typed.
    a.run(CommandId::CompareWithRevision);
    settle(&mut a, |a| a.session.state().branches().is_some_and(|b| !b.loading));
    draw(&mut a, 100, 20);
    assert_eq!(prompt_kind(&a), Some(Prompt::Branches));
    assert!(a.modal_title().starts_with("Compare with Branch, Tag or Revision · type to filter or a revision"), "{}", a.modal_title());
    assert!(a.choices().iter().any(|c| c.label.ends_with("v1")) && !a.choices().iter().any(|c| c.label.contains("New Branch")));
    typed(&mut a, "oth");
    key(&mut a, KeyCode::Down, KeyModifiers::NONE);
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(foreground_args(&mut a), Some(vec!["refs/heads/other".to_string()]));
    a.run(CommandId::CompareWithRevision);
    settle(&mut a, |a| a.session.state().branches().is_some_and(|b| !b.loading));
    // A listed name is picked from the list; other text compares as typed.
    typed(&mut a, "v1");
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(foreground_args(&mut a), Some(vec!["refs/tags/v1".to_string()]));
    a.run(CommandId::CompareWithRevision);
    settle(&mut a, |a| a.session.state().branches().is_some_and(|b| !b.loading));
    typed(&mut a, "HEAD");
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(foreground_args(&mut a), Some(vec!["HEAD".to_string()]));
    assert!(a.modal.is_none());
    // A tiny terminal draws the list.
    a.run(CommandId::CompareWithRevision);
    settle(&mut a, |a| a.session.state().branches().is_some_and(|b| !b.loading));
    draw(&mut a, 20, 6);
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    assert!(a.modal.is_none() && a.session.state().branches().is_none());
    assert_eq!(a.session.branch_label().as_deref(), Some("⎇ main"), "nothing checked out");
    assert_eq!(std::fs::read_to_string(r.path().join("a.txt")).unwrap(), "main\n");
}

#[test]
fn rows_compare_and_compare_with_worktree_lists_the_other_worktree() {
    let (r, wt, mut a) = branches_app();
    let _bin = lenz_diff_beside(&mut a);
    open_popup(&mut a);
    typed(&mut a, "other");
    key(&mut a, KeyCode::Right, KeyModifiers::NONE);
    assert_eq!(a.choices().into_iter().map(|c| c.label).collect::<Vec<_>>(), ["Checkout", "New Branch from Here…", "Compare with Working Tree", "Show Changes Since Branching from Here"]);
    for _ in 0..3 { key(&mut a, KeyCode::Down, KeyModifiers::NONE); }
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    let split = lenz_content::git::testing::git(r.path(), &["rev-parse", "main"]).trim().to_string();
    assert_eq!(foreground_args(&mut a), Some(vec![split]));
    assert!(a.modal.is_none() && a.session.state().branches().is_none());
    a.run(CommandId::CompareWithWorktree);
    settle(&mut a, |a| a.session.state().branches().is_some_and(|b| !b.loading));
    draw(&mut a, 100, 20);
    assert_eq!(prompt_kind(&a), Some(Prompt::Branches));
    assert!(a.modal_title().starts_with("Compare with Worktree · type to filter · Enter compares"), "{}", a.modal_title());
    assert_eq!(a.choices().len(), 1);
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    let args = foreground_args(&mut a).unwrap();
    assert_eq!(args.last().map(String::as_str), Some(&*wt.path().join("second").to_string_lossy()));
    // Gone: the popup says so in a tiny terminal; Enter does nothing and Esc closes.
    std::fs::remove_dir_all(wt.path().join("second")).unwrap();
    a.run(CommandId::CompareWithWorktree);
    settle(&mut a, |a| a.session.state().branches().is_some_and(|b| !b.loading));
    draw(&mut a, 20, 6);
    assert_eq!(a.modal_title(), "Compare with Worktree · No other worktree");
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    key(&mut a, KeyCode::End, KeyModifiers::NONE);
    key(&mut a, KeyCode::Up, KeyModifiers::NONE);
    draw(&mut a, 20, 6);
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    assert!(a.modal.is_none() && a.session.state().branches().is_none() && a.foreground.is_none());
}

#[test]
fn space_or_ctrl_click_marks_two_entries_and_ctrl_d_compares_them() {
    use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "one\n").unwrap();
    std::fs::write(dir.path().join("b.txt"), "two\n").unwrap();
    let state = tempfile::tempdir().unwrap();
    let mut a = App::new(config(state.path()), Some(dir.path().to_path_buf()));
    settle(&mut a, |a| a.session.state().tree_rows().any(|r| &*r.path == "b.txt"));
    let _bin = lenz_diff_beside(&mut a);
    draw(&mut a, 100, 20);
    let at = |a: &App, name: &str| a.session.state().tree_rows().position(|r| &*r.path == name).unwrap();
    // Keyboard: Space marks the highlighted entry, twice.
    a.focus = Focus::Tree;
    a.tree_at = at(&a, "a.txt");
    key(&mut a, KeyCode::Char(' '), KeyModifiers::NONE);
    a.tree_at = at(&a, "b.txt");
    key(&mut a, KeyCode::Char(' '), KeyModifiers::NONE);
    let shown = draw_screen(&mut a, 100, 20);
    assert_eq!(a.session.state().tree_marks().len(), 2);
    assert_eq!(shown.matches('✓').count(), 2, "both marked rows show a check");
    key(&mut a, KeyCode::Char('d'), KeyModifiers::CONTROL);
    let launch = a.foreground.take().expect("Lenz Diff in this terminal");
    assert_eq!(launch.args, ["--no-index".to_string(), dir.path().join("a.txt").to_string_lossy().into_owned(), dir.path().join("b.txt").to_string_lossy().into_owned()]);
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    assert!(a.session.state().tree_marks().is_empty(), "Esc forgets the marks");
    // Mouse: click a.txt, Ctrl+click b.txt.
    let (x, y) = (a.layout.tree.x + 2, a.layout.tree.y);
    let row = |a: &App, name: &str| y + (at(a, name) - a.tree_top) as u16;
    let click = |a: &mut App, row: u16, modifiers| { a.handle(Event::Mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: x, row, modifiers })); };
    let (ra, rb) = (row(&a, "a.txt"), row(&a, "b.txt"));
    click(&mut a, ra, KeyModifiers::NONE);
    click(&mut a, rb, KeyModifiers::CONTROL);
    assert_eq!(a.session.state().tree_marks(), ["a.txt".into(), "b.txt".into()] as [std::sync::Arc<str>; 2]);
    click(&mut a, ra, KeyModifiers::NONE);
    assert!(a.session.state().tree_marks().is_empty(), "a plain click forgets them");
    draw(&mut a, 20, 6);
}

/// The screen as text after a draw.
pub(crate) fn draw_screen(a: &mut App, width: u16, height: u16) -> String {
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
    terminal.draw(|f| a.draw(f)).unwrap();
    terminal.backend().buffer().content.iter().map(|c| c.symbol()).collect()
}

#[test]
fn completion_popup_takes_only_its_keys_and_fits_a_tiny_terminal() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"completion\"\nversion = \"0.1.0\"\n").unwrap();
    let path = root.join("main.rs");
    let source = "fn main() {  }\nfn helper() {}\nfn hello() {}\n";
    std::fs::write(&path, source).unwrap();
    let profile = std::env::current_exe().unwrap().parent().unwrap().parent().unwrap().to_owned();
    let mut build = std::process::Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    build.args(["build", "-q", "-p", "code-core", "--example", "fake_lsp"]).current_dir(env!("CARGO_MANIFEST_DIR"));
    if profile.ends_with("release") { build.arg("--release"); }
    assert!(build.status().unwrap().success());
    let fake = profile.join(format!("examples/fake_lsp{}", std::env::consts::EXE_SUFFIX));
    let mut cfg = config(dir.path());
    cfg.idle = code_core::semantics::IDLE;
    std::fs::create_dir_all(cfg.providers.parent().unwrap()).unwrap();
    std::fs::write(&cfg.providers, serde_json::json!({ "providers": { "rust-analyzer": { "executable": fake, "env": { "FAKE_LSP_NO_DECOR": "1" } } } }).to_string()).unwrap();
    let mut a = App::new(cfg, Some(root));
    a.enable_draft_input_for_test();
    a.dispatch(Command::Open(path.clone()));
    settle(&mut a, |a| a.session.state().provider().is_some_and(|p| p.status == code_core::semantics::Status::Ready)
        && a.session.state().view(a.session.state().active().unwrap().id()).unwrap().highlights().is_some());
    let text = |a: &App| { let s = a.session.state().current_source().unwrap(); s.copy_range(0..s.len_bytes()).unwrap() };
    let labels = |a: &App| a.session.state().completion().map(|c| c.rows().map(|r| r.label.clone()).collect::<Vec<_>>());
    let typed = |a: &mut App, s: &str| for c in s.chars() { key(a, KeyCode::Char(c), KeyModifiers::NONE) };
    let at = source.find("{  }").unwrap() + 2;
    a.dispatch(Command::Select(code_core::session::Selection { anchor: at, head: at }));
    draw(&mut a, 100, 30);
    // Typing opens it under the name; Down and Enter accept the second suggestion as one Undo step.
    typed(&mut a, "he");
    settle(&mut a, |a| a.session.state().completion().is_some());
    assert_eq!(labels(&a).unwrap(), ["hello", "helper"]);
    draw(&mut a, 100, 30);
    let popup = a.layout.completion;
    let caret_row = a.layout.panes[0].hits.iter().find(|h| h.byte == at).unwrap().y;
    assert_eq!((popup.y, popup.height), (caret_row + 1, 2), "{popup:?}");
    key(&mut a, KeyCode::Down, KeyModifiers::NONE);
    assert_eq!(a.session.state().completion().unwrap().selected(), 1);
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(text(&a), source.replacen("{  }", "{ helper }", 1));
    assert!(a.session.state().completion().is_none());
    key(&mut a, KeyCode::Char('z'), KeyModifiers::CONTROL);
    assert_eq!(text(&a), source.replacen("{  }", "{ he }", 1));
    // Esc dismisses without editing; Tab accepts; punctuation types and closes it.
    typed(&mut a, "l");
    settle(&mut a, |a| a.session.state().completion().is_some());
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    assert!(a.session.state().completion().is_none());
    assert_eq!(text(&a), source.replacen("{  }", "{ hel }", 1));
    typed(&mut a, "l");
    settle(&mut a, |a| a.session.state().completion().is_some());
    key(&mut a, KeyCode::Tab, KeyModifiers::NONE);
    assert_eq!(text(&a), source.replacen("{  }", "{ hello }", 1));
    typed(&mut a, ".he");
    settle(&mut a, |a| a.session.state().completion().is_some());
    // Punctuation is not accepted as a suggestion. (A bracket would also open its pair, so a comma keeps this
    // test about completion.)
    typed(&mut a, ",");
    assert!(a.session.state().completion().is_none());
    assert_eq!(text(&a), source.replacen("{  }", "{ hello.he, }", 1));
    // Off: typing opens nothing; Ctrl+Space (or its NUL) still asks.
    a.run(CommandId::AutoCompletion);
    assert!(!a.session.state().auto_completion());
    key(&mut a, KeyCode::Backspace, KeyModifiers::NONE);
    typed(&mut a, "l");
    let until = Instant::now() + Duration::from_millis(300);
    while Instant::now() < until { if let Ok(c) = a.session.completions().try_recv() { a.apply(c); } }
    assert!(a.session.state().completion().is_none());
    key(&mut a, KeyCode::Char(' '), KeyModifiers::CONTROL);
    settle(&mut a, |a| a.session.state().completion().is_some());
    // A modal takes every key first and leaves the draft unchanged.
    let before = text(&a);
    a.run(CommandId::FindAction);
    assert!(a.modal.is_some());
    typed(&mut a, "zz");
    key(&mut a, KeyCode::Tab, KeyModifiers::NONE);
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    assert!(a.modal.is_none());
    assert_eq!(text(&a), before);
    if a.session.state().completion().is_none() {
        key(&mut a, KeyCode::Null, KeyModifiers::NONE);
        settle(&mut a, |a| a.session.state().completion().is_some());
    }
    // A tiny terminal keeps it inside; a click on a row accepts it.
    draw(&mut a, 30, 6);
    let popup = a.layout.completion;
    assert!(popup.width > 0 && popup.right() <= 30 && popup.bottom() <= 6, "{popup:?}");
    a.handle(Event::Mouse(MouseEvent { kind: MouseEventKind::Down(crossterm::event::MouseButton::Left), column: popup.x + 1, row: popup.y + 1, modifiers: KeyModifiers::NONE }));
    assert_eq!(text(&a), source.replacen("{  }", "{ hello.helper }", 1));
    // Split, the same document in both panes: the popup opens in the focused (right) one.
    a.dispatch(Command::Split(code_core::session::Split::Columns));
    draw(&mut a, 120, 30);
    assert_eq!(a.session.state().focused_pane(), 1);
    a.dispatch(Command::Select(code_core::session::Selection { anchor: at, head: at }));
    a.dispatch(Command::Complete);
    settle(&mut a, |a| a.session.state().completion().is_some());
    draw(&mut a, 120, 30);
    let (popup, right) = (a.layout.completion, a.layout.panes.iter().find(|g| g.pane == 1).unwrap().text);
    assert!(popup.width > 0 && popup.x >= right.x, "{popup:?} outside {right:?}");
    assert_eq!(std::fs::read_to_string(path).unwrap(), source);
}

#[test]
fn snippet_placeholders_are_tabbed_through_and_escape_ends_them() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"snippets\"\nversion = \"0.1.0\"\n").unwrap();
    let path = root.join("main.rs");
    let source = "fn main() {  }\n";
    std::fs::write(&path, source).unwrap();
    let profile = std::env::current_exe().unwrap().parent().unwrap().parent().unwrap().to_owned();
    let mut build = std::process::Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    build.args(["build", "-q", "-p", "code-core", "--example", "fake_lsp"]).current_dir(env!("CARGO_MANIFEST_DIR"));
    if profile.ends_with("release") { build.arg("--release"); }
    assert!(build.status().unwrap().success());
    let fake = profile.join(format!("examples/fake_lsp{}", std::env::consts::EXE_SUFFIX));
    let cfg = config(dir.path());
    std::fs::create_dir_all(cfg.providers.parent().unwrap()).unwrap();
    std::fs::write(&cfg.providers, serde_json::json!({ "providers": { "rust-analyzer": { "executable": fake, "env": { "FAKE_LSP_NO_DECOR": "1" } } } }).to_string()).unwrap();
    let mut a = App::new(cfg, Some(root));
    a.enable_draft_input_for_test();
    a.dispatch(Command::Open(path.clone()));
    settle(&mut a, |a| a.session.state().provider().is_some_and(|p| p.status == code_core::semantics::Status::Ready)
        && a.session.state().view(a.session.state().active().unwrap().id()).unwrap().highlights().is_some());
    let text = |a: &App| { let s = a.session.state().current_source().unwrap(); s.copy_range(0..s.len_bytes()).unwrap() };
    let selected = |a: &App| { let s = a.session.state(); let doc = s.current_source().unwrap(); s.selection().map(|sel| doc.copy_range(sel.range()).unwrap()).unwrap_or_default() };
    let typed = |a: &mut App, t: &str| for c in t.chars() { key(a, KeyCode::Char(c), KeyModifiers::NONE) };
    let at = source.find("{  }").unwrap() + 2;
    a.dispatch(Command::Select(code_core::session::Selection { anchor: at, head: at }));
    draw(&mut a, 100, 30);
    typed(&mut a, "snippet");
    settle(&mut a, |a| a.session.state().completion().is_some());
    // Tab accepts the suggestion; the terminal draws the snippet's first placeholder in its status bar.
    key(&mut a, KeyCode::Tab, KeyModifiers::NONE);
    assert_eq!(text(&a), "fn main() { if cond {\n    body();\n} }\n");
    assert_eq!(selected(&a), "body");
    draw(&mut a, 100, 30);
    let screen = draw_screen(&mut a, 100, 30);
    assert!(screen.contains("Snippet: placeholder 1 of 3"), "{screen}");
    key(&mut a, KeyCode::Tab, KeyModifiers::NONE);
    assert_eq!(selected(&a), "cond", "Tab visits the next placeholder instead of indenting");
    key(&mut a, KeyCode::Tab, KeyModifiers::NONE);
    assert_eq!(a.session.state().selection().unwrap().range(), 35..35, "then the final caret");
    key(&mut a, KeyCode::Tab, KeyModifiers::NONE);
    assert_eq!(text(&a), "fn main() { if cond {\n    body();\n} }\n", "the last Tab only ends the snippet");
    assert_eq!(a.session.snippet_hint(), None);
    key(&mut a, KeyCode::Char('z'), KeyModifiers::CONTROL);
    assert_eq!(text(&a), "fn main() { snippet }\n", "one Undo takes the whole expansion back");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), source);
    // One placeholder written three times: Escape ends the snippet and Tab indents again.
    key(&mut a, KeyCode::Char(' '), KeyModifiers::CONTROL);
    settle(&mut a, |a| a.session.state().completion().is_some());
    // By its label, not its position: the fake server's list grows with the features it covers.
    let repeat = a.session.state().completion().unwrap().rows().position(|r| r.label == "snippet_repeat").unwrap();
    for _ in 0..repeat { key(&mut a, KeyCode::Down, KeyModifiers::NONE); }
    key(&mut a, KeyCode::Tab, KeyModifiers::NONE);
    assert_eq!(text(&a), "fn main() { sum + sum = sum }\n");
    assert_eq!(a.session.state().active().unwrap().selections().count(), 3, "one stop selects every occurrence");
    typed(&mut a, "x");
    assert_eq!(text(&a), "fn main() { x + x = x }\n", "typing changes every occurrence in one transaction");
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    assert_eq!(a.session.snippet_hint(), None, "Escape ends the snippet");
    assert_eq!(text(&a), "fn main() { x + x = x }\n", "and edits nothing");
    key(&mut a, KeyCode::Tab, KeyModifiers::NONE);
    assert_eq!(text(&a), "fn main() { x     + x = x }\n", "Tab indents again at the caret");
    assert_eq!(std::fs::read_to_string(path).unwrap(), source, "no file is written");
}

#[test]
fn ctrl_alt_down_adds_a_caret_and_typing_changes_every_selection() {
    let dir = tempfile::tempdir().unwrap(); let path = dir.path().join("carets.txt"); std::fs::write(&path, "aa bb aa\none").unwrap();
    let mut a = app(dir.path()); a.dispatch(Command::OpenRetained(path.clone())); settle(&mut a, |a| a.session.state().current_source().is_some()); a.enable_draft_input_for_test(); draw(&mut a, 80, 16);
    let text = |a: &App| { let source = a.session.state().current_source().unwrap(); source.copy_range(0..source.len_bytes()).unwrap() };
    let carets = |a: &App| { let mut at: Vec<_> = a.session.state().active().unwrap().selections().filter(|s| s.anchor == s.head).map(|s| s.head).collect(); at.sort_unstable(); at };
    key(&mut a, KeyCode::Right, KeyModifiers::NONE);
    key(&mut a, KeyCode::Down, KeyModifiers::CONTROL | KeyModifiers::ALT);
    assert_eq!(carets(&a), [1, 10], "the next line's column, as the menu command does");
    key(&mut a, KeyCode::Char('x'), KeyModifiers::NONE);
    assert_eq!(text(&a), "axa bb aa\noxne");
    assert_eq!(carets(&a), [2, 12]);
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    assert_eq!(carets(&a), [12], "Esc keeps the primary cursor before it leaves the text");
    key(&mut a, KeyCode::Char('x'), KeyModifiers::NONE);
    assert_eq!(text(&a), "axa bb aa\noxxne", "one caret types again");
    key(&mut a, KeyCode::Char('z'), KeyModifiers::CONTROL);
    assert_eq!(text(&a), "axa bb aa\noxne");
    assert_eq!(std::fs::read(&path).unwrap(), b"aa bb aa\none", "the terminal never writes the file");
}

#[test]
fn terminal_menu_route_and_modal_precedence_hold_for_several_cursors() {
    let dir = tempfile::tempdir().unwrap(); let path = dir.path().join("modal.txt"); std::fs::write(&path, "one\ntwo\n").unwrap();
    let mut a = app(dir.path()); a.dispatch(Command::OpenRetained(path)); settle(&mut a, |a| a.session.state().current_source().is_some()); a.enable_draft_input_for_test(); draw(&mut a, 80, 16);
    let text = |a: &App| { let source = a.session.state().current_source().unwrap(); source.copy_range(0..source.len_bytes()).unwrap() };
    key(&mut a, KeyCode::Right, KeyModifiers::NONE);
    key(&mut a, KeyCode::F(10), KeyModifiers::NONE);
    let at = a.choices().iter().position(|choice| matches!(choice.target, crate::app::ChoiceTarget::Action(CommandId::AddCursorBelow))).unwrap();
    for _ in 0..at { key(&mut a, KeyCode::Down, KeyModifiers::NONE); }
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(a.session.state().active().unwrap().selections().count(), 2, "the palette reaches the command too");
    a.run(CommandId::FindAction);
    assert!(a.modal.is_some());
    key(&mut a, KeyCode::Down, KeyModifiers::ALT | KeyModifiers::SHIFT);
    key(&mut a, KeyCode::Char('x'), KeyModifiers::NONE);
    assert_eq!((a.session.state().active().unwrap().selections().count(), text(&a)), (2, "one\ntwo\n".to_owned()), "the open dialog takes the keys first");
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    assert!(a.modal.is_none());
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    assert_eq!(a.session.state().active().unwrap().selections().count(), 1, "then Esc drops the extra cursor");
}

#[test]
fn terminal_draws_a_caret_for_every_selection() {
    let dir = tempfile::tempdir().unwrap(); let path = dir.path().join("carets.txt"); std::fs::write(&path, "aa bb aa\none\n").unwrap();
    let mut a = app(dir.path()); a.dispatch(Command::OpenRetained(path)); settle(&mut a, |a| a.session.state().current_source().is_some()); a.enable_draft_input_for_test(); draw(&mut a, 80, 16);
    key(&mut a, KeyCode::Right, KeyModifiers::NONE);
    a.run(CommandId::AddCursorBelow);
    assert_eq!(a.session.state().active().unwrap().selections().count(), 2);
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 16)).unwrap();
    terminal.draw(|f| a.draw(f)).unwrap();
    let buffer = terminal.backend().buffer();
    let carets = buffer.content.iter().filter(|cell| cell.modifier.contains(ratatui::style::Modifier::REVERSED)).count();
    assert!(carets >= 2, "each caret is drawn: {carets}");
    a.run(CommandId::SelectAll);
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 16)).unwrap();
    terminal.draw(|f| a.draw(f)).unwrap();
    let buffer = terminal.backend().buffer();
    let highlighted = buffer.content.iter().filter(|cell| cell.bg == crate::render::color(a.session.state().palette().sel_bg)).count();
    assert!(highlighted >= 4, "the selected text is shown: {highlighted}");
}

#[test]
fn structural_keys_and_the_palette_indent_comment_move_and_duplicate_in_the_terminal() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("edit.rs");
    let original = "one\r\ntwo\r\nthree\r\n";
    std::fs::write(&path, original).unwrap();
    let mut a = app(dir.path());
    a.dispatch(Command::OpenRetained(path.clone()));
    settle(&mut a, |a| a.session.state().current_source().is_some());
    a.enable_draft_input_for_test();
    draw(&mut a, 80, 16);
    let text = |a: &App| { let source = a.session.state().current_source().unwrap(); source.copy_range(0..source.len_bytes()).unwrap() };
    // Tab inserts the file's indentation step; Shift+Tab (BackTab, or Tab with Shift) takes it back.
    key(&mut a, KeyCode::Tab, KeyModifiers::NONE);
    assert_eq!(text(&a), "    one\r\ntwo\r\nthree\r\n");
    key(&mut a, KeyCode::Tab, KeyModifiers::NONE);
    assert_eq!(text(&a), "        one\r\ntwo\r\nthree\r\n", "a second Tab adds a second step");
    key(&mut a, KeyCode::BackTab, KeyModifiers::SHIFT);
    key(&mut a, KeyCode::Tab, KeyModifiers::SHIFT);
    assert_eq!(text(&a), "one\r\ntwo\r\nthree\r\n", "Shift+Tab removes one step");
    // The preset's chords move, duplicate and comment; each is one undo step.
    key(&mut a, KeyCode::Down, KeyModifiers::CONTROL | KeyModifiers::SHIFT);
    assert_eq!(text(&a), "two\r\none\r\nthree\r\n");
    key(&mut a, KeyCode::Down, KeyModifiers::ALT | KeyModifiers::SHIFT);
    assert_eq!(text(&a), "two\r\none\r\none\r\nthree\r\n");
    key(&mut a, KeyCode::Char('z'), KeyModifiers::CONTROL);
    key(&mut a, KeyCode::Char('z'), KeyModifiers::CONTROL);
    assert_eq!(text(&a), "one\r\ntwo\r\nthree\r\n");
    // Ctrl+/ is often Ctrl+_ in a terminal, so Alt+/ is the route that always arrives.
    a.dispatch(Command::GoToLine(lenz_content::goto::LineColumn { line: 1, column: Some(1) }));
    key(&mut a, KeyCode::Char('/'), KeyModifiers::ALT);
    assert_eq!(text(&a), "// one\r\ntwo\r\nthree\r\n");
    key(&mut a, KeyCode::Char('/'), KeyModifiers::CONTROL);
    assert_eq!(text(&a), "one\r\ntwo\r\nthree\r\n", "Ctrl+/ is the same toggle");
    // Every action is in the palette with its label, and one of them runs from it.
    key(&mut a, KeyCode::F(10), KeyModifiers::NONE);
    for id in [CommandId::IndentSelection, CommandId::OutdentSelection, CommandId::ToggleComment, CommandId::MoveLinesUp, CommandId::MoveLinesDown, CommandId::DuplicateLinesUp, CommandId::DuplicateLinesDown] {
        let at = a.choices().iter().position(|choice| matches!(choice.target, crate::app::ChoiceTarget::Action(chosen) if chosen == id));
        assert!(at.is_some(), "{id:?} is in the palette ({})", id.label());
        let expected = if id == CommandId::MoveLinesUp { Some("Already on the first line") } else { None };
        assert_eq!(a.session.disabled(id), expected, "{id:?} availability");
    }
    let at = a.choices().iter().position(|choice| matches!(choice.target, crate::app::ChoiceTarget::Action(CommandId::DuplicateLinesDown))).unwrap();
    for _ in 0..at { key(&mut a, KeyCode::Down, KeyModifiers::NONE); }
    key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(text(&a), "one\r\none\r\ntwo\r\nthree\r\n", "the palette duplicates the caret's line");
    key(&mut a, KeyCode::Char('z'), KeyModifiers::CONTROL);
    assert_eq!(text(&a), "one\r\ntwo\r\nthree\r\n");
    // An open dialog takes the keys first and leaves the draft alone.
    key(&mut a, KeyCode::F(10), KeyModifiers::NONE);
    assert!(a.modal.is_some());
    let before = text(&a);
    for (code, modifiers) in [
        (KeyCode::Tab, KeyModifiers::NONE),
        (KeyCode::Down, KeyModifiers::CONTROL | KeyModifiers::SHIFT),
        (KeyCode::Down, KeyModifiers::ALT | KeyModifiers::SHIFT),
        (KeyCode::Char('/'), KeyModifiers::ALT),
    ] {
        key(&mut a, code, modifiers);
    }
    assert_eq!(text(&a), before, "the command menu keeps every structural key away from the text");
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    // Read-only mode refuses them with the shared reason, and the text stays as it is.
    a.dispatch(Command::SetReadOnly(true));
    let frozen = text(&a);
    key(&mut a, KeyCode::Tab, KeyModifiers::NONE);
    key(&mut a, KeyCode::Down, KeyModifiers::CONTROL | KeyModifiers::SHIFT);
    key(&mut a, KeyCode::Char('/'), KeyModifiers::ALT);
    assert_eq!(text(&a), frozen);
    for id in [CommandId::IndentSelection, CommandId::ToggleComment, CommandId::MoveLinesDown, CommandId::DuplicateLinesDown] {
        assert_eq!(a.session.disabled(id), Some("Read-only mode is on"), "{id:?}");
    }
    assert_eq!(std::fs::read(&path).unwrap(), original.as_bytes(), "the terminal never writes the file");
    // An empty file: indenting writes the step, and the commands with nothing to do say why and change nothing.
    let empty = dir.path().join("empty.rs");
    std::fs::write(&empty, b"").unwrap();
    a.dispatch(Command::SetReadOnly(false));
    a.dispatch(Command::OpenRetained(empty));
    settle(&mut a, |a| a.session.state().current_source().is_some_and(|d| d.source().ends_with("empty.rs")));
    draw(&mut a, 80, 16);
    a.focus = Focus::Text;
    // The empty document says what it cannot do before the first step writes anything.
    for (id, reason) in [(CommandId::ToggleComment, "There is no text to comment"), (CommandId::MoveLinesDown, "Already on the last line"), (CommandId::MoveLinesDown, "Already on the last line"), (CommandId::DuplicateLinesDown, "Nothing to duplicate")] {
        assert_eq!(a.session.disabled(id), Some(reason), "{id:?}");
    }
    key(&mut a, KeyCode::Char('/'), KeyModifiers::ALT);
    key(&mut a, KeyCode::Down, KeyModifiers::CONTROL | KeyModifiers::SHIFT);
    key(&mut a, KeyCode::Down, KeyModifiers::ALT | KeyModifiers::SHIFT);
    assert_eq!(text(&a), "", "an empty document has nothing to comment, move or duplicate");
    key(&mut a, KeyCode::Tab, KeyModifiers::NONE);
    assert_eq!(text(&a), "    ", "indenting an empty document writes the file's step");
    key(&mut a, KeyCode::BackTab, KeyModifiers::SHIFT);
    assert_eq!(text(&a), "");
}

#[test]
fn home_and_end_move_the_editing_caret_and_shift_extends_without_writing() {
    let dir=tempfile::tempdir().unwrap();let path=dir.path().join("keys.txt");let text="α😀 first\r\n  λ second\nlast";
    std::fs::write(&path,text).unwrap();let mut a=app(dir.path());a.dispatch(Command::OpenRetained(path.clone()));settle(&mut a,|a|a.session.state().current_source().is_some());a.enable_draft_input_for_test();
    let start=text.find("  λ").unwrap();let end=text.find("\nlast").unwrap();
    a.dispatch(Command::Select(Selection{anchor:start+4,head:start+4}));
    key(&mut a,KeyCode::Home,KeyModifiers::NONE);assert_eq!(a.session.state().selection(),Some(Selection{anchor:start,head:start}));
    key(&mut a,KeyCode::End,KeyModifiers::SHIFT);assert_eq!(a.session.state().selection(),Some(Selection{anchor:start,head:end}));
    key(&mut a,KeyCode::Home,KeyModifiers::CONTROL);assert_eq!(a.session.state().selection(),Some(Selection{anchor:0,head:0}));
    key(&mut a,KeyCode::End,KeyModifiers::CONTROL|KeyModifiers::SHIFT);assert_eq!(a.session.state().selection(),Some(Selection{anchor:0,head:text.len()}));
    assert_eq!(copied_selection(a.session.state()).as_deref(),Some(text));assert_eq!(std::fs::read_to_string(path).unwrap(),text);
}

#[test]
fn completion_documentation_scroll_reaches_the_end_without_editing() {
    let dir = tempfile::tempdir().unwrap(); let root = dir.path().join("project");
    std::fs::create_dir_all(&root).unwrap(); std::fs::write(root.join("Cargo.toml"), "[package]\nname='docs'\nversion='0.1.0'\n").unwrap();
    let text = "fn main() { hel }\nfn hello() {}\nfn helper() {}\n"; let path=root.join("main.rs"); std::fs::write(&path,text).unwrap();
    let profile=std::env::current_exe().unwrap().parent().unwrap().parent().unwrap().to_owned();
    let mut build=std::process::Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    build.args(["build","-q","-p","code-core","--example","fake_lsp"]); if profile.ends_with("release") { build.arg("--release"); } assert!(build.status().unwrap().success());
    let mut cfg=config(dir.path()); cfg.idle=code_core::semantics::IDLE;
    std::fs::create_dir_all(cfg.providers.parent().unwrap()).unwrap(); std::fs::write(&cfg.providers,serde_json::json!({"providers":{"rust-analyzer":{"executable":profile.join("examples/fake_lsp"),"env":{"FAKE_LSP_COMPLETION_DOC":"1","FAKE_LSP_NO_DECOR":"1"}}}}).to_string()).unwrap();
    let mut a=App::new(cfg,Some(root)); a.enable_draft_input_for_test(); a.dispatch(Command::Open(path.clone())); settle(&mut a,|a|a.session.state().provider().is_some_and(|p|p.status==code_core::semantics::Status::Ready));
    let at=text.find("hel }").unwrap()+3; a.dispatch(Command::Select(Selection{anchor:at,head:at})); a.dispatch(Command::Complete);
    settle(&mut a,|a|a.session.state().completion().is_some_and(|c|c.documentation_preview().is_some())); draw(&mut a,100,30);
    assert!(a.layout.completion_doc.width>0,"documentation drawn: {}",draw_screen(&mut a,100,30));
    let selection=a.session.state().selection();
    for _ in 0..200 { let r=a.layout.completion_doc; a.handle(Event::Mouse(MouseEvent{kind:MouseEventKind::ScrollDown,column:r.x,row:r.y,modifiers:KeyModifiers::NONE})); draw(&mut a,100,30); }
    let shown=draw_screen(&mut a,100,30); assert!(shown.contains("FINAL DOCUMENTATION LINE"),"all documentation must be reachable: {shown}");
    assert_eq!(a.session.state().selection(),selection);
    for theme in ["darcula","github-light"] {
        a.dispatch(Command::Theme(lenz_content::theme::ThemeCommand::Select(lenz_content::theme::ThemeSelection::from_id(theme).unwrap())));
        for (w,h) in [(100,30),(30,12),(16,8)] {
            key(&mut a,KeyCode::Home,KeyModifiers::ALT); draw(&mut a,w,h); assert_eq!(a.layout.completion_doc_top,0);
            assert!(a.layout.completion_doc.right()<=w && a.layout.completion_doc.bottom()<=h);
            key(&mut a,KeyCode::PageDown,KeyModifiers::ALT); draw(&mut a,w,h); assert!(a.layout.completion_doc_top>0);
            key(&mut a,KeyCode::End,KeyModifiers::ALT); let shown=draw_screen(&mut a,w,h);
            assert_eq!(a.layout.completion_doc_top,a.layout.completion_doc_rows.saturating_sub(a.layout.completion_doc.height as usize));
            assert!(shown.contains("LINE"),"last doc row is painted in {w}x{h}: {shown}");
            let r=a.layout.completion_doc; click(&mut a,r.x,r.y); assert!(a.session.state().completion().is_some()); assert_eq!(a.session.state().selection(),selection);
        }
    }
    draw(&mut a,100,30); key(&mut a,KeyCode::Home,KeyModifiers::ALT);
    let mut terminal=ratatui::Terminal::new(ratatui::backend::TestBackend::new(100,30)).unwrap(); terminal.draw(|frame|a.draw(frame)).unwrap();
    let popup=a.layout.completion; let cell=&terminal.backend().buffer()[(popup.x+1,popup.y)];
    assert!(cell.modifier.contains(ratatui::style::Modifier::BOLD|ratatui::style::Modifier::UNDERLINED),"matched text is emphasized");
    assert_eq!(terminal.backend().buffer()[(popup.x,popup.y)].symbol(),"›");
    assert_eq!(std::fs::read_to_string(path).unwrap(),text);
}

#[test]
fn typing_keeps_unchanged_source_colours_before_the_new_parse_arrives() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("colours.rs");
    let original = b"fn main() { let text = \"steady\"; }\n";
    std::fs::write(&path, original).unwrap();
    let mut a = App::new(config(dir.path()), Some(path.clone()));
    a.enable_draft_input_for_test();
    settle(&mut a, |a| a.session.state().highlights().is_some());
    draw(&mut a, 100, 20);
    key(&mut a, KeyCode::End, KeyModifiers::CONTROL);
    key(&mut a, KeyCode::Char('x'), KeyModifiers::NONE);
    assert!(a.session.state().highlights().is_none(), "new parse has not been applied");
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 20)).unwrap();
    terminal.draw(|frame| a.draw(frame)).unwrap();
    let hit = a.layout.panes[0].hits.iter().find(|hit| hit.line == 0 && hit.byte == 0).unwrap();
    let keyword = a.session.state().palette().syntax_color(lenz_content::highlight::Capture::Keyword);
    let expected = ratatui::style::Color::Rgb((keyword >> 16) as u8, (keyword >> 8) as u8, keyword as u8);
    assert_eq!(terminal.backend().buffer()[(hit.x, hit.y)].fg, expected, "the unchanged fn keeps its painted keyword colour");
    assert_eq!(std::fs::read(path).unwrap(), original);
}

#[test]
fn java_member_semantic_colour_stays_painted_during_typing_and_lexical_arrival() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Main.java");
    let source = "public static void main(String[] args) {\n    System.out.println(\"Hello Lenz!\");\n    \n}\n";
    std::fs::write(&path, source).unwrap();
    let profile = std::env::current_exe().unwrap().parent().unwrap().parent().unwrap().to_owned();
    let mut build = std::process::Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    build.args(["build", "-q", "-p", "code-core", "--example", "fake_lsp"]);
    if profile.ends_with("release") { build.arg("--release"); }
    assert!(build.status().unwrap().success());
    let cfg = config(dir.path());
    let gate = dir.path().join("token-gate");
    std::fs::create_dir_all(cfg.providers.parent().unwrap()).unwrap();
    let mut providers = serde_json::Map::new();
    for p in code_core::semantics::provider::PROVIDERS.iter() {
        providers.insert(p.id.into(), serde_json::json!({"executable":format!("/nonexistent/{}", p.id)}));
    }
    providers.insert("jdtls".into(), serde_json::json!({"executable":profile.join("examples/fake_lsp"),"env":{"FAKE_LSP_TOKEN_GATE":gate}}));
    std::fs::write(&cfg.providers, serde_json::json!({"providers":providers}).to_string()).unwrap();
    let mut a = App::new(cfg, Some(path.clone()));
    a.enable_draft_input_for_test();
    settle(&mut a, |a| a.session.state().active().and_then(|tab| a.session.state().view(tab.id())).is_some_and(|view| view.tokens().is_some() && view.highlights().is_some()));
    let colour = |a: &mut App| {
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 20)).unwrap();
        terminal.draw(|frame| a.draw(frame)).unwrap();
        let hit = a.layout.panes[0].hits.iter().find(|hit| hit.line == 1 && hit.byte == 11).unwrap();
        terminal.backend().buffer()[(hit.x, hit.y)].fg
    };
    for theme in ["darcula", "github-light"] {
        a.dispatch(Command::Theme(lenz_content::theme::ThemeCommand::Select(lenz_content::theme::ThemeSelection::from_id(theme).unwrap())));
        let expected = crate::render::color(a.session.state().palette().syntax_color(lenz_content::highlight::Capture::Property));
        assert_eq!(colour(&mut a), expected);
        std::fs::write(&gate, b"hold fresh semantic responses").unwrap();
        let at = a.session.state().current_source().unwrap().line_range(2).unwrap().start + 4;
        a.dispatch(Command::Select(Selection { anchor: at, head: at }));
        key(&mut a, KeyCode::Char('x'), KeyModifiers::NONE);
        assert_eq!(colour(&mut a), expected, "typing must keep out's actual terminal colour");
        settle(&mut a, |a| a.session.state().highlights().is_some());
        assert_eq!(colour(&mut a), expected, "fresh syntax must keep out's semantic colour while tokens wait");
        std::fs::remove_file(&gate).unwrap();
        settle(&mut a, |a| a.session.state().active().and_then(|tab| a.session.state().view(tab.id())).is_some_and(|view| view.tokens().is_some()));
    }
    assert_eq!(std::fs::read(path).unwrap(), source.as_bytes());
}
#[path = "input_tests/xml.rs"]
mod xml;

#[test]
fn results_panel_hides_on_escape_and_shows_again_with_its_selection() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("proj");
    std::fs::create_dir_all(root.join("src")).unwrap();
    for name in ["src/a.rs", "src/b.rs"] { std::fs::write(root.join(name), "needle\n").unwrap(); }
    let mut a = app(dir.path());
    a.dispatch(Command::OpenRoot(root.clone()));
    settle(&mut a, |a| a.session.state().project().is_some_and(|p| matches!(p.coverage(), code_core::project::Coverage::Complete(_))));
    a.dispatch(Command::Search { query: "needle".into(), scope: SearchIn::Project });
    settle(&mut a, |a| a.session.state().results().is_some_and(|r| r.done() && r.hits().len() == 2));
    draw(&mut a, 100, 30);
    a.focus = Focus::Results;
    a.dispatch(Command::SelectResult(1));
    // Escape hides the panel and gives the keys back to the text; the results stay.
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    draw(&mut a, 100, 30);
    assert!(a.session.state().results().is_none() && a.focus == Focus::Text && a.layout.results.is_empty());
    assert!(a.session.disabled(CommandId::ToggleResults).is_none());
    // The status bar says they are waiting; a click there shows them again.
    let (_, waiting) = *a.layout.controls.iter().find(|(id, _)| *id == CommandId::ToggleResults).expect("the status bar offers the hidden results");
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
    terminal.draw(|frame| a.draw(frame)).unwrap();
    let row: String = (waiting.x..waiting.right()).map(|x| terminal.backend().buffer()[(x, waiting.y)].symbol().to_owned()).collect();
    assert_eq!(row.trim_end(), "Results (2)");
    mouse(&mut a, MouseButton::Left, waiting.x, waiting.y);
    draw(&mut a, 100, 30);
    assert!(a.session.state().results().is_some() && !a.layout.controls.iter().any(|(id, r)| *id == CommandId::ToggleResults && r.y == waiting.y));
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    // Ctrl+J shows them again with their selection, and they take the keys.
    key(&mut a, KeyCode::Char('j'), KeyModifiers::CONTROL);
    draw(&mut a, 100, 30);
    assert_eq!(a.session.state().results().map(|r| (r.hits().len(), r.selected())), Some((2, Some(1))));
    assert!(a.focus == Focus::Results && !a.layout.results.is_empty());
    for name in ["src/a.rs", "src/b.rs"] { assert_eq!(std::fs::read(root.join(name)).unwrap(), b"needle\n"); }
}

#[test]
fn escape_stops_a_running_search_before_it_hides_the_panel() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("proj");
    std::fs::create_dir_all(&root).unwrap();
    for i in 0..40 { std::fs::write(root.join(format!("f{i:02}.txt")), "needle\n".repeat(50)).unwrap(); }
    let mut a = app(dir.path());
    a.dispatch(Command::OpenRoot(root.clone()));
    settle(&mut a, |a| a.session.state().project().is_some_and(|p| matches!(p.coverage(), code_core::project::Coverage::Complete(_))));
    a.dispatch(Command::Search { query: "needle".into(), scope: SearchIn::Project });
    a.focus = Focus::Results;
    assert!(a.session.disabled(CommandId::StopSearch).is_none());
    // The first Escape stops the search and keeps its panel; the second hides it.
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    assert!(a.session.state().results().is_some_and(|r| r.done()) && a.session.disabled(CommandId::StopSearch).is_some());
    assert!(a.session.results_header().unwrap().detail.contains("stopped after"));
    key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    assert!(a.session.state().results().is_none() && a.focus == Focus::Text);
}

#[test]
fn long_result_lines_show_their_match_and_a_cut_heading_ends_in_an_ellipsis() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("proj");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("a.rs"), format!("    {}needle tail\n", "x".repeat(150))).unwrap();
    let mut a = app(dir.path());
    a.dispatch(Command::OpenRoot(root.clone()));
    settle(&mut a, |a| a.session.state().project().is_some_and(|p| matches!(p.coverage(), code_core::project::Coverage::Complete(_))));
    a.dispatch(Command::Search { query: "needle".into(), scope: SearchIn::Project });
    settle(&mut a, |a| a.session.state().results().is_some_and(|r| r.done() && r.hits().len() == 1));
    a.dispatch(Command::CloseFind);
    let screen = draw_screen(&mut a, 60, 30);
    assert!(screen.contains("1  …xxxxxxxxxxxneedle tail"), "the row starts shortly before its match: {screen}");
    assert!(screen.contains("“needle”  1 mat…  Rerun"), "a cut detail ends in … before the buttons: {screen}");
}

#[test]
fn a_short_dialog_fits_its_text_and_wraps_between_words() {
    let dir = tempfile::tempdir().unwrap();
    let mut a = app(dir.path());
    let body = "Restore Last Deleted brings back the entry moved to the trash in this session. Undo is not offered.".to_string();
    a.prompt(Prompt::Details { title: "Trash", body: body.clone(), logo: false, kind: crate::app::Details::Text });
    let screen = draw_screen(&mut a, 120, 35);
    let modal = a.layout.modal;
    assert!(modal.height < 12 && modal.width < 110, "not the full screen: {modal:?}");
    assert!(screen.contains("Restore Last Deleted brings back"), "{screen}");
    draw(&mut a, 50, 20);
    let words: Vec<_> = crate::render::wrap_text(&body, a.layout.modal_rows.width as usize);
    assert!(words.len() > 1 && words.iter().all(|row| body.split(' ').any(|w| row.ends_with(w))), "rows end at words: {words:?}");
}

#[test]
fn the_find_box_is_small_and_leaves_the_matches_in_view() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.txt");
    std::fs::write(&path, (0..30).map(|i| format!("line {i} needle\n")).collect::<String>()).unwrap();
    let mut a = app(dir.path());
    a.dispatch(Command::Open(path.clone()));
    settle(&mut a, |a| a.session.state().current_source().is_some());
    key(&mut a, KeyCode::Char('f'), KeyModifiers::CONTROL);
    a.handle(Event::Paste("needle".into()));
    let screen = draw_screen(&mut a, 120, 30);
    assert!(a.layout.modal.height <= 5 && a.layout.modal.width <= 64, "{:?}", a.layout.modal);
    assert!(screen.contains("line 10 needle") && screen.contains("Enter next"), "{screen}");
    key(&mut a, KeyCode::Tab, KeyModifiers::NONE);
    assert!(draw_screen(&mut a, 120, 30).contains("Enter replaces"));
    for (w, h) in [(30, 8), (8, 4), (1, 1)] { draw(&mut a, w, h); }
}

#[test]
fn results_heading_buttons_collapse_expand_and_close() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("proj");
    std::fs::create_dir_all(root.join("src")).unwrap();
    for name in ["src/a.rs", "src/b.rs"] { std::fs::write(root.join(name), "needle\n").unwrap(); }
    let mut a = app(dir.path());
    a.dispatch(Command::OpenRoot(root.clone()));
    settle(&mut a, |a| a.session.state().project().is_some_and(|p| matches!(p.coverage(), code_core::project::Coverage::Complete(_))));
    a.dispatch(Command::Search { query: "needle".into(), scope: SearchIn::Project });
    settle(&mut a, |a| a.session.state().results().is_some_and(|r| r.done() && r.hits().len() == 2));
    let press = |a: &mut App, id: CommandId| {
        draw(a, 120, 30);
        let (_, rect) = *a.layout.controls.iter().find(|(c, r)| *c == id && r.y < a.layout.results.y).expect("a heading button");
        mouse(a, MouseButton::Left, rect.x + 1, rect.y);
    };
    press(&mut a, CommandId::FoldAllResults);
    assert_eq!(a.session.state().results().unwrap().rows().len(), 1, "only the folder row");
    press(&mut a, CommandId::FoldAllResults);
    assert_eq!(a.session.state().results().unwrap().rows().len(), 5);
    press(&mut a, CommandId::CloseResults);
    assert!(a.session.disabled(CommandId::ToggleResults).is_some(), "closed, not hidden");
}

#[test]
fn f4_steps_through_results_and_earlier_lists_come_back_by_keys_and_buttons() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("proj");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("a.rs"), "needle\nother\n").unwrap();
    std::fs::write(root.join("src/b.rs"), "x\nneedle\n").unwrap();
    let mut a = app(dir.path());
    a.dispatch(Command::OpenRoot(root.clone()));
    settle(&mut a, |a| a.session.state().project().is_some_and(|p| matches!(p.coverage(), code_core::project::Coverage::Complete(_))));
    let search = |a: &mut App, q: &str| {
        a.dispatch(Command::Search { query: q.into(), scope: SearchIn::Project });
        settle(a, |a| a.session.state().results().is_some_and(|r| r.done() && r.query() == q && r.preview().is_some()));
    };
    let active = |a: &App| a.session.state().active().map(|t| t.title().to_owned());
    search(&mut a, "needle");
    a.focus = Focus::Text;
    a.dispatch(Command::SelectResult(0));
    // F4 from the text opens the next result there, Shift+F4 the one before (src/b.rs lists first, as in the
    // explorer).
    key(&mut a, KeyCode::F(4), KeyModifiers::NONE);
    settle(&mut a, |a| a.session.state().active().is_some_and(|t| t.title() == "a.rs"));
    key(&mut a, KeyCode::F(4), KeyModifiers::SHIFT);
    settle(&mut a, |a| a.session.state().active().is_some_and(|t| t.title() == "b.rs"));
    assert_eq!(active(&a).as_deref(), Some("b.rs"));
    search(&mut a, "other");
    draw(&mut a, 120, 30);
    // Alt+Left in the panel shows the earlier list with its selection on its row; the › button the later one.
    a.focus = Focus::Results;
    key(&mut a, KeyCode::Left, KeyModifiers::ALT);
    let r = a.session.state().results().unwrap();
    assert_eq!((r.query(), r.selected()), ("needle", Some(0)));
    assert_eq!(Some(a.results_at), r.row_of_hit(0));
    draw(&mut a, 120, 30);
    let (_, next) = *a.layout.controls.iter().find(|(id, r)| *id == CommandId::NextResults && r.y < a.layout.results.y).unwrap();
    mouse(&mut a, MouseButton::Left, next.x + 1, next.y);
    assert_eq!(a.session.state().results().unwrap().query(), "other");
    key(&mut a, KeyCode::Left, KeyModifiers::ALT);
    key(&mut a, KeyCode::Right, KeyModifiers::ALT);
    assert_eq!(a.session.state().results().unwrap().query(), "other");
    assert_eq!(std::fs::read(root.join("a.rs")).unwrap(), b"needle\nother\n");
}

#[test]
fn results_tree_folds_by_keys_and_clicks_as_in_the_desktop() {
    use code_core::search::Row;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("proj");
    std::fs::create_dir_all(root.join("src")).unwrap();
    for name in ["src/a.rs", "src/b.rs"] { std::fs::write(root.join(name), "needle\n").unwrap(); }
    let mut a = app(dir.path());
    a.dispatch(Command::OpenRoot(root.clone()));
    settle(&mut a, |a| a.session.state().project().is_some_and(|p| matches!(p.coverage(), code_core::project::Coverage::Complete(_))));
    a.dispatch(Command::Search { query: "needle".into(), scope: SearchIn::Project });
    settle(&mut a, |a| a.session.state().results().is_some_and(|r| r.done() && r.hits().len() == 2));
    draw(&mut a, 100, 30);
    a.focus = Focus::Results;
    let rows = |a: &App| a.session.state().results().unwrap().rows().len();
    assert_eq!(rows(&a), 5, "src/, a.rs, its line, b.rs, its line");
    // On a folder: Left folds, Left again does nothing, Right unfolds, Right again does nothing.
    a.results_at = 0;
    key(&mut a, KeyCode::Left, KeyModifiers::NONE);
    assert_eq!(rows(&a), 1);
    key(&mut a, KeyCode::Left, KeyModifiers::NONE);
    assert_eq!(rows(&a), 1);
    key(&mut a, KeyCode::Right, KeyModifiers::NONE);
    key(&mut a, KeyCode::Right, KeyModifiers::NONE);
    assert_eq!(rows(&a), 5);
    // On a match: Left folds its file and the highlight moves to that file's row.
    a.results_at = 4;
    key(&mut a, KeyCode::Left, KeyModifiers::NONE);
    assert!(matches!(a.session.state().results().unwrap().rows().get(a.results_at), Some(Row::File { open: false, .. })));
    assert_eq!(rows(&a), 4);
    // On a folded file: Left steps out to its folder's row, then folds it.
    key(&mut a, KeyCode::Left, KeyModifiers::NONE);
    assert_eq!((a.results_at, rows(&a)), (0, 4));
    key(&mut a, KeyCode::Left, KeyModifiers::NONE);
    assert_eq!(rows(&a), 1);
    key(&mut a, KeyCode::Right, KeyModifiers::NONE);
    // A click on a folder row folds it; another unfolds it.
    draw(&mut a, 100, 30);
    let list = a.layout.results;
    click(&mut a, list.x + 3, list.y);
    assert_eq!(rows(&a), 1);
    click(&mut a, list.x + 3, list.y);
    assert_eq!(rows(&a), 4);
    for name in ["src/a.rs", "src/b.rs"] { assert_eq!(std::fs::read(root.join(name)).unwrap(), b"needle\n"); }
}

#[test]
fn keyboard_shortcuts_returns_to_the_menu_root_by_left_escape_and_its_back_row() {
    use code_core::chrome::MenuTarget;
    let dir = tempfile::tempdir().unwrap();
    let mut a = app(dir.path());
    draw(&mut a, 100, 30);
    let shortcuts = a.session.menu(None).iter().position(|row| row.target == MenuTarget::Group(CommandGroup::Keybindings)).unwrap();
    for leave in [KeyCode::Left, KeyCode::Esc, KeyCode::Enter] {
        a.open_menu(None, shortcuts);
        key(&mut a, KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(a.modal.as_ref().map(|m| m.kind.clone()), Some(Prompt::Menu(Some(CommandGroup::Keybindings))));
        // The Back row is the group's first row; Enter there goes back like Left and Escape.
        if leave == KeyCode::Enter { a.modal.as_mut().unwrap().at = 0; }
        key(&mut a, leave, KeyModifiers::NONE);
        assert_eq!(a.modal.as_ref().map(|m| (m.kind.clone(), m.at)), Some((Prompt::Menu(None), shortcuts)), "{leave:?}");
        key(&mut a, KeyCode::Esc, KeyModifiers::NONE);
    }
}

#[test]
fn results_heading_stop_rerun_hide_and_back_buttons_run() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("proj");
    std::fs::create_dir_all(&root).unwrap();
    for i in 0..40 { std::fs::write(root.join(format!("f{i:02}.txt")), "needle\n".repeat(50)).unwrap(); }
    let mut a = app(dir.path());
    a.dispatch(Command::OpenRoot(root.clone()));
    settle(&mut a, |a| a.session.state().project().is_some_and(|p| matches!(p.coverage(), code_core::project::Coverage::Complete(_))));
    let press = |a: &mut App, id: CommandId| {
        draw(a, 120, 30);
        let (_, rect) = *a.layout.controls.iter().find(|(c, r)| *c == id && r.y < a.layout.results.y).expect("a heading button");
        mouse(a, MouseButton::Left, rect.x + 1, rect.y);
    };
    // Stop while the search runs keeps what it found.
    a.dispatch(Command::Search { query: "needle".into(), scope: SearchIn::Project });
    press(&mut a, CommandId::StopSearch);
    assert!(a.session.results_header().unwrap().detail.contains("stopped after"));
    // Rerun asks again and finishes.
    press(&mut a, CommandId::RerunSearch);
    settle(&mut a, |a| a.session.state().results().is_some_and(|r| r.done() && r.hits().len() == 2000));
    // Another search, then ‹ goes back to this one; Hide leaves "Results (N)" in the status bar.
    a.dispatch(Command::Search { query: "absent".into(), scope: SearchIn::Project });
    settle(&mut a, |a| a.session.state().results().is_some_and(|r| r.done() && r.query() == "absent"));
    a.dispatch(Command::Search { query: "needle".into(), scope: SearchIn::Project });
    settle(&mut a, |a| a.session.state().results().is_some_and(|r| r.done() && r.query() == "needle"));
    a.dispatch(Command::Search { query: "need".into(), scope: SearchIn::Project });
    settle(&mut a, |a| a.session.state().results().is_some_and(|r| r.done() && r.query() == "need"));
    press(&mut a, CommandId::PreviousResults);
    assert_eq!(a.session.state().results().unwrap().query(), "needle");
    press(&mut a, CommandId::ToggleResults);
    draw(&mut a, 120, 30);
    assert!(a.session.state().results().is_none() && a.layout.controls.iter().any(|(id, r)| *id == CommandId::ToggleResults && r.y == 29));
}
