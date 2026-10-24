//! Real input through GPUI's test platform: keys, clicks, drags, wheel and resizes reach the session only as
//! commands, and the file on disk never changes.
use super::*;
use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext, WindowHandle, point, size};
use reader_core::session::Config;
use std::path::Path;

struct Fixture {
    dir: tempfile::TempDir,
    view: Entity<ReaderView>,
    window: WindowHandle<ReaderView>,
}

fn copied_selection(state: &reader_core::session::State) -> Option<String> {
    state.view(state.active()?.id())?.copy_selection()
}

fn fixture(cx: &mut TestAppContext, file: Option<(&str, &[u8])>) -> Fixture {
    fixture_with(cx, file, Path::new("/nonexistent/rust-analyzer"))
}

/// A window whose rust-analyzer is `server` (never a real one unless a test says so).
fn fixture_with(cx: &mut TestAppContext, file: Option<(&str, &[u8])>, server: &Path) -> Fixture {
    fixture_providers(cx, file, &[("rust-analyzer", server, None, None)])
}

/// A window whose providers `(id, executable, PATH, settings JSON)` are these; every other provider is pointed
/// nowhere.
fn fixture_providers(cx: &mut TestAppContext, file: Option<(&str, &[u8])>, servers: &[(&str, &Path, Option<&str>, Option<&str>)]) -> Fixture {
    fixture_setup(cx,file,servers,false)
}
fn fixture_setup(cx: &mut TestAppContext,file:Option<(&str,&[u8])>,servers:&[(&str,&Path,Option<&str>,Option<&str>)],production:bool)->Fixture {
    let dir = tempfile::tempdir().unwrap();
    let path = file.map(|(name, bytes)| {
        let p = dir.path().join(name);
        std::fs::write(&p, bytes).unwrap();
        p
    });
    // Never a real language server: the settings name one that does not exist.
    let providers = dir.path().join("config/lenz-reader/providers.json");
    std::fs::create_dir_all(providers.parent().unwrap()).unwrap();
    // Every other provider is pointed nowhere, so no installed server ever runs in a test.
    let mut all = serde_json::Map::new();
    for p in reader_core::semantics::provider::PROVIDERS.iter() {
        let mut entry = serde_json::json!({ "executable": format!("/nonexistent/{}", p.id) });
        if let Some((_, exe, path, settings)) = servers.iter().find(|(id, ..)| *id == p.id) {
            // An empty executable keeps the provider's own (and its fallbacks).
            entry = if exe.as_os_str().is_empty() { serde_json::json!({}) } else { serde_json::json!({ "executable": exe }) };
            if let Some(path) = path {
                entry["env"] = serde_json::json!({ "PATH": path });
            }
            if let Some(settings) = settings {
                entry["settings"] = serde_json::from_str(settings).unwrap();
            }
        }
        all.insert(p.id.into(), entry);
    }
    std::fs::write(&providers, serde_json::json!({ "providers": all }).to_string()).unwrap();
    let config = Config {
        themes: lenz_content::theme::ThemePaths::under(&dir.path().join("config"), &dir.path().join("state"), "lenz-reader"),
        recent: dir.path().join("state/lenz-reader/recent-roots.json"),
        settings: dir.path().join("state/lenz-reader/settings.json"),
        providers: providers.clone(),

        cache: dir.path().join("cache"),
        idle: reader_core::semantics::IDLE,
        clock: std::sync::Arc::new(std::time::Instant::now),
        overrides: Default::default(),
        dark: true,
    };
    let base = dir.path().to_owned();
    let make: std::rc::Rc<dyn Fn(bool) -> Config> = std::rc::Rc::new(move |dark| Config {
        themes: lenz_content::theme::ThemePaths::under(&base.join("config"), &base.join("state"), "lenz-reader"),
        recent: base.join("state/lenz-reader/recent-roots.json"),
        settings: base.join("state/lenz-reader/settings.json"),
        providers: providers.clone(),

        cache: base.join("cache"),
        idle: reader_core::semantics::IDLE,
        clock: std::sync::Arc::new(std::time::Instant::now),
        overrides: Default::default(),
        dark,
    });
    let (view,window)=if production {
        let window=cx.update(|cx|open_reader_window(cx,make,path)).unwrap();
        let view=window.root(cx).unwrap();(view,window)
    } else {
        let view = cx.new(|cx| ReaderView::new(Session::new(config), path, cx).with_config(make));
        let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| view.clone())).unwrap();(view,window)
    };
    Fixture { dir, view, window }
}

/// Run the executor until `done` for up to `seconds` (real language servers), letting worker threads finish.
fn settle_for(cx: &mut TestAppContext, f: &Fixture, seconds: u64, done: &dyn Fn(&ReaderView) -> bool) {
    let until = std::time::Instant::now() + std::time::Duration::from_secs(seconds);
    while !f.view.read_with(cx, |v, _| done(v)) {
        assert!(std::time::Instant::now() < until, "timed out: {}", f.view.read_with(cx, |v, _| v.status()));
        cx.run_until_parked();
        f.view.update(cx, |v, cx| v.poll(cx));
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

/// Run the executor until `done`, letting the reader's worker threads finish.
fn settle(cx: &mut TestAppContext, f: &Fixture, done: impl Fn(&ReaderView) -> bool) {
    for _ in 0..500 {
        cx.run_until_parked();
        f.view.update(cx, |v, cx| v.poll(cx));
        if f.view.read_with(cx, |v, _| done(v)) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    panic!("timed out: {}", f.view.read_with(cx, |v, _| v.status()));
}

fn visual(cx: &mut TestAppContext, f: &Fixture) -> VisualTestContext {
    let mut v = VisualTestContext::from_window(f.window.into(), cx);
    let focus = f.view.read_with(&v, |v, _| v.focus_handle());
    v.update(|w, cx| focus.focus(w, cx));
    v
}

fn draw(v: &mut VisualTestContext) {
    v.update(|w, cx| {
        let _ = w.draw(cx);
    });
}

#[gpui::test]
fn unicode_delete_and_cut_paste_use_shared_transactions(cx: &mut TestAppContext) {
    let original = "é😀\r\nsecond";
    let f = fixture(cx, Some(("typing.txt", original.as_bytes())));
    settle(cx, &f, |view| view.session().state().current_source().is_some());
    let mut v = visual(cx, &f); draw(&mut v);
    let text = |v: &VisualTestContext| f.view.read_with(v, |view, _| { let source = view.session().state().current_source().unwrap(); source.copy_range(0..source.len_bytes()).unwrap() });
    v.simulate_keystrokes("right delete"); assert_eq!(text(&v), original, "ordinary constructors keep writing gated");
    f.view.update(&mut v, |view, _| view.enable_draft_input_for_test());
    v.simulate_keystrokes("delete"); assert_eq!(text(&v), "é\r\nsecond");
    v.simulate_keystrokes("ctrl-z"); assert_eq!(text(&v), original);
    v.simulate_keystrokes("backspace"); assert_eq!(text(&v), "é\r\nsecond");
    v.simulate_keystrokes("ctrl-a ctrl-x"); assert_eq!(text(&v), "");
    assert_eq!(v.read_from_clipboard().unwrap().text().as_deref(), Some("é\r\nsecond"));
    v.simulate_keystrokes("ctrl-v"); assert_eq!(text(&v), "é\r\nsecond");
    v.simulate_keystrokes("ctrl-z"); assert_eq!(text(&v), "");
    v.simulate_keystrokes("ctrl-shift-z"); assert_eq!(text(&v), "é\r\nsecond");
    assert_eq!(std::fs::read(f.dir.path().join("typing.txt")).unwrap(), original.as_bytes());
}

#[gpui::test]
fn draft_refresh_and_reload_use_real_focus_keys_and_preserve_modal_precedence(
    cx: &mut TestAppContext,
) {
    let f = fixture_setup(cx, Some(("refresh.txt", b"saved\r\n")), &[], true);
    settle(cx, &f, |v| v.session.state().current_source().is_some());
    let path = f.dir.path().join("refresh.txt");
    let mut v = visual(cx, &f);
    draw(&mut v);
    f.view
        .update(&mut v, |v, _| v.enable_draft_input_for_test());
    v.write_to_clipboard(ClipboardItem::new_string("draft λ\r\n".into()));
    v.simulate_keystrokes("ctrl-a ctrl-v");
    let position = f.view.read_with(&v, |v, _| v.session.state().selection());
    let ordinary = f
        .view
        .read_with(&v, |v, _| v.session.state().active().unwrap().id());
    v.simulate_keystrokes("ctrl-shift-d");
    settle(cx, &f, |v| {
        v.session
            .state()
            .saved_comparison()
            .is_some_and(|c| !c.pending())
    });
    draw(&mut v);
    v.simulate_keystrokes("f5");
    assert!(
        f.view
            .read_with(&v, |v, _| v.session.close_prompt().is_some()
                && v.session.state().saved_comparison().is_none()
                && v.session.state().active().unwrap().id() == ordinary)
    );
    v.simulate_keystrokes("escape");
    assert_eq!(
        f.view.read_with(&v, |v, _| v.session.state().selection()),
        position
    );
    v.deactivate_window();
    std::fs::write(&path, b"external\r\n").unwrap();
    v.update(|w, _| w.activate_window());
    settle(cx, &f, |v| {
        v.session.state().active().unwrap().stale_on_disk()
    });
    draw(&mut v);
    assert!(
        f.view
            .read_with(&v, |v, _| v.status().contains("changed on disk"))
    );
    assert_eq!(
        f.view.read_with(&v, |v, _| v.session.state().selection()),
        position
    );
    v.simulate_keystrokes("f5");
    assert!(
        f.view
            .read_with(&v, |v, _| v.session.close_prompt().is_some())
    );
    draw(&mut v);
    assert!(f.view.read_with(&v, |v, _| {
        v.modal_rows()
            .iter()
            .any(|row| row.0.contains("Choose Save"))
    }));
    v.simulate_keystrokes("ctrl-s ctrl-v end up escape");
    assert_eq!(std::fs::read(&path).unwrap(), b"external\r\n");
    assert_eq!(
        f.view.read_with(&v, |v, _| v.session.state().selection()),
        position
    );
    assert!(f.view.read_with(&v, |v, _| {
        let s = v.session.state().current_source().unwrap();
        s.copy_range(0..s.len_bytes()).as_deref() == Some("draft λ\r\n")
    }));
    for (w, h) in [(1.0, 1.0), (220.0, 100.0)] {
        v.simulate_resize(size(px(w), px(h)));
        v.simulate_keystrokes("f5 end up");
        draw(&mut v);
        v.simulate_keystrokes("escape");
    }
    v.simulate_resize(size(px(800.0), px(400.0)));
    v.simulate_keystrokes("f5 d");
    settle(cx, &f, |v| v.session.state().loading().is_none());
    draw(&mut v);
    assert!(f.view.read_with(&v, |v, _| {
        let s = v.session.state().current_source().unwrap();
        s.copy_range(0..s.len_bytes()).as_deref() == Some("external\r\n")
    }));
    assert!(f.view.read_with(&v, |v, _| {
        v.modal.is_none()
            && !v
                .session
                .state()
                .tab_dirty(v.session.state().active().unwrap().id())
    }));
    assert_eq!(std::fs::read(path).unwrap(), b"external\r\n");
    v.simulate_keystrokes("ctrl-q");
    cx.run_until_parked();
}

#[gpui::test]
fn draft_workspace_changes_use_real_folder_branch_and_new_branch_controls(cx: &mut TestAppContext) {
    use lenz_content::git::testing::git;
    let (f, wt) = branches_fixture(cx);
    let path = f.dir.path().join("a.txt");
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(900.0), px(400.0)));
    draw(&mut v);
    f.view
        .update(&mut v, |v, _| v.enable_draft_input_for_test());
    v.write_to_clipboard(ClipboardItem::new_string("draft λ\r\n".into()));
    v.simulate_keystrokes("ctrl-a ctrl-v");
    let position = f.view.read_with(&v, |v, _| v.session.state().selection());
    v.simulate_keystrokes("f10");
    let row = f.view.read_with(&v, |v, _| {
        v.modal_rows()
            .iter()
            .position(|r| r.0 == CommandId::OpenFolder.label())
            .unwrap()
    });
    for _ in 0..row {
        v.simulate_keystrokes("down");
    }
    v.simulate_keystrokes("enter");
    assert!(cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(|options| {
        assert!(options.directories && !options.files);
        Some(vec![wt.path().join("second")])
    });
    settle(cx, &f, |v| v.session.close_prompt().is_some());
    draw(&mut v);
    v.simulate_keystrokes("escape");
    assert_eq!(
        f.view.read_with(&v, |v, _| v
            .session
            .state()
            .project()
            .unwrap()
            .root()
            .to_owned()),
        f.dir.path()
    );
    assert_eq!(
        f.view.read_with(&v, |v, _| v.session.state().selection()),
        position
    );
    for new_branch in [false, true] {
        v.simulate_keystrokes("ctrl-shift-`");
        settle(cx, &f, |v| {
            v.session.state().branches().is_some_and(|b| !b.loading)
        });
        draw(&mut v);
        if new_branch {
            type_text(&mut v, "new");
            v.simulate_keystrokes("enter");
            assert!(
                f.view
                    .read_with(&v, |v, _| matches!(v.modal, Some(Modal::NewBranch { .. })))
            );
            type_text(&mut v, "draft-guard");
            v.simulate_keystrokes("enter");
        } else {
            type_text(&mut v, "other");
            v.simulate_keystrokes("enter");
        }
        assert!(
            f.view
                .read_with(&v, |v, _| v.session.close_prompt().is_some()),
            "new_branch={new_branch}, modal={:?}",
            f.view.read_with(&v, |v, _| v.modal.clone())
        );
        draw(&mut v);
        assert_eq!(
            git(f.dir.path(), &["branch", "--show-current"]).trim(),
            "main"
        );
        v.simulate_keystrokes("escape");
        assert_eq!(
            f.view.read_with(&v, |v, _| v.session.state().selection()),
            position
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"main\n");
    }
    v.simulate_keystrokes("ctrl-shift-`");
    settle(cx, &f, |v| {
        v.session.state().branches().is_some_and(|b| !b.loading)
    });
    draw(&mut v);
    type_text(&mut v, "other");
    v.simulate_keystrokes("enter d");
    settle(cx, &f, |v| {
        !v.session.state().checking_out()
            && v.session
                .state()
                .current_source()
                .is_some_and(|s| s.copy_range(0..s.len_bytes()).as_deref() == Some("other\n"))
    });
    draw(&mut v);
    assert_eq!(
        git(f.dir.path(), &["branch", "--show-current"]).trim(),
        "other"
    );
    assert_eq!(std::fs::read(path).unwrap(), b"other\n");
    assert!(f.view.read_with(&v, |v, _| v.modal.is_none()));
}

#[gpui::test]
fn saved_comparison_uses_real_keys_shared_history_and_read_only_rendering(cx: &mut TestAppContext) {
    let original = "α saved\r\nsecond\n";
    let f = fixture(cx, Some(("comparison.txt", original.as_bytes())));
    settle(cx, &f, |v| v.session().state().current_source().is_some());
    let mut v = visual(cx, &f); draw(&mut v);
    let ordinary = f.view.read_with(&v, |v, _| v.session().state().active().unwrap().id());
    f.view.update(&mut v, |view, _| view.enable_draft_input_for_test());
    v.update(|_, cx| cx.write_to_clipboard(ClipboardItem::new_string("α draft\r\nsecond\n".into())));
    v.simulate_keystrokes("ctrl-a ctrl-v shift-f2 ctrl-shift-d");
    assert!(f.view.read_with(&v, |v, _| v.session().state().saved_comparison().is_none()), "modal consumes comparison shortcut");
    v.simulate_keystrokes("escape ctrl-shift-d");
    settle(cx, &f, |v| v.session().state().saved_comparison().is_some_and(|c| !c.pending()));
    v.simulate_keystrokes("ctrl-g 1 enter"); draw(&mut v);
    let (saved, current) = f.view.read_with(&v, |v, _| { let c = v.session().state().saved_comparison().unwrap(); (c.saved_tab(), c.current_tab()) });
    assert!(f.view.read_with(&v, |v, _| v.tab_hits.iter().any(|(_, _, id)| *id == saved)));
    assert!(f.view.read_with(&v, |v, _| v.visible_rows().iter().any(|r| r.bg.iter().any(|(_, color)| *color == v.session().state().palette().block[2]))));
    for theme in ["github-light", "darcula"] {
        f.view.update(&mut v, |view, cx| view.dispatch(Command::Theme(ThemeCommand::Select(lenz_content::theme::ThemeSelection::from_id(theme).unwrap())), cx)); draw(&mut v);
        assert!(f.view.read_with(&v, |view, _| view.visible_rows().iter().any(|r| r.bg.iter().any(|(_, c)| *c == view.session().state().palette().block[2]))));
    }
    v.simulate_keystrokes("ctrl-z ctrl-shift-z");
    let saved_point = f.view.read_with(&v, |view, _| { let (from, to, _) = view.tab_hits.iter().find(|(_, _, id)| *id == saved).unwrap(); let pane = view.panes.iter().find(|pane| pane.pane == 0).unwrap(); point(px((from + to) / 2.0), px(pane.bar.2 + line_h() / 2.0)) });
    v.simulate_click(saved_point, Modifiers::default());
    assert!(f.view.read_with(&v, |view, _| view.session().state().saved_comparison().is_some() && view.session().state().active().unwrap().id() == saved));
    v.simulate_keystrokes("ctrl-a ctrl-v ctrl-z ctrl-s"); draw(&mut v);
    assert_eq!(f.view.read_with(&v, |v, _| { let s = v.session().state().view(saved).unwrap().snapshot(); s.copy_range(0..s.len_bytes()).unwrap() }), original);
    f.view.update(&mut v, |view, cx| view.dispatch(Command::FocusPane(1), cx));
    v.simulate_keystrokes("ctrl-s");
    settle(cx, &f, |v| !v.session().state().view(current).unwrap().dirty() && !v.session().state().saved_comparison().unwrap().pending());
    assert_eq!(std::fs::read(f.dir.path().join("comparison.txt")).unwrap(), "α draft\r\nsecond\n".as_bytes());
    v.simulate_resize(size(px(280.0), px(160.0))); draw(&mut v);
    v.simulate_keystrokes("ctrl-f4"); draw(&mut v);
    assert!(f.view.read_with(&v, |v, _| v.session().state().saved_comparison().is_none()));
    assert_eq!(f.view.read_with(&v, |v, _| v.session().state().active().unwrap().id()), ordinary);
    v.simulate_keystrokes("ctrl-z");
    assert!(f.view.read_with(&v, |v, _| v.session().state().tab_dirty(ordinary)));
}

#[gpui::test]
fn checked_save_allows_fresh_discard_after_current_recheck_without_changing_disk(cx:&mut TestAppContext) {
    use lenz_content::write::readiness_fault::{at_path,Fault};
    use reader_core::save::{DiskMatch,Status};
    for changed in [false,true] {
        let f=fixture_setup(cx,Some(("close.txt",b"saved")),&[],true);let path=f.dir.path().join("close.txt");settle(cx,&f,|v|v.session.state().current_source().is_some());let mut v=visual(cx,&f);draw(&mut v);f.view.update(&mut v,|v,_|v.enable_draft_input_for_test());v.write_to_clipboard(ClipboardItem::new_string("draft λ".into()));v.simulate_keystrokes("ctrl-a ctrl-v");let tab=f.view.read_with(&v,|v,_|v.session.state().active().unwrap().id());let guard=at_path(&path,Fault::AfterReplace);v.simulate_keystrokes("ctrl-s");settle(cx,&f,|v|matches!(v.session.state().view(tab).unwrap().save_status(),Status::Uncertain(_)));
        v.simulate_keystrokes("ctrl-f4 d");assert!(f.view.read_with(&v,|v,_|v.session.close_prompt().is_some_and(|p|p.discard_disabled.is_some())));v.simulate_keystrokes("escape");drop(guard);
        if changed {std::fs::write(&path,b"external content").unwrap();}let disk=std::fs::read(&path).unwrap();
        v.simulate_keystrokes("f10");let row=f.view.read_with(&v,|v,_|v.modal_rows().iter().position(|r|r.0==CommandId::RecheckSave.label()).unwrap());for _ in 0..row {v.simulate_keystrokes("down");}v.simulate_keystrokes("enter");
        settle(cx,&f,|v|matches!(v.session.state().view(tab).unwrap().save_status(),Status::Checked(_)));assert_eq!(f.view.read_with(&v,|v,_|v.session.state().view(tab).unwrap().save_status().clone()),Status::Checked(if changed {DiskMatch::Changed}else{DiskMatch::Attempted}));assert!(f.view.read_with(&v,|v,_|v.session.state().tab_dirty(tab)));
        v.simulate_keystrokes("ctrl-f4");assert!(f.view.read_with(&v,|v,_|v.session.close_prompt().is_some_and(|p|p.save_disabled.is_some() && p.discard_disabled.is_none())));draw(&mut v);v.simulate_keystrokes("d");assert!(f.view.read_with(&v,|v,_|v.session.state().active().is_none() && v.modal.is_none()));assert_eq!(std::fs::read(path).unwrap(),disk);
        v.simulate_keystrokes("ctrl-q");cx.run_until_parked();
    }
}

#[gpui::test]
fn production_empty_close_and_dirty_tab_decisions_remain_modal_safe_at_tiny_sizes(cx:&mut TestAppContext) {
    let empty=fixture_setup(cx,None,&[],true);let mut e=visual(cx,&empty);e.simulate_keystrokes("ctrl-z f5 f10 end up");
    for (w,h) in [(1.0,1.0),(120.0,40.0)] {e.simulate_resize(size(px(w),px(h)));draw(&mut e);}
    e.simulate_keystrokes("ctrl-q");assert!(empty.view.read_with(cx,|v,_|v.modal.is_some()));e.simulate_keystrokes("escape");
    assert!(!empty.view.update(cx,|v,cx|v.window_should_close(cx)));cx.run_until_parked();assert!(cx.update(|cx|cx.windows().is_empty()));
    let f=fixture_setup(cx,Some(("dirty.txt",b"saved")),&[],true);settle(cx,&f,|v|v.session.state().current_source().is_some());let mut v=visual(cx,&f);draw(&mut v);
    f.view.update(&mut v,|v,_|v.enable_draft_input_for_test());v.write_to_clipboard(ClipboardItem::new_string("draft λ".into()));v.simulate_keystrokes("ctrl-a ctrl-v ctrl-\\ f10");
    let unsplit=f.view.read_with(&v,|view,_|view.modal_rows().iter().position(|row|row.0==CommandId::Unsplit.label()).unwrap());
    for _ in 0..unsplit {v.simulate_keystrokes("down");}v.simulate_keystrokes("enter");
    assert!(f.view.read_with(&v,|view,_|view.session.state().split().is_none() && view.session.state().tabs().len()==1 && view.session.state().tab_dirty(view.session.state().active().unwrap().id())));
    v.simulate_keystrokes("ctrl-f4");
    assert!(f.view.read_with(cx,|v,_|v.session.close_prompt().is_some()));
    for (w,h) in [(1.0,1.0),(120.0,40.0)] {v.simulate_resize(size(px(w),px(h)));v.simulate_keystrokes("end up");draw(&mut v);}
    v.simulate_keystrokes("ctrl-s ctrl-q escape");assert!(f.view.read_with(cx,|v,_|v.modal.is_none() && v.session.state().active().is_some()));
    assert_eq!(std::fs::read(f.dir.path().join("dirty.txt")).unwrap(),b"saved");v.simulate_keystrokes("ctrl-f4 d");
    assert!(f.view.read_with(cx,|v,_|v.session.state().active().is_none()));assert_eq!(std::fs::read(f.dir.path().join("dirty.txt")).unwrap(),b"saved");
}

#[gpui::test]
fn production_quit_all_prepares_both_windows_before_discard_or_explicit_save(cx:&mut TestAppContext) {
    for first in ["d","s"] {
        let a=fixture_setup(cx,Some(("a.txt",b"saved\r\n")),&[],true);
        let b=fixture_setup(cx,Some(("b.txt",b"saved\r\n")),&[],true);
        for f in [&a,&b] {
            settle(cx,f,|v|v.session.state().current_source().is_some());
            let mut visual=visual(cx,f);draw(&mut visual);
            f.view.update(&mut visual,|v,_|v.enable_draft_input_for_test());
            visual.write_to_clipboard(ClipboardItem::new_string("draft λ\r\n".into()));visual.simulate_keystrokes("ctrl-a ctrl-v");
            draw(&mut visual);
        }
        let positions:Vec<_>=[&a,&b].into_iter().map(|f|f.view.read_with(cx,|v,_|(v.session.id(),v.session.state().selection(),v.session.state().active().unwrap().top_line()))).collect();
        let mut av=visual(cx,&a);av.simulate_keystrokes("ctrl-shift-q");cx.run_until_parked();
        assert_eq!(cx.update(|cx|cx.windows().len()),2);
        assert!(a.view.read_with(cx,|v,_|v.modal_rows().iter().any(|r|r.0.contains("Unsaved changes") || r.0.contains("Choose Save"))));
        av.simulate_keystrokes("ctrl-shift-q ctrl-s");av.write_to_clipboard(ClipboardItem::new_string("ignored".into()));av.simulate_keystrokes("ctrl-v");
        assert_eq!(std::fs::read(a.dir.path().join("a.txt")).unwrap(),b"saved\r\n");
        av.simulate_keystrokes(first);cx.run_until_parked();
        settle(cx,&b,|v|matches!(v.modal,Some(Modal::StatusDetails {kind:Details::Close,..})));
        assert_eq!(cx.update(|cx|cx.windows().len()),2,"early consent cannot remove its window");
        let mut bv=visual(cx,&b);bv.simulate_keystrokes("end up escape");cx.run_until_parked();
        for (i,f) in [&a,&b].into_iter().enumerate() {
            assert_eq!(f.view.read_with(cx,|v,_|(v.session.id(),v.session.state().selection(),v.session.state().active().unwrap().top_line())),positions[i]);
            assert_eq!(f.view.read_with(cx,|v,_| {let s=v.session.state().current_source().unwrap();s.copy_range(0..s.len_bytes()).unwrap()}),"draft λ\r\n");
            assert!(f.view.read_with(cx,|v,_|v.modal.is_none() && !v.session.closed()));
        }
        assert_eq!(std::fs::read(a.dir.path().join("a.txt")).unwrap(),if first=="s" {"draft λ\r\n".as_bytes()} else {b"saved\r\n"});
        assert_eq!(std::fs::read(b.dir.path().join("b.txt")).unwrap(),b"saved\r\n");
        if first=="d" {
            // A prepared clean window remains editable while the other window asks. New work cancels the batch.
            bv.simulate_keystrokes("ctrl-z");assert!(b.view.read_with(cx,|v,_|!v.session.state().tab_dirty(v.session.state().active().unwrap().id())));
            av.simulate_keystrokes("ctrl-shift-q");cx.run_until_parked();assert!(b.view.read_with(cx,|v,_|v.modal.is_none()));
            bv.write_to_clipboard(ClipboardItem::new_string("new work in prepared window".into()));bv.simulate_keystrokes("ctrl-a ctrl-v");cx.run_until_parked();
            assert_eq!(cx.update(|cx|cx.windows().len()),2);assert!(a.view.read_with(cx,|v,_|v.modal.is_none() && !v.session.closed()));
            assert_eq!(b.view.read_with(cx,|v,_|{let s=v.session.state().current_source().unwrap();s.copy_range(0..s.len_bytes()).unwrap()}),"new work in prepared window");
            assert_eq!(std::fs::read(a.dir.path().join("a.txt")).unwrap(),b"saved\r\n");assert_eq!(std::fs::read(b.dir.path().join("b.txt")).unwrap(),b"saved\r\n");
        }
        // Call the exact callback installed by production setup: it vetoes before asynchronous preparation.
        assert!(!b.view.update(cx,|v,cx|v.window_should_close(cx)));cx.run_until_parked();
        assert_eq!(cx.update(|cx|cx.windows().len()),2);
        bv.simulate_keystrokes("escape");cx.run_until_parked();
        // Authorize the final batch; effects occur only after both shared commits.
        av.simulate_keystrokes("ctrl-shift-q");cx.run_until_parked();
        if a.view.read_with(cx,|v,_|v.session.close_prompt().is_some()) {av.simulate_keystrokes("d");cx.run_until_parked();}
        bv.simulate_keystrokes("d");cx.run_until_parked();
        assert!(cx.update(|cx|cx.windows().is_empty()));
    }
}

#[gpui::test]
fn save_rejection_uncertainty_and_explicit_recheck_are_visible_and_modal_safe(cx: &mut TestAppContext) {
    use lenz_content::write::readiness_fault::{at_path, Fault};
    use reader_core::save::{Status, DiskMatch};
    for fault in [Fault::BeforeReplace, Fault::AfterReplace] {
        let f = fixture(cx, Some(("fault.txt", b"old"))); let path = f.dir.path().join("fault.txt");
        settle(cx, &f, |view| view.session().state().current_source().is_some());
        let mut v = visual(cx, &f); draw(&mut v);
        f.view.update(&mut v, |view, _| view.enable_draft_input_for_test());
        v.update(|_, cx| cx.write_to_clipboard(ClipboardItem::new_string("new".into())));
        v.simulate_keystrokes("ctrl-a ctrl-v");
        let tab = f.view.read_with(&v, |view, _| view.session().state().active().unwrap().id());
        let guard = at_path(&path, fault); v.simulate_keystrokes("ctrl-s");
        settle(cx, &f, |view| matches!(view.session().state().view(tab).unwrap().save_status(), Status::Rejected(_) | Status::Uncertain(_)));
        let disk = std::fs::read(&path).unwrap(); assert_eq!(disk, if fault == Fault::AfterReplace { b"new" } else { b"old" });
        draw(&mut v);
        let message = f.view.read_with(&v, |view, _| view.session().status_message());
        assert!(message.contains(if fault == Fault::AfterReplace { "uncertain" } else { "rejected" }), "{message}");
        v.simulate_keystrokes("shift-f2");
        assert!(f.view.read_with(&v, |view, _| view.modal.is_some()));
        v.simulate_keystrokes("ctrl-s escape"); assert_eq!(std::fs::read(&path).unwrap(), disk);
        if fault==Fault::AfterReplace {
            let before=f.view.read_with(&v,|view,_|view.session.state().current_source().unwrap().key());v.simulate_keystrokes("ctrl-f4 d s");
            assert!(f.view.read_with(&v,|view,_|view.session.close_prompt().is_some_and(|p|p.save_disabled.is_some() && p.discard_disabled.is_some())));
            assert_eq!(f.view.read_with(&v,|view,_|view.session.state().current_source().unwrap().key()),before);v.simulate_keystrokes("escape");
        }
        drop(guard);
        f.view.update_in(&mut v, |view, window, cx| view.run(CommandId::RecheckSave, window, cx));
        settle(cx, &f, |view| matches!(view.session().state().view(tab).unwrap().save_status(), Status::Checked(_)));
        let expected = if fault == Fault::AfterReplace { DiskMatch::Attempted } else { DiskMatch::Baseline };
        assert_eq!(f.view.read_with(&v, |view, _| view.session().state().view(tab).unwrap().save_status().clone()), Status::Checked(expected));
        assert!(f.view.read_with(&v, |view, _| view.session().state().tab_dirty(tab)));
        v.simulate_resize(size(px(280.0), px(160.0))); draw(&mut v);
        if fault==Fault::AfterReplace {
            v.simulate_keystrokes("ctrl-f4");assert!(f.view.read_with(&v,|view,_|view.session.close_prompt().is_some_and(|p|p.discard_disabled.is_none())));v.simulate_keystrokes("escape");
        }
        f.view.update_in(&mut v, |view, window, cx| view.run(CommandId::ConfirmRecheckedSave, window, cx));
        assert_eq!(f.view.read_with(&v, |view, _| view.session().state().tab_dirty(tab)), fault != Fault::AfterReplace);
        assert_eq!(std::fs::read(path).unwrap(), disk);
        if fault==Fault::AfterReplace {v.simulate_keystrokes("ctrl-f4");assert!(f.view.read_with(&v,|view,_|view.session.state().active().is_none()));}
    }
}

#[gpui::test]
fn save_shortcut_keeps_history_after_actual_cache_eviction(cx: &mut TestAppContext) {
    let f = fixture(cx, Some(("save.txt", "\u{feff}α😀\r\nbeta\n".as_bytes())));
    let path = f.dir.path().join("save.txt");
    settle(cx, &f, |view| view.session().state().current_source().is_some());
    let mut v = visual(cx, &f); draw(&mut v);
    let tab = f.view.read_with(&v, |view, _| view.session().state().active().unwrap().id());
    f.view.update(&mut v, |view, _| view.enable_draft_input_for_test());
    v.update(|_, cx| cx.write_to_clipboard(ClipboardItem::new_string("é".into())));
    v.simulate_keystrokes("home shift-right ctrl-v ctrl-s");
    settle(cx, &f, |view| !view.session().state().tab_dirty(tab));
    assert_eq!(std::fs::read(&path).unwrap(), "\u{feff}é😀\r\nbeta\n".as_bytes());
    draw(&mut v);
    let clean = f.dir.path().join("evicted.txt"); let other = f.dir.path().join("pressure.txt");
    std::fs::write(&clean, vec![b'a'; 17 << 20]).unwrap(); std::fs::write(&other, vec![b'b'; 17 << 20]).unwrap();
    f.view.update(&mut v, |view, cx| view.dispatch(Command::OpenRetained(clean.clone()), cx));
    settle(cx, &f, |view| view.session().state().current_source().is_some_and(|s| s.source() == clean));
    draw(&mut v);
    let weak = f.view.read_with(&v, |view, _| Arc::downgrade(view.session().state().current_source().unwrap()));
    f.view.update(&mut v, |view, cx| view.dispatch(Command::OpenRetained(other.clone()), cx));
    settle(cx, &f, |view| view.session().state().current_source().is_some_and(|s| s.source() == other));
    draw(&mut v);
    f.view.update(&mut v, |view, cx| view.dispatch(Command::OpenRetained(path.clone()), cx));
    settle(cx, &f, |view| view.session().state().current_source().is_some_and(|s| s.source() == path));
    draw(&mut v);
    assert!(weak.upgrade().is_none(), "real inactive-cache pressure released the clean source");
    v.simulate_keystrokes("ctrl-z"); draw(&mut v);
    assert_eq!(f.view.read_with(&v, |view, _| { let source = view.session().state().current_source().unwrap(); source.copy_range(0..source.len_bytes()).unwrap() }), "α😀\r\nbeta\n");
    assert!(f.view.read_with(&v, |view, _| view.session().state().tab_dirty(tab)));
    assert_eq!(std::fs::read(path).unwrap(), "\u{feff}é😀\r\nbeta\n".as_bytes());
}

#[gpui::test]
fn opted_in_draft_input_replaces_unicode_and_replays_shared_undo_without_saving(cx: &mut TestAppContext) {
    let original = "é界\r\nline two\r\n";
    let f = fixture(cx, Some(("draft.txt", original.as_bytes())));
    settle(cx, &f, |view| view.session().state().current_source().is_some());
    let mut v = visual(cx, &f);
    draw(&mut v);
    let tab_width = |v: &VisualTestContext, id: TabId| f.view.read_with(v, |view, _| {
        let (from, to, _) = view.tab_hits.iter().find(|(_, _, tab)| *tab == id).unwrap(); to - from
    });
    let id = f.view.read_with(&v, |view, _| view.session().state().active().unwrap().id());
    let clean_width = tab_width(&v, id);
    let text = |v: &VisualTestContext| f.view.read_with(v, |view, _| {
        let source = view.session().state().current_source().unwrap();
        source.copy_range(0..source.len_bytes()).unwrap()
    });
    v.update(|_, cx| cx.write_to_clipboard(ClipboardItem::new_string("ignored".into())));
    v.simulate_keystrokes("λ ctrl-v");
    assert_eq!(text(&v), original, "ordinary constructors remain read-only");
    f.view.update(&mut v, |view, _| view.enable_draft_input_for_test());
    v.simulate_keystrokes("right shift-right");
    assert_eq!(f.view.read_with(&v, |view, _| view.session().state().selection().unwrap().range()), 2..5);
    v.simulate_keystrokes("λ");
    assert_eq!(text(&v), "éλ\r\nline two\r\n");
    assert_eq!(f.view.read_with(&v, |view, _| view.session().state().selection().unwrap()), Selection { anchor: 4, head: 4 });
    draw(&mut v);
    assert_eq!(f.view.read_with(&v, |view, _| view.visible_rows()[0].text.clone()), "éλ");
    let char_w = f.view.read_with(&v, |view, _| view.char_w);
    assert!((tab_width(&v, id) - clean_width - 2.0 * char_w).abs() < 0.01, "dirty tab indicator has its own measured cells");
    assert!(f.view.read_with(&v, |view, _| { let state = view.session().state(); state.view(state.active().unwrap().id()).unwrap().dirty() }));
    for theme in ["github-light", "darcula"] {
        f.view.update(&mut v, |view, cx| view.dispatch(Command::Theme(ThemeCommand::Select(ThemeSelection::from_id(theme).unwrap())), cx));
        draw(&mut v);
        assert!((tab_width(&v, id) - clean_width - 2.0 * char_w).abs() < 0.01, "dirty indicator remains laid out in {theme}");
    }
    v.update(|_, cx| cx.write_to_clipboard(ClipboardItem::new_string("👩🏽‍💻".into())));
    v.simulate_keystrokes("ctrl-v");
    assert_eq!(text(&v), "éλ👩🏽‍💻\r\nline two\r\n");
    assert_eq!(f.view.read_with(&v, |view, _| view.session().state().selection().unwrap().head), 19);
    v.simulate_keystrokes("ctrl-z");
    assert_eq!(text(&v), "éλ\r\nline two\r\n");
    assert_eq!(f.view.read_with(&v, |view, _| view.session().state().selection().unwrap().head), 4);
    v.simulate_keystrokes("ctrl-z");
    assert_eq!(text(&v), original);
    assert_eq!(f.view.read_with(&v, |view, _| view.session().state().selection().unwrap().head), 5);
    assert!(!f.view.read_with(&v, |view, _| { let state = view.session().state(); state.view(state.active().unwrap().id()).unwrap().dirty() }));
    draw(&mut v);
    assert!((tab_width(&v, id) - clean_width).abs() < 0.01, "undo to clean text removes the dirty indicator");
    v.simulate_keystrokes("ctrl-shift-z ctrl-shift-z");
    assert_eq!(text(&v), "éλ👩🏽‍💻\r\nline two\r\n");
    let other = f.dir.path().join("other.txt");
    std::fs::write(&other, "other text\n").unwrap();
    f.view.update(&mut v, |view, cx| view.dispatch(Command::OpenRetained(other.clone()), cx));
    settle(cx, &f, |view| view.session().state().current_source().is_some_and(|source| source.source() == other));
    draw(&mut v);
    assert!(f.view.read_with(&v, |view, _| view.session().state().view(id).is_none() && view.session().state().tab_dirty(id)));
    assert!((tab_width(&v, id) - clean_width - 2.0 * char_w).abs() < 0.01, "inactive dirty tab retains its indicator");
    v.simulate_keystrokes("ctrl-shift-tab");
    settle(cx, &f, |view| view.session().state().active().is_some_and(|tab| tab.id() == id && tab.loaded()));
    draw(&mut v);
    assert_eq!(text(&v), "éλ👩🏽‍💻\r\nline two\r\n");
    let before_modal = f.view.read_with(&v, |view, _| view.session().state().current_source().unwrap().key());
    v.simulate_keystrokes("f10 x ctrl-v ctrl-z");
    draw(&mut v);
    assert_eq!(f.view.read_with(&v, |view, _| view.session().state().current_source().unwrap().key()), before_modal);
    v.simulate_keystrokes("escape");
    assert_eq!(std::fs::read_to_string(f.dir.path().join("draft.txt")).unwrap(), original);
}

#[gpui::test]
fn an_empty_window_renders_and_its_menu_explains_disabled_commands(cx: &mut TestAppContext) {
    let f = fixture(cx, None);
    let mut v = visual(cx, &f);
    draw(&mut v);
    assert!(f.view.read_with(&v, |v, _| v.status()).starts_with("No file open"));
    v.simulate_keystrokes("f10");
    let rows = f.view.read_with(&v, |v, _| v.modal_rows());
    let reload = rows.iter().position(|r| r.0 == "Reload").unwrap();
    assert_eq!(rows[reload], ("Reload".into(), Some("No file is open")));
    assert_eq!(rows[0].1, None, "Open is always available");
    // Enter on a disabled row does nothing; the menu closes.
    for _ in 0..reload {
        v.simulate_keystrokes("down");
    }
    v.simulate_keystrokes("enter");
    assert!(f.view.read_with(&v, |v, _| v.modal.is_none() && v.session().state().current_source().is_none()));
    for (w, h) in [(1.0, 1.0), (120.0, 40.0), (800.0, 600.0)] {
        v.simulate_resize(size(px(w), px(h)));
        v.simulate_keystrokes("f10 end up");
        draw(&mut v);
        v.simulate_keystrokes("escape");
    }
    assert!(f.view.read_with(&v, |v, _| v.modal.is_none()));
}

#[gpui::test]
fn server_status_click_shows_setup_and_shared_recovery_actions(cx: &mut TestAppContext) {
    let f = fixture(cx, None);
    let root = f.dir.path();
    let path = root.join("main.rs");
    let source = "fn main() {}\n";
    std::fs::write(&path, source).unwrap();
    std::fs::write(root.join("Cargo.toml"), "[package]\nname='status'\nversion='0.1.0'\n").unwrap();
    f.view.update(cx, |r, cx| {
        r.open_root(root.into(), cx);
        r.open(path.clone(), cx);
    });
    settle(cx, &f, |r| r.session().state().provider().is_some_and(|p| matches!(p.status, reader_core::semantics::Status::Missing(_))));
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(800.0), px(400.0)));
    draw(&mut v);
    assert!(f.view.read_with(&v, |r, _| r.status_segments().0.1).starts_with("LSP unavailable"));
    v.simulate_keystrokes("ctrl-a");
    let selected = f.view.read_with(&v, |r, _| r.session().state().selection());
    v.simulate_click(point(px(20.0), px(400.0 - status_h() / 2.0)), Modifiers::default());
    assert!(f.view.read_with(&v, |r, _| matches!(r.modal, Some(Modal::LanguageServer { .. }))));
    assert!(f.view.read_with(&v, |r, _| r.server_rows().len() == 9 && r.server_rows().iter().all(|(_, id)| id.is_some())));
    let button = f.view.read_with(&v, |r, _| r.modal_layout().unwrap().bounds.origin + point(px(20.0), px(4.0 + line_h() / 2.0)));
    v.simulate_click(button, Modifiers::default());
    assert!(f.view.read_with(&v, |r, _| matches!(r.modal, Some(Modal::StatusDetails { title: "Server details", .. }))));
    let text = f.view.read_with(&v, |r, _| r.modal_rows().into_iter().map(|r| r.0).collect::<Vec<_>>().join(""));
    assert!(text.contains("was not found") && text.contains("rustup component add rust-analyzer") && text.contains("providers.json"), "{text}");
    v.simulate_keystrokes("ctrl-a f5");
    v.simulate_event(gpui::MouseDownEvent { button: MouseButton::Right, position: point(px(600.0), px(75.0)), modifiers: Modifiers::default(), click_count: 1, first_mouse: false });
    assert!(f.view.read_with(&v, |r, _| matches!(r.modal, Some(Modal::StatusDetails { .. }))));
    assert_eq!(f.view.read_with(&v, |r, _| r.session().state().selection()), selected);
    for (w, h) in [(180.0, 90.0), (1.0, 1.0), (800.0, 400.0)] {
        v.simulate_resize(size(px(w), px(h)));
        v.simulate_keystrokes("end up");
        draw(&mut v);
        assert!(f.view.read_with(&v, |r, _| !r.modal_lines().is_empty()));
    }
    let expected = f.view.read_with(&v, |r, _| r.session().language_server_details());
    v.simulate_keystrokes("ctrl-c");
    assert_eq!(clipboard(&mut v).as_deref(), Some(expected.as_str()));
    assert!(expected.contains("was not found") && expected.contains("rustup component add rust-analyzer"));
    assert!(f.view.read_with(&v, |r, _| matches!(r.modal, Some(Modal::StatusDetails { .. }))));
    assert_eq!(f.view.read_with(&v, |r, _| r.session().state().selection()), selected);
    draw(&mut v);
    let [copy, close] = f.view.read_with(&v, |r, _| <[_; 2]>::try_from(r.modal_layout().unwrap().details_buttons(r.char_w, &r.details_labels(Details::Text))).unwrap());
    v.update(|_, cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string("sentinel".into())));
    v.simulate_click(copy.center(), Modifiers::default());
    assert_eq!(clipboard(&mut v).as_deref(), Some(expected.as_str()));
    v.simulate_click(close.center(), Modifiers::default());
    assert!(f.view.read_with(&v, |r, _| r.modal.is_none()));
    let activate = |v: &mut VisualTestContext, id: CommandId| {
        v.simulate_keystrokes("home");
        let at = f.view.read_with(v, |r, _| r.server_rows().iter().position(|(_, action)| *action == Some(id)).unwrap());
        for _ in 0..at { v.simulate_keystrokes("down"); }
        v.simulate_keystrokes("enter");
        assert!(f.view.read_with(v, |r, _| r.modal.is_none()));
    };
    v.simulate_keystrokes("f2");
    activate(&mut v, CommandId::DisableServer);
    assert_eq!(f.view.read_with(&v, |r, _| r.session().state().language_server_label()), "LSP off");
    drop(v);
    settle(cx, &f, |r| r.session().disabled(CommandId::EnableServer).is_none());
    let mut v = visual(cx, &f);
    v.simulate_keystrokes("f2");
    activate(&mut v, CommandId::EnableServer);
    drop(v);
    settle(cx, &f, |r| r.session().state().provider().is_some_and(|p| matches!(p.status, reader_core::semantics::Status::Missing(_))));
    let mut v = visual(cx, &f);
    v.simulate_keystrokes("f2");
    activate(&mut v, CommandId::RestartServer);
    assert_eq!(std::fs::read_to_string(path).unwrap(), source);
    assert_eq!(f.view.read_with(&v, |r, _| r.session().state().selection()), selected);
}

#[gpui::test]
fn trackpad_scroll_preserves_small_movements_and_reverses_at_edges(cx: &mut TestAppContext) {
    let source = format!("{}\n", "x".repeat(300)).repeat(200);
    let f = fixture(cx, Some(("main.txt", source.as_bytes())));
    settle(cx, &f, |r| r.session().state().current_source().is_some());
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(800.0), px(400.0)));
    draw(&mut v);
    let position = f.view.read_with(&v, |r, _| r.text_bounds.unwrap().center());
    let char_w = f.view.read_with(&v, |r, _| r.char_w);
    for _ in 0..8 {
        v.simulate_event(ScrollWheelEvent { position, delta: ScrollDelta::Pixels(point(px(-0.25 * char_w), px(-line_h() * 0.25))), ..Default::default() });
    }
    assert_eq!(f.view.read_with(&v, |r, _| r.session().state().top_line()), 2);
    assert_eq!(f.view.read_with(&v, |r, _| r.session().state().active().unwrap().column()), 2);
    v.simulate_keystrokes("home");
    for _ in 0..11 {
        v.simulate_event(ScrollWheelEvent { position, delta: ScrollDelta::Pixels(point(px(0.0), px(line_h() * 0.25))), ..Default::default() });
    }
    for _ in 0..4 {
        v.simulate_event(ScrollWheelEvent { position, delta: ScrollDelta::Pixels(point(px(0.0), px(-line_h() * 0.25))), ..Default::default() });
    }
    assert_eq!(f.view.read_with(&v, |r, _| r.session().state().top_line()), 1);
    v.simulate_keystrokes("end");
    let last = f.view.read_with(&v, |r, _| r.session().state().top_line());
    for _ in 0..3 {
        v.simulate_event(ScrollWheelEvent { position, delta: ScrollDelta::Pixels(point(px(0.0), px(-line_h() * 0.25))), ..Default::default() });
    }
    for _ in 0..4 {
        v.simulate_event(ScrollWheelEvent { position, delta: ScrollDelta::Pixels(point(px(0.0), px(line_h() * 0.25))), ..Default::default() });
    }
    assert_eq!(f.view.read_with(&v, |r, _| r.session().state().top_line()), last - 1);
    f.view.update(&mut v, |r, cx| r.scroll_to_x(f32::MAX, cx));
    let right = f.view.read_with(&v, |r, _| r.session().state().active().unwrap().column());
    for dx in [-0.25, -0.25, -0.25, 0.25, 0.25, 0.25, 0.25] {
        v.simulate_event(ScrollWheelEvent { position, delta: ScrollDelta::Pixels(point(px(dx * char_w), px(0.0))), ..Default::default() });
    }
    assert_eq!(f.view.read_with(&v, |r, _| r.session().state().active().unwrap().column()), right - 1);
    for _ in 0..3 {
        v.simulate_event(ScrollWheelEvent { position, delta: ScrollDelta::Pixels(point(px(0.0), px(-line_h() * 0.25))), ..Default::default() });
    }
    let other = f.dir.path().join("other.txt");
    std::fs::write(&other, &source).unwrap();
    f.view.update(&mut v, |r, cx| r.open(other.clone(), cx));
    drop(v);
    settle(cx, &f, |r| r.session().state().current_source().is_some_and(|d| d.source() == other));
    let mut v = visual(cx, &f);
    draw(&mut v);
    v.simulate_event(ScrollWheelEvent { position, delta: ScrollDelta::Pixels(point(px(0.0), px(-line_h() * 0.25))), ..Default::default() });
    assert_eq!(f.view.read_with(&v, |r, _| r.session().state().top_line()), 0, "a new tab must not inherit the previous tab's fraction");
    assert_eq!(std::fs::read(f.dir.path().join("main.txt")).unwrap(), source.as_bytes());
}

#[gpui::test]
fn trackpad_reaches_the_last_partial_column_across_repaints(cx: &mut TestAppContext) {
    let source = format!("{}\n", "x".repeat(80));
    let f = fixture(cx, Some(("wide.txt", source.as_bytes())));
    settle(cx, &f, |r| r.session().state().current_source().is_some());
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(480.0), px(300.0)));
    draw(&mut v);
    let (position, char_w, max) = f.view.read_with(&v, |r, _| (r.text_bounds.unwrap().center(), r.char_w, r.max_hscroll()));
    assert!(max > char_w && (max / char_w).fract() > 0.01, "fixture needs a partly visible last column");
    for _ in 0..((max / char_w).ceil() as usize + 1) * 4 {
        v.simulate_event(ScrollWheelEvent { position, delta: ScrollDelta::Pixels(point(px(-char_w * 0.25), px(0.0))), ..Default::default() });
        draw(&mut v);
    }
    let offset = f.view.read_with(&v, |r, _| r.hscroll());
    assert!(offset >= max && offset < max + char_w, "last partial column remains clipped: {offset} < {max}");
}

#[gpui::test]
fn source_windows_scroll_and_select_past_the_old_long_line_prefix(cx: &mut TestAppContext) {
    let source = format!("{}END😀\t\x07{}\r\n", "a".repeat(12_000), "tail ".repeat(1_000));
    let f = fixture(cx, Some(("long.txt", source.as_bytes())));
    settle(cx, &f, |r| r.session().state().current_source().is_some());
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(800.0), px(300.0)));
    draw(&mut v);
    f.view.update(&mut v, |r, cx| r.dispatch(Command::ScrollColumn(12_000), cx));
    for _ in 0..200 {
        cx.run_until_parked();
        f.view.update(&mut v, |r, cx| r.poll(cx));
        draw(&mut v);
        if f.view.read_with(&v, |r, _| r.visible_rows().first().is_some_and(|row| row.text.contains("END"))) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let (bounds, column, char_w, shown) = f.view.read_with(&v, |r, _| {
        (r.text_bounds.unwrap(), r.session().state().column(), r.char_w, r.visible_rows()[0].text.clone())
    });
    assert!(shown.contains("END"), "the viewport must reach the original tail: {shown:?}");
    assert!(column > 10_000);
    let x = f32::from(bounds.origin.x) + num_w() + 8.0 + (12_000 - column) as f32 * char_w;
    let y = bounds.origin.y + px(line_h() / 2.0);
    v.simulate_click(point(px(x + char_w * 0.1), y), Modifiers::default());
    v.simulate_click(point(px(x + char_w * 3.1), y), Modifiers::shift());
    assert_eq!(f.view.read_with(&v, |r, _| r.session().state().selection().unwrap().range()), 12_000..12_003);
    v.simulate_keystrokes("ctrl-c");
    assert_eq!(v.read_from_clipboard().and_then(|item| item.text()).as_deref(), Some("END"));
    assert_eq!(std::fs::read(f.dir.path().join("long.txt")).unwrap(), source.as_bytes());
}

#[gpui::test]
fn source_windows_keep_native_emoji_origins_across_projection_boundaries(cx: &mut TestAppContext) {
    let prefix = "a".repeat(40) + &"👨‍👩‍👧‍👦".repeat(2_000);
    let end = prefix.len();
    let source = prefix.clone() + "END" + &" tail".repeat(500) + "\r\n";
    let f = fixture(cx, Some(("family.txt", source.as_bytes())));
    settle(cx, &f, |r| r.session().state().current_source().is_some());
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(800.0), px(300.0)));
    draw(&mut v);
    let (font, char_w) = f.view.read_with(&v, |r, _| (r.font.clone(), r.char_w));
    let native_end = v.update(|window, _| {
        let measure = |text: &str| {
            let run = gpui::TextRun { len: text.len(), font: font.clone(), color: gpui::Hsla::default(), background_color: None, underline: None, strikethrough: None };
            f32::from(window.text_system().shape_line(text.to_owned().into(), px(font_size()), &[run], None).width)
        };
        // A whole-prefix f32 layout accumulates rounding over thousands of glyphs.
        measure(&"a".repeat(40)) as f64 + 2_000.0 * measure("👨‍👩‍👧‍👦") as f64
    }) as f32;
    let column = (native_end / char_w).floor() as usize - 20;
    f.view.update(&mut v, |r, cx| r.dispatch(Command::ScrollColumn(column), cx));
    for _ in 0..200 {
        cx.run_until_parked();
        f.view.update(&mut v, |r, cx| r.poll(cx));
        draw(&mut v);
        if f.view.read_with(&v, |r, _| r.visible_rows().first().is_some_and(|row| row.text.contains("END"))) { break; }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(f.view.read_with(&v, |r, _| r.visible_rows().first().is_some_and(|row| row.text.contains("END"))));
    let (bounds, column, shown, origin) = f.view.read_with(&v, |r, _| {
        let bounds = r.text_bounds.unwrap();
        let view = r.session.state().view(r.session.state().active().unwrap().id()).unwrap();
        let width = (f32::from(bounds.size.width) - num_w() - 16.0).max(1.0);
        let row = r.source.get(0, &view, 0, r.session.state().column() as f32 * r.char_w, width).unwrap();
        (bounds, r.session.state().column(), row.text.clone(), row.origin)
    });
    let at = shown.find("END").unwrap();
    let painted_end = v.update(|window, _| {
        let run = gpui::TextRun { len: shown.len(), font, color: gpui::Hsla::default(), background_color: None, underline: None, strikethrough: None };
        origin + f32::from(window.text_system().shape_line(shown.into(), px(font_size()), &[run], None).x_for_index(at))
    });
    assert!((painted_end - native_end).abs() < 1.0, "bounded native advances must retain the family glyph's width");
    let x = f32::from(bounds.origin.x) + num_w() + 8.0 + painted_end - column as f32 * char_w;
    let y = bounds.origin.y + px(line_h()/2.0);
    v.simulate_click(point(px(x+char_w*0.1),y),Modifiers::default());
    v.simulate_click(point(px(x+char_w*3.1),y),Modifiers::shift());
    assert_eq!(f.view.read_with(&v, |r,_| r.session().state().selection().unwrap().range()),end..end+3);
    v.simulate_keystrokes("ctrl-c");
    assert_eq!(v.read_from_clipboard().and_then(|item|item.text()).as_deref(),Some("END"));
    assert_eq!(std::fs::read(f.dir.path().join("family.txt")).unwrap(),source.as_bytes());
}

fn native_literal_point(f: &Fixture, v: &mut VisualTestContext, literal: &str) -> Option<(gpui::Point<Pixels>, f32)> {
    v.update(|window, cx| {
        let reader = f.view.read(cx);
        let state = reader.session.state();
        let tab = state.active()?;
        if !reader.current_source_geometry(tab) { return None; }
        let bounds = reader.text_bounds?;
        let view = state.view(tab.id())?;
        let logical = view.layout_rows(reader.rows, state.wrap_cols(tab)).rows;
        let width = f32::from(bounds.size.width) - num_w() - 16.0;
        for row in reader.visible_rows() {
            let Some(at) = row.text.find(literal) else { continue };
            let logical_row = logical.get(row.y as usize)?;
            reader.ready_source_row(tab, logical_row)?;
            let parts = reader.source_parts(tab, logical_row, &row, window);
            let left = native_x(window, reader.metrics(), &row, &parts, at) - reader.hscroll();
            let right = native_x(window, reader.metrics(), &row, &parts, at + literal.len()) - reader.hscroll();
            if left >= 0.0 && right <= width {
                return Some((point(bounds.origin.x + px(num_w() + 8.0 + left), bounds.origin.y + px((row.y + 0.5) * line_h())), reader.char_w));
            }
        }
        None
    })
}

fn settle_native_literal(cx: &mut TestAppContext, f: &Fixture, v: &mut VisualTestContext, literal: &str) -> (gpui::Point<Pixels>, f32) {
    for _ in 0..400 {
        cx.run_until_parked();
        f.view.update(v, |reader, cx| reader.poll(cx));
        draw(v);
        if let Some(point) = native_literal_point(f, v, literal) { return point; }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    panic!("native literal {literal:?} remains outside the painted viewport: {:?}", f.view.read_with(v, |r, _| {
        let state=r.session.state();let tab=state.active().unwrap();let view=state.view(tab.id()).unwrap();
        let logical=view.layout_rows(r.rows,state.wrap_cols(tab)).rows;
        (r.status(),state.column(),r.session.native_reveal(tab.pane()).map(|g|(g.request,g.source,g.context)),r.visible_rows().iter().map(|row|(row.y,row.text.find(literal),row.text.len())).collect::<Vec<_>>(),logical.iter().map(|row|r.source_row(tab,row).map(|native|(native.skip(),native.pending(),r.ready_source_row(tab,row).is_some()))).collect::<Vec<_>>())
    }));
}

#[gpui::test]
fn source_windows_native_go_to_and_find_reveal_the_source_before_trailing_spaces(cx: &mut TestAppContext) {
    let source = "👨‍👩‍👧‍👦".repeat(2_000) + " END" + &" ".repeat(100) + "\n";
    let f = fixture(cx, Some(("go-to.txt", source.as_bytes())));
    settle(cx, &f, |r| r.session.state().current_source().is_some());
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(500.0), px(240.0)));
    draw(&mut v);
    v.simulate_keystrokes("ctrl-g 1 : 1 4 0 0 2 enter");
    let (position, char_w) = settle_native_literal(cx, &f, &mut v, "END");
    let target = source.find("END").unwrap();
    assert_eq!(f.view.read_with(&v, |r, _| r.session.state().selection().unwrap().head), target);
    v.simulate_click(position + point(px(char_w * 0.1), px(0.0)), Modifiers::default());
    v.simulate_click(position + point(px(char_w * 3.1), px(0.0)), Modifiers::shift());
    assert_eq!(f.view.read_with(&v, |r, _| r.session.state().selection().unwrap().range()), target..target + 3);
    v.simulate_keystrokes("ctrl-c");
    assert_eq!(v.read_from_clipboard().and_then(|item| item.text()).as_deref(), Some("END"));
    v.simulate_keystrokes("home ctrl-f");
    assert!(f.view.read_with(&v, |r, _| r.session.state().active().unwrap().find().unwrap().query() == "END"));
    settle(cx, &f, find_settled);
    v.simulate_keystrokes("enter");
    settle_native_literal(cx, &f, &mut v, "END");
    assert_eq!(std::fs::read(f.dir.path().join("go-to.txt")).unwrap(), source.as_bytes());
}

#[gpui::test]
fn source_windows_native_wrapped_reveal_pans_wide_rows_and_unwinds_for_short_rows(cx: &mut TestAppContext) {
    let source = "    ".to_owned() + &"1️⃣".repeat(80) + " END tail\nshort\n";
    let f = fixture(cx, Some(("wrapped-pan.txt", source.as_bytes())));
    settle(cx, &f, |r| r.session.state().current_source().is_some());
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(400.0), px(260.0)));
    draw(&mut v);
    v.simulate_keystrokes("alt-z ctrl-g 1 : 6 5 enter");
    for _ in 0..400 {
        cx.run_until_parked();
        f.view.update(&mut v, |r, cx| r.poll(cx));
        draw(&mut v);
        if f.view.read_with(&v, |r, _| r.session.state().column() > 0 && r.current_source_geometry(r.session.state().active().unwrap())) { break; }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(f.view.read_with(&v, |r, _| r.session.state().selection().unwrap().head), 4 + 20 * "1️⃣".len());
    assert!(f.view.read_with(&v, |r, _| r.session.state().column() > 0), "a native-overwide wrapped row must expose its caret");
    v.simulate_keystrokes("ctrl-g 2 : 1 enter");
    settle_native_literal(cx, &f, &mut v, "short");
    assert_eq!(f.view.read_with(&v, |r, _| r.session.state().column()), 0, "a short wrapped target unwinds the temporary pan");
    assert_eq!(std::fs::read(f.dir.path().join("wrapped-pan.txt")).unwrap(), source.as_bytes());
}

#[gpui::test]
fn source_windows_compact_suppressed_source_and_retain_caret_mapping(cx: &mut TestAppContext) {
    let source = "a".to_owned() + &"\u{200b}".repeat(50_000) + "END\n";
    let f = fixture(cx, Some(("invisible.txt", source.as_bytes())));
    settle(cx, &f, |r| r.session.state().current_source().is_some());
    let mut v = visual(cx, &f);
    draw(&mut v);
    for _ in 0..300 {
        cx.run_until_parked();
        f.view.update(&mut v, |r, cx| r.poll(cx));
        draw(&mut v);
        if f.view.read_with(&v, |r, _| r.visible_rows().iter().any(|row| row.text.contains("END"))) { break; }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    f.view.read_with(&v, |r, _| {
        let tab = r.session.state().active().unwrap();
        let view = r.session.state().view(tab.id()).unwrap();
        let logical = view.layout_rows(r.rows, None).rows;
        let row = r.source_row(tab, &logical[0]).unwrap();
        assert!(row.text.contains("END"));
        assert!(row.text.len() < 256);
        assert!(r.source.retained_windows() <= 64, "suppressed source must not accumulate projected windows: {}", r.source.retained_windows());
        let middle = 1 + 25_000 * "\u{200b}".len();
        assert_eq!(row.caret(middle), row.caret(source.find("END").unwrap()), "hidden source retains the displayed boundary for its caret");
    });
    assert_eq!(std::fs::read(f.dir.path().join("invisible.txt")).unwrap(), source.as_bytes());
}

#[gpui::test]
fn source_windows_wrapped_end_and_up_reach_the_original_tail(cx: &mut TestAppContext) {
    let source = "👨‍👩‍👧‍👦".repeat(2_000) + " END tail\r\n";
    let f = fixture(cx, Some(("wrapped.txt", source.as_bytes())));
    settle(cx, &f, |r| r.session().state().current_source().is_some());
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(400.0), px(220.0)));
    draw(&mut v);
    v.simulate_keystrokes("alt-z end up");
    for _ in 0..300 {
        cx.run_until_parked();
        f.view.update(&mut v, |r, cx| r.poll(cx));
        draw(&mut v);
        if f.view.read_with(&v, |r, _| r.visible_rows().iter().any(|row| row.text.contains("END"))) { break; }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(f.view.read_with(&v, |r, _| r.visible_rows().iter().any(|row| row.text.contains("END"))), "End followed by Up must paint the original tail: {:?}", f.view.read_with(&v, |r, _| { let t=r.session.state().active().unwrap(); let layout=r.session.state().view(t.id()).unwrap().layout_rows(r.rows,r.session.state().wrap_cols(t)); ((t.top_line(),t.top_row()),format!("{:?}",layout.rows),r.visible_rows().iter().map(|r|r.text.clone()).collect::<Vec<_>>()) }));
    v.simulate_keystrokes("up");
    for _ in 0..100 {
        cx.run_until_parked();
        f.view.update(&mut v, |r, cx| r.poll(cx));
        draw(&mut v);
        if f.view.read_with(&v, |r, _| r.session.state().active().unwrap().top_line() == 0) { break; }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let (head, tail, first_byte) = f.view.read_with(&v, |r, _| {
        let tab = r.session.state().active().unwrap();
        let layout = r.session.state().view(tab.id()).unwrap().layout_rows(r.rows, r.session.state().wrap_cols(tab));
        (tab.top_row(), r.visible_rows().iter().any(|row| row.text.contains("END")), layout.rows[0].source.start)
    });
    assert!(first_byte > 40_000 && tail, "Up must move within the original source tail: row {head}, byte {first_byte}");
    let (bounds, row, origin, font, char_w) = f.view.read_with(&v, |r, _| {
        let tab = r.session.state().active().unwrap();
        let logical = r.session.state().view(tab.id()).unwrap().layout_rows(r.rows, r.session.state().wrap_cols(tab));
        let row = r.visible_rows().into_iter().find(|r| r.text.contains("END")).unwrap();
        let origin = r.source_row(tab, &logical.rows[row.y as usize]).unwrap().origin;
        (r.text_bounds.unwrap(), row, origin, r.font.clone(), r.char_w)
    });
    let at = row.text.find("END").unwrap();
    let x = v.update(|window, _| {
        let run = gpui::TextRun { len: row.text.len(), font, color: gpui::Hsla::default(), background_color: None, underline: None, strikethrough: None };
        f32::from(bounds.origin.x) + num_w() + 8.0 + origin + f32::from(window.text_system().shape_line(row.text.clone().into(), px(font_size()), &[run], None).x_for_index(at))
    });
    let y = bounds.origin.y + px((row.y + 0.5) * line_h());
    v.simulate_click(point(px(x + char_w * 0.1), y), Modifiers::default());
    v.simulate_click(point(px(x + char_w * 3.1), y), Modifiers::shift());
    let end = source.find("END").unwrap();
    assert_eq!(f.view.read_with(&v, |r, _| r.session.state().selection().unwrap().range()), end..end + 3);
    v.simulate_keystrokes("ctrl-c");
    assert_eq!(v.read_from_clipboard().and_then(|item| item.text()).as_deref(), Some("END"));
    assert_eq!(std::fs::read(f.dir.path().join("wrapped.txt")).unwrap(), source.as_bytes());
}

#[gpui::test]
fn source_windows_reject_scroll_geometry_until_repaint(cx: &mut TestAppContext) {
    let source = "same long source row\n".repeat(50);
    let f = fixture(cx, Some(("stale.txt", source.as_bytes())));
    settle(cx, &f, |r| r.session().state().current_source().is_some());
    let mut v = visual(cx, &f);
    draw(&mut v);
    // Both lines are cached: the old viewport must still be rejected after scrolling.
    let hit = v.update(|window, cx| f.view.update(cx, |r, _| {
        let bounds = r.text_bounds.unwrap();
        r.session.dispatch(Command::Scroll(1));
        r.byte_at(window, f32::from(bounds.origin.x) + num_w() + 8.0, f32::from(bounds.origin.y) + line_h() / 2.0)
    }));
    assert_eq!(hit, None, "input must not reuse the previous painted viewport");
    draw(&mut v);
    let hit = v.update(|window, cx| f.view.read(cx).byte_at(window, 100.0, 100.0));
    assert!(hit.is_some());
    assert_eq!(std::fs::read(f.dir.path().join("stale.txt")).unwrap(), source.as_bytes());
}

#[gpui::test]
fn source_windows_reject_font_geometry_until_repaint(cx: &mut TestAppContext) {
    let f = fixture(cx, Some(("font.txt", b"source coordinates\n")));
    settle(cx, &f, |r| r.session.state().current_source().is_some());
    let mut v = visual(cx, &f);
    draw(&mut v);
    let hit = v.update(|window, cx| f.view.update(cx, |r, _| {
        let bounds = r.text_bounds.unwrap();
        r.font.weight = gpui::FontWeight::BOLD;
        r.byte_at(window, f32::from(bounds.origin.x) + num_w() + 10.0, f32::from(bounds.origin.y) + line_h() / 2.0)
    }));
    assert_eq!(hit, None, "input must reject font changes before the native geometry is repainted");
    draw(&mut v);
    assert!(v.update(|window, cx| f.view.read(cx).byte_at(window, 100.0, 100.0)).is_some());
    assert_eq!(std::fs::read(f.dir.path().join("font.txt")).unwrap(), b"source coordinates\n");
}

#[gpui::test]
fn source_windows_release_obsolete_reload_metadata(cx: &mut TestAppContext) {
    let f = fixture(cx, Some(("closed.txt", b"owned source")));
    settle(cx, &f, |r| r.session().state().current_source().is_some());
    let mut v = visual(cx, &f);
    draw(&mut v);
    let metadata = f.view.read_with(&v, |r, _| {
        let tab = r.session().state().active().unwrap();
        Arc::downgrade(r.session().state().view(tab.id()).unwrap().snapshot().source_meta())
    });
    std::fs::write(f.dir.path().join("closed.txt"), b"replacement source").unwrap();
    f.view.update(&mut v, |r, cx| r.dispatch(Command::Reload, cx));
    settle(cx, &f, |r| r.session().state().current_source().is_some_and(|doc| doc.copy_range(0..doc.len_bytes()).as_deref() == Some("replacement source")));
    draw(&mut v);
    assert!(metadata.upgrade().is_none(), "a refreshed source view must release obsolete projection metadata");
    assert_eq!(std::fs::read(f.dir.path().join("closed.txt")).unwrap(), b"replacement source");
}

#[gpui::test]
fn reading_selecting_copying_and_scrolling_never_change_the_file(cx: &mut TestAppContext) {
    let long = "x".repeat(lenz_content::display::LONG_LINE + 5);
    let source = format!("fn café() {{}}\r\n\tlet bell = \"\x07\";\r\n{long}\r\n{}", "line\n".repeat(200));
    let f = fixture(cx, Some(("main.rs", source.as_bytes())));
    settle(cx, &f, |v| v.session().state().current_source().is_some());
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(800.0), px(400.0)));
    draw(&mut v);
    // Controls are escaped and long lines use bounded viewport excerpts without changing the source.
    let rows = f.view.read_with(&v, |v, _| v.visible_rows());
    assert_eq!(rows[0].text, "fn café() {}");
    assert!(rows[1].text.contains('\u{FFFD}') && rows[1].text.starts_with("    let"));
    assert!(!rows[2].text.is_empty() && rows[2].text.len() < long.len());
    assert!(rows[2].text.chars().all(|c| c == 'x'), "the viewport contains source text, without the old truncation label");
    // Click at the start of line 1's text, shift-click further on: a source selection on character boundaries.
    let text = f.view.read_with(&v, |v, _| v.text_bounds.unwrap());
    let x0 = f32::from(text.origin.x) + num_w() + 8.0;
    let line_y = |l: f32| text.origin.y + px(l * line_h() + line_h() / 2.0);
    v.simulate_click(point(px(x0 - 4.0), line_y(0.0)), Modifiers::default());
    v.simulate_click(point(px(x0 + 400.0), line_y(0.0)), Modifiers::shift());
    let selection = f.view.read_with(&v, |v, _| v.session().state().selection().unwrap());
    assert_eq!(selection.range(), 0.."fn café() {}".len());
    v.simulate_keystrokes("ctrl-c");
    assert_eq!(v.read_from_clipboard().and_then(|c| c.text()).as_deref(), Some("fn café() {}"));
    draw(&mut v);
    assert_eq!(f.view.read_with(&v, |v, _| v.visible_rows()[0].selected.clone()), Some(0.."fn café() {}".len()));
    // Drag from line 0 into line 1.
    v.simulate_mouse_down(point(px(x0 - 4.0), line_y(0.0)), MouseButton::Left, Modifiers::default());
    v.simulate_mouse_move(point(px(x0 + 80.0), line_y(1.0)), Some(MouseButton::Left), Modifiers::default());
    v.simulate_mouse_up(point(px(x0 + 80.0), line_y(1.0)), MouseButton::Left, Modifiers::default());
    let dragged = f.view.read_with(&v, |v, _| v.session().state().selection().unwrap());
    assert!(dragged.anchor == 0 && dragged.head > "fn café() {}\r\n\t".len(), "past the tab on line 1: {dragged:?}");
    // End and the wheel scroll independently of the caret.
    v.simulate_keystrokes("end");
    let (top, lines) = f.view.read_with(&v, |v, _| (v.session().state().top_line(), v.session().state().current_source().unwrap().line_count()));
    assert_eq!(top, lines - 1);
    v.simulate_event(ScrollWheelEvent { delta: ScrollDelta::Lines(point(0.0, 10.0)), ..Default::default() });
    assert_eq!(f.view.read_with(&v, |v, _| v.session().state().top_line()), lines - 1 - 30);
    v.simulate_keystrokes("ctrl-a ctrl-c");
    assert_eq!(v.read_from_clipboard().and_then(|c| c.text()).as_deref(), Some(source.as_str()));
    // Narrow and tiny windows still render.
    for (w, h) in [(1.0, 1.0), (60.0, 30.0)] {
        v.simulate_resize(size(px(w), px(h)));
        draw(&mut v);
    }
    assert_eq!(std::fs::read(f.dir.path().join("main.rs")).unwrap(), source.as_bytes());
}

#[gpui::test]
fn encoding_notices_and_unavailable_files_show_in_the_status_bar(cx: &mut TestAppContext) {
    let f = fixture(cx, Some(("latin.txt", b"caf\xe9 cr\xe8me br\xfbl\xe9e au lait\n")));
    settle(cx, &f, |v| v.session().state().current_source().is_some());
    assert!(f.view.read_with(cx, |v, _| v.status()).contains("Encoding: windows-1252"));
    let missing = f.dir.path().join("missing.txt");
    f.view.update(cx, |v, cx| v.dispatch(Command::Open(missing), cx));
    settle(cx, &f, |v| v.status().contains("Cannot open"));
    let empty = f.dir.path().join("empty.txt");
    std::fs::write(&empty, b"").unwrap();
    f.view.update(cx, |v, cx| v.dispatch(Command::Open(empty), cx));
    settle(cx, &f, |v| v.session().state().current_source().is_some_and(|d| d.source().ends_with("empty.txt")));
    let mut v = visual(cx, &f);
    draw(&mut v);
    assert_eq!(f.view.read_with(&v, |v, _| v.visible_rows().len()), 1);
}

#[gpui::test]
fn theme_chooser_navigation_previews_without_committing_and_escape_restores(cx: &mut TestAppContext) {
    let f = fixture(cx, Some(("a.rs", b"fn a() {}\n")));
    settle(cx, &f, |r| r.session().state().current_source().is_some());
    let mut v = visual(cx, &f);
    draw(&mut v);
    let initial = f.view.read_with(&v, |r, _| (r.session.theme_id().to_string(), r.session.state().palette().bg));
    let appearance = f.dir.path().join("config/lenz-reader/appearance.json");
    let persisted = std::fs::read(&appearance).ok();
    f.view.update_in(&mut v, |r, w, cx| r.run(CommandId::ChooseTheme, w, cx));
    v.simulate_keystrokes("home down down");
    let expected = lenz_content::theme::builtin_entries()[1].palette.bg;
    assert_eq!(f.view.read_with(&v, |r, _| r.session.state().palette().bg), expected, "navigation should preview before Enter");
    assert_eq!(f.view.read_with(&v, |r, _| r.session.theme_id().to_string()), initial.0);
    assert_eq!(std::fs::read(&appearance).ok(), persisted);
    v.simulate_keystrokes("escape");
    assert_eq!(f.view.read_with(&v, |r, _| r.session.state().palette().bg), initial.1);
    assert_eq!(std::fs::read(&appearance).ok(), persisted);
}

#[gpui::test]
fn theme_chooser_is_a_direct_menu_action_with_inert_headers_and_scroll(cx: &mut TestAppContext) {
    let f = fixture(cx, Some(("a.rs", b"fn a() {}\n")));
    settle(cx, &f, |r| r.session().state().current_source().is_some());
    let mut v = visual(cx, &f);
    for dark in [true, false] {
        let committed = if dark { "darcula" } else { "github-light" };
        f.view.update(&mut v, |r, cx| r.dispatch(Command::Theme(ThemeCommand::Select(ThemeSelection::from_id(committed).unwrap())), cx));
        v.simulate_resize(size(px(800.0), px(500.0)));
        draw(&mut v);
        let menu = f.view.read_with(&v, |r, _| r.app_menu_button().center());
        v.simulate_click(menu, Modifiers::default());
        let row = f.view.read_with(&v, |r, _| r.modal_rows().iter().position(|(s, _)| s == "Change theme").unwrap());
        click_modal_row(&f, &mut v, row);
        let chosen = f.view.read_with(&v, |r, _| match &r.modal {
            Some(Modal::Themes { ids, at }) => { assert_eq!(ids[*at].0, committed); *at }
            other => panic!("{other:?}"),
        });
        assert!(chosen > 0);
        assert!(f.view.read_with(&v, |r, _| r.modal_rows()[1].0.starts_with("Current: ")));
        v.simulate_keystrokes("home");
        click_modal_row(&f, &mut v, 0);
        click_modal_row(&f, &mut v, 1);
        assert!(f.view.read_with(&v, |r, _| matches!(r.modal, Some(Modal::Themes { at: 0, .. }))));
        v.simulate_keystrokes("end pageup");
        assert!(f.view.read_with(&v, |r, _| matches!(&r.modal, Some(Modal::Themes { ids, at }) if *at == ids.len().saturating_sub(9))));
        v.simulate_keystrokes("pagedown home down down");
        let preview = f.view.read_with(&v, |r, _| (r.modal.as_ref().unwrap().at(), r.session.state().palette().bg));
        v.simulate_resize(size(px(800.0), px(180.0)));
        draw(&mut v);
        let inside = f.view.read_with(&v, |r, _| r.modal_layout().unwrap().bounds.center());
        v.simulate_event(ScrollWheelEvent { position: inside, delta: ScrollDelta::Lines(point(0.0, -1000.0)), ..Default::default() });
        draw(&mut v);
        assert!(f.view.read_with(&v, |r, _| r.modal_layout().unwrap().first > 0));
        assert_eq!(f.view.read_with(&v, |r, _| (r.modal.as_ref().unwrap().at(), r.session.state().palette().bg)), preview);
        v.simulate_event(ScrollWheelEvent { position: inside, delta: ScrollDelta::Lines(point(0.0, 1.0)), ..Default::default() });
        draw(&mut v);
        assert_eq!(f.view.read_with(&v, |r, _| (r.modal.as_ref().unwrap().at(), r.session.state().palette().bg)), preview);
        for (w, h) in [(120.0, 80.0), (1.0, 1.0)] {
            v.simulate_resize(size(px(w), px(h)));
            v.simulate_keystrokes("end up");
            draw(&mut v);
            assert!(f.view.read_with(&v, |r, _| {
                let b = r.modal_layout().unwrap().bounds;
                f32::from(b.size.width) <= w && f32::from(b.size.height) <= h
            }));
        }
        v.simulate_resize(size(px(800.0), px(500.0)));
        draw(&mut v);
        v.simulate_click(point(px(1.0), px(1.0)), Modifiers::default());
        assert!(f.view.read_with(&v, |r, _| r.modal.is_none()));
        let bg = lenz_content::theme::builtin_entries().iter().find(|e| e.id == committed).unwrap().palette.bg;
        assert_eq!(f.view.read_with(&v, |r, _| r.session.state().palette().bg), bg);
        assert_eq!(f.view.read_with(&v, |r, _| r.session.state().active().unwrap().top_line()), 0);
    }
    assert_eq!(std::fs::read(f.dir.path().join("a.rs")).unwrap(), b"fn a() {}\n");
}

#[gpui::test]
fn theme_chooser_registry_refresh_retains_ids_and_handles_a_removed_preview(cx: &mut TestAppContext) {
    let f = fixture(cx, None);
    let themes = f.dir.path().join("config/lenz-reader/themes");
    std::fs::create_dir_all(&themes).unwrap();
    std::fs::write(themes.join("loaded.toml"), include_str!("../../content/src/theme/fixtures/custom.toml")).unwrap();
    let mut v = visual(cx, &f);
    f.view.update_in(&mut v, |r, w, cx| {
        r.dispatch(Command::Theme(ThemeCommand::Select(ThemeSelection::from_id("light").unwrap())), cx);
        r.run(CommandId::ChooseTheme, w, cx);
    });
    drop(v);
    settle(cx, &f, |r| r.session.theme_entries().iter().any(|e| e.id == "user:loaded"));
    let mut v = visual(cx, &f);
    assert!(f.view.read_with(&v, |r, _| matches!(&r.modal, Some(Modal::Themes { ids, .. }) if ids.iter().any(|(id, _)| id == "user:loaded"))));
    let one = lenz_content::theme::ThemeEntry { id: "user:one".into(), label: "First user theme".into(), palette: Arc::new(lenz_content::theme::DARCULA) };
    let two = lenz_content::theme::ThemeEntry { id: "user:two".into(), label: "Second user theme".into(), palette: Arc::new(lenz_content::theme::LIGHT) };
    f.view.update(&mut v, |r, cx| r.dispatch(Completion::Theme(Box::new(lenz_content::theme::ThemeUpdate::Registry { request: 1, entries: vec![two.clone(), one.clone()], notices: Vec::new() })), cx));
    assert!(f.view.read_with(&v, |r, _| matches!(&r.modal, Some(Modal::Themes { ids, at }) if ids[*at].0 == "light")));
    // Newly discovered themes appear in the open chooser, and reordering retains the highlighted ID.
    v.simulate_keystrokes("end");
    assert_eq!(f.view.read_with(&v, |r, _| r.session.state().palette().bg), lenz_content::theme::DARCULA.bg);
    f.view.update(&mut v, |r, cx| r.dispatch(Completion::Theme(Box::new(lenz_content::theme::ThemeUpdate::Registry { request: 1, entries: vec![one, two], notices: Vec::new() })), cx));
    assert!(f.view.read_with(&v, |r, _| matches!(&r.modal, Some(Modal::Themes { ids, at }) if ids[*at].0 == "user:one" && *at == ids.len() - 2)));
    f.view.update(&mut v, |r, cx| r.dispatch(Completion::Theme(Box::new(lenz_content::theme::ThemeUpdate::Registry { request: 1, entries: Vec::new(), notices: Vec::new() })), cx));
    assert!(f.view.read_with(&v, |r, _| matches!(r.modal, Some(Modal::Themes { at: 0, .. }))));
    assert_eq!(f.view.read_with(&v, |r, _| r.session.state().palette().bg), lenz_content::theme::LIGHT.bg);
    v.simulate_keystrokes("enter");
    assert_eq!(f.view.read_with(&v, |r, _| r.session.theme_id().to_string()), "system");
}

#[gpui::test]
fn themes_change_through_the_chooser_and_persist_for_the_reader_only(cx: &mut TestAppContext) {
    let f = fixture(cx, Some(("a.rs", b"fn a() {}\n")));
    settle(cx, &f, |v| v.session().state().current_source().is_some());
    let mut v = visual(cx, &f);
    draw(&mut v);
    let themes = lenz_content::theme::builtin_entries();
    for entry in &themes {
        let (id, bg) = (entry.id.as_str(), entry.palette.bg);
        // Theme… from the command menu (Ctrl+T finds declarations, as the spec's table says).
        let theme = CommandId::ALL.iter().position(|&c| c == CommandId::ChooseTheme).unwrap();
        v.simulate_keystrokes("f10");
        for _ in 0..theme {
            v.simulate_keystrokes("down");
        }
        v.simulate_keystrokes("enter");
        let at = f.view.read_with(&v, |v, _| match &v.modal {
            Some(Modal::Themes { ids, .. }) => ids.iter().position(|(i, _)| i == id).unwrap(),
            other => panic!("{other:?}"),
        });
        v.simulate_keystrokes("home");
        for _ in 0..at {
            v.simulate_keystrokes("down");
        }
        v.simulate_keystrokes("enter");
        assert_eq!(f.view.read_with(&v, |v, _| v.session().state().palette().bg), bg);
        for (w, h) in [(800.0, 600.0), (90.0, 50.0)] {
            v.simulate_resize(size(px(w), px(h)));
            draw(&mut v);
        }
    }
    settle(cx, &f, |_| true);
    let saved = |p: &Path| std::fs::read_to_string(p).unwrap_or_default();
    let reader = f.dir.path().join("config/lenz-reader/appearance.json");
    let last = &themes.last().unwrap().id;
    for _ in 0..200 {
        if saved(&reader).contains(last) { break; }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(saved(&reader).contains(last), "{}", saved(&reader));
    assert!(!f.dir.path().join("config/lenz").exists());
}

#[test]
fn wrapping_keeps_every_character_and_prefers_word_breaks() {
    assert_eq!(wrap("", 10), [""]);
    assert_eq!(wrap("Reload — No file is open", 12), ["Reload — No ", "file is open"]);
    assert_eq!(wrap("abcdefghij", 4), ["abcd", "efgh", "ij"]);
    assert_eq!(wrap("x", 0), ["x"]);
    let text = "a.txt · Encoding: windows-1252 · undecodable bytes shown as �";
    assert_eq!(wrap(text, 7).concat(), text);
}

#[gpui::test]
fn narrow_windows_cut_the_status_bar_and_wrap_menu_reasons_and_shift_keys_select(cx: &mut TestAppContext) {
    let f = fixture(cx, Some(("latin.txt", b"caf\xe9 au lait\nsecond line\nthird\n")));
    settle(cx, &f, |v| v.session().state().current_source().is_some());
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(160.0), px(300.0)));
    draw(&mut v);
    // The server and message use separate cells; long notices end with an ellipsis.
    let (line, cols, toolbar, text) = f.view.read_with(&v, |v, _| (v.status_line(), v.columns(f32::from(v.status_segments().1.0.size.width)), v.panes[0].bar.2, v.text_bounds.unwrap()));
    assert!(f.view.read_with(&v, |r, _| r.status_segments().0.1).starts_with("LSP"));
    assert!(line.ends_with('…') && line.chars().count() == cols, "{line:?} in {cols} columns");
    assert_eq!(toolbar, toolbar_h(), "tabs sit below the dedicated toolbar");
    assert!(f32::from(text.origin.y + text.size.height) <= 300.0 - status_h(), "the text ends above the status bar: {text:?}");
    // The status control opens details without selecting text; right-click does not open a context menu.
    v.simulate_click(point(px(20.0), px(300.0 - status_h() / 2.0)), Modifiers::shift());
    assert_eq!(f.view.read_with(&v, |v, _| v.session().state().selection()), None);
    assert!(f.view.read_with(&v, |v, _| matches!(v.modal, Some(Modal::LanguageServer { .. }))));
    v.simulate_keystrokes("escape");
    let p = point(px(20.0), px(300.0 - status_h() / 2.0));
    v.simulate_event(gpui::MouseDownEvent { button: MouseButton::Right, position: p, modifiers: Modifiers::default(), click_count: 1, first_mouse: false });
    v.simulate_event(gpui::MouseUpEvent { button: MouseButton::Right, position: p, modifiers: Modifiers::default(), click_count: 1 });
    assert!(f.view.read_with(&v, |v, _| v.modal.is_none()), "no menu over the status bar");
    assert_eq!(f.view.read_with(&v, |v, _| v.session().state().selection()), None);
    v.simulate_keystrokes("ctrl-f4 f10");
    draw(&mut v);
    let menu = f.view.read_with(&v, |v, _| v.modal_lines());
    let index = CommandId::ALL.iter().position(|&c| c == CommandId::Reload).unwrap();
    let reload: String = menu.iter().filter(|(i, _, _)| *i == index).map(|(_, l, _)| l.as_str()).collect();
    assert_eq!(reload, "Reload — No file is open", "the whole reason is shown");
    v.simulate_keystrokes("escape");
    // Reopen, then select with shift and the arrows.
    let path = f.dir.path().join("latin.txt");
    f.view.update(&mut v, |view, cx| view.open(path, cx));
    drop(v);
    settle(cx, &f, |v| v.session().state().current_source().is_some());
    let mut v = visual(cx, &f);
    v.simulate_keystrokes("shift-down shift-right shift-end");
    let s = f.view.read_with(&v, |v, _| v.session().state().selection().unwrap());
    assert_eq!(s.range(), 0.."café au lait\nsecond line".len());
    v.simulate_keystrokes("ctrl-c");
    assert_eq!(v.read_from_clipboard().and_then(|c| c.text()).as_deref(), Some("café au lait\nsecond line"));
}

fn project(dir: &Path, files: &[&str]) -> PathBuf {
    let root = dir.join("proj");
    for f in files {
        let p = root.join(f);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, f.as_bytes()).unwrap();
    }
    root
}

fn scanned(v: &ReaderView) -> bool {
    v.session().state().project().is_some_and(|p| matches!(p.coverage(), reader_core::project::Coverage::Complete(_) | reader_core::project::Coverage::Unavailable(_)))
}

#[gpui::test]
fn the_tree_and_finder_open_files_by_mouse_and_keyboard(cx: &mut TestAppContext) {
    let f = fixture(cx, None);
    let root = project(f.dir.path(), &["src/main.rs", "src/ui/main.rs", "README.md", ".env.example", "target/x.rs", ".gitignore"]);
    std::fs::write(root.join(".gitignore"), "target/\n").unwrap();
    f.view.update(cx, |v, cx| v.open_root(root.clone(), cx));
    settle(cx, &f, scanned);
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(900.0), px(500.0)));
    draw(&mut v);
    assert!(f.view.read_with(&v, |v, _| v.status()).contains("proj: 5 files"), "hidden files kept, ignored ones not");
    // Click the "src" row (rows: src, .env.example, .gitignore, README.md: folders first), then the file inside it.
    let body = f.view.read_with(&v, |v, _| v.text_bounds.unwrap());
    let row_y = |i: f32| body.origin.y + px(i * line_h() + line_h() / 2.0);
    v.simulate_click(point(px(20.0), row_y(0.0)), Modifiers::default());
    draw(&mut v);
    let rows: Vec<String> = f.view.read_with(&v, |v, _| v.session().state().project().unwrap().rows().iter().map(|r| r.path.to_string()).collect());
    assert_eq!(rows, ["src", "src/ui", "src/main.rs", ".env.example", ".gitignore", "README.md"]);
    v.simulate_click(point(px(20.0), row_y(2.0)), Modifiers::default());
    settle(cx, &f, |v| v.session().state().current_source().is_some_and(|d| d.source().ends_with("src/main.rs")));
    // Keyboard: the tree has focus after the click; Up, Right expands src/ui, Down, Enter opens its main.rs.
    let mut v = visual(cx, &f);
    v.simulate_keystrokes("up right down enter");
    settle(cx, &f, |v| v.session().state().current_source().is_some_and(|d| d.source().ends_with("ui/main.rs")));
    // The finder: type, see results over the whole inventory, pick the second.
    let mut v = visual(cx, &f);
    v.simulate_keystrokes("escape ctrl-p m a i n");
    settle(cx, &f, |v| v.session().state().navigation().is_some_and(|f| !f.pending && f.query == "main"));
    let mut v = visual(cx, &f);
    let rows = f.view.read_with(&v, |v, _| v.modal_rows());
    assert_eq!(rows[1].0, "Search › main");
    assert!(rows[0].0.contains("[Files]"));
    assert!(rows[3].0.contains("main.rs") && rows[3].0.contains("src"));
    assert!(rows[4].0.contains("main.rs") && rows[4].0.contains("src/ui"));
    v.simulate_keystrokes("backspace n");
    drop(v);
    settle(cx, &f, |r| r.session.state().navigation().is_some_and(|n| !n.pending && n.query == "main"));
    let mut v = visual(cx, &f);
    v.simulate_keystrokes("down enter");
    settle(cx, &f, |v| v.modal.is_none() && v.session().state().current_source().is_some_and(|d| d.source().ends_with("ui/main.rs")));
    assert!(f.view.read_with(cx, |v, _| v.session().state().navigation().is_none()));
    assert_eq!(std::fs::read(root.join("src/main.rs")).unwrap(), b"src/main.rs");
}

#[gpui::test]
fn folders_open_through_the_command_menu_recent_list_and_narrow_windows(cx: &mut TestAppContext) {
    let f = fixture(cx, None);
    let root = project(f.dir.path(), &["a.rs", "b/c.rs"]);
    let missing = f.dir.path().join("gone");
    f.view.update(cx, |v, cx| v.open_root(missing.clone(), cx));
    settle(cx, &f, scanned);
    assert!(f.view.read_with(cx, |v, _| v.status()).contains("gone: unavailable"));
    f.view.update(cx, |v, cx| v.open_root(root.clone(), cx));
    settle(cx, &f, scanned);
    let mut v = visual(cx, &f);
    draw(&mut v);
    // F10 lists the folder commands; Open Recent shows both roots, newest first, and reopens the missing one.
    v.simulate_keystrokes("f10");
    let menu = f.view.read_with(&v, |v, _| v.modal_rows());
    assert_eq!(&menu[1..4], &[("Open Folder…".to_string(), None), ("Open Recent Folder…".to_string(), None), ("Find File…".to_string(), None)]);
    v.simulate_keystrokes("down down enter");
    let recent = f.view.read_with(&v, |v, _| v.modal_rows());
    assert_eq!(recent.iter().map(|r| r.0.clone()).collect::<Vec<_>>(), [root.display().to_string(), missing.display().to_string()]);
    v.simulate_keystrokes("escape f10 down down down enter");
    assert!(matches!(f.view.read_with(&v, |v, _| v.modal.clone()), Some(Modal::Navigation { .. })), "Find File from the menu");
    v.simulate_keystrokes("escape");
    // Narrow: the tree hides until focused, then takes the width; Escape gives the text back.
    v.simulate_resize(size(px(300.0), px(200.0)));
    draw(&mut v);
    assert_eq!(f.view.read_with(&v, |v, _| v.sidebar_w), 0.0);
    v.simulate_keystrokes("ctrl-shift-e");
    draw(&mut v);
    assert_eq!(f.view.read_with(&v, |v, _| v.sidebar_w), 300.0);
    v.simulate_keystrokes("end enter escape");
    draw(&mut v);
    assert_eq!(f.view.read_with(&v, |v, _| (v.sidebar_w, v.tree_focus)), (0.0, false));
    for (w, h) in [(1.0, 1.0), (200.0, 60.0)] {
        v.simulate_resize(size(px(w), px(h)));
        v.simulate_keystrokes("ctrl-shift-e down ctrl-p a");
        draw(&mut v);
        v.simulate_keystrokes("escape escape");
    }
}

#[gpui::test]
fn dismissal_scrolling_and_arrivals_keep_the_tree_and_finder_consistent(cx: &mut TestAppContext) {
    let f = fixture(cx, None);
    let files: Vec<String> = (0..60).map(|i| format!("f{i:02}.rs")).collect();
    let root = project(f.dir.path(), &files.iter().map(String::as_str).collect::<Vec<_>>());
    f.view.update(cx, |v, cx| v.open_root(root.clone(), cx));
    settle(cx, &f, scanned);
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(700.0), px(300.0)));
    draw(&mut v);
    // Clicking outside the finder closes it in the core too: later batches start no ranking.
    v.simulate_keystrokes("ctrl-p f");
    v.simulate_click(point(px(2.0), px(2.0)), Modifiers::default());
    assert!(f.view.read_with(&v, |v, _| v.modal.is_none() && v.session().state().navigation().is_none()));
    // The wheel scrolls the tree, and a repaint keeps that scroll.
    v.simulate_event(ScrollWheelEvent { position: point(px(20.0), px(100.0)), delta: ScrollDelta::Lines(point(0.0, -5.0)), ..Default::default() });
    draw(&mut v);
    let scroll = f.view.read_with(&v, |v, _| v.tree_scroll);
    assert_eq!(scroll, 15);
    draw(&mut v);
    assert_eq!(f.view.read_with(&v, |v, _| v.tree_scroll), scroll);
    // A trackpad's small movements add up, as in the text (each alone is under half a row).
    let pad = |v: &mut gpui::VisualTestContext, dy: f32| v.simulate_event(ScrollWheelEvent { position: point(px(20.0), px(100.0)), delta: ScrollDelta::Pixels(point(px(0.0), px(dy))), ..Default::default() });
    for _ in 0..10 { pad(&mut v, -line_h() / 5.0); }
    assert_eq!(f.view.read_with(&v, |v, _| v.tree_scroll), scroll + 2);
    // At the top, movement beyond the edge is dropped: reversing responds at once.
    f.view.update(&mut v, |v, _| v.tree_scroll = 0);
    for _ in 0..10 { pad(&mut v, line_h() / 5.0); }
    pad(&mut v, -line_h());
    assert_eq!(f.view.read_with(&v, |v, _| v.tree_scroll), 1);
    f.view.update(&mut v, |v, _| v.tree_scroll = scroll);
    // Highlight a file, then files arrive that sort above it: the highlight stays on the same file.
    let body = f.view.read_with(&v, |v, _| v.text_bounds.unwrap());
    v.simulate_click(point(px(20.0), body.origin.y + px(line_h() / 2.0)), Modifiers::default());
    let chosen = f.view.read_with(&v, |v, _| v.tree_sel.clone().unwrap());
    let generation = f.view.read_with(&v, |v, _| v.session().state().project().unwrap().generation());
    let batch = reader_core::project::Batch { paths: vec!["a0.rs".into(), "a1.rs".into()], done: true, ..Default::default() };
    f.view.update(&mut v, |v, cx| v.dispatch(Completion::Scanned { generation, batch }, cx));
    draw(&mut v);
    let (at, row) = f.view.read_with(&v, |v, _| (v.tree_at(), v.session().state().project().unwrap().rows()[v.tree_at()].path.clone()));
    assert_eq!(row, chosen, "row {at} is still the chosen file");
    // A short finder: End then Up stays within the results.
    v.simulate_keystrokes("escape ctrl-p f 0 1");
    drop(v);
    settle(cx, &f, |v| v.session().state().navigation().is_some_and(|f| !f.pending && f.query == "f01"));
    let mut v = visual(cx, &f);
    v.simulate_keystrokes("end up up up");
    assert!(matches!(f.view.read_with(&v, |v, _| v.modal.clone()), Some(Modal::Navigation { at: 0, .. })));
    v.simulate_keystrokes("end enter");
    drop(v);
    settle(cx, &f, |v| v.session().state().current_source().is_some_and(|d| d.source().ends_with("f01.rs")));
}

fn tab_titles(v: &ReaderView) -> Vec<String> {
    let s = v.session().state();
    s.tabs().iter().map(|t| format!("{}{}{}", t.title(), if t.retained() { "" } else { "~" }, if s.active().is_some_and(|a| a.id() == t.id()) { "*" } else { "" })).collect()
}

fn active_is(name: &'static str) -> impl Fn(&ReaderView) -> bool {
    move |v| v.session().state().current_source().is_some_and(|d| d.source().ends_with(name)) && v.session().state().loading().is_none()
}

#[gpui::test]
fn tabs_open_reuse_keep_and_close_through_real_input(cx: &mut TestAppContext) {
    let f = fixture(cx, None);
    let long = format!("{}\n", "w".repeat(400));
    let root = project(f.dir.path(), &["a.rs", "b.rs", "c.rs"]);
    std::fs::write(root.join("a.rs"), long.repeat(80)).unwrap();
    f.view.update(cx, |v, cx| v.open_root(root.clone(), cx));
    settle(cx, &f, scanned);
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(900.0), px(400.0)));
    draw(&mut v);
    // Tree rows as drawn: from below the explorer's title row, whatever the tab bar and breadcrumbs take.
    let row = |i: f32| point(px(20.0), px(toolbar_h() + line_h() + i * line_h() + line_h() / 2.0));
    // Single clicks in the tree reuse the preview tab; a double-click keeps one.
    v.simulate_click(row(0.0), Modifiers::default());
    drop(v);
    settle(cx, &f, active_is("a.rs"));
    let mut v = visual(cx, &f);
    draw(&mut v);
    v.simulate_click(row(1.0), Modifiers::default());
    drop(v);
    settle(cx, &f, active_is("b.rs"));
    assert_eq!(f.view.read_with(cx, |v, _| tab_titles(v)), ["b.rs~*"]);
    let mut v = visual(cx, &f);
    draw(&mut v);
    v.simulate_event(gpui::MouseDownEvent { button: MouseButton::Left, position: row(1.0), modifiers: Modifiers::default(), click_count: 2, first_mouse: false });
    v.simulate_event(gpui::MouseUpEvent { button: MouseButton::Left, position: row(1.0), modifiers: Modifiers::default(), click_count: 2 });
    assert_eq!(f.view.read_with(&v, |v, _| tab_titles(v)), ["b.rs*"]);
    v.simulate_click(row(0.0), Modifiers::default());
    drop(v);
    settle(cx, &f, active_is("a.rs"));
    assert_eq!(f.view.read_with(cx, |v, _| tab_titles(v)), ["b.rs", "a.rs~*"]);
    // Horizontal position belongs to the tab: scroll a.rs sideways, switch tabs, come back.
    let mut v = visual(cx, &f);
    draw(&mut v);
    v.simulate_event(ScrollWheelEvent { position: point(px(600.0), px(200.0)), delta: ScrollDelta::Pixels(point(px(-300.0), px(0.0))), ..Default::default() });
    let column = f.view.read_with(&v, |v, _| v.session().state().column());
    assert!(column > 0);
    v.simulate_keystrokes("ctrl-tab");
    drop(v);
    settle(cx, &f, active_is("b.rs"));
    assert_eq!(f.view.read_with(cx, |v, _| v.session().state().column()), 0);
    let mut v = visual(cx, &f);
    v.simulate_keystrokes("ctrl-shift-tab");
    drop(v);
    settle(cx, &f, active_is("a.rs"));
    assert_eq!(f.view.read_with(cx, |v, _| v.session().state().column()), column);
    // The tab bar: click b's label to activate it, double-click a's to keep it, its × to close it.
    let mut v = visual(cx, &f);
    draw(&mut v);
    let (hits, bar_y) = f.view.read_with(&v, |v, _| (v.tab_hits.clone(), toolbar_h() + line_h() / 2.0));
    v.simulate_click(point(px(hits[0].0 + 10.0), px(bar_y)), Modifiers::default());
    drop(v);
    settle(cx, &f, active_is("b.rs"));
    let mut v = visual(cx, &f);
    v.simulate_event(gpui::MouseDownEvent { button: MouseButton::Left, position: point(px(hits[1].0 + 10.0), px(bar_y)), modifiers: Modifiers::default(), click_count: 2, first_mouse: false });
    assert_eq!(f.view.read_with(&v, |v, _| tab_titles(v)), ["b.rs*", "a.rs"]);
    v.simulate_click(point(px(hits[0].1 - 4.0), px(bar_y)), Modifiers::default());
    drop(v);
    settle(cx, &f, active_is("a.rs"));
    assert_eq!(f.view.read_with(cx, |v, _| tab_titles(v)), ["a.rs*"]);
    // Ctrl+W closes the last tab: an empty reader that still renders, with Back available.
    let mut v = visual(cx, &f);
    v.simulate_keystrokes("ctrl-f4");
    draw(&mut v);
    assert!(f.view.read_with(&v, |v, _| v.session().state().tabs().is_empty() && v.session().state().can_back()));
    v.simulate_keystrokes("alt-left");
    drop(v);
    settle(cx, &f, |v| v.session().state().current_source().is_some());
    for (w, h) in [(1.0, 1.0), (180.0, 80.0)] {
        let mut v = visual(cx, &f);
        v.simulate_resize(size(px(w), px(h)));
        draw(&mut v);
    }
}

#[gpui::test]
fn history_keys_and_menu_go_back_and_forward(cx: &mut TestAppContext) {
    let f = fixture(cx, Some(("one.txt", "1\n".repeat(80).as_bytes())));
    settle(cx, &f, active_is("one.txt"));
    let two = f.dir.path().join("two.txt");
    std::fs::write(&two, "2\n".repeat(80)).unwrap();
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(600.0), px(300.0)));
    draw(&mut v);
    v.simulate_keystrokes("pagedown");
    let top = f.view.read_with(&v, |v, _| v.session().state().top_line());
    assert!(top > 0);
    f.view.update(&mut v, |view, cx| view.open(two.clone(), cx));
    drop(v);
    settle(cx, &f, active_is("two.txt"));
    let mut v = visual(cx, &f);
    v.simulate_keystrokes("f10");
    let menu = f.view.read_with(&v, |v, _| v.modal_rows());
    assert!(menu.contains(&("Forward".to_string(), Some("Nothing to go forward to"))));
    let back = menu.iter().position(|r| r.0 == "Back").unwrap();
    assert_eq!(menu[back].1, None);
    for _ in 0..back {
        v.simulate_keystrokes("down");
    }
    v.simulate_keystrokes("enter");
    drop(v);
    settle(cx, &f, active_is("one.txt"));
    assert_eq!(f.view.read_with(cx, |v, _| v.session().state().top_line()), top, "Back restores the position");
    let mut v = visual(cx, &f);
    v.simulate_keystrokes("alt-right");
    drop(v);
    settle(cx, &f, active_is("two.txt"));
    let mut v = visual(cx, &f);
    v.simulate_keystrokes("cmd-[");
    drop(v);
    settle(cx, &f, active_is("one.txt"));
    let mut v = visual(cx, &f);
    v.simulate_keystrokes("cmd-]");
    drop(v);
    settle(cx, &f, active_is("two.txt"));
    // Many tabs in a narrow window: the active one stays in view and the toolbar never paints over the bar.
    for i in 0..12 {
        let p = f.dir.path().join(format!("tab-number-{i}.txt"));
        std::fs::write(&p, b"x\n").unwrap();
        f.view.update(cx, |v, cx| v.dispatch(Command::OpenRetained(p.clone()), cx));
        settle(cx, &f, |v| v.session().state().current_source().is_some_and(|d| d.source() == p) && v.session().state().loading().is_none());
    }
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(300.0), px(200.0)));
    draw(&mut v);
    let (hits, active) = f.view.read_with(&v, |v, _| (v.tab_hits.clone(), v.session().state().active().unwrap().id()));
    let shown = hits.iter().find(|h| h.2 == active).unwrap();
    assert!(shown.0 >= 0.0 - 1.0 && shown.1 <= 300.0 + 0.5, "{shown:?}");
}

#[gpui::test]
fn a_new_window_has_its_own_session(cx: &mut TestAppContext) {
    let f = fixture(cx, Some(("one.txt", b"1\n")));
    settle(cx, &f, active_is("one.txt"));
    let before = cx.update(|cx| cx.windows().len());
    let mut v = visual(cx, &f);
    v.simulate_keystrokes("f10");
    for _ in 0..CommandId::ALL.iter().position(|id| *id == CommandId::NewWindow).unwrap() { v.simulate_keystrokes("down"); }
    v.simulate_keystrokes("enter");
    drop(v);
    cx.run_until_parked();
    let windows = cx.update(|cx| cx.windows());
    assert_eq!(windows.len(), before + 1);
    let other = windows.iter().find_map(|w| w.downcast::<ReaderView>().filter(|w| *w != f.window)).unwrap();
    let other_view = other.update(cx, |_, _, cx| cx.entity()).unwrap();
    assert!(other_view.read_with(cx, |v, _| v.session().state().tabs().is_empty()), "a fresh session");
    // Closing the first window's tab does not touch the second, and vice versa.
    f.view.update(cx, |v, cx| v.dispatch(Command::Close, cx));
    other_view.update(cx, |v, cx| v.open(f.dir.path().join("one.txt"), cx));
    for _ in 0..500 {
        cx.run_until_parked();
        other_view.update(cx, |v, cx| v.poll(cx));
        if other_view.read_with(cx, |v, _| v.session().state().current_source().is_some()) { break; }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(other_view.read_with(cx, |v, _| v.session().state().current_source().is_some()));
    assert!(f.view.read_with(cx, |v, _| v.session().state().tabs().is_empty()));
}

#[gpui::test]
fn external_changes_show_up_and_deletions_keep_the_last_content(cx: &mut TestAppContext) {
    let f = fixture(cx, Some(("live.rs", b"fn before() {}\n")));
    settle(cx, &f, active_is("live.rs"));
    let path = f.dir.path().join("live.rs");
    let temp = f.dir.path().join("live.tmp");
    std::fs::write(&temp, b"fn after() {}\nfn more() {}\n").unwrap();
    std::fs::rename(&temp, &path).unwrap();
    settle(cx, &f, |v| v.session().state().current_source().is_some_and(|d| d.line_count() == 3));
    let mut v = visual(cx, &f);
    draw(&mut v);
    assert_eq!(f.view.read_with(&v, |v, _| v.visible_rows()[0].text.clone()), "fn after() {}");
    std::fs::remove_file(&path).unwrap();
    drop(v);
    settle(cx, &f, |v| v.status().contains("Unavailable"));
    let mut v = visual(cx, &f);
    draw(&mut v);
    assert_eq!(f.view.read_with(&v, |v, _| v.visible_rows()[0].text.clone()), "fn after() {}", "the last content stays on screen");
    // The menu's Reload says the same instead of recreating anything.
    v.simulate_keystrokes("f5");
    drop(v);
    settle(cx, &f, |v| v.session().state().loading().is_none());
    assert!(!path.exists());
    assert!(f.view.read_with(cx, |v, _| v.status().contains("Unavailable")));
}

fn search_done(v: &ReaderView) -> bool {
    v.session().state().results().is_some_and(|r| r.done() && (r.hits().is_empty() || r.preview().is_some()))
}

#[gpui::test]
fn project_search_takes_a_file_pattern_and_its_tree_folds_by_click_and_keys(cx: &mut TestAppContext) {
    let f = fixture(cx, None);
    let root = project(f.dir.path(), &["top.rs", "src/a.rs", "src/b.rs", "web/c.ts"]);
    for name in ["top.rs", "src/a.rs", "src/b.rs", "web/c.ts"] {
        std::fs::write(root.join(name), "needle\n").unwrap();
    }
    f.view.update(cx, |v, cx| v.open_root(root.clone(), cx));
    settle(cx, &f, scanned);
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(900.0), px(600.0)));
    // Tab moves to the files field; the pattern limits the search.
    v.simulate_keystrokes("ctrl-shift-f n e e d l e tab s r c");
    assert_eq!(f.view.read_with(&v, |v, _| v.modal_rows()[1].0.clone()), "Files › src");
    assert_eq!(f.view.read_with(&v, |v, _| v.modal.as_ref().unwrap().at()), 1, "the files field is the active one");
    v.simulate_keystrokes("enter");
    drop(v);
    settle(cx, &f, search_done);
    let mut v = visual(cx, &f);
    draw(&mut v);
    let labels = |v: &mut VisualTestContext| f.view.update(v, |v, _| v.panel_contents(10, 0).0.into_iter().map(|r| r.label).collect::<Vec<_>>());
    assert_eq!(labels(&mut v), ["src/  2", "a.rs  1", "1: needle", "b.rs  1", "1: needle"]);
    let status = f.view.read_with(&v, |v, _| v.results_status().unwrap());
    assert!(status.starts_with("Text matches for “needle” in 2 files matching “src”: 2 in 2 files"), "{status}");
    // A click on a file folds it, on a folder folds everything in it; the main area and files stay as they were.
    let row_y = |v: &VisualTestContext, row: usize| f.view.read_with(v, |v, _| v.panel_list_y + row as f32 * line_h() + line_h() / 2.0);
    let y = row_y(&v, 1);
    v.simulate_click(point(px(120.0), px(y)), Modifiers::default());
    draw(&mut v);
    assert_eq!(labels(&mut v), ["src/  2", "a.rs  1", "b.rs  1", "1: needle"]);
    let y = row_y(&v, 0);
    v.simulate_click(point(px(120.0), px(y)), Modifiers::default());
    draw(&mut v);
    assert_eq!(labels(&mut v), ["src/  2"]);
    // Right shows the folded selection again, Left folds its file, Down moves to the next match shown.
    assert!(f.view.read_with(&v, |v, _| v.results_focus));
    v.simulate_keystrokes("right");
    draw(&mut v);
    assert_eq!(labels(&mut v), ["src/  2", "a.rs  1", "1: needle", "b.rs  1", "1: needle"]);
    v.simulate_keystrokes("left");
    draw(&mut v);
    assert_eq!(labels(&mut v).len(), 4);
    v.simulate_keystrokes("down");
    assert_eq!(f.view.read_with(&v, |v, _| v.session().state().results().unwrap().selected()), Some(1));
    // Left twice folds the selection's file, then its folder.
    v.simulate_keystrokes("left left");
    draw(&mut v);
    assert_eq!(labels(&mut v), ["src/  2"]);
    v.simulate_keystrokes("right");
    draw(&mut v);
    assert_eq!(labels(&mut v), ["src/  2", "a.rs  1", "b.rs  1", "1: needle"], "Right shows the selection, a.rs stays folded");
    // The search box opens again with the same files.
    v.simulate_keystrokes("ctrl-shift-f");
    assert_eq!(f.view.read_with(&v, |v, _| v.modal_rows()[1].0.clone()), "Files › src");
    v.simulate_keystrokes("escape");
    assert!(f.view.read_with(&v, |v, _| v.session().state().current_source().is_none()));
    for name in ["top.rs", "src/a.rs", "src/b.rs", "web/c.ts"] {
        assert_eq!(std::fs::read(root.join(name)).unwrap(), b"needle\n");
    }
}

#[gpui::test]
fn search_previews_in_the_panel_and_opens_on_enter(cx: &mut TestAppContext) {
    let f = fixture(cx, None);
    let root = project(f.dir.path(), &["a.rs", "b.rs", "notes.md"]);
    std::fs::write(root.join("a.rs"), "fn main() {}\r\nlet café = 1;\n").unwrap();
    std::fs::write(root.join("b.rs"), "// Café notes\n").unwrap();
    f.view.update(cx, |v, cx| v.open_root(root.clone(), cx));
    settle(cx, &f, scanned);
    f.view.update(cx, |v, cx| v.open(root.join("notes.md"), cx));
    settle(cx, &f, active_is("notes.md"));
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(900.0), px(500.0)));
    v.simulate_keystrokes("ctrl-shift-f c a f é enter");
    drop(v);
    settle(cx, &f, search_done);
    let mut v = visual(cx, &f);
    draw(&mut v);
    let status = f.view.read_with(&v, |v, _| v.results_status().unwrap());
    assert!(status.starts_with("Text matches for “café” in 3 files: 2"), "{status}");
    // A tree: the folder, each file with its count, each match's line number and text, coloured once the core has
    // made the lines' colours (asked for by showing them).
    f.view.update(&mut v, |v, _| v.panel_contents(6, 5));
    drop(v);
    settle(cx, &f, |v| v.session().state().results().unwrap().line_spans(0).is_some() && v.session().state().results().unwrap().line_spans(1).is_some());
    let mut v = visual(cx, &f);
    let (list, _) = f.view.update(&mut v, |v, _| v.panel_contents(6, 5));
    let labels: Vec<_> = list.iter().map(|r| r.label.as_str()).collect();
    assert_eq!(labels, ["proj/  2", "a.rs  1", "2: let café = 1;", "b.rs  1", "1: // Café notes"]);
    let (row, palette) = (&list[2], f.view.read_with(&v, |v, _| **v.session().state().palette()));
    let found = row.highlight.clone().unwrap();
    assert_eq!(&row.label[found.clone()], "café", "the match is emphasized on its line");
    assert_eq!(row.fg[0], (found, super::match_colors(&palette).1), "over the syntax colours");
    assert!(row.fg.iter().any(|(r, c)| &row.label[r.clone()] == "let" && *c == palette.syntax_color(lenz_content::highlight::Capture::Keyword)), "{:?}", row.fg);
    assert_ne!(super::match_colors(&palette).0, palette.found_bg);
    let a = row.hit.unwrap();
    // Arrows preview without moving the main area.
    let first = f.view.read_with(&v, |v, _| v.session().state().results().unwrap().selected());
    v.simulate_keystrokes(if first == Some(a) { "down up" } else { "down" });
    drop(v);
    settle(cx, &f, |v| v.session().state().results().unwrap().preview().is_some_and(|p| p.doc.source().ends_with("a.rs") && p.highlights().is_some()));
    assert!(f.view.read_with(cx, |v, _| v.session().state().current_source().unwrap().source().ends_with("notes.md")));
    let mut v = visual(cx, &f);
    draw(&mut v);
    let (_, preview) = f.view.update(&mut v, |v, _| v.panel_contents(5, 5));
    assert!(preview.iter().any(|r| r.text == "let café = 1;" && r.bg.len() == 1 && r.fg.len() > 1), "the preview is syntax coloured, the match emphasized");
    // A menu over the panel takes the keys first.
    v.simulate_keystrokes("f10 down escape");
    assert!(f.view.read_with(&v, |v, _| v.modal.is_none() && v.results_focus));
    // Enter opens at the match and records history; Escape then closes the panel and returns to the code.
    v.simulate_keystrokes("enter");
    drop(v);
    settle(cx, &f, active_is("a.rs"));
    let picked = f.view.read_with(cx, |v, _| { let s = v.session().state(); s.selection().map(|x| s.current_source().unwrap().copy_range(x.range()).unwrap()) });
    assert_eq!(picked.as_deref(), Some("café"));
    assert!(f.view.read_with(cx, |v, _| v.session().state().can_back()));
    let mut v = visual(cx, &f);
    v.simulate_keystrokes("escape");
    assert!(f.view.read_with(&v, |v, _| v.session().state().results().is_none() && !v.results_focus));
    for (w, h) in [(1.0, 1.0), (200.0, 90.0)] {
        v.simulate_resize(size(px(w), px(h)));
        v.simulate_keystrokes("ctrl-f x enter");
        draw(&mut v);
        v.simulate_keystrokes("end up escape");
    }
}

#[gpui::test]
fn result_rows_open_on_double_click_and_say_when_nothing_matched(cx: &mut TestAppContext) {
    let f = fixture(cx, Some(("doc.txt", "alpha\nbeta needle\ngamma\n".as_bytes())));
    settle(cx, &f, active_is("doc.txt"));
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(700.0), px(400.0)));
    v.simulate_keystrokes("ctrl-f n o n e alt-enter");
    drop(v);
    settle(cx, &f, search_done);
    assert!(f.view.read_with(cx, |v, _| v.results_status().unwrap().ends_with("no matches")));
    let mut v = visual(cx, &f);
    v.simulate_keystrokes("ctrl-f backspace backspace backspace backspace n e e d l e alt-enter");
    drop(v);
    settle(cx, &f, |v| v.session().state().results().is_some_and(|r| r.query() == "needle" && r.done()));
    let mut v = visual(cx, &f);
    draw(&mut v);
    let y = f.view.read_with(&v, |v, _| v.panel_list_y + line_h() / 2.0);
    v.simulate_event(gpui::MouseDownEvent { button: MouseButton::Left, position: point(px(40.0), px(y)), modifiers: Modifiers::default(), click_count: 2, first_mouse: false });
    drop(v);
    settle(cx, &f, |v| v.session().state().selection().is_some());
    let sel = f.view.read_with(cx, |v, _| v.session().state().selection().unwrap().range());
    assert_eq!(sel, 11..17);
}

#[gpui::test]
fn focus_moves_out_of_the_results_and_the_wheel_scrolls_them(cx: &mut TestAppContext) {
    let f = fixture(cx, None);
    let root = project(f.dir.path(), &["a.rs"]);
    std::fs::write(root.join("a.rs"), "hit\n".repeat(200)).unwrap();
    f.view.update(cx, |v, cx| v.open_root(root.clone(), cx));
    settle(cx, &f, scanned);
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(700.0), px(500.0)));
    v.simulate_keystrokes("ctrl-shift-f h i t enter");
    drop(v);
    settle(cx, &f, search_done);
    let mut v = visual(cx, &f);
    draw(&mut v);
    assert!(f.view.read_with(&v, |v, _| v.results_focus));
    let y = f.view.read_with(&v, |v, _| v.panel_list_y + 5.0);
    v.simulate_event(ScrollWheelEvent { position: point(px(100.0), px(y)), delta: ScrollDelta::Lines(point(0.0, -4.0)), ..Default::default() });
    assert_eq!(f.view.read_with(&v, |v, _| v.results_scroll), 12);
    draw(&mut v);
    assert_eq!(f.view.read_with(&v, |v, _| v.results_scroll), 12, "kept across a repaint");
    // A trackpad's small movements add up there too.
    for _ in 0..10 {
        v.simulate_event(ScrollWheelEvent { position: point(px(100.0), px(y)), delta: ScrollDelta::Pixels(point(px(0.0), px(-line_h() / 5.0))), ..Default::default() });
    }
    assert_eq!(f.view.read_with(&v, |v, _| v.results_scroll), 14);
    // The status bar below the panel takes no wheel.
    v.simulate_event(ScrollWheelEvent { position: point(px(100.0), px(500.0 - status_h() / 2.0)), delta: ScrollDelta::Lines(point(0.0, -4.0)), ..Default::default() });
    assert_eq!(f.view.read_with(&v, |v, _| v.results_scroll), 14, "the status bar does not scroll the results");
    // Focusing the tree takes the keys from the results.
    v.simulate_keystrokes("ctrl-shift-e");
    assert!(f.view.read_with(&v, |v, _| v.tree_focus && !v.results_focus));
    let before = f.view.read_with(&v, |v, _| v.session().state().results().unwrap().selected());
    v.simulate_keystrokes("down");
    assert_eq!(f.view.read_with(&v, |v, _| v.session().state().results().unwrap().selected()), before);
}

#[gpui::test]
fn outline_and_declaration_search_through_keys(cx: &mut TestAppContext) {
    let f = fixture(cx, None);
    let root = project(f.dir.path(), &["src/lib.rs", "app.py"]);
    std::fs::write(root.join("src/lib.rs"), "mod shapes {\n    pub struct Circle;\n    impl Circle { pub fn area(&self) -> f64 { 1.0 } }\n}\nfn main() {}\n").unwrap();
    std::fs::write(root.join("app.py"), "class CircleView:\n    def draw(self): pass\n").unwrap();
    f.view.update(cx, |v, cx| v.open_root(root.clone(), cx));
    settle(cx, &f, |v| scanned(v) && v.session().state().index_coverage().pending == 0 && v.session().state().index_coverage().indexed == 2);
    f.view.update(cx, |v, cx| v.open(root.join("src/lib.rs"), cx));
    settle(cx, &f, active_is("lib.rs"));
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(900.0), px(500.0)));
    // Ctrl+Shift+O: the outline, nested, labelled as syntactic candidates.
    v.simulate_keystrokes("ctrl-shift-o");
    drop(v);
    settle(cx, &f, |v| v.session().state().results().is_some_and(|r| r.done()));
    let mut v = visual(cx, &f);
    draw(&mut v);
    assert!(f.view.read_with(&v, |v, _| v.results_status().unwrap()).starts_with("Outline (syntactic candidates) of lib.rs: 5 declarations"));
    let (rows, _) = f.view.update(&mut v, |v, _| v.panel_contents(6, 3));
    let labels: Vec<String> = rows.iter().map(|r| r.label.clone()).collect();
    assert_eq!(labels, ["module shapes  :1", "  type Circle  :2", "  impl Circle  :3", "    method area  :3", "function main  :5"]);
    assert_eq!(&rows[1].label[rows[1].highlight.clone().unwrap()], "Circle");
    v.simulate_keystrokes("down down down enter");
    drop(v);
    settle(cx, &f, |v| v.session().state().selection().is_some_and(|s| { let d = v.session().state().current_source().unwrap(); d.copy_range(s.range()).as_deref() == Some("area") }));
    // Ctrl+T: declarations across the project, live as typed; Enter browses, Enter opens.
    let mut v = visual(cx, &f);
    v.simulate_keystrokes("ctrl-t c i r c l e");
    drop(v);
    settle(cx, &f, |v| v.session().state().results().is_some_and(|r| r.done() && r.query() == "circle"));
    let mut v = visual(cx, &f);
    let status = f.view.read_with(&v, |v, _| v.results_status().unwrap());
    assert!(status.starts_with("Declarations (syntactic candidates) for “circle”: 2; indexed 2 of 2 files"), "{status}");
    v.simulate_keystrokes("enter down enter");
    drop(v);
    settle(cx, &f, active_is("app.py"));
    for (w, h) in [(1.0, 1.0), (220.0, 90.0)] {
        let mut v = visual(cx, &f);
        v.simulate_resize(size(px(w), px(h)));
        v.simulate_keystrokes("ctrl-shift-o");
        draw(&mut v);
        v.simulate_keystrokes("end up escape ctrl-t x escape");
    }
}

#[gpui::test]
fn declaration_search_follows_a_coarse_checkout_through_keys(cx: &mut TestAppContext) {
    let f = fixture(cx, None);
    let root = project(f.dir.path(), &["app/a.rb"]);
    let path = root.join("app/a.rb");
    std::fs::write(&path, "class WatchAlpha; end\n").unwrap();
    f.view.update(cx, |v, cx| v.open_root(root.clone(), cx));
    settle(cx, &f, |v| scanned(v) && v.session().state().index_coverage().indexed == 1 && v.session().state().index_coverage().complete());
    let mut window = visual(cx, &f);
    window.simulate_keystrokes("ctrl-t w a t c h");
    drop(window);
    settle(cx, &f, |v| v.session().state().results().is_some_and(|r| r.done() && r.hits().len() == 1));
    std::fs::write(&path, "class WatchOmega; end\n").unwrap();
    f.view.update(cx, |v, cx| v.dispatch(Completion::Changed(reader_core::watch::Change { paths: [root.join("app")].into(), ..Default::default() }), cx));
    settle(cx, &f, |v| v.session().state().results().is_some_and(|r| r.done() && r.hits().first().is_some_and(|h| &*h.text == "WatchOmega")));
    let mut window = visual(cx, &f);
    for (w, h) in [(220., 90.), (900., 500.)] {
        window.simulate_resize(size(px(w), px(h)));
        draw(&mut window);
    }
    let (rows, _) = f.view.update(&mut window, |v, _| v.panel_contents(6, 3));
    assert!(rows.iter().any(|r| r.label.contains("WatchOmega")));
    assert!(!rows.iter().any(|r| r.label.contains("WatchAlpha")));
    window.simulate_keystrokes("enter enter");
    drop(window);
    settle(cx, &f, |v| copied_selection(v.session().state()).as_deref() == Some("WatchOmega"));
    assert_eq!(std::fs::read_to_string(path).unwrap(), "class WatchOmega; end\n");
}

#[gpui::test]
fn a_rust_trait_is_labelled_trait_in_the_outline(cx: &mut TestAppContext) {
    let f = fixture(cx, None);
    let root = project(f.dir.path(), &["src/lib.rs"]);
    std::fs::write(root.join("src/lib.rs"), "pub trait Shape {}\npub struct Circle;\n").unwrap();
    f.view.update(cx, |v, cx| v.open_root(root.clone(), cx));
    settle(cx, &f, scanned);
    f.view.update(cx, |v, cx| v.open(root.join("src/lib.rs"), cx));
    settle(cx, &f, active_is("lib.rs"));
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(900.0), px(500.0)));
    v.simulate_keystrokes("ctrl-shift-o");
    drop(v);
    settle(cx, &f, |v| v.session().state().results().is_some_and(|r| r.done()));
    let mut v = visual(cx, &f);
    draw(&mut v);
    let (rows, _) = f.view.update(&mut v, |v, _| v.panel_contents(6, 3));
    let labels: Vec<String> = rows.iter().map(|r| r.label.clone()).collect();
    assert_eq!(labels, ["trait Shape  :1", "type Circle  :2"]);
    let before = std::fs::read(root.join("src/lib.rs")).unwrap();
    assert_eq!(before, b"pub trait Shape {}\npub struct Circle;\n", "the file is unchanged");
}

#[gpui::test]
fn definitions_follow_f12_alt_f12_and_ctrl_click_but_never_a_plain_click(cx: &mut TestAppContext) {
    // A server that takes seconds to start: a request waits visibly, and Escape cancels it.
    let scripts = tempfile::tempdir().unwrap();
    let slow = scripts.path().join("slow-server");
    std::fs::write(&slow, "#!/bin/sh\nsleep 2\n").unwrap();
    std::fs::set_permissions(&slow, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let f = fixture_with(cx, None, &slow);
    let pkg = f.dir.path().join("pkg");
    std::fs::create_dir_all(pkg.join("src")).unwrap();
    std::fs::write(pkg.join("Cargo.toml"), "[package]\nname = \"pkg\"\n").unwrap();
    let main = pkg.join("src/main.rs");
    std::fs::write(&main, "fn main() { helper(); }\n").unwrap();
    f.view.update(cx, |v, cx| v.dispatch(Command::OpenRoot(pkg.clone()), cx));
    f.view.update(cx, |v, cx| v.dispatch(Command::Open(main.clone()), cx));
    settle(cx, &f, |v| v.session().state().provider().is_some_and(|p| p.status == reader_core::semantics::Status::Starting));
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(800.0), px(400.0)));
    draw(&mut v);
    let asking = |v: &mut VisualTestContext| f.view.read_with(v, |r, _| r.session().state().asking());
    let text = f.view.read_with(&v, |r, _| r.text_bounds.unwrap());
    let char_w = f.view.read_with(&v, |r, _| r.char_w);
    let x = |col: f32| px(f32::from(text.origin.x) + num_w() + 8.0 + col * char_w);
    let y = text.origin.y + px(line_h() / 2.0);
    v.simulate_keystrokes("f12");
    assert_eq!(asking(&mut v), None, "no caret yet: {:?}", f.view.read_with(&v, |r, _| r.session().disabled(CommandId::Definition)));
    v.simulate_click(point(x(14.5), y), Modifiers::default());
    v.simulate_keystrokes("f12");
    assert!(asking(&mut v).is_some_and(|a| a.starts_with("Finding the definition")));
    let status = f.view.read_with(&v, |r, _| r.status());
    assert!(status.starts_with("Finding the definition: waiting for the language server to start… (Escape cancels)"), "{status}");
    assert!(status.contains("rust-analyzer: starting…"), "{status}");
    v.simulate_keystrokes("escape");
    assert_eq!(asking(&mut v), None);
    v.simulate_keystrokes("alt-f12");
    assert!(asking(&mut v).is_some_and(|a| a.starts_with("Peeking at the definition")));
    v.simulate_keystrokes("escape");
    // A plain click places the caret and asks nothing; Ctrl-click asks about the name under the pointer.
    v.simulate_click(point(x(14.5), y), Modifiers::default());
    assert_eq!(asking(&mut v), None);
    let caret = f.view.read_with(&v, |r, _| r.session().state().selection().unwrap());
    assert_eq!((caret.anchor, caret.head), (14, 14));
    // Holding Ctrl over a name shows it as a link (the whole name); off a name, or once Ctrl is let go, nothing.
    let link = |v: &mut VisualTestContext| f.view.read_with(v, |r, _| r.link.clone());
    v.simulate_mouse_move(point(x(16.5), y), None, Modifiers::default());
    assert_eq!(link(&mut v), None);
    v.simulate_modifiers_change(Modifiers::control());
    assert_eq!(link(&mut v), Some((0, 12..18)));
    v.simulate_mouse_move(point(x(9.5), y), None, Modifiers::control());
    assert_eq!(link(&mut v), None, "a space is not a name");
    settle(cx, &f, |r| r.session().state().active().is_some_and(|t| r.session().state().view(t.id()).is_some_and(|view| view.highlights().is_some())));
    v.simulate_mouse_move(point(x(0.5), y), None, Modifiers::control());
    assert_eq!(link(&mut v), None, "a keyword leads nowhere");
    v.simulate_mouse_move(point(x(4.5), y), None, Modifiers::control());
    assert_eq!(link(&mut v), Some((0, 3..7)));
    v.simulate_mouse_move(point(x(12.2), y), None, Modifiers::control());
    assert_eq!(link(&mut v), Some((0, 12..18)));
    draw(&mut v);
    v.simulate_modifiers_change(Modifiers::default());
    assert_eq!(link(&mut v), None);
    v.simulate_click(point(x(16.5), y), Modifiers::control());
    assert!(asking(&mut v).is_some());
    assert_eq!(f.view.read_with(&v, |r, _| r.session().state().selection().unwrap().head), 16);
    v.simulate_keystrokes("escape");
    // The command menu offers both, and the workspace root: prefilled, edited, remembered.
    v.simulate_keystrokes("f10");
    let rows = f.view.read_with(&v, |r, _| r.modal_rows());
    for label in ["Go to Definition", "Peek Definition", "Set Workspace Root…"] {
        assert_eq!(rows.iter().find(|r| r.0 == label).map(|r| r.1), Some(None), "{label}");
    }
    let at = rows.iter().position(|r| r.0 == "Set Workspace Root…").unwrap();
    for _ in 0..at {
        v.simulate_keystrokes("down");
    }
    v.simulate_keystrokes("enter");
    assert_eq!(f.view.read_with(&v, |r, _| r.modal.clone()), Some(Modal::Root { path: pkg.display().to_string() }));
    v.simulate_keystrokes("backspace enter");
    let shorter = PathBuf::from(&pkg.display().to_string()[..pkg.display().to_string().len() - 1]);
    settle(cx, &f, |r| r.session().state().provider().is_some_and(|p| p.root.as_deref() == Some(shorter.as_path())));
    assert_eq!(std::fs::read_to_string(&main).unwrap(), "fn main() { helper(); }\n");
}

/// The core's fake language server (its `fake_lsp` example), built for this profile.
fn fake_lsp() -> PathBuf {
    let profile = std::env::current_exe().unwrap().parent().unwrap().parent().unwrap().to_owned();
    static BUILD: std::sync::Once = std::sync::Once::new();
    BUILD.call_once(|| {
        let mut cargo = std::process::Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
        cargo.args(["build", "-q", "-p", "reader-core", "--example", "fake_lsp"]).current_dir(env!("CARGO_MANIFEST_DIR"));
        if profile.ends_with("release") {
            cargo.arg("--release");
        }
        assert!(cargo.status().is_ok_and(|s| s.success()), "building the fake server failed");
    });
    profile.join("examples/fake_lsp")
}

#[gpui::test]
fn indexing_percentage_is_visible_in_the_footer_and_status_dialog(cx: &mut TestAppContext) {
    let f = fixture_with(cx, Some(("main.rs", b"fn main() {}\n")), &fake_lsp());
    std::fs::write(f.dir.path().join("Cargo.toml"), "[package]\nname='progress'\nversion='0.1.0'\n").unwrap();
    settle(cx, &f, |r| r.session().state().provider().is_some_and(|p| p.status == reader_core::semantics::Status::Ready));
    f.view.update(cx, |r, cx| r.dispatch(Completion::Semantic(reader_core::semantics::Event::Message {
        session: 1,
        message: serde_json::json!({"jsonrpc":"2.0", "method":"$/progress", "params":{"token":"ruby-index", "value":{"kind":"begin", "title":"Indexing Ruby files", "percentage":42}}}),
    }), cx));
    let mut v = visual(cx, &f);
    for (w, h) in [(900.0, 500.0), (480.0, 300.0)] {
        v.simulate_resize(size(px(w), px(h)));
        draw(&mut v);
        assert!(f.view.read_with(&v, |r, _| r.status_segments().0.1).contains("42%"));
        v.simulate_keystrokes("f2 enter");
        assert!(f.view.read_with(&v, |r, _| r.modal_lines().iter().any(|(_, text, _)| text.contains("42%"))));
        v.simulate_keystrokes("escape");
    }
    // An error it logs: marked first in the footer (kept when narrow), its words in the details.
    f.view.update(cx, |r, cx| r.dispatch(Completion::Semantic(reader_core::semantics::Event::Message {
        session: 1,
        message: serde_json::json!({"jsonrpc":"2.0", "method":"window/logMessage", "params":{"type":1, "message":"Error processing initialized: (Gem::ConflictError)"}}),
    }), cx));
    for (w, h) in [(900.0, 500.0), (480.0, 300.0)] {
        v.simulate_resize(size(px(w), px(h)));
        draw(&mut v);
        assert!(f.view.read_with(&v, |r, _| r.status_segments().0.1).starts_with("LSP error"));
        v.simulate_keystrokes("f2 enter");
        assert!(f.view.read_with(&v, |r, _| r.modal_lines().iter().any(|(_, text, _)| text.contains("Server error: Error processing"))));
        v.simulate_keystrokes("escape");
    }
    assert_eq!(std::fs::read(f.dir.path().join("main.rs")).unwrap(), b"fn main() {}\n");
}

#[gpui::test]
fn definition_candidates_and_peek_preview_in_the_panel_and_open_on_enter(cx: &mut TestAppContext) {
    let f = fixture_with(cx, None, &fake_lsp());
    let pkg = f.dir.path().join("pkg");
    for (name, text) in [("Cargo.toml", "[package]\nname = \"pkg\"\n"), ("src/a.rs", "fn dup() {}\n"), ("src/b.rs", "fn other() {}\nfn dup() {}\n"), ("src/main.rs", "fn main() { dup(); }\nfn single() {}\nfn call() { single(); }\n")] {
        std::fs::create_dir_all(pkg.join(name).parent().unwrap()).unwrap();
        std::fs::write(pkg.join(name), text).unwrap();
    }
    f.view.update(cx, |v, cx| v.dispatch(Command::OpenRoot(pkg.clone()), cx));
    f.view.update(cx, |v, cx| v.dispatch(Command::Open(pkg.join("src/main.rs")), cx));
    settle(cx, &f, |v| v.session().state().provider().is_some_and(|p| p.status == reader_core::semantics::Status::Ready));
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(900.0), px(500.0)));
    draw(&mut v);
    let text = f.view.read_with(&v, |r, _| r.text_bounds.unwrap());
    let char_w = f.view.read_with(&v, |r, _| r.char_w);
    let at = |col: f32, line: f32| point(px(f32::from(text.origin.x) + num_w() + 8.0 + col * char_w), text.origin.y + px(line * line_h() + line_h() / 2.0));
    // F12 on `dup`: two candidates in the panel, keys go to them, the main area stays.
    v.simulate_click(at(13.5, 0.0), Modifiers::default());
    v.simulate_keystrokes("f12");
    drop(v);
    settle(cx, &f, |r| r.session().state().results().is_some_and(|x| x.hits().len() == 2 && x.preview().is_some()));
    let mut v = visual(cx, &f);
    draw(&mut v);
    assert!(f.view.read_with(&v, |r, _| r.results_focus));
    assert_eq!(f.view.read_with(&v, |r, _| r.results_status()).as_deref(), Some("Definitions (rust-analyzer · pkg): 2 — Enter opens, Escape closes"));
    let (rows, preview) = f.view.update(&mut v, |r, _| r.panel_contents(5, 3));
    assert!(rows[0].label.contains("a.rs:1: fn dup() {}") && rows[1].label.contains("b.rs:2: fn dup() {}"), "{:?}", rows.iter().map(|r| &r.label).collect::<Vec<_>>());
    assert!(!preview.is_empty());
    assert!(f.view.read_with(&v, |r, _| r.session().state().current_source().unwrap().source().ends_with("main.rs")));
    v.simulate_keystrokes("down enter");
    drop(v);
    settle(cx, &f, |r| r.session().state().current_source().is_some_and(|d| d.source().ends_with("b.rs")));
    let mut v = visual(cx, &f);
    v.simulate_keystrokes("escape alt-left");
    drop(v);
    settle(cx, &f, |r| r.session().state().current_source().is_some_and(|d| d.source().ends_with("main.rs")) && r.session().state().results().is_none());
    // Alt+F12 on `single`: a peek previews it below; Escape closes it; the file never changes.
    let mut v = visual(cx, &f);
    draw(&mut v);
    v.simulate_click(at(13.5, 2.0), Modifiers::default());
    v.simulate_keystrokes("alt-f12");
    drop(v);
    settle(cx, &f, |r| r.session().state().results().is_some_and(|x| x.results_kind() == reader_core::search::ResultsKind::Peek && x.preview().is_some()));
    let mut v = visual(cx, &f);
    assert!(f.view.read_with(&v, |r, _| r.session().state().current_source().unwrap().source().ends_with("main.rs")));
    v.simulate_keystrokes("escape");
    assert!(f.view.read_with(&v, |r, _| r.session().state().results().is_none()));
    assert_eq!(std::fs::read_to_string(pkg.join("src/main.rs")).unwrap(), "fn main() { dup(); }\nfn single() {}\nfn call() { single(); }\n");
}

#[gpui::test]
fn usages_symbols_and_server_controls_through_keys_and_the_menu(cx: &mut TestAppContext) {
    let f = fixture_with(cx, None, &fake_lsp());
    let pkg = f.dir.path().join("pkg");
    for (name, text) in [("Cargo.toml", "[package]\nname = \"pkg\"\n"), ("src/lib.rs", "pub fn helper() {}\n"), ("src/main.rs", "fn main() { helper(); helper(); }\n")] {
        std::fs::create_dir_all(pkg.join(name).parent().unwrap()).unwrap();
        std::fs::write(pkg.join(name), text).unwrap();
    }
    f.view.update(cx, |v, cx| v.dispatch(Command::OpenRoot(pkg.clone()), cx));
    f.view.update(cx, |v, cx| v.dispatch(Command::Open(pkg.join("src/main.rs")), cx));
    settle(cx, &f, |v| v.session().state().provider().is_some_and(|p| p.status == reader_core::semantics::Status::Ready));
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(900.0), px(500.0)));
    draw(&mut v);
    let text = f.view.read_with(&v, |r, _| r.text_bounds.unwrap());
    let char_w = f.view.read_with(&v, |r, _| r.char_w);
    v.simulate_click(point(px(f32::from(text.origin.x) + num_w() + 8.0 + 13.5 * char_w), text.origin.y + px(line_h() / 2.0)), Modifiers::default());
    // Shift+F12: usages in the panel, labelled with their provider.
    v.simulate_keystrokes("shift-f12");
    drop(v);
    settle(cx, &f, |r| r.session().state().results().is_some_and(|x| x.results_kind() == reader_core::search::ResultsKind::Usages && x.hits().len() == 2));
    let mut v = visual(cx, &f);
    assert_eq!(f.view.read_with(&v, |r, _| r.results_status()).as_deref(), Some("Usages (rust-analyzer · pkg): 2 — Enter opens, Escape closes"));
    v.simulate_keystrokes("escape");
    // Ctrl+T, then Tab: the language server's symbols instead of syntactic declarations.
    v.simulate_keystrokes("ctrl-t tab h e l p");
    assert!(f.view.read_with(&v, |r, _| r.modal_rows()[1].0.starts_with("Find symbols (language server)")));
    drop(v);
    settle(cx, &f, |r| r.session().state().results().is_some_and(|x| x.results_kind() == reader_core::search::ResultsKind::Symbols && x.done() && x.hits().len() == 1));
    let mut v = visual(cx, &f);
    v.simulate_keystrokes("tab");
    drop(v);
    settle(cx, &f, |r| r.session().state().results().is_some_and(|x| x.results_kind() == reader_core::search::ResultsKind::Declarations));
    let mut v = visual(cx, &f);
    v.simulate_keystrokes("escape escape");
    // The outline's Tab: the language server's file symbols, and back.
    v.simulate_keystrokes("ctrl-shift-o");
    assert!(f.view.read_with(&v, |r, _| r.results_status()).unwrap().contains("(Tab: language server symbols)"));
    v.simulate_keystrokes("tab");
    drop(v);
    settle(cx, &f, |r| r.session().state().results().is_some_and(|x| x.results_kind() == reader_core::search::ResultsKind::FileSymbols));
    let mut v = visual(cx, &f);
    assert!(f.view.read_with(&v, |r, _| r.results_status()).unwrap().starts_with("File symbols (rust-analyzer · pkg) of main.rs: "));
    v.simulate_keystrokes("tab");
    assert_eq!(f.view.read_with(&v, |r, _| r.session().state().results().map(|x| x.results_kind())), Some(reader_core::search::ResultsKind::Outline));
    v.simulate_keystrokes("escape");
    // The menu disables it for this folder (reading continues), then enables it again.
    let run = |v: &mut VisualTestContext, label: &str| {
        v.simulate_keystrokes("f10");
        let rows = f.view.read_with(v, |r, _| r.modal_rows());
        let at = rows.iter().position(|r| r.0 == label).unwrap();
        assert_eq!(rows[at].1, None, "{label}");
        for _ in 0..at {
            v.simulate_keystrokes("down");
        }
        v.simulate_keystrokes("enter");
    };
    run(&mut v, "Disable Language Server for This Folder");
    assert!(f.view.read_with(&v, |r, _| r.status()).contains("rust-analyzer: disabled for this folder"));
    drop(v);
    settle(cx, &f, |r| r.session().disabled(CommandId::EnableServer).is_none());
    let mut v = visual(cx, &f);
    run(&mut v, "Enable Language Server for This Folder");
    drop(v);
    settle(cx, &f, |v| v.session().state().provider().is_some_and(|p| p.status == reader_core::semantics::Status::Ready));
    let mut v = visual(cx, &f);
    run(&mut v, "Restart Language Server");
    drop(v);
    settle(cx, &f, |v| v.session().state().provider().is_some_and(|p| p.status == reader_core::semantics::Status::Ready));
    assert_eq!(std::fs::read_to_string(pkg.join("src/main.rs")).unwrap(), "fn main() { helper(); helper(); }\n");
}

/// Real Pyright through real input: F12 to a definition, Shift+F12 usages, the outline's language-server
/// view. Runs only when Pyright is installed (`READER_PYRIGHT`, `READER_PYRIGHT_PATH`).
#[gpui::test]
#[ignore = "needs an installed Pyright"]
fn real_pyright_through_the_window(cx: &mut TestAppContext) {
    let path = std::env::var("READER_PYRIGHT_PATH").ok();
    let exe = std::env::var_os("READER_PYRIGHT").map(PathBuf::from).filter(|e| e.is_file());
    let Some(exe) = exe else {
        println!("SKIPPED: Pyright is not installed");
        return;
    };
    let f = fixture_providers(cx, None, &[("pyright", &exe, path.as_deref(), None)]);
    let root = tempfile::Builder::new().prefix("reader-gui-py-").tempdir().unwrap();
    let pkg = root.path().join("proj");
    for (name, text) in [("pyproject.toml", "[project]\nname = \"p\"\nversion = \"0\"\n"), ("geo.py", "def make() -> int:\n    return 1\n"), ("app.py", "from geo import make\nprint(make(), make())\n")] {
        std::fs::create_dir_all(pkg.join(name).parent().unwrap()).unwrap();
        std::fs::write(pkg.join(name), text).unwrap();
    }
    f.view.update(cx, |v, cx| v.dispatch(Command::OpenRoot(pkg.clone()), cx));
    f.view.update(cx, |v, cx| v.dispatch(Command::Open(pkg.join("app.py")), cx));
    for _ in 0..60 {
        if f.view.read_with(cx, |v, _| v.session().state().provider().is_some_and(|p| p.status == reader_core::semantics::Status::Ready)) {
            break;
        }
        settle(cx, &f, |v| v.session().state().provider().is_some());
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(900.0), px(500.0)));
    draw(&mut v);
    let text = f.view.read_with(&v, |r, _| r.text_bounds.unwrap());
    let char_w = f.view.read_with(&v, |r, _| r.char_w);
    let at = |col: f32, line: f32| point(px(f32::from(text.origin.x) + num_w() + 8.0 + col * char_w), text.origin.y + px(line * line_h() + line_h() / 2.0));
    v.simulate_click(at(7.5, 1.0), Modifiers::control());
    drop(v);
    settle_for(cx, &f, 30, &|r| r.session().state().current_source().is_some_and(|d| d.source().ends_with("geo.py")) && r.session().state().selection().is_some());
    let mut v = visual(cx, &f);
    v.simulate_keystrokes("shift-f12");
    drop(v);
    settle_for(cx, &f, 30, &|r| r.session().state().results().is_some_and(|x| x.results_kind() == reader_core::search::ResultsKind::Usages && x.hits().len() == 3));
    let mut v = visual(cx, &f);
    assert!(f.view.read_with(&v, |r, _| r.results_status()).unwrap().starts_with("Usages (Pyright · proj): 3"));
    v.simulate_keystrokes("escape ctrl-shift-o tab");
    drop(v);
    settle_for(cx, &f, 30, &|r| r.session().state().results().is_some_and(|x| x.results_kind() == reader_core::search::ResultsKind::FileSymbols && !x.hits().is_empty()));
    assert_eq!(std::fs::read_to_string(pkg.join("app.py")).unwrap(), "from geo import make\nprint(make(), make())\n");
}


/// Real typescript-language-server through real input: Ctrl-click in TSX lands in TypeScript, Shift+F12 lists
/// usages in TS and TSX. Runs only when installed (`READER_TYPESCRIPT`, `READER_TYPESCRIPT_PATH`, and
/// `READER_TSSERVER` naming a TypeScript tsserver.js when the global TypeScript has none).
#[gpui::test]
#[ignore = "needs an installed typescript-language-server"]
fn real_typescript_through_the_window(cx: &mut TestAppContext) {
    let path = std::env::var("READER_TYPESCRIPT_PATH").ok();
    let Some(exe) = std::env::var_os("READER_TYPESCRIPT").map(PathBuf::from).filter(|e| e.is_file()) else {
        println!("SKIPPED: typescript-language-server is not installed");
        return;
    };
    let settings = std::env::var("READER_TSSERVER").ok().map(|p| serde_json::json!({ "tsserver": { "path": p } }).to_string());
    let f = fixture_providers(cx, None, &[("typescript-language-server", &exe, path.as_deref(), settings.as_deref())]);
    let root = tempfile::Builder::new().prefix("reader-gui-ts-").tempdir().unwrap();
    let pkg = root.path().join("proj");
    for (name, text) in [("package.json", "{}"), ("tsconfig.json", r#"{"compilerOptions":{"jsx":"preserve","strict":true}}"#), ("geo.ts", "export function make(): number {\n  return 1;\n}\n"), ("view.tsx", "import { make } from \"./geo\";\nexport const B = () => <b>{make()}</b>;\n")] {
        std::fs::create_dir_all(pkg.join(name).parent().unwrap()).unwrap();
        std::fs::write(pkg.join(name), text).unwrap();
    }
    f.view.update(cx, |v, cx| v.dispatch(Command::OpenRoot(pkg.clone()), cx));
    f.view.update(cx, |v, cx| v.dispatch(Command::Open(pkg.join("view.tsx")), cx));
    settle_for(cx, &f, 30, &|r| r.session().state().provider().is_some_and(|p| p.status == reader_core::semantics::Status::Ready));
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(900.0), px(500.0)));
    draw(&mut v);
    let text = f.view.read_with(&v, |r, _| r.text_bounds.unwrap());
    let char_w = f.view.read_with(&v, |r, _| r.char_w);
    let at = |col: f32, line: f32| point(px(f32::from(text.origin.x) + num_w() + 8.0 + col * char_w), text.origin.y + px(line * line_h() + line_h() / 2.0));
    // `make()` inside the JSX on line 2.
    let col = "export const B = () => <b>{m".len() as f32;
    // Ctrl-click until tsserver has loaded the project (early answers stop at the import, and say so).
    let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        v.simulate_click(at(col, 1.0), Modifiers::control());
        drop(v);
        settle_for(cx, &f, 30, &|r| r.session().state().asking().is_none() && r.session().state().loading().is_none());
        if f.view.read_with(cx, |r, _| r.session().state().current_source().is_some_and(|d| d.source().ends_with("geo.ts"))) {
            break;
        }
        assert!(std::time::Instant::now() < until);
        f.view.update(cx, |r, cx| r.dispatch(Command::Open(pkg.join("view.tsx")), cx));
        settle_for(cx, &f, 10, &|r| r.session().state().current_source().is_some_and(|d| d.source().ends_with("view.tsx")));
        std::thread::sleep(std::time::Duration::from_millis(250));
        v = visual(cx, &f);
        draw(&mut v);
    }
    // Shift+F12 until the answer is complete (an early empty one says it may be incomplete).
    loop {
        let mut v = visual(cx, &f);
        v.simulate_keystrokes("shift-f12");
        drop(v);
        settle_for(cx, &f, 30, &|r| r.session().state().asking().is_none());
        if f.view.read_with(cx, |r, _| r.session().state().results().is_some_and(|x| x.results_kind() == reader_core::search::ResultsKind::Usages && x.hits().len() >= 2)) {
            break;
        }
        let (bar, panel) = f.view.read_with(cx, |r, _| (r.status(), r.results_status()));
        println!("early usages: {panel:?} / {bar}");
        assert!(std::time::Instant::now() < until && (bar.contains("may still be loading") || panel.is_some_and(|s| s.contains("may be incomplete"))), "{bar}");
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    assert!(f.view.read_with(cx, |r, _| r.results_status()).unwrap().starts_with("Usages (typescript-language-server · proj): "));
}

/// Real Eclipse JDT LS through real input: Ctrl-click on a JDK method opens its source, retrieved from the
/// server into a kept read-only tab whose title and status bar say where it came from. Runs only when installed
/// (`READER_JDTLS`, optional `READER_JDTLS_PATH`).
#[gpui::test]
#[ignore = "needs an installed jdtls and Java 21+"]
fn real_jdtls_through_the_window(cx: &mut TestAppContext) {
    let path = std::env::var("READER_JDTLS_PATH").ok();
    let Some(exe) = std::env::var_os("READER_JDTLS").map(PathBuf::from).filter(|e| e.is_file()) else {
        println!("SKIPPED: jdtls is not installed");
        return;
    };
    let f = fixture_providers(cx, None, &[("jdtls", &exe, path.as_deref(), None)]);
    let root = tempfile::Builder::new().prefix("reader-gui-java-").tempdir().unwrap();
    let pkg = root.path().join("proj");
    std::fs::create_dir_all(pkg.join("src")).unwrap();
    std::fs::write(pkg.join("src/App.java"), "public class App {\n    public static void main(String[] args) {\n        System.out.println(twice(1) + twice(2));\n    }\n\n    static int twice(int x) {\n        return x * 2;\n    }\n}\n").unwrap();
    f.view.update(cx, |v, cx| v.dispatch(Command::OpenRoot(pkg.clone()), cx));
    f.view.update(cx, |v, cx| v.dispatch(Command::Open(pkg.join("src/App.java")), cx));
    settle_for(cx, &f, 120, &|r| r.session().state().provider().is_some_and(|p| p.status == reader_core::semantics::Status::Ready));
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(1000.0), px(500.0)));
    draw(&mut v);
    let text = f.view.read_with(&v, |r, _| r.text_bounds.unwrap());
    let char_w = f.view.read_with(&v, |r, _| r.char_w);
    let col = "        System.out.pr".len() as f32;
    v.simulate_click(point(px(f32::from(text.origin.x) + num_w() + 8.0 + col * char_w), text.origin.y + px(2.0 * line_h() + line_h() / 2.0)), Modifiers::control());
    drop(v);
    settle_for(cx, &f, 60, &|r| r.session().state().asking().is_none() && r.session().state().active().is_some_and(|t| t.retrieved().is_some()));
    let mut v = visual(cx, &f);
    draw(&mut v);
    assert!(f.view.read_with(&v, |r, _| tab_titles(r)).iter().any(|t| t.starts_with("PrintStream.java (Eclipse JDT LS)")), "{:?}", f.view.read_with(&v, |r, _| tab_titles(r)));
    assert!(f.view.read_with(&v, |r, _| r.status()).contains("Read-only source retrieved from Eclipse JDT LS (jdt://contents/java.base/java.io/PrintStream.java"));
    let rows = f.view.read_with(&v, |r, _| r.visible_rows());
    assert!(rows.iter().any(|r| r.text.contains("println")), "the definition is in view");
    // Back returns to the file; the retrieved tab stays.
    v.simulate_keystrokes("alt-left");
    drop(v);
    settle_for(cx, &f, 10, &|r| r.session().state().current_source().is_some_and(|d| d.source().ends_with("App.java")));
    assert!(f.view.read_with(cx, |r, _| tab_titles(r)).iter().any(|t| t.starts_with("PrintStream.java")));
    // Shift+F12 at `twice`'s declaration: both calls, labelled with the provider and project.
    let mut v = visual(cx, &f);
    draw(&mut v);
    let col = "    static int tw".len() as f32;
    v.simulate_click(point(px(f32::from(text.origin.x) + num_w() + 8.0 + col * char_w), text.origin.y + px(5.0 * line_h() + line_h() / 2.0)), Modifiers::default());
    v.simulate_keystrokes("shift-f12");
    drop(v);
    settle_for(cx, &f, 30, &|r| r.session().state().asking().is_none() && r.session().state().results().is_some());
    assert!(f.view.read_with(cx, |r, _| r.results_status()).unwrap().starts_with("Usages (Eclipse JDT LS · proj): 2"), "{:?}", f.view.read_with(cx, |r, _| r.results_status()));
    // Ctrl+T, Tab: the server's workspace symbols.
    let mut v = visual(cx, &f);
    v.simulate_keystrokes("escape ctrl-t tab a p p"); // JDT LS lists types (methods are not workspace symbols)
    drop(v);
    settle_for(cx, &f, 30, &|r| r.session().state().asking().is_none() && r.session().state().results().is_some_and(|x| x.results_kind() == reader_core::search::ResultsKind::Symbols && !x.hits().is_empty()));
}

/// Draw, let background work (an image's texture) finish, and draw again.
fn show(v: &mut VisualTestContext) {
    draw(v);
    v.run_until_parked();
    draw(v);
}

fn png(w: u32, h: u32, rgba: [u8; 4]) -> Vec<u8> {
    use image::ImageEncoder;
    let pixels: Vec<u8> = (0..w * h).flat_map(|_| rgba).collect();
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out).write_image(&pixels, w, h, image::ExtendedColorType::Rgba8).unwrap();
    out
}

#[gpui::test]
fn images_open_in_tabs_fit_the_view_and_release_their_textures(cx: &mut TestAppContext) {
    let f = fixture(cx, None);
    let dir = f.dir.path().join("imgs");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.png"), png(40, 30, [200, 10, 10, 128])).unwrap();
    std::fs::write(dir.join("b.png"), png(5, 7, [0, 0, 255, 255])).unwrap();
    std::fs::write(dir.join("bad.png"), &png(4, 4, [0; 4])[..24]).unwrap();
    std::fs::write(dir.join("logo.svg"), r#"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"><rect width="20" height="10" fill="red"/></svg>"#).unwrap();
    std::fs::write(dir.join("notes.txt"), "plain text\n").unwrap();
    std::fs::write(dir.join("broken.svg"), "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"10\"><unclosed\n").unwrap();
    let open = |cx: &mut TestAppContext, name: &str, retain: bool| {
        let p = dir.join(name);
        f.view.update(cx, |v, cx| v.dispatch(if retain { Command::OpenRetained(p.clone()) } else { Command::Open(p.clone()) }, cx));
        settle(cx, &f, |v| v.session().state().current_source().is_some_and(|d| d.source() == p));
    };
    open(cx, "a.png", true);
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(800.0), px(500.0)));
    show(&mut v);
    assert_eq!(f.view.read_with(&v, |r, _| (r.image_shown(), r.uploads.len())), (Some((40, 30)), 1));
    assert!(f.view.read_with(&v, |r, _| r.status()).contains("PNG image, 40×30 px"));
    assert!(f.view.read_with(&v, |r, _| r.visible_rows()).iter().all(|r| r.text.is_empty()), "no text for a raster image");
    // Another image in the preview tab, then text: only the shown texture stays in the atlas.
    drop(v);
    open(cx, "b.png", false);
    let mut v = visual(cx, &f);
    show(&mut v);
    assert_eq!(f.view.read_with(&v, |r, _| (r.image_shown(), r.uploads.len())), (Some((5, 7)), 1));
    drop(v);
    open(cx, "notes.txt", false);
    let mut v = visual(cx, &f);
    show(&mut v);
    assert_eq!(f.view.read_with(&v, |r, _| (r.image_shown(), r.uploads.len())), (None, 0));
    // Back to the kept image tab: shown again.
    v.simulate_keystrokes("ctrl-tab");
    drop(v);
    settle(cx, &f, |r| r.session().state().current_source().is_some_and(|d| d.source().ends_with("a.png")));
    let mut v = visual(cx, &f);
    show(&mut v);
    assert_eq!(f.view.read_with(&v, |r, _| r.image_shown()), Some((40, 30)));
    // A broken image says why, in place of the picture.
    drop(v);
    open(cx, "bad.png", false);
    let mut v = visual(cx, &f);
    show(&mut v);
    assert!(f.view.read_with(&v, |r, _| r.unshowable_image()).is_some_and(|w| w.starts_with("This PNG image cannot be shown: ")));
    assert_eq!(f.view.read_with(&v, |r, _| r.uploads.len()), 0);
    // A broken SVG shows its source, with the reason in the status bar.
    drop(v);
    open(cx, "broken.svg", false);
    let mut v = visual(cx, &f);
    show(&mut v);
    assert_eq!(f.view.read_with(&v, |r, _| (r.image_shown(), r.unshowable_image())), (None, None));
    assert!(f.view.read_with(&v, |r, _| r.visible_rows())[0].text.starts_with("<svg"));
    assert!(f.view.read_with(&v, |r, _| r.status()).contains("SVG image, "), "{}", f.view.read_with(&v, |r, _| r.status()));
    // An SVG: rendered, or its source through the menu; the choice belongs to the tab.
    drop(v);
    open(cx, "logo.svg", false);
    let mut v = visual(cx, &f);
    show(&mut v);
    assert_eq!(f.view.read_with(&v, |r, _| r.image_shown()), Some((20, 10)));
    v.simulate_keystrokes("f10");
    let rows = f.view.read_with(&v, |r, _| r.modal_rows());
    let at = rows.iter().position(|r| r.0 == "Image Source / Rendered").unwrap();
    assert_eq!(rows[at].1, None);
    for _ in 0..at {
        v.simulate_keystrokes("down");
    }
    v.simulate_keystrokes("enter");
    show(&mut v);
    assert_eq!(f.view.read_with(&v, |r, _| (r.image_shown(), r.uploads.len())), (None, 0));
    assert!(f.view.read_with(&v, |r, _| r.visible_rows())[0].text.starts_with("<svg"));
    // Tiny and narrow windows draw without trouble.
    for (w, h) in [(1.0, 1.0), (120.0, 60.0)] {
        v.simulate_resize(size(px(w), px(h)));
        show(&mut v);
    }
    // Repeated opening and closing never accumulates textures.
    for i in 0..20 {
        drop(v);
        open(cx, if i % 2 == 0 { "a.png" } else { "b.png" }, false);
        v = visual(cx, &f);
        show(&mut v);
        assert!(f.view.read_with(&v, |r, _| r.uploads.len()) <= 1);
    }
    for name in ["a.png", "b.png", "logo.svg"] {
        assert!(std::fs::read(dir.join(name)).is_ok_and(|b| !b.is_empty()), "never written");
    }
}

#[gpui::test]
fn markdown_reads_as_a_preview_and_switches_to_source_at_the_same_place(cx: &mut TestAppContext) {
    let long = "Words that wrap in a narrow column. ".repeat(12);
    let source = format!("# Title\n\n{long}\n\n![logo](img/logo.png)\n\n```rust\nfn main() {{}}\n```\n\n{}## End\n\nLast line.\n", "- item\n".repeat(80));
    let f = fixture(cx, None);
    std::fs::create_dir_all(f.dir.path().join("img")).unwrap();
    std::fs::write(f.dir.path().join("img/logo.png"), png(30, 20, [10, 200, 10, 255])).unwrap();
    let md = f.dir.path().join("README.md");
    std::fs::write(&md, &source).unwrap();
    f.view.update(cx, |v, cx| v.dispatch(Command::Open(md.clone()), cx));
    // Opened as source; the file header offers the preview.
    settle(cx, &f, |v| matches!(v.session().state().rendering(), Some(Rendering::Source)));
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(700.0), px(400.0)));
    show(&mut v);
    assert!(f.view.read_with(&v, |r, _| r.visible_rows()).first().is_some_and(|r| r.text == "# Title"));
    assert_eq!(click_preview(&mut v, &f, 0), "Preview");
    drop(v);
    settle(cx, &f, |v| matches!(v.session().state().rendering(), Some(Rendering::Preview(p)) if p.images.is_some()));
    let mut v = visual(cx, &f);
    show(&mut v);
    show(&mut v);
    assert_eq!(f.view.read_with(&v, |r, _| r.panes.first().and_then(|g| r.preview_button(*g)).map(|(_, control)| control.label)), Some("Source"), "the button now goes back to the source");
    // The rendered text, its image drawn, no source rows.
    let rows = f.view.read_with(&v, |r, _| r.preview_rows());
    assert_eq!(rows[0], (0, "Title".to_string()));
    assert!(rows.iter().filter(|r| r.0 == 2).count() > 1, "the long paragraph wraps: {rows:?}");
    assert!(f.view.read_with(&v, |r, _| r.visible_rows()).is_empty());
    assert_eq!(f.view.read_with(&v, |r, _| (r.preview_state().map(|s| s.1), r.uploads.len())), (Some(1), 1));
    // A click focuses without selecting source; keys and the wheel scroll, reporting the reading line.
    v.simulate_click(point(px(300.0), px(200.0)), Modifiers::default());
    assert!(f.view.read_with(&v, |r, _| r.session().state().selection()).is_none());
    v.simulate_keystrokes("pagedown pagedown");
    show(&mut v);
    let top = f.view.read_with(&v, |r, _| r.session().state().top_line());
    assert!(top > 6, "{top}");
    let first = f.view.read_with(&v, |r, _| r.preview_rows()[0].0);
    assert_eq!(first, top, "the core holds the preview's first line");
    v.simulate_event(ScrollWheelEvent { delta: ScrollDelta::Lines(point(0.0, -2.0)), ..Default::default() });
    show(&mut v);
    let top = f.view.read_with(&v, |r, _| r.session().state().top_line());
    assert_eq!(f.view.read_with(&v, |r, _| r.preview_rows()[0].0), top);
    assert_eq!(f.view.read_with(&v, |r, _| r.uploads.len()), 0, "the image scrolled away: its texture is released");
    // Ctrl+Shift+V: the source at that line; and back to the preview at the same line.
    v.simulate_keystrokes("ctrl-shift-v");
    show(&mut v);
    assert_eq!(f.view.read_with(&v, |r, _| r.visible_rows()[0].number as usize), top + 1);
    assert_eq!(f.view.read_with(&v, |r, _| r.preview_state()), None);
    v.simulate_keystrokes("ctrl-shift-v");
    settle(cx, &f, |v| matches!(v.session().state().rendering(), Some(Rendering::Preview(_))));
    show(&mut v);
    show(&mut v);
    assert_eq!(f.view.read_with(&v, |r, _| r.preview_rows()[0].0), top);
    // The command menu offers it too.
    v.simulate_keystrokes("f10");
    let rows = f.view.read_with(&v, |r, _| r.modal_rows());
    assert!(rows.contains(&("Markdown Source / Preview".to_string(), None)));
    v.simulate_keystrokes("escape");
    // End then Up stays in range; light appearance and narrow or tiny windows render.
    v.simulate_keystrokes("end up");
    show(&mut v);
    assert!(f.view.read_with(&v, |r, _| r.preview_rows()).iter().any(|r| r.1 == "Last line."));
    f.view.update(&mut v, |r, cx| r.dispatch(Command::Theme(ThemeCommand::Select(ThemeSelection::from_id("light").unwrap())), cx));
    show(&mut v);
    assert!(f.view.read_with(&v, |r, _| r.reading.as_ref().is_some_and(|(p, _)| p.bg == lenz_content::theme::LIGHT.bg)));
    for (w, h) in [(1.0, 1.0), (120.0, 60.0), (320.0, 200.0)] {
        v.simulate_resize(size(px(w), px(h)));
        show(&mut v);
        show(&mut v);
        assert!(!f.view.read_with(&v, |r, _| r.preview_rows()).is_empty() || h < 100.0, "{w}×{h}");
    }
    // Another file in the preview tab: the preview and its textures go.
    drop(v);
    let txt = f.dir.path().join("notes.txt");
    std::fs::write(&txt, "plain\n").unwrap();
    f.view.update(cx, |v, cx| v.dispatch(Command::Open(txt.clone()), cx));
    settle(cx, &f, |v| v.session().state().current_source().is_some_and(|d| d.source() == txt));
    let mut v = visual(cx, &f);
    show(&mut v);
    assert_eq!(f.view.read_with(&v, |r, _| (r.preview_state(), r.uploads.len())), (None, 0));
    assert_eq!(std::fs::read_to_string(&md).unwrap(), source);
}

#[gpui::test]
fn history_returns_to_a_markdown_preview_at_its_reading_line(cx: &mut TestAppContext) {
    let source = format!("# Top\n\n{}", (0..120).map(|i| format!("Paragraph {i}.\n\n")).collect::<String>());
    let f = fixture(cx, Some(("doc.md", source.as_bytes())));
    settle(cx, &f, |v| matches!(v.session().state().rendering(), Some(Rendering::Source)));
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(700.0), px(400.0)));
    draw(&mut v);
    click_preview(&mut v, &f, 0);
    drop(v);
    settle(cx, &f, |v| matches!(v.session().state().rendering(), Some(Rendering::Preview(_))));
    let mut v = visual(cx, &f);
    show(&mut v);
    show(&mut v);
    // The default appearance is dark, and the preview is drawn with it.
    assert!(f.view.read_with(&v, |r, _| r.reading.as_ref().is_some_and(|(p, _)| lenz_content::theme::luminance(p.bg) < 0.5)));
    v.simulate_keystrokes("pagedown pagedown pagedown");
    show(&mut v);
    let line = f.view.read_with(&v, |r, _| r.session().state().top_line());
    assert!(line > 10 && f.view.read_with(&v, |r, _| r.preview_rows()[0].0) == line);
    // A resize keeps the current layout until the new width has held, then reshapes once.
    v.simulate_resize(size(px(500.0), px(400.0)));
    show(&mut v);
    assert_eq!(f.view.read_with(&v, |r, _| r.preview_width()), Some(700.0));
    std::thread::sleep(SETTLE);
    v.executor().advance_clock(SETTLE);
    show(&mut v);
    show(&mut v);
    assert_eq!(f.view.read_with(&v, |r, _| r.preview_width()), Some(500.0));
    assert_eq!(f.view.read_with(&v, |r, _| r.preview_rows()[0].0), line, "the reading line holds across the reshape");
    drop(v);
    let txt = f.dir.path().join("other.txt");
    std::fs::write(&txt, "other\n").unwrap();
    f.view.update(cx, |v, cx| v.dispatch(Command::Open(txt.clone()), cx));
    settle(cx, &f, |v| v.session().state().current_source().is_some_and(|d| d.source() == txt));
    let mut v = visual(cx, &f);
    v.simulate_keystrokes("alt-left");
    drop(v);
    settle(cx, &f, |v| matches!(v.session().state().rendering(), Some(Rendering::Preview(_))));
    let mut v = visual(cx, &f);
    show(&mut v);
    show(&mut v);
    assert_eq!(f.view.read_with(&v, |r, _| (r.session().state().top_line(), r.preview_rows()[0].0)), (line, line), "back at the same line in the preview");
}

/// VS Code's built-in HTML and CSS servers (run with `node` from `READER_NODE_PATH`) through real input: Ctrl-click
/// in a page's script after a non-ASCII line, Shift+F12, the CSS server for a stylesheet in the same folder, and
/// workspace symbols explained as unavailable in the command menu.
#[gpui::test]
#[ignore = "needs VS Code (or vscode-langservers-extracted) and Node.js"]
fn real_html_and_css_through_the_window(cx: &mut TestAppContext) {
    let Ok(path) = std::env::var("READER_NODE_PATH") else {
        println!("SKIPPED: set READER_NODE_PATH to a PATH with node");
        return;
    };
    let f = fixture_providers(cx, None, &[("vscode-html", Path::new(""), Some(&path), None), ("vscode-css", Path::new(""), Some(&path), None)]);
    let root = tempfile::Builder::new().prefix("reader-gui-web-").tempdir().unwrap();
    let site = root.path().join("site");
    std::fs::create_dir_all(&site).unwrap();
    std::fs::write(site.join("index.html"), "<!doctype html>\n<p>𝄞 é</p>\n<script>\nfunction greet(n) { return n; }\nconst é = greet(1); greet(é);\n</script>\n").unwrap();
    std::fs::write(site.join("style.css"), ":root { --accent: red; }\na { color: var(--accent); }\n").unwrap();
    f.view.update(cx, |v, cx| v.dispatch(Command::OpenRoot(site.clone()), cx));
    f.view.update(cx, |v, cx| v.dispatch(Command::Open(site.join("index.html")), cx));
    settle_for(cx, &f, 30, &|r| r.session().state().provider().is_some_and(|p| p.status == reader_core::semantics::Status::Ready));
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(900.0), px(500.0)));
    draw(&mut v);
    let text = f.view.read_with(&v, |r, _| r.text_bounds.unwrap());
    let char_w = f.view.read_with(&v, |r, _| r.char_w);
    let at = |col: f32, line: f32| point(px(f32::from(text.origin.x) + num_w() + 8.0 + col * char_w), text.origin.y + px(line * line_h() + line_h() / 2.0));
    // `greet(é)` on line 5 (after `const é = greet(1); `).
    v.simulate_click(at("const é = greet(1); gr".chars().count() as f32, 4.0), Modifiers::control());
    drop(v);
    settle_for(cx, &f, 30, &|r| r.session().state().asking().is_none() && r.session().state().loading().is_none());
    let (line, word) = f.view.read_with(cx, |r, _| {
        let s = r.session().state();
        let (doc, sel) = (s.current_source().unwrap(), s.selection().unwrap());
        (doc.line_of(sel.head), doc.copy_range(sel.range()).unwrap())
    });
    assert_eq!((line, word.as_str()), (3, "greet"), "{}", f.view.read_with(cx, |r, _| r.status()));
    let mut v = visual(cx, &f);
    v.simulate_keystrokes("shift-f12");
    drop(v);
    settle_for(cx, &f, 30, &|r| r.session().state().asking().is_none() && r.session().state().results().is_some());
    assert!(f.view.read_with(cx, |r, _| r.results_status()).unwrap().starts_with("Usages (VS Code HTML · site): 3"), "{:?}", f.view.read_with(cx, |r, _| r.results_status()));
    let mut v = visual(cx, &f);
    v.simulate_keystrokes("escape f10");
    let rows = f.view.read_with(&v, |r, _| r.modal_rows());
    assert!(rows.iter().any(|(l, why)| l.starts_with("Find Symbol") && why.is_some_and(|w| w.contains("do not provide workspace symbols"))), "{rows:?}");
    v.simulate_keystrokes("escape");
    drop(v);
    // An external edit moves the function: asked again, the new text is what it answers about.
    std::fs::write(site.join("index.html"), "<!doctype html>\n\n\n<p>𝄞 é</p>\n<script>\nfunction greet(n) { return n; }\nconst é = greet(1); greet(é);\n</script>\n").unwrap();
    settle_for(cx, &f, 10, &|r| r.session().state().current_source().is_some_and(|d| d.line_range(1).is_some_and(|range| range.is_empty())));
    let mut v = visual(cx, &f);
    v.simulate_keystrokes("home");
    draw(&mut v);
    v.simulate_click(at("const é = greet(1); gr".chars().count() as f32, 6.0), Modifiers::control());
    drop(v);
    settle_for(cx, &f, 30, &|r| r.session().state().asking().is_none() && r.session().state().loading().is_none());
    assert_eq!(f.view.read_with(cx, |r, _| { let s = r.session().state(); s.current_source().unwrap().line_of(s.selection().unwrap().head) }), 5);
    // A workspace root set for the page: its server restarts there and still answers.
    let sub = site.join("pages");
    std::fs::create_dir_all(&sub).unwrap();
    f.view.update(cx, |v, cx| v.dispatch(Command::SetWorkspaceRoot(Some(site.clone())), cx));
    settle_for(cx, &f, 30, &|r| r.session().state().provider().is_some_and(|p| p.status == reader_core::semantics::Status::Ready && p.root.as_deref() == Some(site.as_path())));
    // The stylesheet in the same folder has its own server.
    f.view.update(cx, |v, cx| v.dispatch(Command::Open(site.join("style.css")), cx));
    settle_for(cx, &f, 30, &|r| r.session().state().provider().is_some_and(|p| p.status == reader_core::semantics::Status::Ready && p.name == "VS Code CSS"));
    let mut v = visual(cx, &f);
    draw(&mut v);
    v.simulate_click(at("a { color: var(--ac".len() as f32, 1.0), Modifiers::control());
    drop(v);
    settle_for(cx, &f, 30, &|r| r.session().state().asking().is_none());
    assert_eq!(f.view.read_with(cx, |r, _| r.session().state().selection().map(|s| s.range())), Some(8..16), "`--accent` selected");
    assert_eq!(f.view.read_with(cx, |r, _| r.session().state().servers().len()), 2);
    // Back (Alt+Left) returns to the page.
    let mut v = visual(cx, &f);
    v.simulate_keystrokes("alt-left");
    drop(v);
    settle_for(cx, &f, 10, &|r| r.session().state().current_source().is_some_and(|d| d.source().ends_with("index.html")));
    for name in ["index.html", "style.css"] {
        assert!(std::fs::read(site.join(name)).is_ok_and(|b| !b.is_empty()), "never written");
    }
}

/// Ruby LSP (`READER_RUBY_LSP`, with `READER_RUBY_LSP_PATH`) through real input: Ctrl-click a class after a
/// non-ASCII line, Shift+F12 on a method, a workspace root set from the command menu's modal, and nothing written
/// into the project.
#[gpui::test]
#[ignore = "needs an installed Ruby LSP"]
fn real_ruby_through_the_window(cx: &mut TestAppContext) {
    let path = std::env::var("READER_RUBY_LSP_PATH").ok();
    let Some(exe) = std::env::var_os("READER_RUBY_LSP").map(PathBuf::from).filter(|e| e.is_file()) else {
        println!("SKIPPED: Ruby LSP is not installed");
        return;
    };
    let f = fixture_providers(cx, None, &[("ruby-lsp", &exe, path.as_deref(), None)]);
    let root = tempfile::Builder::new().prefix("reader-gui-rb-").tempdir().unwrap();
    let proj = root.path().join("proj");
    for (name, text) in [("Gemfile", "source \"https://rubygems.org\"\n"), ("lib/shapes.rb", "module Shapes\n  class Círculo\n    def área = 1\n  end\nend\n"), ("lib/app.rb", "s = \"𝄞\"\nc = Shapes::Círculo.new\nc.área\nc.área\n")] {
        std::fs::create_dir_all(proj.join(name).parent().unwrap()).unwrap();
        std::fs::write(proj.join(name), text).unwrap();
    }
    f.view.update(cx, |v, cx| v.dispatch(Command::OpenRoot(proj.clone()), cx));
    f.view.update(cx, |v, cx| v.dispatch(Command::Open(proj.join("lib/app.rb")), cx));
    settle_for(cx, &f, 60, &|r| r.session().state().provider().is_some_and(|p| p.status == reader_core::semantics::Status::Ready));
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(900.0), px(500.0)));
    draw(&mut v);
    let text = f.view.read_with(&v, |r, _| r.text_bounds.unwrap());
    let char_w = f.view.read_with(&v, |r, _| r.char_w);
    let at = |col: f32, line: f32| point(px(f32::from(text.origin.x) + num_w() + 8.0 + col * char_w), text.origin.y + px(line * line_h() + line_h() / 2.0));
    // Ctrl-click `Círculo` on line 2 until the index has it (an early answer says it may be incomplete).
    let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        v.simulate_click(at("c = Shapes::Cír".chars().count() as f32, 1.0), Modifiers::control());
        drop(v);
        settle_for(cx, &f, 30, &|r| r.session().state().asking().is_none() && r.session().state().loading().is_none());
        if f.view.read_with(cx, |r, _| r.session().state().current_source().is_some_and(|d| d.source().ends_with("shapes.rb"))) {
            break;
        }
        assert!(std::time::Instant::now() < until, "{}", f.view.read_with(cx, |r, _| r.status()));
        f.view.update(cx, |r, cx| r.dispatch(Command::Open(proj.join("lib/app.rb")), cx));
        settle_for(cx, &f, 10, &|r| r.session().state().current_source().is_some_and(|d| d.source().ends_with("app.rb")));
        std::thread::sleep(std::time::Duration::from_millis(250));
        v = visual(cx, &f);
        draw(&mut v);
    }
    let selected = f.view.read_with(cx, |r, _| { let s = r.session().state(); s.current_source().unwrap().copy_range(s.selection().unwrap().range()).unwrap() });
    assert_eq!(selected, "Círculo");
    // Back, then Shift+F12 on `área` (line 3): the method's usages by name, its declaration included.
    let mut v = visual(cx, &f);
    v.simulate_keystrokes("alt-left");
    drop(v);
    settle_for(cx, &f, 10, &|r| r.session().state().current_source().is_some_and(|d| d.source().ends_with("app.rb")));
    let mut v = visual(cx, &f);
    draw(&mut v);
    v.simulate_click(at(3.0, 2.0), Modifiers::default());
    v.simulate_keystrokes("shift-f12");
    drop(v);
    settle_for(cx, &f, 30, &|r| r.session().state().asking().is_none() && r.session().state().results().is_some());
    assert!(f.view.read_with(cx, |r, _| r.results_status()).unwrap().starts_with("Usages (Ruby LSP · proj): 3"), "{:?}", f.view.read_with(cx, |r, _| r.results_status()));
    // The workspace root, typed into the command menu's modal: the server restarts there.
    let mut v = visual(cx, &f);
    v.simulate_keystrokes("escape");
    let root_at = f.view.read_with(&v, |r, _| r.session().actions().iter().position(|a| a.id == CommandId::WorkspaceRoot).unwrap());
    v.simulate_keystrokes("f10");
    for _ in 0..root_at {
        v.simulate_keystrokes("down");
    }
    v.simulate_keystrokes("enter");
    // Prefilled with the current root: append to it.
    assert_eq!(f.view.read_with(&v, |r, _| r.modal.clone()), Some(Modal::Root { path: proj.display().to_string() }));
    v.simulate_input("/lib");
    v.simulate_keystrokes("enter");
    drop(v);
    let lib = proj.join("lib");
    settle_for(cx, &f, 60, &|r| r.session().state().provider().is_some_and(|p| p.status == reader_core::semantics::Status::Ready && p.root.as_deref() == Some(lib.as_path())));
    assert!(!proj.join(".ruby-lsp").exists() && !proj.join("Gemfile.lock").exists() && !lib.join(".ruby-lsp").exists(), "nothing written into the project");
}

/// Kotlin LSP (`READER_KOTLIN`) on a Maven project through real input: Ctrl-click into another Kotlin file after a
/// non-ASCII line, then into Java, and Shift+F12; nothing written into the project. Needs Kotlin 2.2.21 in ~/.m2.
#[gpui::test]
#[ignore = "needs an installed Kotlin LSP and Kotlin 2.2.21 in ~/.m2"]
fn real_kotlin_through_the_window(cx: &mut TestAppContext) {
    let Some(exe) = std::env::var_os("READER_KOTLIN").map(PathBuf::from).filter(|e| e.is_file()) else {
        println!("SKIPPED: Kotlin LSP is not installed");
        return;
    };
    let m2 = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join(".m2/repository/org/jetbrains/kotlin/kotlin-maven-plugin/2.2.21");
    if !m2.is_dir() {
        println!("SKIPPED: Kotlin 2.2.21 is not in ~/.m2");
        return;
    }
    let f = fixture_providers(cx, None, &[("kotlin-lsp", &exe, None, None)]);
    let root = tempfile::Builder::new().prefix("reader-gui-kt-").tempdir().unwrap();
    let proj = root.path().join("proj");
    let pom = "<project xmlns=\"http://maven.apache.org/POM/4.0.0\"><modelVersion>4.0.0</modelVersion><groupId>f</groupId><artifactId>m</artifactId><version>1</version><dependencies><dependency><groupId>org.jetbrains.kotlin</groupId><artifactId>kotlin-stdlib</artifactId><version>2.2.21</version></dependency></dependencies><build><plugins><plugin><groupId>org.jetbrains.kotlin</groupId><artifactId>kotlin-maven-plugin</artifactId><version>2.2.21</version><configuration><sourceDirs><sourceDir>src/main/kotlin</sourceDir><sourceDir>src/main/java</sourceDir></sourceDirs></configuration></plugin></plugins></build></project>\n";
    for (name, text) in [("pom.xml", pom), ("src/main/kotlin/geo/Shapes.kt", "package geo\n\nfun make(r: Double) = r * 2\n\nfun useJava() = app.Helper.twice(2)\n"), ("src/main/kotlin/geo/Main.kt", "package geo\n\nfun main() {\n    val s = \"𝄞 é\"; println(make(2.0))\n}\n"), ("src/main/java/app/Helper.java", "package app;\n\npublic class Helper {\n    public static int twice(int x) { return x * 2; }\n}\n")] {
        std::fs::create_dir_all(proj.join(name).parent().unwrap()).unwrap();
        std::fs::write(proj.join(name), text).unwrap();
    }
    f.view.update(cx, |v, cx| v.dispatch(Command::OpenRoot(proj.clone()), cx));
    f.view.update(cx, |v, cx| v.dispatch(Command::Open(proj.join("src/main/kotlin/geo/Main.kt")), cx));
    // Its Maven import reports progress (shown as indexing): wait until it has begun and finished.
    settle_for(cx, &f, 60, &|r| r.session().state().provider().is_some_and(|p| matches!(p.status, reader_core::semantics::Status::Indexing(_))));
    settle_for(cx, &f, 120, &|r| r.session().state().provider().is_some_and(|p| p.status == reader_core::semantics::Status::Ready));
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(900.0), px(500.0)));
    draw(&mut v);
    let text = f.view.read_with(&v, |r, _| r.text_bounds.unwrap());
    let char_w = f.view.read_with(&v, |r, _| r.char_w);
    let at = |col: f32, line: f32| point(px(f32::from(text.origin.x) + num_w() + 8.0 + col * char_w), text.origin.y + px(line * line_h() + line_h() / 2.0));
    // Ctrl-click `make` on line 4 until the index has it (an early answer says it may be incomplete).
    let until = std::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        v.simulate_click(at("    val s = \"𝄞 é\"; println(ma".chars().count() as f32, 3.0), Modifiers::control());
        drop(v);
        settle_for(cx, &f, 30, &|r| r.session().state().asking().is_none() && r.session().state().loading().is_none());
        if f.view.read_with(cx, |r, _| r.session().state().current_source().is_some_and(|d| d.source().ends_with("Shapes.kt"))) {
            break;
        }
        assert!(std::time::Instant::now() < until, "{}", f.view.read_with(cx, |r, _| r.status()));
        std::thread::sleep(std::time::Duration::from_secs(2));
        v = visual(cx, &f);
        draw(&mut v);
    }
    // In Shapes.kt: Ctrl-click `twice` (line 5) into the Java file.
    let until = std::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        let mut v = visual(cx, &f);
        v.simulate_keystrokes("home");
        draw(&mut v);
        // The toolbar sets where the text starts.
        let text = f.view.read_with(&v, |r, _| r.text_bounds.unwrap());
        let at = |col: f32, line: f32| point(px(f32::from(text.origin.x) + num_w() + 8.0 + col * char_w), text.origin.y + px(line * line_h() + line_h() / 2.0));
        v.simulate_click(at("fun useJava() = app.Helper.tw".len() as f32, 4.0), Modifiers::control());
        drop(v);
        settle_for(cx, &f, 30, &|r| r.session().state().asking().is_none() && r.session().state().loading().is_none());
        if f.view.read_with(cx, |r, _| r.session().state().current_source().is_some_and(|d| d.source().ends_with("Helper.java"))) {
            break;
        }
        assert!(std::time::Instant::now() < until, "{}", f.view.read_with(cx, |r, _| r.status()));
        std::thread::sleep(std::time::Duration::from_secs(2));
    }
    // Back to Shapes.kt, Shift+F12 on `make` (line 3): its Kotlin caller.
    let mut v = visual(cx, &f);
    v.simulate_keystrokes("alt-left");
    drop(v);
    settle_for(cx, &f, 10, &|r| r.session().state().current_source().is_some_and(|d| d.source().ends_with("Shapes.kt")));
    let mut v = visual(cx, &f);
    draw(&mut v);
    let text = f.view.read_with(&v, |r, _| r.text_bounds.unwrap());
    let at = |col: f32, line: f32| point(px(f32::from(text.origin.x) + num_w() + 8.0 + col * char_w), text.origin.y + px(line * line_h() + line_h() / 2.0));
    v.simulate_click(at(5.0, 2.0), Modifiers::default());
    v.simulate_keystrokes("shift-f12");
    drop(v);
    settle_for(cx, &f, 30, &|r| r.session().state().asking().is_none() && r.session().state().results().is_some());
    assert!(f.view.read_with(cx, |r, _| r.results_status()).unwrap().starts_with("Usages (Kotlin LSP · proj): 1"), "{:?}", f.view.read_with(cx, |r, _| r.results_status()));
    let mut written = walk(&proj);
    written.sort();
    assert_eq!(written, ["pom.xml", "src/main/java/app/Helper.java", "src/main/kotlin/geo/Main.kt", "src/main/kotlin/geo/Shapes.kt"], "nothing written into the project");
}

/// Files under `root`, relative.
fn walk(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut dirs = vec![root.to_path_buf()];
    while let Some(d) = dirs.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() { dirs.push(p) } else { out.push(p.strip_prefix(root).unwrap().to_string_lossy().into_owned()) }
        }
    }
    out
}

/// rust-analyzer (`READER_RUST_ANALYZER`) on a two-crate workspace through real input: Ctrl-click across crates
/// after a non-ASCII line, Alt+F12 peek, Shift+F12, Ctrl+T then Tab for workspace symbols; nothing written into
/// the project.
#[gpui::test]
#[ignore = "needs an installed rust-analyzer"]
fn real_rust_analyzer_through_the_window(cx: &mut TestAppContext) {
    let Some(exe) = std::env::var_os("READER_RUST_ANALYZER").map(PathBuf::from).filter(|e| e.is_file()) else {
        println!("SKIPPED: rust-analyzer is not installed");
        return;
    };
    let f = fixture_with(cx, None, &exe);
    let root = tempfile::Builder::new().prefix("reader-gui-rs-").tempdir().unwrap();
    let ws = root.path().join("ws");
    for (name, text) in [
        ("Cargo.toml", "[workspace]\nmembers = [\"geo\", \"app\"]\nresolver = \"2\"\n"),
        ("geo/Cargo.toml", "[package]\nname = \"geo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n"),
        ("geo/src/lib.rs", "pub fn make(r: f64) -> f64 {\n    r * 2.0\n}\n"),
        ("app/Cargo.toml", "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\ngeo = { path = \"../geo\" }\n"),
        ("app/src/main.rs", "fn main() {\n    let s = \"𝄞 é\"; let x = geo::make(1.0);\n    println!(\"{s} {x} {}\", geo::make(2.0));\n}\n"),
    ] {
        std::fs::create_dir_all(ws.join(name).parent().unwrap()).unwrap();
        std::fs::write(ws.join(name), text).unwrap();
    }
    assert!(std::process::Command::new("cargo").args(["generate-lockfile", "--offline"]).current_dir(&ws).status().is_ok_and(|s| s.success()));
    let before = { let mut w = walk(&ws); w.sort(); w };
    f.view.update(cx, |v, cx| v.dispatch(Command::OpenRoot(ws.clone()), cx));
    f.view.update(cx, |v, cx| v.dispatch(Command::Open(ws.join("app/src/main.rs")), cx));
    settle_for(cx, &f, 120, &|r| r.session().state().provider().is_some_and(|p| p.status == reader_core::semantics::Status::Ready));
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(900.0), px(500.0)));
    draw(&mut v);
    let char_w = f.view.read_with(&v, |r, _| r.char_w);
    let at = |v: &VisualTestContext, col: f32, line: f32| {
        let text = f.view.read_with(v, |r, _| r.text_bounds.unwrap());
        point(px(f32::from(text.origin.x) + num_w() + 8.0 + col * char_w), text.origin.y + px(line * line_h() + line_h() / 2.0))
    };
    // Ctrl-click `make` on line 2.
    let p = at(&v, "    let s = \"𝄞 é\"; let x = geo::ma".chars().count() as f32, 1.0);
    v.simulate_click(p, Modifiers::control());
    drop(v);
    settle_for(cx, &f, 30, &|r| r.session().state().asking().is_none() && r.session().state().loading().is_none());
    assert!(f.view.read_with(cx, |r, _| r.session().state().current_source().is_some_and(|d| d.source().ends_with("geo/src/lib.rs"))), "{}", f.view.read_with(cx, |r, _| r.status()));
    // Back; Alt+F12 peeks at the same target without moving.
    let mut v = visual(cx, &f);
    v.simulate_keystrokes("alt-left");
    drop(v);
    settle_for(cx, &f, 10, &|r| r.session().state().current_source().is_some_and(|d| d.source().ends_with("main.rs")));
    let mut v = visual(cx, &f);
    draw(&mut v);
    let p = at(&v, "    println!(\"{s} {x} {}\", geo::ma".len() as f32, 2.0);
    v.simulate_click(p, Modifiers::default());
    v.simulate_keystrokes("alt-f12");
    drop(v);
    settle_for(cx, &f, 30, &|r| r.session().state().asking().is_none() && r.session().state().results().is_some());
    assert!(f.view.read_with(cx, |r, _| r.session().state().current_source().is_some_and(|d| d.source().ends_with("main.rs"))), "a peek does not move");
    // Shift+F12: both calls.
    let mut v = visual(cx, &f);
    v.simulate_keystrokes("escape shift-f12");
    drop(v);
    settle_for(cx, &f, 30, &|r| r.session().state().asking().is_none() && r.session().state().results().is_some_and(|x| x.results_kind() == reader_core::search::ResultsKind::Usages));
    assert!(f.view.read_with(cx, |r, _| r.results_status()).unwrap().starts_with("Usages (rust-analyzer · ws): 2"), "{:?}", f.view.read_with(cx, |r, _| r.results_status()));
    // Ctrl+T, Tab: workspace symbols.
    let mut v = visual(cx, &f);
    v.simulate_keystrokes("escape ctrl-t tab m a k e");
    drop(v);
    settle_for(cx, &f, 30, &|r| r.session().state().asking().is_none() && r.session().state().results().is_some_and(|x| x.results_kind() == reader_core::search::ResultsKind::Symbols && !x.hits().is_empty()));
    assert!(f.view.read_with(cx, |r, _| r.session().state().results().unwrap().hits().iter().any(|h| h.text.contains("pub fn make"))));
    // Ctrl+Alt+H on `make`: its caller, main (calling it twice); Ctrl+Q: its documentation.
    let main = std::fs::read_to_string(ws.join("app/src/main.rs")).unwrap();
    let make = main.find("make(1.0)").unwrap() + 1;
    let mut v = visual(cx, &f);
    v.simulate_keystrokes("escape");
    f.view.update(&mut v, |r, cx| r.dispatch(Command::Select(Selection { anchor: make, head: make }), cx));
    v.simulate_keystrokes("ctrl-alt-h");
    drop(v);
    settle_for(cx, &f, 30, &|r| r.session().state().results().is_some_and(|x| x.results_kind() == reader_core::search::ResultsKind::Hierarchy && x.hits().len() >= 2));
    let caller = f.view.read_with(cx, |r, _| r.session().state().results().unwrap().call(1).unwrap());
    assert_eq!((caller.name.as_str(), caller.calls), ("main", 2), "{caller:?}");
    let mut v = visual(cx, &f);
    v.simulate_keystrokes("escape escape ctrl-q");
    drop(v);
    settle_for(cx, &f, 30, &|r| r.session().state().documentation().is_some());
    assert!(f.view.read_with(cx, |r, _| r.session().state().documentation().unwrap().preview.doc.source.contains("make")));
    let after = { let mut w = walk(&ws); w.sort(); w };
    assert_eq!(after, before, "nothing written into the project");
}

#[gpui::test]
fn back_and_forward_buttons_navigate_and_are_disabled_when_there_is_nowhere_to_go(cx: &mut TestAppContext) {
    let f = fixture(cx, Some(("one.txt", "1\n".repeat(80).as_bytes())));
    settle(cx, &f, active_is("one.txt"));
    let two = f.dir.path().join("two.txt");
    std::fs::write(&two, "2\n".repeat(80)).unwrap();
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(600.0), px(300.0)));
    draw(&mut v);
    let button = |v: &VisualTestContext, id: CommandId| {
        let (x0, x1, enabled) = f.view.read_with(v, |r, _| r.nav_buttons().into_iter().find(|b| b.2 == id).map(|b| (b.0, b.1, b.3)).unwrap());
        (point(px((x0 + x1) / 2.0), px(line_h() / 2.0)), enabled)
    };
    // Nothing behind or ahead yet: both disabled, and a click does nothing.
    let (back, enabled) = button(&v, CommandId::Back);
    assert!(!enabled && !button(&v, CommandId::Forward).1);
    v.simulate_click(back, Modifiers::default());
    assert!(f.view.read_with(&v, |r, _| r.session().state().current_source().unwrap().source().ends_with("one.txt")));
    // After opening another file, Back is enabled and returns; then Forward goes ahead again.
    f.view.update(&mut v, |view, cx| view.open(two.clone(), cx));
    drop(v);
    settle(cx, &f, active_is("two.txt"));
    let mut v = visual(cx, &f);
    draw(&mut v);
    let (back, enabled) = button(&v, CommandId::Back);
    assert!(enabled);
    v.simulate_click(back, Modifiers::default());
    drop(v);
    settle(cx, &f, active_is("one.txt"));
    let mut v = visual(cx, &f);
    draw(&mut v);
    let (forward, enabled) = button(&v, CommandId::Forward);
    assert!(enabled);
    // Hovering highlights it; clicking goes forward.
    v.simulate_mouse_move(forward, None, Modifiers::default());
    assert_eq!(f.view.read_with(&v, |r, _| r.nav_hover), Some(CommandId::Forward));
    v.simulate_click(forward, Modifiers::default());
    drop(v);
    settle(cx, &f, active_is("two.txt"));
    // The buttons stay usable in a narrow window, and the tabs never cover them.
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(160.0), px(200.0)));
    draw(&mut v);
    assert!(f.view.read_with(&v, |r, _| r.panes.iter().all(|p| p.bar.2 >= toolbar_h())));
    let (back, enabled) = button(&v, CommandId::Back);
    assert!(enabled);
    v.simulate_click(back, Modifiers::default());
    drop(v);
    settle(cx, &f, active_is("one.txt"));
    for name in ["one.txt", "two.txt"] {
        assert!(std::fs::read(f.dir.path().join(name)).is_ok_and(|b| !b.is_empty()));
    }
}

#[gpui::test]
fn a_folder_given_at_launch_opens_as_the_project(cx: &mut TestAppContext) {
    let f = fixture(cx, None);
    let proj = f.dir.path().join("proj");
    std::fs::create_dir_all(proj.join("src")).unwrap();
    std::fs::write(proj.join("src/a.rs"), "fn a() {}\n").unwrap();
    // A new window given the folder (with the fixture's disposable configuration): the folder is the project, its
    // tree lists the file, and no document is open.
    let config = f.view.read_with(cx, |r, _| (r.make_config)(true));
    let view = cx.new(|cx| ReaderView::new(Session::new(config), Some(proj.clone()), cx));
    for _ in 0..200 {
        cx.run_until_parked();
        view.update(cx, |v, cx| v.poll(cx));
        if view.read_with(cx, |v, _| v.session().state().project().is_some_and(|p| p.files() == 1)) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(view.read_with(cx, |v, _| v.session().state().project().map(|p| p.root().to_owned())), Some(proj.clone()));
    assert!(view.read_with(cx, |v, _| v.session().state().current_source().is_none()));
}

fn click_preview(v: &mut VisualTestContext, f: &Fixture, pane: u8) -> &'static str {
    let (button, control) = f.view.read_with(v, |r, _| r.preview_button(*r.panes.iter().find(|g| g.pane == pane).unwrap()).unwrap());
    v.simulate_click(button.center(), Modifiers::default());
    control.label
}

/// Click the toolbar button for `id` (it must be shown) and return its label before the click.
fn click_toolbar(v: &mut VisualTestContext, f: &Fixture, id: CommandId) -> &'static str {
    let (x0, x1, label) = f.view.read_with(v, |r, _| {
        let b = r.nav_buttons().into_iter().find(|b| b.2 == id).unwrap_or_else(|| panic!("no {id:?} button"));
        (b.0, b.1, b.4)
    });
    v.simulate_click(point(px((x0 + x1) / 2.0), px(line_h() / 2.0)), Modifiers::default());
    label
}

#[gpui::test]
fn the_tree_shows_folder_and_file_type_icons(cx: &mut TestAppContext) {
    let f = fixture(cx, None);
    let proj = f.dir.path().join("proj");
    for (name, text) in [("src/main.rs", "fn main() {}\n"), ("README.md", "# R\n"), ("Cargo.toml", "[package]\n"), ("web/app.ts", "export {};\n")] {
        std::fs::create_dir_all(proj.join(name).parent().unwrap()).unwrap();
        std::fs::write(proj.join(name), text).unwrap();
    }
    f.view.update(cx, |v, cx| v.dispatch(Command::OpenRoot(proj.clone()), cx));
    settle(cx, &f, |v| v.session().state().project().is_some_and(|p| p.files() == 4));
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(800.0), px(400.0)));
    draw(&mut v);
    let icon_of = |v: &VisualTestContext, name: &str| f.view.read_with(v, |r, _| r.tree_lines(usize::MAX).into_iter().find(|l| l.name == name).map(|l| (l.icon, l.expanded)));
    assert_eq!(icon_of(&v, "src"), Some((icons::folder(false), Some(false))));
    assert_eq!(icon_of(&v, "Cargo.toml").map(|i| i.0), Some(icons::file("main.rs")), "a Cargo manifest shows as Rust");
    assert_eq!(icon_of(&v, "README.md").map(|i| i.0.glyph), Some('\u{e73e}'));
    // Expanding a folder opens its icon and shows its files with theirs.
    f.view.update(&mut v, |r, cx| r.dispatch(Command::ToggleDir("src".into()), cx));
    draw(&mut v);
    assert_eq!(icon_of(&v, "src"), Some((icons::folder(true), Some(true))));
    assert_eq!(icon_of(&v, "main.rs").map(|i| i.0.glyph), Some('\u{e7a8}'));
    // The logos shown are drawn once each and uploaded; folders and plain kinds have none.
    let logos = |v: &VisualTestContext| f.view.read_with(v, |r, _| { let mut l: Vec<_> = r.logos.keys().copied().collect(); l.sort(); (l, r.logo_uploads.len()) });
    assert_eq!(logos(&v), (vec!["markdown", "rust"], 2), "web/ is closed: its TypeScript logo isn't needed yet");
    f.view.update(&mut v, |r, cx| r.dispatch(Command::ToggleDir("web".into()), cx));
    draw(&mut v);
    assert_eq!(logos(&v), (vec!["markdown", "rust", "typescript-icon"], 3));
    // Narrow and tiny windows still draw the tree.
    for (w, h) in [(1.0, 1.0), (200.0, 120.0)] {
        v.simulate_resize(size(px(w), px(h)));
        draw(&mut v);
    }
}

#[test]
fn a_match_stands_out_on_light_and_dark_themes() {
    use lenz_content::theme::{self, DARCULA, LIGHT};
    for p in [DARCULA, LIGHT] {
        let (bg, fg) = super::match_colors(&p);
        let (lb, lf) = (theme::luminance(bg), theme::luminance(fg));
        assert!((lb.max(lf) + 0.05) / (lb.min(lf) + 0.05) >= 4.5, "text on the match is readable: {bg:06x} {fg:06x}");
        assert_ne!(bg, p.found_bg, "stronger than the plain find colour");
    }
}

#[gpui::test]
fn the_explorer_hides_by_its_button_and_selects_the_opened_file_by_its_target(cx: &mut TestAppContext) {
    let f = fixture(cx, None);
    let root = project(f.dir.path(), &["src/ui/view.rs", "src/lib.rs", "a.md"]);
    f.view.update(cx, |v, cx| v.open_root(root.clone(), cx));
    settle(cx, &f, scanned);
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(900.0), px(500.0)));
    draw(&mut v);
    // Open a nested file without the tree: its folders stay closed.
    f.view.update(&mut v, |r, cx| r.dispatch(Command::Open(root.join("src/ui/view.rs")), cx));
    drop(v);
    settle(cx, &f, |v| v.session().state().current_source().is_some_and(|d| d.source().ends_with("view.rs")));
    let mut v = visual(cx, &f);
    draw(&mut v);
    let (sidebar, tabs) = f.view.read_with(&v, |r, _| (r.sidebar_w, r.tab_hits.clone()));
    assert!(sidebar > 0.0 && tabs[0].0 >= sidebar, "tabs sit right of the explorer");
    assert!(f.view.read_with(&v, |r, _| r.panes[0].bar.2 >= toolbar_h()), "toolbar has its own row");
    // The target: the file's folders open and its row is selected.
    let (x0, x1, enabled) = f.view.read_with(&v, |r, _| r.explorer_buttons().into_iter().find(|b| b.2 == CommandId::LocateFile).map(|b| (b.0, b.1, b.3)).unwrap());
    assert!(enabled && x1 <= sidebar);
    let y = toolbar_h() + line_h() / 2.0;
    v.simulate_click(point(px((x0 + x1) / 2.0), px(y)), Modifiers::default());
    draw(&mut v);
    let (rows, sel) = f.view.read_with(&v, |r, _| (r.session().state().project().unwrap().rows().iter().map(|r| r.path.to_string()).collect::<Vec<_>>(), r.tree_sel.clone()));
    assert_eq!(rows, ["src", "src/ui", "src/ui/view.rs", "src/lib.rs", "a.md"]);
    assert_eq!(sel.as_deref(), Some("src/ui/view.rs"));
    assert!(f.view.read_with(&v, |r, _| r.tree_lines(r.tree_at())[2].selected));
    // The switch hides the explorer: the files panel takes the width; again shows it.
    click_toolbar(&mut v, &f, CommandId::ToggleExplorer);
    draw(&mut v);
    let (sidebar, first) = f.view.read_with(&v, |r, _| (r.sidebar_w, r.nav_buttons()[0].0));
    assert_eq!(sidebar, 0.0);
    assert_eq!(first, f.view.read_with(&v, |r, _| f32::from(r.app_menu_button().right()) + 6.0));
    assert!(f.view.read_with(&v, |r, _| r.explorer_buttons().is_empty()));
    click_toolbar(&mut v, &f, CommandId::ToggleExplorer);
    draw(&mut v);
    assert!(f.view.read_with(&v, |r, _| r.sidebar_w) > 0.0);
    // The order switch (a command-menu entry): all by name.
    let view = f.view.clone();
    v.update(|window, cx| view.update(cx, |r, cx| r.run(CommandId::AllByName, window, cx)));
    let rows = f.view.read_with(&v, |r, _| r.session().state().project().unwrap().rows().iter().map(|r| r.path.to_string()).collect::<Vec<_>>());
    assert_eq!(rows, ["a.md", "src", "src/lib.rs", "src/ui", "src/ui/view.rs"]);
}

fn pane_titles(v: &ReaderView, pane: u8) -> Vec<String> {
    v.session().state().pane_tabs(pane).map(|t| t.title()).collect()
}

/// Press at `from`, move with the button held to `to`, release there.
fn drag(v: &mut VisualTestContext, from: (f32, f32), to: (f32, f32)) {
    v.simulate_mouse_down(point(px(from.0), px(from.1)), MouseButton::Left, Modifiers::default());
    v.simulate_mouse_move(point(px((from.0 + to.0) / 2.0), px((from.1 + to.1) / 2.0)), MouseButton::Left, Modifiers::default());
    v.simulate_mouse_move(point(px(to.0), px(to.1)), MouseButton::Left, Modifiers::default());
    v.simulate_mouse_up(point(px(to.0), px(to.1)), MouseButton::Left, Modifiers::default());
    draw(v);
}

#[gpui::test]
fn panes_split_tabs_drag_between_them_and_dividers_resize(cx: &mut TestAppContext) {
    let f = fixture(cx, None);
    let root = project(f.dir.path(), &["a.rs", "b.rs", "c.rs"]);
    f.view.update(cx, |v, cx| v.open_root(root.clone(), cx));
    settle(cx, &f, scanned);
    for name in ["a.rs", "b.rs", "c.rs"] {
        f.view.update(cx, |v, cx| v.dispatch(Command::OpenRetained(root.join(name)), cx));
        settle(cx, &f, active_is(name));
    }
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(1000.0), px(600.0)));
    draw(&mut v);
    // Ctrl+\: side by side, the new pane focused and showing the same file.
    v.simulate_keystrokes("ctrl-\\");
    draw(&mut v);
    let panes = f.view.read_with(&v, |r, _| r.panes.clone());
    assert_eq!(panes.len(), 2);
    assert!(panes[1].text.origin.x > panes[0].text.origin.x && panes[0].bar.2 == panes[1].bar.2, "side by side, bars in one row");
    assert_eq!(f.view.read_with(&v, |r, _| (r.session().state().focused_pane(), pane_titles(r, 1))), (1, vec!["c.rs".to_string()]));
    // A click in the first pane's text focuses it.
    let t0 = panes[0].text;
    v.simulate_click(point(t0.origin.x + px(100.0), t0.origin.y + px(10.0)), Modifiers::default());
    assert_eq!(f.view.read_with(&v, |r, _| r.session().state().focused_pane()), 0);
    // Drag a.rs from the first bar to the end of the second: it moves there, shown and focused.
    let hit = |v: &mut VisualTestContext, name: &str| f.view.read_with(v, |r, _| {
        let id = r.session().state().tabs().iter().find(|t| t.title() == name && t.pane() == 0).unwrap().id();
        r.tab_hits.iter().find(|h| h.2 == id).copied().unwrap()
    });
    let bar_y = panes[0].bar.2 + line_h() / 2.0;
    let a = hit(&mut v, "a.rs");
    drag(&mut v, ((a.0 + a.1) / 2.0, bar_y), (panes[1].bar.1 - 20.0, bar_y));
    assert_eq!(f.view.read_with(&v, |r, _| (pane_titles(r, 0), pane_titles(r, 1))), (vec!["b.rs".to_string(), "c.rs".to_string()], vec!["c.rs".to_string(), "a.rs".to_string()]));
    assert_eq!(f.view.read_with(&v, |r, _| (r.session().state().focused_pane(), r.session().state().active().unwrap().title())), (1, "a.rs".to_string()));
    // Within a bar: c.rs before b.rs.
    let (b, c) = (hit(&mut v, "b.rs"), hit(&mut v, "c.rs"));
    let bar_y = f.view.read_with(&v, |r, _| r.panes[0].bar.2) + line_h() / 2.0;
    drag(&mut v, ((c.0 + c.1) / 2.0, bar_y), (b.0 + 4.0, bar_y));
    assert_eq!(f.view.read_with(&v, |r, _| pane_titles(r, 0)), ["c.rs", "b.rs"]);
    // The dividers: the explorer's edge and the split move, and are remembered.
    let (sidebar, mid) = f.view.read_with(&v, |r, _| (r.sidebar_w, f32::from(r.panes[0].text.origin.y) + 40.0));
    drag(&mut v, (sidebar, mid), (350.0, mid));
    assert_eq!(f.view.read_with(&v, |r, _| (r.sidebar_w, r.session().state().layout().explorer_width)), (350.0, Some(350)));
    let split = f.view.read_with(&v, |r, _| r.panes[1].text.origin.x);
    drag(&mut v, (f32::from(split) - 1.0, mid), (500.0, mid));
    let (first_w, permille) = f.view.read_with(&v, |r, _| (f32::from(r.panes[0].text.size.width), r.session().state().layout().split_permille));
    assert!((first_w - 150.0).abs() <= 2.0 && permille == Some(231), "{first_w} {permille:?}");
    // Ctrl+Shift+\: one above the other, the lower pane with its own bar; its divider moves by whole rows.
    v.simulate_keystrokes("ctrl-shift-\\");
    draw(&mut v);
    let panes = f.view.read_with(&v, |r, _| r.panes.clone());
    assert!(panes[1].bar.2 > panes[0].bar.2 && panes[1].text.origin.x == panes[0].text.origin.x);
    let rows = panes[0].rows;
    drag(&mut v, (600.0, panes[1].bar.2), (600.0, panes[1].bar.2 + 4.0 * line_h()));
    assert_eq!(f.view.read_with(&v, |r, _| r.panes[0].rows), rows + 4);
    // The results panel's top moves too.
    f.view.update(&mut v, |r, cx| r.dispatch(Command::Search { query: "rs".into(), scope: SearchIn::Project }, cx));
    draw(&mut v);
    let top = f.view.read_with(&v, |r, _| r.panel_top);
    drag(&mut v, (600.0, top), (600.0, top - 3.0 * line_h()));
    assert_eq!(f.view.read_with(&v, |r, _| r.panel_top), top - 3.0 * line_h());
    assert!(f.view.read_with(&v, |r, _| r.session().state().layout().results_permille).is_some());
    // Tiny windows keep both panes' state and paint.
    for (w, h) in [(1.0, 1.0), (180.0, 80.0), (1000.0, 600.0)] {
        v.simulate_resize(size(px(w), px(h)));
        draw(&mut v);
    }
    // A Markdown preview in the second pane is let go when the split closes.
    std::fs::write(root.join("d.md"), "# D\n").unwrap();
    f.view.update(&mut v, |r, cx| {
        r.dispatch(Command::FocusPane(1), cx);
        r.dispatch(Command::Open(root.join("d.md")), cx);
    });
    drop(v);
    settle(cx, &f, active_is("d.md"));
    f.view.update(cx, |r, cx| r.dispatch(Command::ToggleMarkdownSource, cx));
    // Its layout is made when drawn, off the UI thread.
    for _ in 0..200 {
        let mut v = visual(cx, &f);
        draw(&mut v);
        drop(v);
        settle(cx, &f, |_| true);
        if f.view.read_with(cx, |r, _| r.markdown[1].as_ref().is_some_and(|m| m.layout.is_some())) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(f.view.read_with(cx, |r, _| r.markdown[1].as_ref().is_some_and(|m| m.layout.is_some())), "the second pane shows the preview");
    let mut v = visual(cx, &f);
    // The command menu's Close Split: one pane with every tab.
    let view = f.view.clone();
    v.update(|window, cx| view.update(cx, |r, cx| r.run(CommandId::Unsplit, window, cx)));
    draw(&mut v);
    assert_eq!(f.view.read_with(&v, |r, _| (r.panes.len(), pane_titles(r, 0).len())), (1, 4));
    assert!(f.view.read_with(&v, |r, _| r.markdown[1].is_none() && r.image[1].is_none()));
}

#[gpui::test]
fn right_click_opens_the_context_menu_at_the_pointer_and_runs_entries(cx: &mut TestAppContext) {
    let f = fixture(cx, Some(("main.rs", b"fn main() {\n    helper();\n}\nfn helper() {}\n")));
    settle(cx, &f, active_is("main.rs"));
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(800.0), px(400.0)));
    draw(&mut v);
    let text = f.view.read_with(&v, |r, _| r.text_bounds.unwrap());
    let char_w = f.view.read_with(&v, |r, _| r.char_w);
    let at = |col: f32, row: f32| point(px(f32::from(text.origin.x) + num_w() + 8.0 + col * char_w), text.origin.y + px(line_h() * (row + 0.5)));
    let right = |v: &mut VisualTestContext, p| {
        v.simulate_event(gpui::MouseDownEvent { button: MouseButton::Right, position: p, modifiers: Modifiers::default(), click_count: 1, first_mouse: false });
        v.simulate_event(gpui::MouseUpEvent { button: MouseButton::Right, position: p, modifiers: Modifiers::default(), click_count: 1 });
    };
    // Select "main", then right-click inside the selection: the selection stays.
    v.simulate_click(at(3.2, 0.0), Modifiers::default());
    v.simulate_keystrokes("shift-right shift-right");
    right(&mut v, at(4.2, 0.0));
    let sel = f.view.read_with(&v, |r, _| r.session().state().selection().unwrap());
    assert_eq!((sel.anchor, sel.head), (3, 5), "a right-click inside the selection keeps it");
    assert!(matches!(f.view.read_with(&v, |r, _| r.modal.clone()), Some(Modal::Context { .. })));
    let rows = f.view.read_with(&v, |r, _| r.modal_rows());
    assert_eq!(rows.first().map(|r| r.0.as_str()), Some("Go to Definition"));
    assert_eq!(rows.iter().find(|r| r.0 == "Copy").map(|r| r.1), Some(None), "copying the selection is offered");
    draw(&mut v);
    // Escape closes; a right-click elsewhere moves the caret first; Down…Enter runs the highlighted entry.
    v.simulate_keystrokes("escape");
    assert_eq!(f.view.read_with(&v, |r, _| r.modal.clone()), None);
    right(&mut v, at(6.5, 1.0));
    assert_eq!(f.view.read_with(&v, |r, _| r.session().state().selection().unwrap().head), "fn main() {\n    he".len());
    let index = rows.iter().position(|r| r.0 == "Copy Path:Line").unwrap();
    let downs = index - rows[..index].iter().filter(|r| r.0 == "—").count();
    for _ in 0..downs {
        v.simulate_keystrokes("down");
    }
    v.simulate_keystrokes("enter");
    assert_eq!(f.view.read_with(&v, |r, _| r.modal.clone()), None);
    assert!(v.read_from_clipboard().and_then(|c| c.text()).is_some_and(|t| t.ends_with("main.rs:2")), "{:?}", v.read_from_clipboard());
    // A click on an entry runs it; a click outside the menu closes it without running anything.
    right(&mut v, at(6.5, 1.0));
    draw(&mut v);
    let (menu, entry_y) = f.view.read_with(&v, |r, _| (r.context_bounds().unwrap(), r.context_row_y("Copy Path").unwrap()));
    v.simulate_click(point(menu.origin.x + px(10.0), px(entry_y + line_h() / 2.0)), Modifiers::default());
    assert!(v.read_from_clipboard().and_then(|c| c.text()).is_some_and(|t| t.ends_with("main.rs")));
    right(&mut v, at(6.5, 1.0));
    v.simulate_click(point(px(790.0), px(390.0)), Modifiers::default());
    assert_eq!(f.view.read_with(&v, |r, _| r.modal.clone()), None);
    // Split: a right-click in the unfocused pane focuses it first, then opens its menu.
    v.simulate_keystrokes("ctrl-\\");
    draw(&mut v);
    v.simulate_keystrokes("ctrl-1");
    draw(&mut v);
    let other = f.view.read_with(&v, |r, _| r.panes.iter().find(|g| g.pane == 1).unwrap().text);
    right(&mut v, point(other.origin.x + px(num_w() + 8.0 + 4.5 * char_w), other.origin.y + px(line_h() / 2.0)));
    assert_eq!(f.view.read_with(&v, |r, _| r.session().state().focused_pane()), 1);
    assert!(matches!(f.view.read_with(&v, |r, _| r.modal.clone()), Some(Modal::Context { .. })));
}

/// Let background work run and its results arrive for `ms` milliseconds of real time (for checks that something
/// does not happen).
fn drain(cx: &mut TestAppContext, f: &Fixture, ms: u64) {
    let until = std::time::Instant::now() + std::time::Duration::from_millis(ms);
    while std::time::Instant::now() < until {
        std::thread::sleep(std::time::Duration::from_millis(10));
        f.view.update(cx, |v, cx| v.poll(cx));
        cx.run_until_parked();
    }
}

/// A Cargo package `pkg` in `f`'s folder with `files`, opened as the project with `open` shown, once the fake
/// language server has started.
fn fake_package(cx: &mut TestAppContext, f: &Fixture, files: &[(&str, &str)], open: &str) -> PathBuf {
    let pkg = f.dir.path().join("pkg");
    std::fs::create_dir_all(pkg.join("src")).unwrap();
    std::fs::write(pkg.join("Cargo.toml"), "[package]\nname = \"pkg\"\n").unwrap();
    for (name, text) in files {
        std::fs::write(pkg.join(name), text).unwrap();
    }
    f.view.update(cx, |v, cx| v.dispatch(Command::OpenRoot(pkg.clone()), cx));
    f.view.update(cx, |v, cx| v.dispatch(Command::Open(pkg.join(open)), cx));
    settle(cx, f, |v| v.session().state().provider().is_some_and(|p| !p.supports.is_empty()));
    pkg
}

#[gpui::test]
fn ctrl_alt_b_and_ctrl_shift_b_ask_for_implementations_and_type_definitions(cx: &mut TestAppContext) {
    let lib = "pub trait Shape {}\npub struct Circle;\npub struct Square;\nimpl Shape for Circle {}\nimpl Shape for Square {}\n";
    let main = "fn main() {\n    let c: Circle = Circle;\n    let s: &dyn Shape = &c;\n}\n";
    let f = fixture_with(cx, None, &fake_lsp());
    let pkg = fake_package(cx, &f, &[("src/lib.rs", lib), ("src/main.rs", main)], "src/main.rs");
    let mut v = visual(cx, &f);
    let at = main.find("c: Circle").unwrap();
    f.view.update(&mut v, |r, cx| r.dispatch(Command::Select(Selection { anchor: at, head: at }), cx));
    v.simulate_keystrokes("ctrl-shift-b");
    settle(cx, &f, |r| r.session().state().current_source().is_some_and(|d| d.source().ends_with("lib.rs")));
    v.simulate_keystrokes("alt-left");
    settle(cx, &f, |r| r.session().state().current_source().is_some_and(|d| d.source().ends_with("main.rs")));
    let at = main.find("Shape =").unwrap();
    f.view.update(&mut v, |r, cx| r.dispatch(Command::Select(Selection { anchor: at, head: at }), cx));
    v.simulate_keystrokes("ctrl-alt-b");
    settle(cx, &f, |r| r.session().state().results().is_some_and(|r| r.results_kind() == reader_core::search::ResultsKind::Implementations && r.hits().len() == 2));
    assert!(f.view.read_with(&v, |r, _| r.results_focus), "keys go to the listed implementations");
    let shown = f.view.read_with(&v, |r, _| r.tree_shown);
    v.simulate_keystrokes("escape escape ctrl-b");
    assert_ne!(f.view.read_with(&v, |r, _| r.tree_shown), shown, "Ctrl+B still toggles the explorer");
    assert_eq!(std::fs::read_to_string(pkg.join("src/main.rs")).unwrap(), main);
}

#[gpui::test]
fn the_call_hierarchy_unfolds_by_keys_and_clicks_and_switches_direction(cx: &mut TestAppContext) {
    let calls = "fn main() {\n    helper(); helper();\n    other();\n}\nfn other() {\n    helper();\n}\nfn helper() {}\n";
    let f = fixture_with(cx, None, &fake_lsp());
    let pkg = fake_package(cx, &f, &[("src/main.rs", calls)], "src/main.rs");
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(900.0), px(500.0)));
    let at = calls.find("helper() {}").unwrap() + 1;
    f.view.update(&mut v, |r, cx| r.dispatch(Command::Select(Selection { anchor: at, head: at }), cx));
    v.simulate_keystrokes("ctrl-alt-h");
    settle(cx, &f, |r| r.session().state().results().is_some_and(|r| r.hits().len() == 3));
    draw(&mut v);
    let labels = |v: &mut VisualTestContext| f.view.update(v, |r, _| r.panel_contents(10, 0).0.into_iter().map(|row| row.label).collect::<Vec<_>>());
    let list = labels(&mut v);
    assert!(list[0].starts_with("▾ function helper") && list[0].contains("main.rs:8"), "{list:?}");
    assert!(list[2].starts_with("  ▸ function other") && list[2].contains("main.rs:6"), "{list:?}");
    assert!(list[1].starts_with("  ▸ function main ×2  "), "the count follows the name, before a long path: {list:?}");
    assert!(f.view.read_with(&v, |r, _| r.results_focus), "keys go to the hierarchy");
    // Keys: Down to `other`, Right unfolds (loads its level), Left folds it.
    v.simulate_keystrokes("down down right");
    settle(cx, &f, |r| r.session().state().results().is_some_and(|r| r.hits().len() == 4));
    assert!(labels(&mut v)[3].starts_with("    ▸ function main"), "{:?}", labels(&mut v));
    v.simulate_keystrokes("left");
    assert_eq!(f.view.read_with(&v, |r, _| r.session().state().results().unwrap().hits().len()), 3);
    // A click on a row's fold mark unfolds it.
    draw(&mut v);
    let row_y = f.view.read_with(&v, |r, _| r.panel_list_y);
    let char_w = f.view.read_with(&v, |r, _| r.char_w);
    v.simulate_click(point(px(8.0 + 2.0 * char_w + 2.0), px(row_y + 2.5 * line_h())), Modifiers::default());
    settle(cx, &f, |r| r.session().state().results().is_some_and(|r| r.hits().len() == 4));
    // The other way round, from the same function.
    v.simulate_keystrokes("tab");
    settle(cx, &f, |r| r.session().state().results().is_some_and(|r| r.kind().starts_with("Callees of helper")));
    assert_eq!(std::fs::read_to_string(pkg.join("src/main.rs")).unwrap(), calls);
}

#[gpui::test]
fn occurrences_tint_after_the_caret_rests(cx: &mut TestAppContext) {
    let text = "fn main() {\n    let total = 1;\n    let x = total + total;\n}\n";
    let f = fixture_with(cx, None, &fake_lsp());
    fake_package(cx, &f, &[("src/main.rs", text)], "src/main.rs");
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(800.0), px(400.0)));
    draw(&mut v);
    let tinted = |v: &mut VisualTestContext| f.view.read_with(v, |r, _| r.visible_rows().iter().map(|row| row.bg.len()).sum::<usize>());
    let at = text.find("total + ").unwrap() + 1;
    f.view.update(&mut v, |r, cx| r.dispatch(Command::Select(Selection { anchor: at, head: at }), cx));
    assert_eq!(tinted(&mut v), 0, "nothing before the caret rests");
    cx.executor().advance_clock(reader_core::session::HIGHLIGHT_REST);
    settle(cx, &f, |r| r.session().state().occurrences().is_some());
    assert_eq!(tinted(&mut v), 3);
    // Moving off the name clears them at once.
    f.view.update(&mut v, |r, cx| r.dispatch(Command::Select(Selection { anchor: 1, head: 1 }), cx));
    assert_eq!(tinted(&mut v), 0);
}

#[gpui::test]
fn documentation_opens_on_ctrl_q_and_on_rest_closes_and_lets_go(cx: &mut TestAppContext) {
    let text = "fn main() { helper(); }\nfn helper() {}\n";
    let f = fixture_with(cx, None, &fake_lsp());
    let pkg = fake_package(cx, &f, &[("src/main.rs", text)], "src/main.rs");
    settle(cx, &f, |r| r.session().state().active().is_some_and(|t| r.session().state().view(t.id()).is_some_and(|view| view.highlights().is_some())));
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(800.0), px(400.0)));
    draw(&mut v);
    let tb = f.view.read_with(&v, |r, _| r.text_bounds.unwrap());
    let char_w = f.view.read_with(&v, |r, _| r.char_w);
    let at = |col: f32| point(px(f32::from(tb.origin.x) + num_w() + 8.0 + col * char_w), tb.origin.y + px(line_h() / 2.0));
    let shown = |v: &mut VisualTestContext| f.view.read_with(v, |r, _| r.session().state().documentation().is_some());
    // Ctrl+Q at the caret: a popup below the name.
    v.simulate_click(at(14.5), Modifiers::default());
    v.simulate_keystrokes("ctrl-q");
    settle(cx, &f, |r| r.session().state().documentation().is_some());
    draw(&mut v);
    let b = f.view.read_with(&v, |r, _| r.doc_popup_bounds()).expect("a popup is drawn");
    assert!(b.origin.y >= tb.origin.y + px(line_h()), "below the name: {b:?}");
    v.simulate_keystrokes("escape");
    draw(&mut v);
    assert!(!shown(&mut v) && f.view.read_with(&v, |r, _| r.doc_popup_bounds().is_none()));
    // A right-click closes it before its menu opens.
    v.simulate_keystrokes("ctrl-q");
    settle(cx, &f, |r| r.session().state().documentation().is_some());
    v.simulate_event(gpui::MouseDownEvent { button: MouseButton::Right, position: at(14.5), modifiers: Modifiers::default(), click_count: 1, first_mouse: false });
    assert!(!shown(&mut v), "the menu does not open over the documentation");
    v.simulate_keystrokes("escape");
    // Resting the pointer on `main`; the caret stays.
    let caret = f.view.read_with(&v, |r, _| r.session().state().selection());
    v.simulate_mouse_move(at(4.5), None, Modifiers::default());
    cx.executor().advance_clock(DOC_REST);
    settle(cx, &f, |r| r.session().state().documentation().is_some());
    assert_eq!(f.view.read_with(&v, |r, _| r.session().state().selection()), caret);
    draw(&mut v);
    // Moving away closes it after a moment.
    v.simulate_mouse_move(at(40.0), None, Modifiers::default());
    cx.executor().advance_clock(DOC_GRACE);
    drain(cx, &f, 100);
    assert!(!shown(&mut v));
    // Not while Ctrl is held, and never once switched off.
    v.simulate_mouse_move(at(4.5), None, Modifiers::control());
    cx.executor().advance_clock(DOC_REST);
    drain(cx, &f, 300);
    assert!(!shown(&mut v));
    v.simulate_mouse_move(at(40.0), None, Modifiers::default());
    f.view.update(&mut v, |r, cx| r.dispatch(Command::SetDocumentationOnHover(false), cx));
    v.simulate_mouse_move(at(15.5), None, Modifiers::default());
    cx.executor().advance_clock(DOC_REST);
    drain(cx, &f, 300);
    assert!(!shown(&mut v));
    // Opening and closing again and again lets go of the popup's layout each time.
    for _ in 0..20 {
        v.simulate_keystrokes("ctrl-q");
        settle(cx, &f, |r| r.session().state().documentation().is_some());
        draw(&mut v);
        v.simulate_keystrokes("escape");
        draw(&mut v);
        assert!(f.view.read_with(&v, |r, _| r.doc_popup_bounds().is_none() && r.doc_popup.is_none()));
    }
    // Another tab closes it too.
    v.simulate_keystrokes("ctrl-q");
    settle(cx, &f, |r| r.session().state().documentation().is_some());
    f.view.update(&mut v, |r, cx| r.dispatch(Command::Open(pkg.join("Cargo.toml")), cx));
    settle(cx, &f, |r| r.session().state().current_source().is_some_and(|d| d.source().ends_with("Cargo.toml")));
    draw(&mut v);
    assert!(f.view.read_with(&v, |r, _| r.doc_popup_bounds().is_none()));
}

#[gpui::test]
fn documentation_on_rest_shows_in_either_pane_and_never_under_a_menu(cx: &mut TestAppContext) {
    let text = "fn main() { helper(); }\nfn helper() {}\n";
    let f = fixture_with(cx, None, &fake_lsp());
    fake_package(cx, &f, &[("src/main.rs", text)], "src/main.rs");
    settle(cx, &f, |r| r.session().state().active().is_some_and(|t| r.session().state().view(t.id()).is_some_and(|view| view.highlights().is_some())));
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(1000.0), px(400.0)));
    draw(&mut v);
    let char_w = f.view.read_with(&v, |r, _| r.char_w);
    let shown = |v: &mut VisualTestContext| f.view.read_with(v, |r, _| r.session().state().documentation().is_some());
    // A menu opened by key before the rest ends: nothing shows under it.
    let tb = f.view.read_with(&v, |r, _| r.text_bounds.unwrap());
    let at = |b: Bounds<Pixels>, col: f32| point(b.origin.x + px(num_w() + 8.0 + col * char_w), b.origin.y + px(line_h() / 2.0));
    v.simulate_mouse_move(at(tb, 4.5), None, Modifiers::default());
    v.simulate_keystrokes("f10");
    cx.executor().advance_clock(DOC_REST);
    drain(cx, &f, 300);
    assert!(!shown(&mut v), "no documentation under the command menu");
    v.simulate_keystrokes("escape");
    // Split: the pane without focus shows it too.
    v.simulate_keystrokes("ctrl-\\");
    draw(&mut v);
    v.simulate_keystrokes("ctrl-1");
    draw(&mut v);
    let other = f.view.read_with(&v, |r, _| r.panes.iter().find(|g| g.pane == 1).unwrap().text);
    v.simulate_mouse_move(at(other, 4.5), None, Modifiers::default());
    cx.executor().advance_clock(DOC_REST);
    settle(cx, &f, |r| r.session().state().documentation().is_some());
    draw(&mut v);
    let b = f.view.read_with(&v, |r, _| r.doc_popup_bounds()).expect("drawn in the other pane");
    assert!(b.origin.x >= other.origin.x, "{b:?} {other:?}");
}

#[gpui::test]
fn folds_by_keys_gutter_and_placeholder_in_both_panes(cx: &mut TestAppContext) {
    let src = "fn f() {\n    if x {\n        y();\n    }\n}\nfn g() {\n    z();\n}\n";
    let f = fixture(cx, Some(("a.rs", src.as_bytes())));
    settle(cx, &f, |v| v.session().state().active().is_some_and(|t| v.session().state().view(t.id()).is_some_and(|view| view.folds().is_some())));
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(800.0), px(400.0)));
    draw(&mut v);
    let numbers = |v: &mut VisualTestContext| f.view.read_with(v, |v, _| v.visible_rows().iter().map(|r| r.number).collect::<Vec<_>>());
    let text = f.view.read_with(&v, |v, _| v.text_bounds.unwrap());
    let row_y = |r: f32| text.origin.y + px(r * line_h() + line_h() / 2.0);
    let x0 = f32::from(text.origin.x) + num_w() + 8.0;
    // Marks on region starts; a click on fn f's mark collapses it, and its placeholder carries the closing brace.
    assert_eq!(f.view.read_with(&v, |v, _| v.fold_marks()), vec![(0, false), (1, false), (5, false)]);
    v.simulate_click(point(px(f32::from(text.origin.x) + num_w() - fold_w() / 2.0), row_y(0.0)), Modifiers::default());
    draw(&mut v);
    assert_eq!(numbers(&mut v), vec![1, 6, 7, 8, 9]);
    assert_eq!(f.view.read_with(&v, |v, _| v.visible_rows()[0].text.clone()), format!("fn f() {{{PLACEHOLDER}}}"));
    assert_eq!(f.view.read_with(&v, |v, _| v.fold_marks())[0], (0, true));
    // A click on the placeholder unfolds.
    let char_w = f.view.read_with(&v, |v, _| v.char_w);
    v.simulate_click(point(px(x0 + ("fn f() {".len() as f32 + 1.5) * char_w), row_y(0.0)), Modifiers::default());
    draw(&mut v);
    assert_eq!(numbers(&mut v).len(), 9);
    // Ctrl+- collapses at the caret (line 2, inside `if x`), Ctrl+= expands, Ctrl+Shift+- collapses all.
    v.simulate_click(point(px(x0 + 4.0), row_y(2.0)), Modifiers::default());
    v.simulate_keystrokes("ctrl--");
    draw(&mut v);
    assert_eq!(numbers(&mut v), vec![1, 2, 5, 6, 7, 8, 9]);
    v.simulate_keystrokes("ctrl-=");
    v.simulate_keystrokes("ctrl-shift--");
    draw(&mut v);
    assert_eq!(numbers(&mut v), vec![1, 6, 9]);
    // The wheel scrolls by shown rows.
    v.simulate_event(ScrollWheelEvent { delta: ScrollDelta::Lines(point(0.0, -1.0 / 3.0)), ..Default::default() });
    assert_eq!(f.view.read_with(&v, |v, _| v.session().state().top_line()), 5);
    v.simulate_keystrokes("ctrl-shift-=");
    draw(&mut v);
    assert_eq!(numbers(&mut v), vec![6, 7, 8, 9]);
    // The other pane folds on its own.
    v.simulate_keystrokes("ctrl-\\");
    v.simulate_keystrokes("ctrl-shift--");
    v.simulate_keystrokes("ctrl-1");
    draw(&mut v);
    assert_eq!(numbers(&mut v), vec![6, 7, 8, 9], "pane 0 keeps its folds open");
    // Linux reports Ctrl+Shift+- and Ctrl+Shift+= as `_` and `+` without Shift.
    v.simulate_keystrokes("ctrl-_");
    draw(&mut v);
    assert_eq!(numbers(&mut v), vec![6, 9]);
    v.simulate_keystrokes("ctrl-+");
    draw(&mut v);
    assert_eq!(numbers(&mut v), vec![6, 7, 8, 9]);
    assert_eq!(key_hint(CommandId::Collapse), Some("Ctrl+-"));
    assert_eq!(key_hint(CommandId::ExpandAll), Some("Ctrl+Shift+="));
    for (w, h) in [(1.0, 1.0), (60.0, 30.0)] {
        v.simulate_resize(size(px(w), px(h)));
        draw(&mut v);
    }
    assert_eq!(std::fs::read(f.dir.path().join("a.rs")).unwrap(), src.as_bytes());
}

#[gpui::test]
fn ctrl_w_extends_ctrl_shift_w_shrinks_and_ctrl_f4_closes(cx: &mut TestAppContext) {
    let src = "fn f() {\n    g(\"hello world\");\n}\n";
    let f = fixture(cx, Some(("a.rs", src.as_bytes())));
    settle(cx, &f, |v| v.session().state().current_source().is_some());
    let mut v = visual(cx, &f);
    let at = src.find("world").unwrap() + 1;
    f.view.update(&mut v, |v, cx| v.dispatch(Command::Select(Selection { anchor: at, head: at }), cx));
    v.simulate_keystrokes("ctrl-w");
    drop(v);
    settle(cx, &f, |v| v.session().state().selection().is_some_and(|s| !s.range().is_empty()));
    let mut v = visual(cx, &f);
    let sel = |v: &mut VisualTestContext| f.view.read_with(v, |v, _| src[v.session().state().selection().unwrap().range()].to_string());
    assert_eq!(sel(&mut v), "world");
    v.simulate_keystrokes("ctrl-w");
    assert_eq!(sel(&mut v), "hello world");
    v.simulate_keystrokes("ctrl-shift-w");
    assert_eq!(sel(&mut v), "world");
    assert_eq!(key_hint(CommandId::Close), Some("Ctrl+F4"));
    assert_eq!(key_hint(CommandId::ExtendSelection), Some("Ctrl+W"));
    assert_eq!(key_hint(CommandId::ShrinkSelection), Some("Ctrl+Shift+W"));
    v.simulate_keystrokes("ctrl-f4");
    assert!(f.view.read_with(&v, |v, _| v.session().state().active().is_none()));
}

#[gpui::test]
fn selections_reach_the_primary_selection_and_ctrl_c_copies_the_caret_line(cx: &mut TestAppContext) {
    let src = "first line\nsecond\n";
    let f = fixture(cx, Some(("a.txt", src.as_bytes())));
    settle(cx, &f, |v| v.session().state().current_source().is_some());
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(800.0), px(400.0)));
    draw(&mut v);
    let text = f.view.read_with(&v, |v, _| v.text_bounds.unwrap());
    let x0 = f32::from(text.origin.x) + num_w() + 8.0;
    let y = text.origin.y + px(line_h() / 2.0);
    v.simulate_click(point(px(x0 - 4.0), y), Modifiers::default());
    v.simulate_keystrokes("ctrl-c");
    assert_eq!(v.read_from_clipboard().and_then(|c| c.text()).as_deref(), Some("first line\n"));
    v.simulate_keystrokes("shift-end");
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    assert_eq!(v.update(|_, cx| cx.read_from_primary()).and_then(|c| c.text()).as_deref(), Some("first line"));
}

/// A window showing `name` (`src`) as its Markdown preview, laid out.
fn previewed(cx: &mut TestAppContext, name: &str, src: &[u8]) -> (Fixture, VisualTestContext) {
    let f = fixture(cx, Some((name, src)));
    settle(cx, &f, |v| matches!(v.session().state().rendering(), Some(Rendering::Source)));
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(800.0), px(500.0)));
    show(&mut v);
    v.simulate_keystrokes("ctrl-shift-v");
    drop(v);
    settle(cx, &f, |v| matches!(v.session().state().rendering(), Some(Rendering::Preview(p)) if p.images.is_some()));
    let mut v = visual(cx, &f);
    show(&mut v);
    show(&mut v);
    (f, v)
}

fn clipboard(v: &mut VisualTestContext) -> Option<String> {
    v.read_from_clipboard().and_then(|c| c.text())
}

#[gpui::test]
fn the_markdown_preview_selects_and_copies_rendered_text(cx: &mut TestAppContext) {
    let (f, mut v) = previewed(cx, "r.md", b"# Title\n\nSome **bold** words here.\n\n- one\n- two\n");
    assert!(!f.view.read_with(&v, |v, _| v.preview_rows()).is_empty());
    // Ctrl+A Ctrl+C: the whole rendered text, list markers kept, no Markdown syntax.
    v.simulate_keystrokes("ctrl-a ctrl-c");
    let all = clipboard(&mut v).unwrap();
    assert!(all.starts_with("Title\n") && all.contains("Some bold words here.") && all.contains("one") && !all.contains("**"), "{all:?}");
    // A double-click selects a word; a shift-click extends it.
    let (wx, wy) = f.view.read_with(&v, |v, _| v.preview_point_of("bold").unwrap());
    v.simulate_event(MouseDownEvent { position: point(px(wx), px(wy)), button: MouseButton::Left, modifiers: Modifiers::default(), click_count: 2, first_mouse: false });
    v.simulate_event(gpui::MouseUpEvent { position: point(px(wx), px(wy)), button: MouseButton::Left, modifiers: Modifiers::default(), click_count: 2 });
    v.simulate_keystrokes("ctrl-c");
    assert_eq!(clipboard(&mut v).as_deref(), Some("bold"));
    let (ex, ey) = f.view.read_with(&v, |v, _| v.preview_point_of("here").unwrap());
    v.simulate_click(point(px(ex + 40.0), px(ey)), Modifiers::shift());
    v.simulate_keystrokes("ctrl-c");
    assert_eq!(clipboard(&mut v).as_deref(), Some("bold words here."));
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    assert_eq!(v.update(|_, cx| cx.read_from_primary()).and_then(|c| c.text()).as_deref(), Some("bold words here."));
    // A drag across rows, then the context menu's Copy.
    let (sx, sy) = f.view.read_with(&v, |v, _| v.preview_point_of("Title").unwrap());
    let (tx, ty) = f.view.read_with(&v, |v, _| v.preview_point_of("Some").unwrap());
    v.simulate_mouse_down(point(px(sx), px(sy)), MouseButton::Left, Modifiers::default());
    v.simulate_mouse_move(point(px(tx), px(ty)), Some(MouseButton::Left), Modifiers::default());
    v.simulate_mouse_up(point(px(tx), px(ty)), MouseButton::Left, Modifiers::default());
    f.view.update_in(&mut v, |v, w, cx| v.run(CommandId::Copy, w, cx));
    assert_eq!(clipboard(&mut v).as_deref(), Some("itle\nS"), "from inside `Title` to inside `Some`");
    for (w, h) in [(1.0, 1.0), (60.0, 30.0)] {
        v.simulate_resize(size(px(w), px(h)));
        draw(&mut v);
        v.simulate_keystrokes("ctrl-a ctrl-c");
    }
}

#[gpui::test]
fn preview_selection_with_only_an_image(cx: &mut TestAppContext) {
    let (f, mut v) = previewed(cx, "i.md", b"![alt](missing.png)\n");
    let text = f.view.read_with(&v, |v, _| v.text_bounds.unwrap());
    v.simulate_mouse_down(text.origin + point(px(40.0), px(30.0)), MouseButton::Left, Modifiers::default());
    v.simulate_mouse_move(text.origin + point(px(200.0), px(60.0)), Some(MouseButton::Left), Modifiers::default());
    v.simulate_mouse_up(text.origin + point(px(200.0), px(60.0)), MouseButton::Left, Modifiers::default());
    v.simulate_keystrokes("ctrl-a ctrl-c");
    draw(&mut v);
    assert!(clipboard(&mut v).is_none_or(|c| !c.contains('!')), "rendered text only");
}

#[gpui::test]
fn focusing_the_other_pane_leaves_the_primary_selection_alone(cx: &mut TestAppContext) {
    let f = fixture(cx, Some(("a.txt", b"first line\n")));
    settle(cx, &f, |v| v.session().state().current_source().is_some());
    f.view.update(cx, |v, cx| {
        v.dispatch(Command::Select(Selection { anchor: 0, head: 5 }), cx);
        v.dispatch(Command::Split(reader_core::session::Split::Columns), cx);
        v.dispatch(Command::Select(Selection { anchor: 6, head: 10 }), cx);
    });
    cx.update(|cx| cx.write_to_primary(ClipboardItem::new_string("elsewhere".into())));
    f.view.update(cx, |v, cx| v.dispatch(Command::FocusPane(0), cx));
    assert_eq!(f.view.read_with(cx, |v, _| v.session().state().selection()), Some(Selection { anchor: 0, head: 5 }));
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    assert_eq!(cx.update(|cx| cx.read_from_primary()).and_then(|c| c.text()).as_deref(), Some("elsewhere"), "no new selection was made");
}

#[gpui::test]
fn a_window_too_short_for_the_status_bar_keeps_its_toolbar(cx: &mut TestAppContext) {
    let f = fixture(cx, None);
    let root = project(f.dir.path(), &["a.rs"]);
    f.view.update(cx, |v, cx| v.open_root(root.clone(), cx));
    settle(cx, &f, scanned);
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(600.0), px(40.0)));
    draw(&mut v);
    // Too short for the toolbar and the bar: the bar goes, the toolbar's buttons still take clicks.
    let b = f.view.read_with(&v, |r, _| r.nav_buttons().into_iter().find(|b| b.2 == CommandId::ToggleExplorer).unwrap());
    assert!(f.view.read_with(&v, |r, _| r.tree_shown));
    v.simulate_click(point(px((b.0 + b.1) / 2.0), px(line_h() - 2.0)), Modifiers::default());
    assert!(!f.view.read_with(&v, |r, _| r.tree_shown), "the click on the toolbar's lower edge reached the button");
    draw(&mut v);
}

fn find_counter(v: &VisualTestContext, f: &Fixture) -> String {
    f.view.read_with(v, |r, _| r.session().state().active().and_then(|t| t.find()).map(|x| x.counter()).unwrap_or_default())
}

fn find_settled(v: &ReaderView) -> bool {
    v.session().state().active().and_then(|t| t.find()).is_some_and(|x| !x.pending())
}

#[gpui::test]
fn ctrl_f_finds_as_you_type_with_toggles_f3_and_list_all(cx: &mut TestAppContext) {
    let f = fixture(cx, Some(("a.rs", b"Foo foo\nfood foo\n")));
    settle(cx, &f, |v| v.session().state().current_source().is_some());
    let mut v = visual(cx, &f);
    v.simulate_keystrokes("ctrl-f f o o");
    drop(v);
    settle(cx, &f, find_settled);
    let mut v = visual(cx, &f);
    draw(&mut v);
    assert_eq!(find_counter(&v, &f), "1/4");
    v.simulate_keystrokes("alt-c");
    drop(v);
    settle(cx, &f, find_settled);
    let mut v = visual(cx, &f);
    assert_eq!(find_counter(&v, &f), "1/3");
    v.simulate_keystrokes("f3 f3");
    assert_eq!(find_counter(&v, &f), "3/3");
    // A click on the W toggle.
    draw(&mut v);
    let w = f.view.read_with(&v, |r, _| r.find_box(&r.panes[0]).unwrap().toggles[1].1);
    v.simulate_click(w.center(), Modifiers::default());
    drop(v);
    settle(cx, &f, find_settled);
    let mut v = visual(cx, &f);
    assert_eq!(find_counter(&v, &f), "2/2", "the current match stays when an option changes");
    // Every match is tinted in the drawn rows.
    draw(&mut v);
    let tinted = f.view.read_with(&v, |r, _| r.tab_rows(r.session().state().active().unwrap(), 5).0.iter().map(|row| row.bg.len()).sum::<usize>());
    assert!(tinted >= 2, "{tinted}");
    v.simulate_keystrokes("alt-enter");
    drop(v);
    settle(cx, &f, |v| v.session().state().results().is_some_and(|r| r.done()));
    let mut v = visual(cx, &f);
    // The keys are still the find box's: Escape closes it.
    v.simulate_keystrokes("escape");
    assert!(f.view.read_with(&v, |r, _| r.session().state().active().unwrap().find().is_none()));
}

#[gpui::test]
fn the_find_box_fits_narrow_panes_and_keys_go_back_to_the_text_on_click(cx: &mut TestAppContext) {
    let f = fixture(cx, Some(("a.rs", b"foo\nfoo\n")));
    settle(cx, &f, |v| v.session().state().current_source().is_some());
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(180.0), px(200.0)));
    v.simulate_keystrokes("ctrl-f f o o");
    draw(&mut v);
    let b = f.view.read_with(&v, |r, _| r.find_box(&r.panes[0]).unwrap());
    assert!(b.toggles.iter().all(|(_, t)| t.size.width == px(0.0)), "toggles hidden below 200 px");
    assert!(f32::from(b.bounds.size.width) <= 180.0);
    let text = f.view.read_with(&v, |r, _| r.text_bounds.unwrap());
    v.simulate_click(point(text.origin.x + px(90.0), text.origin.y + px(50.0)), Modifiers::default());
    assert_eq!(f.view.read_with(&v, |r, _| r.find_focus), None);
    drop(v);
    settle(cx, &f, find_settled);
    let mut v = visual(cx, &f);
    v.simulate_keystrokes("f3");
    assert!(f.view.read_with(&v, |r, _| r.session().state().active().unwrap().find().is_some()), "the box stays open");
    assert_eq!(find_counter(&v, &f), "2/2", "F3 still steps");
}

#[gpui::test]
fn ctrl_g_goes_to_a_line_and_keeps_the_box_open_on_bad_input(cx: &mut TestAppContext) {
    let f = fixture(cx, Some(("a.rs", b"a\nbb\nccc\n")));
    settle(cx, &f, |v| v.session().state().current_source().is_some());
    let mut v = visual(cx, &f);
    v.simulate_keystrokes("ctrl-g x enter");
    assert!(f.view.read_with(&v, |r, _| matches!(&r.modal, Some(Modal::GoTo { error: Some(e), .. }) if e == "expected line or line:column")));
    draw(&mut v);
    v.simulate_keystrokes("backspace 3 : 2 enter");
    assert!(f.view.read_with(&v, |r, _| r.modal.is_none()));
    let s = f.view.read_with(&v, |r, _| r.session().state().selection().unwrap());
    assert_eq!(s.head, "a\nbb\n".len() + 1);
}

#[gpui::test]
fn project_search_takes_the_find_options_and_refuses_a_bad_expression(cx: &mut TestAppContext) {
    let f = fixture(cx, None);
    let root = project(f.dir.path(), &["a.rs"]);
    std::fs::write(root.join("a.rs"), "Foo foo\n").unwrap();
    f.view.update(cx, |v, cx| v.open_root(root.clone(), cx));
    settle(cx, &f, scanned);
    let mut v = visual(cx, &f);
    v.simulate_keystrokes("ctrl-shift-f ( alt-r");
    let rows = f.view.read_with(&v, |r, _| r.modal_rows().into_iter().map(|(l, _)| l).collect::<Vec<_>>());
    assert!(rows.iter().any(|l| l.contains("[.*]")), "{rows:?}");
    assert!(rows.iter().any(|l| l.starts_with("unclosed group")), "{rows:?}");
    v.simulate_keystrokes("enter");
    assert!(f.view.read_with(&v, |r, _| matches!(r.modal, Some(Modal::Search { .. })) && r.session().state().results().is_none()), "a bad expression does not search");
    v.simulate_keystrokes("backspace F o o alt-r alt-c enter");
    drop(v);
    settle(cx, &f, search_done);
    let (kind, hits) = f.view.read_with(cx, |v, _| { let r = v.session().state().results().unwrap(); (r.kind(), r.rows().len()) });
    assert_eq!(kind, "Text matches (case)");
    assert!(hits >= 1);
}

#[gpui::test]
fn alt_z_wraps_long_lines_into_unnumbered_indented_rows(cx: &mut TestAppContext) {
    let text = format!("    {}\nshort\n", "word ".repeat(60));
    let f = fixture(cx, Some(("a.txt", text.as_bytes())));
    settle(cx, &f, |v| v.session().state().current_source().is_some());
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(400.0), px(300.0)));
    draw(&mut v);
    v.simulate_keystrokes("alt-z");
    draw(&mut v);
    assert!(f.view.read_with(&v, |r, _| r.session().state().soft_wrap()));
    let rows = f.view.read_with(&v, |r, _| r.tab_rows(r.session().state().active().unwrap(), 20).0);
    assert_eq!(rows[0].number, 1);
    assert_eq!(rows[1].number, 0, "a continuation row has no number");
    assert!(rows[1].text.starts_with("    w"), "indented like its line: {:?}", rows[1].text);
    assert!(rows.iter().any(|r| r.number == 2));
    v.simulate_keystrokes("alt-z");
    draw(&mut v);
    let rows = f.view.read_with(&v, |r, _| r.tab_rows(r.session().state().active().unwrap(), 8).0);
    assert_eq!(rows[1].number, 2, "unwrapped again");
}

#[gpui::test]
fn clicks_on_wrapped_rows_land_on_their_bytes(cx: &mut TestAppContext) {
    let text = "\tab 日本語 ".repeat(20) + "\n";
    let f = fixture(cx, Some(("a.txt", text.as_bytes())));
    settle(cx, &f, |v| v.session().state().current_source().is_some());
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(300.0), px(300.0)));
    draw(&mut v);
    v.simulate_keystrokes("alt-z");
    draw(&mut v);
    let (text_b, char_w, row1) = f.view.read_with(&v, |r, _| {
        let s = r.session().state();
        let t = s.active().unwrap();
        let layout = s.view(t.id()).unwrap().layout_rows(3, s.wrap_cols(t));
        assert!(layout.pending.is_none(), "the rendered short fixture has complete shared rows");
        (r.text_bounds.unwrap(), r.char_w, layout.rows[1].clone())
    });
    assert_eq!(row1.index, 1, "the line wrapped");
    // The first character of the second row, just right of its indent.
    let x = f32::from(text_b.origin.x) + num_w() + 8.0 + (row1.indent as f32 + 0.3) * char_w;
    let y = f32::from(text_b.origin.y) + line_h() * 1.5;
    v.simulate_click(point(px(x), px(y)), Modifiers::default());
    let s = f.view.read_with(&v, |r, _| r.session().state().selection().unwrap());
    assert_eq!(s.head, row1.source.start);
}

/// A reader showing `a.rs`: `fn f` (a body taller than the window) inside `mod m`, scrolled into the body.
fn sticky_fixture(cx: &mut TestAppContext) -> (Fixture, VisualTestContext) {
    let f = fixture(cx, None);
    let path = f.dir.path().join("a.rs");
    std::fs::write(&path, format!("mod m {{\n    fn f() {{\n{}    }}\n}}\n", "        a;\n".repeat(60))).unwrap();
    f.view.update(cx, |v, cx| v.open(path.clone(), cx));
    settle(cx, &f, |v| v.session().state().active().is_some_and(|t| v.session().state().view(t.id()).is_some_and(|view| view.declarations().is_some())));
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(900.0), px(500.0)));
    draw(&mut v);
    f.view.update(&mut v, |v, cx| v.dispatch(Command::Scroll(10), cx));
    draw(&mut v);
    (f, v)
}

fn sticky_lines(f: &Fixture, v: &VisualTestContext) -> Vec<usize> {
    f.view.read_with(v, |v, _| v.sticky_hits.iter().map(|h| h.2).collect())
}

#[gpui::test]
fn sticky_headers_draw_over_the_text_and_a_click_jumps(cx: &mut TestAppContext) {
    let (f, mut v) = sticky_fixture(cx);
    assert_eq!(sticky_lines(&f, &v), [0, 1], "mod m, then fn f");
    let b = f.view.read_with(&v, |v, _| v.sticky_hits[1].0);
    v.simulate_click(b.center(), Modifiers::default());
    draw(&mut v);
    let line = f.view.read_with(&v, |v, _| {
        let s = v.session().state();
        s.current_source().unwrap().line_of(s.selection().unwrap().head)
    });
    assert_eq!(line, 1, "the caret on fn f");
}

#[gpui::test]
fn sticky_scroll_off_draws_none_and_a_tiny_window_pins_nothing(cx: &mut TestAppContext) {
    let (f, mut v) = sticky_fixture(cx);
    f.view.update(&mut v, |v, cx| v.dispatch(Command::SetStickyScroll(false), cx));
    draw(&mut v);
    assert!(sticky_lines(&f, &v).is_empty(), "off");
    f.view.update(&mut v, |v, cx| v.dispatch(Command::SetStickyScroll(true), cx));
    draw(&mut v);
    assert_eq!(sticky_lines(&f, &v), [0, 1], "on again");
    v.simulate_resize(size(px(900.0), px(line_h() * 7.0)));
    draw(&mut v);
    assert!(f.view.read_with(&v, |v, _| v.panes[0].rows) < 6, "the window leaves fewer than 6 text rows");
    assert!(sticky_lines(&f, &v).is_empty(), "a text area under 6 rows pins nothing");
}

#[gpui::test]
fn the_find_box_takes_a_click_over_a_sticky_line(cx: &mut TestAppContext) {
    let (f, mut v) = sticky_fixture(cx);
    v.simulate_keystrokes("ctrl-f");
    draw(&mut v);
    let b = f.view.read_with(&v, |v, _| v.find_box(&v.panes[0]).unwrap().bounds);
    assert!(f.view.read_with(&v, |v, _| v.sticky_hits.iter().any(|h| h.0.intersects(&b))), "the box sits over the sticky band");
    v.simulate_click(b.center(), Modifiers::default());
    draw(&mut v);
    assert_eq!(f.view.read_with(&v, |v, _| v.session().state().active().unwrap().top_line()), 10, "no jump");
}

#[gpui::test]
fn each_pane_draws_its_own_sticky_lines(cx: &mut TestAppContext) {
    let (f, mut v) = sticky_fixture(cx);
    f.view.update(&mut v, |v, cx| {
        v.dispatch(Command::Split(reader_core::session::Split::Columns), cx);
        v.dispatch(Command::Scroll(30), cx);
    });
    draw(&mut v);
    let panes: Vec<(u8, usize)> = f.view.read_with(&v, |v, _| v.sticky_hits.iter().map(|h| (h.1, h.2)).collect());
    assert_eq!(panes, [(0, 0), (0, 1), (1, 0), (1, 1)], "both panes, each from its own top line");
    let tops = f.view.read_with(&v, |v, _| [0, 1].map(|p| v.session().state().shown(p).unwrap().top_line()));
    assert_eq!(tops, [10, 30]);
}

const CRUMBS_SRC: &str = "mod m {\n    fn f() {\n        a;\n    }\n    fn g() {}\n}\n";

/// A project with `src/ui/a.rs` (CRUMBS_SRC) and `src/ui/b.rs`, `a.rs` shown with the caret in `fn f`, at `width`.
fn crumbs_fixture(cx: &mut TestAppContext, width: f32) -> (Fixture, VisualTestContext) {
    let f = fixture(cx, None);
    let root = project(f.dir.path(), &["src/ui/a.rs", "src/ui/b.rs"]);
    std::fs::write(root.join("src/ui/a.rs"), CRUMBS_SRC).unwrap();
    f.view.update(cx, |v, cx| v.open_root(root.clone(), cx));
    settle(cx, &f, scanned);
    f.view.update(cx, |v, cx| v.open(root.join("src/ui/a.rs"), cx));
    settle(cx, &f, |v| v.session().state().active().is_some_and(|t| v.session().state().view(t.id()).is_some_and(|view| view.declarations().is_some())));
    let at = CRUMBS_SRC.find("a;").unwrap();
    f.view.update(cx, |v, cx| v.dispatch(Command::Select(Selection { anchor: at, head: at }), cx));
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(width), px(500.0)));
    draw(&mut v);
    (f, v)
}

fn crumb_labels(f: &Fixture, v: &VisualTestContext) -> Vec<String> {
    f.view.read_with(v, |v, _| v.crumb_hits.iter().filter(|h| !h.chevron).map(|h| h.label.clone()).collect())
}

fn crumb(f: &Fixture, v: &VisualTestContext, label: &str, chevron: bool) -> gpui::Point<Pixels> {
    f.view.read_with(v, |v, _| {
        let i = v.crumb_hits.iter().position(|h| !h.chevron && h.label == label).unwrap();
        let i = if chevron { i + 1 } else { i };
        assert_eq!(v.crumb_hits[i].chevron, chevron);
        v.crumb_hits[i].bounds.center()
    })
}

#[gpui::test]
fn the_breadcrumb_bar_reveals_jumps_and_lists_what_is_inside(cx: &mut TestAppContext) {
    let (f, mut v) = crumbs_fixture(cx, 900.0);
    assert_eq!(crumb_labels(&f, &v), ["src", "ui", "a.rs", "m", "f"]);
    v.simulate_click(crumb(&f, &v, "m", false), Modifiers::default());
    draw(&mut v);
    assert_eq!(f.view.read_with(&v, |v, _| { let s = v.session().state(); s.current_source().unwrap().line_of(s.selection().unwrap().head) }), 0, "the caret on mod m");
    v.simulate_click(crumb(&f, &v, "ui", false), Modifiers::default());
    draw(&mut v);
    let rows: Vec<String> = f.view.read_with(&v, |v, _| v.session().state().project().unwrap().rows().iter().map(|r| r.path.to_string()).collect());
    assert!(rows.contains(&"src/ui/b.rs".to_string()), "the folder opened in the explorer: {rows:?}");
    v.simulate_click(crumb(&f, &v, "ui", true), Modifiers::default());
    draw(&mut v);
    assert!(matches!(f.view.read_with(&v, |v, _| v.modal.clone()), Some(Modal::Siblings { crumb: 1, .. })));
    assert_eq!(f.view.read_with(&v, |v, _| v.modal_rows().into_iter().skip(2).map(|r| r.0).collect::<Vec<_>>()), ["a.rs", "b.rs"]);
    v.simulate_keystrokes("b enter");
    drop(v);
    settle(cx, &f, active_is("b.rs"));
}

#[gpui::test]
fn ctrl_shift_period_gives_the_bar_the_keys(cx: &mut TestAppContext) {
    let (f, mut v) = crumbs_fixture(cx, 900.0);
    v.simulate_keystrokes("ctrl-shift-.");
    assert_eq!(f.view.read_with(&v, |v, _| v.crumb_focus), Some((0, 4)), "the innermost crumb");
    v.simulate_keystrokes("left enter");
    assert!(matches!(f.view.read_with(&v, |v, _| v.modal.clone()), Some(Modal::Siblings { crumb: 3, .. })), "the list inside mod m");
    v.simulate_keystrokes("escape");
    assert!(f.view.read_with(&v, |v, _| v.modal.is_none() && v.crumb_focus.is_some()));
    v.simulate_keystrokes("escape");
    assert_eq!(f.view.read_with(&v, |v, _| v.crumb_focus), None, "keys back to the text");
}

#[gpui::test]
fn a_narrow_pane_keeps_the_file_and_the_innermost_declaration(cx: &mut TestAppContext) {
    let (f, mut v) = crumbs_fixture(cx, 180.0);
    let labels = crumb_labels(&f, &v);
    assert_eq!(labels.first().map(String::as_str), Some("…"), "{labels:?}");
    assert!(labels.contains(&"a.rs".to_string()) && labels.last().map(String::as_str) == Some("f"), "{labels:?}");
    let before = f.view.read_with(&v, |v, _| v.session().state().selection());
    v.simulate_click(crumb(&f, &v, "…", false), Modifiers::default());
    draw(&mut v);
    assert_eq!(f.view.read_with(&v, |v, _| v.session().state().selection()), before, "the shrunk crumb does nothing");
}

#[gpui::test]
fn breadcrumbs_off_give_their_row_back_to_the_text(cx: &mut TestAppContext) {
    let (f, mut v) = crumbs_fixture(cx, 900.0);
    let on = f.view.read_with(&v, |v, _| v.text_bounds.unwrap().origin.y);
    f.view.update(&mut v, |v, cx| v.dispatch(Command::SetBreadcrumbs(false), cx));
    draw(&mut v);
    let off = f.view.read_with(&v, |v, _| v.text_bounds.unwrap().origin.y);
    assert_eq!(on - off, px(line_h()));
    assert!(f.view.read_with(&v, |v, _| v.crumb_hits.is_empty()));
}

fn bookmarks_fixture(cx: &mut TestAppContext) -> (Fixture, VisualTestContext, PathBuf) {
    let f = fixture(cx, None);
    let root = project(f.dir.path(), &["a.rs", "b.rs"]);
    std::fs::write(root.join("a.rs"), "0\n1\n2\n3\n4\n5\n").unwrap();
    f.view.update(cx, |v, cx| v.open_root(root.clone(), cx));
    settle(cx, &f, scanned);
    f.view.update(cx, |v, cx| v.open(root.join("a.rs"), cx));
    settle(cx, &f, active_is("a.rs"));
    let mut v = visual(cx, &f);
    draw(&mut v);
    (f, v, root)
}

fn caret_to(f: &Fixture, v: &mut VisualTestContext, line: usize) {
    f.view.update(v, |view, cx| {
        let at = view.session().state().current_source().unwrap().line_range(line).unwrap().start;
        view.dispatch(Command::Select(Selection { anchor: at, head: at }), cx);
    });
}

#[gpui::test]
fn bookmark_keys_reach_their_commands(cx: &mut TestAppContext) {
    let (f, mut v, root) = bookmarks_fixture(cx);
    caret_to(&f, &mut v, 2);
    v.simulate_keystrokes("f11");
    draw(&mut v);
    assert_eq!(f.view.read_with(&v, |v, _| v.bookmark_marks()), [(2, None)]);
    caret_to(&f, &mut v, 4);
    v.simulate_keystrokes("ctrl-shift-3");
    draw(&mut v);
    assert_eq!(f.view.read_with(&v, |v, _| v.bookmark_marks()), [(2, None), (4, Some('3'))]);
    caret_to(&f, &mut v, 0);
    v.simulate_keystrokes("alt-3");
    assert_eq!(f.view.read_with(&v, |v, _| { let s = v.session().state(); s.current_source().unwrap().line_of(s.selection().unwrap().head) }), 4);
    v.simulate_keystrokes("shift-f11");
    assert!(matches!(f.view.read_with(&v, |v, _| v.modal.clone()), Some(Modal::Places { list: PlaceList::Bookmarks, .. })));
    v.simulate_keystrokes("escape ctrl-f11");
    assert!(matches!(f.view.read_with(&v, |v, _| v.modal.clone()), Some(Modal::Mnemonic)));
    v.simulate_keystrokes("escape");
    f.view.update(&mut v, |v, cx| v.open(root.join("b.rs"), cx));
    drop(v);
    settle(cx, &f, active_is("b.rs"));
    let mut v = visual(cx, &f);
    v.simulate_keystrokes("ctrl-e");
    assert!(matches!(f.view.read_with(&v, |v, _| v.modal.clone()), Some(Modal::Navigation { .. })));
    v.simulate_keystrokes("escape ctrl-alt-e");
    assert!(matches!(f.view.read_with(&v, |v, _| v.modal.clone()), Some(Modal::Places { list: PlaceList::RecentLocations, .. })));
}

#[gpui::test]
fn bookmark_keys_without_a_text_view_say_why(cx: &mut TestAppContext) {
    let f = fixture(cx, None);
    let mut v = visual(cx, &f);
    for (i, key) in ["f11", "ctrl-f11", "ctrl-shift-3"].into_iter().enumerate() {
        v.simulate_keystrokes(key);
        assert!(f.view.read_with(&v, |v, _| v.modal.is_none()), "{key} opens nothing");
        let said = f.view.read_with(&v, |v, _| v.session().state().notices().iter().filter(|n| *n == "No file is open").count());
        assert_eq!(said, i + 1, "{key} says why");
    }
}

#[gpui::test]
fn a_gutter_mark_follows_its_line_after_the_file_changes(cx: &mut TestAppContext) {
    let (f, mut v, root) = bookmarks_fixture(cx);
    caret_to(&f, &mut v, 3);
    v.simulate_keystrokes("f11");
    std::fs::write(root.join("a.rs"), "new\n0\n1\n2\n3\n4\n5\n").unwrap();
    f.view.update(&mut v, |v, cx| v.dispatch(Command::Reload, cx));
    drop(v);
    settle(cx, &f, |v| v.session().state().current_source().is_some_and(|d| d.line_count() == 8));
    let mut v = visual(cx, &f);
    draw(&mut v);
    assert_eq!(f.view.read_with(&v, |v, _| v.bookmark_marks()), [(4, None)]);
}

#[gpui::test]
fn bookmark_entries_show_in_the_command_menu_with_their_keys(cx: &mut TestAppContext) {
    let (f, v, _) = bookmarks_fixture(cx);
    let labels: Vec<String> = f.view.read_with(&v, |v, _| v.session().actions().iter().map(|a| a.label.to_string()).collect());
    for l in ["Toggle Bookmark", "Set Bookmark Mnemonic…", "Show Bookmarks…", "Next Bookmark", "Previous Bookmark", "Recent Files…", "Recent Locations…"] {
        assert!(labels.iter().any(|x| x == l), "{l}");
    }
    assert_eq!(key_hint(CommandId::ToggleBookmark), Some("F11"));
    assert_eq!(key_hint(CommandId::RecentLocations), Some("Ctrl+Alt+E"));
}

fn modal_lines(f: &Fixture, v: &VisualTestContext) -> Vec<String> {
    f.view.read_with(v, |v, _| v.modal_rows().into_iter().map(|r| r.0).collect())
}

#[gpui::test]
fn the_bookmark_list_filters_jumps_and_deletes(cx: &mut TestAppContext) {
    let (f, mut v, _) = bookmarks_fixture(cx);
    for (line, key) in [(1, "ctrl-shift-1"), (4, "ctrl-shift-2")] {
        caret_to(&f, &mut v, line);
        v.simulate_keystrokes(key);
    }
    v.simulate_keystrokes("shift-f11");
    let rows = modal_lines(&f, &v);
    assert_eq!(rows.len(), 2 + 2, "query and hint, then two bookmarks: {rows:?}");
    v.simulate_keystrokes("a . r s : 5");
    assert_eq!(modal_lines(&f, &v).len(), 3, "filtered to line 5");
    v.simulate_keystrokes("enter");
    assert!(f.view.read_with(&v, |v, _| v.modal.is_none()));
    assert_eq!(f.view.read_with(&v, |v, _| { let s = v.session().state(); s.current_source().unwrap().line_of(s.selection().unwrap().head) }), 4);
    v.simulate_keystrokes("shift-f11 delete");
    assert_eq!(f.view.read_with(&v, |v, _| v.session().state().places().bookmarks().len()), 1);
}

#[gpui::test]
fn delete_in_a_popup_keeps_the_highlight_in_range(cx: &mut TestAppContext) {
    let (f, mut v, _) = bookmarks_fixture(cx);
    for line in [0, 2] {
        caret_to(&f, &mut v, line);
        v.simulate_keystrokes("f11");
    }
    v.simulate_keystrokes("shift-f11 down delete delete");
    assert!(f.view.read_with(&v, |v, _| v.session().state().places().bookmarks().is_empty()));
    v.simulate_keystrokes("enter"); // nothing left: no jump, no panic
    draw(&mut v);
}

#[gpui::test]
fn the_mnemonic_chooser_sets_and_lists_used_keys(cx: &mut TestAppContext) {
    let (f, mut v, _) = bookmarks_fixture(cx);
    caret_to(&f, &mut v, 2);
    v.simulate_keystrokes("ctrl-f11 k");
    assert!(f.view.read_with(&v, |v, _| v.modal.is_none()));
    assert_eq!(f.view.read_with(&v, |v, _| v.session().state().places().mnemonic('K').map(|m| m.line)), Some(2));
    v.simulate_keystrokes("ctrl-f11");
    assert!(modal_lines(&f, &v).iter().any(|r| r.starts_with("K") && r.contains("a.rs:3")), "{:?}", modal_lines(&f, &v));
    v.simulate_keystrokes("escape");
    assert_eq!(f.view.read_with(&v, |v, _| v.session().state().places().bookmarks().len()), 1, "Escape changes nothing");
}

#[gpui::test]
fn recent_files_switch_back_with_enter(cx: &mut TestAppContext) {
    let (f, v, root) = bookmarks_fixture(cx);
    drop(v);
    f.view.update(cx, |v, cx| v.open(root.join("b.rs"), cx));
    settle(cx, &f, active_is("b.rs"));
    let mut v = visual(cx, &f);
    v.simulate_keystrokes("ctrl-e enter");
    drop(v);
    settle(cx, &f, active_is("a.rs"));
}

#[gpui::test]
fn recent_files_preserve_retained_tab_reading_position(cx: &mut TestAppContext) {
    let (f, mut v, root) = bookmarks_fixture(cx);
    f.view.update(&mut v, |r, cx| r.dispatch(Command::OpenRetained(root.join("a.rs")), cx));
    caret_to(&f, &mut v, 3);
    f.view.update(&mut v, |r, cx| r.open(root.join("b.rs"), cx));
    drop(v);
    settle(cx, &f, active_is("b.rs"));
    let mut v = visual(cx, &f);
    v.simulate_keystrokes("ctrl-e enter");
    drop(v);
    settle(cx, &f, active_is("a.rs"));
    let v = visual(cx, &f);
    assert_eq!(caret_line(&f, &v), 3);
}

#[gpui::test]
fn a_scrolled_place_list_in_a_tiny_window_dismisses_cleanly(cx: &mut TestAppContext) {
    let (f, mut v, _) = bookmarks_fixture(cx);
    for line in 0..6 {
        caret_to(&f, &mut v, line);
        v.simulate_keystrokes("f11");
    }
    v.simulate_resize(size(px(240.0), px(160.0)));
    draw(&mut v);
    v.simulate_keystrokes("shift-f11 end down down down down down");
    draw(&mut v);
    v.simulate_keystrokes("escape");
    draw(&mut v);
    assert!(f.view.read_with(&v, |v, _| v.modal.is_none()));
}

#[gpui::test]
fn a_place_list_takes_a_click_over_a_sticky_line(cx: &mut TestAppContext) {
    let (f, mut v) = sticky_fixture(cx);
    v.simulate_keystrokes("f11 shift-f11"); // a bookmark on the caret's line, then its list
    draw(&mut v);
    assert!(matches!(f.view.read_with(&v, |v, _| v.modal.clone()), Some(Modal::Places { .. })));
    let b = f.view.read_with(&v, |v, _| v.sticky_hits[1].0);
    let caret = f.view.read_with(&v, |v, _| v.session().state().selection());
    // Over the sticky line, left of the list's box: the click dismisses the list and reaches nothing else.
    let at = point(b.origin.x + px(4.0), b.center().y);
    assert!(!f.view.read_with(&v, |v, _| v.modal_layout().unwrap().bounds.contains(&at)));
    v.simulate_click(at, Modifiers::default());
    draw(&mut v);
    assert_eq!(f.view.read_with(&v, |v, _| v.session().state().active().unwrap().top_line()), 10, "no jump");
    assert_eq!(f.view.read_with(&v, |v, _| v.session().state().selection()), caret, "the sticky line under the popup was not clicked");
}

#[gpui::test]
fn ctrl_shift_digit_sets_a_numbered_bookmark_as_linux_reports_it(cx: &mut TestAppContext) {
    let (f, mut v, _) = bookmarks_fixture(cx);
    caret_to(&f, &mut v, 3);
    // Linux reports Ctrl+Shift+3 as Ctrl+# (the shifted symbol, shift cleared); Ctrl+Shift+0 as Ctrl+).
    v.simulate_keystrokes("ctrl-#");
    caret_to(&f, &mut v, 5);
    v.simulate_keystrokes("ctrl-)");
    assert_eq!(f.view.read_with(&v, |v, _| v.session().state().places().mnemonic('3').map(|m| m.line)), Some(3));
    assert_eq!(f.view.read_with(&v, |v, _| v.session().state().places().mnemonic('0').map(|m| m.line)), Some(5));
}

const HINTED: &str = "fn main() {\n    let v = add(1);\n}\nfn old_add(a: i32) -> i32 { a }\n";

fn hinted(r: &ReaderView) -> bool {
    let state = r.session().state();
    state.active().and_then(|tab| state.view(tab.id())).is_some_and(|view| {
        view.inlay_hints().is_some_and(|hints| !hints.is_empty()) && view.tokens().is_some()
    })
}

#[gpui::test]
fn inlay_hints_draw_dim_and_the_caret_clicks_copy_and_find_step_over_them(cx: &mut TestAppContext) {
    let f = fixture_with(cx, None, &fake_lsp());
    let pkg = fake_package(cx, &f, &[("src/main.rs", HINTED)], "src/main.rs");
    settle(cx, &f, hinted);
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(900.0), px(500.0)));
    draw(&mut v);
    let row = f.view.read_with(&v, |r, _| r.visible_rows()[1].clone());
    assert_eq!(row.text, "    let v: i32 = add(a: 1);");
    let num_fg = f.view.read_with(&v, |r, _| r.session().state().palette().num_fg);
    let hint = "    let v".len().."    let v: i32".len();
    assert_eq!(row.fg.first(), Some(&(hint.clone(), num_fg)), "the hint first, dim: {:?}", row.fg);
    // The caret after `v` is drawn before the hint; Shift+Right steps over it to the space after it.
    let after_v = HINTED.find("v =").unwrap() + 1;
    f.view.update(&mut v, |r, cx| r.dispatch(Command::Select(Selection { anchor: after_v, head: after_v }), cx));
    draw(&mut v);
    assert_eq!(f.view.read_with(&v, |r, _| r.visible_rows()[1].caret), Some(hint.start));
    v.simulate_keystrokes("shift-right");
    draw(&mut v);
    assert_eq!(f.view.read_with(&v, |r, _| r.session().state().selection().unwrap().head), after_v + 1);
    assert_eq!(f.view.read_with(&v, |r, _| r.visible_rows()[1].caret), Some(hint.end + 1));
    // A click inside the hint lands on its byte.
    let (text_b, char_w) = f.view.read_with(&v, |r, _| (r.text_bounds.unwrap(), r.char_w));
    let x = f32::from(text_b.origin.x) + num_w() + 8.0 + (hint.start as f32 + 2.5) * char_w;
    v.simulate_click(point(px(x), px(f32::from(text_b.origin.y) + line_h() * 1.5)), Modifiers::default());
    assert_eq!(f.view.read_with(&v, |r, _| r.session().state().selection().unwrap().head), after_v);
    // Copy takes the source text.
    let line = HINTED.find("    let").unwrap()..HINTED.find(");").unwrap() + 2;
    f.view.update(&mut v, |r, cx| r.dispatch(Command::Select(Selection { anchor: line.start, head: line.end }), cx));
    v.simulate_keystrokes("ctrl-c");
    assert_eq!(v.read_from_clipboard().and_then(|c| c.text()).as_deref(), Some(&HINTED[line]));
    // Find's tint sits on `add`, after the hint.
    f.view.update(&mut v, |r, cx| r.dispatch(Command::Select(Selection { anchor: 0, head: 0 }), cx));
    v.simulate_keystrokes("ctrl-f a d d");
    drop(v);
    settle(cx, &f, find_settled);
    let mut v = visual(cx, &f);
    draw(&mut v);
    let add = "    let v: i32 = ".len().."    let v: i32 = add".len();
    let row = f.view.read_with(&v, |r, _| r.visible_rows()[1].clone());
    assert!(row.selected.as_ref() == Some(&add) || row.bg.iter().any(|(r, _)| *r == add), "{row:?}");
    assert_eq!(std::fs::read_to_string(pkg.join("src/main.rs")).unwrap(), HINTED);
}

#[gpui::test]
fn mutable_and_deprecated_tokens_are_underlined_and_struck(cx: &mut TestAppContext) {
    let text = HINTED.replace("let v", "let mut v");
    let f = fixture_with(cx, None, &fake_lsp());
    fake_package(cx, &f, &[("src/main.rs", &text)], "src/main.rs");
    settle(cx, &f, hinted);
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(900.0), px(500.0)));
    draw(&mut v);
    let marks = f.view.read_with(&v, |r, _| r.marks_drawn.clone());
    let v_at = "    let mut ".len();
    assert!(marks.iter().any(|(y, r, _, m)| *y == 1.0 && *r == (v_at..v_at + 1) && *m == reader_core::decor::MUTABLE), "{marks:?}");
    assert!(marks.iter().any(|(y, r, _, m)| *y == 3.0 && *r == (3..10) && *m == reader_core::decor::DEPRECATED), "{marks:?}");
}

#[gpui::test]
fn the_toggles_run_from_the_command_menu_and_show_their_state(cx: &mut TestAppContext) {
    let f = fixture_with(cx, None, &fake_lsp());
    fake_package(cx, &f, &[("src/main.rs", HINTED)], "src/main.rs");
    settle(cx, &f, hinted);
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(900.0), px(500.0)));
    let menu = |v: &mut VisualTestContext, id: CommandId| {
        let at = CommandId::ALL.iter().position(|c| *c == id).unwrap();
        v.simulate_keystrokes(&format!("f10{}", " down".repeat(at)));
        draw(v);
        let label = f.view.read_with(v, |r, _| r.modal_rows()[at].0.clone());
        v.simulate_keystrokes("enter");
        label
    };
    assert_eq!(menu(&mut v, CommandId::InlayTypes), "Inlay Hints: Types (on)");
    settle(cx, &f, |r| r.session().state().active().and_then(|tab| r.session().state().view(tab.id())).is_some_and(|view| view.inlay_hints().is_some_and(|h| h.line(1).iter().all(|h| h.kind == reader_core::decor::HintKind::Parameter) && !h.is_empty())));
    draw(&mut v);
    assert_eq!(f.view.read_with(&v, |r, _| r.visible_rows()[1].text.clone()), "    let v = add(a: 1);");
    assert_eq!(menu(&mut v, CommandId::InlayParameters), "Inlay Hints: Parameter Names (on)");
    draw(&mut v);
    assert_eq!(f.view.read_with(&v, |r, _| r.visible_rows()[1].text.clone()), "    let v = add(1);");
    assert_eq!(menu(&mut v, CommandId::SemanticHighlighting), "Semantic Highlighting (on)");
    v.simulate_keystrokes("f10");
    draw(&mut v);
    let at = CommandId::ALL.iter().position(|c| *c == CommandId::SemanticHighlighting).unwrap();
    assert_eq!(f.view.read_with(&v, |r, _| r.modal_rows()[at].0.clone()), "Semantic Highlighting (off)");
}

#[gpui::test]
fn hinted_lines_show_in_both_panes_and_in_sticky_lines(cx: &mut TestAppContext) {
    let body = "        let k = 1;\n".repeat(40);
    let text = format!("mod m {{\n    fn f() {{ let head = add(2); }}\n    fn g() {{ let top = 1;\n{body}    }}\n}}\nfn add(a: i32) -> i32 {{ a }}\n");
    let f = fixture_with(cx, None, &fake_lsp());
    fake_package(cx, &f, &[("src/main.rs", &text)], "src/main.rs");
    settle(cx, &f, |r| hinted(r) && r.session().state().active().is_some_and(|t| r.session().state().view(t.id()).is_some_and(|view| view.declarations().is_some())));
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(900.0), px(500.0)));
    draw(&mut v);
    // Sticky lines carry hints.
    f.view.update(&mut v, |r, cx| r.dispatch(Command::Scroll(10), cx));
    draw(&mut v);
    let sticky = f.view.read_with(&v, |r, _| {
        let t = r.session().state().active().unwrap();
        r.sticky_rows(t, 5).0.into_iter().map(|(_, row)| row.text).collect::<Vec<_>>()
    });
    assert!(sticky.iter().any(|t| t == "    fn g() { let top: i32 = 1;"), "{sticky:?}");
    // The other pane shares the snapshot and its hints.
    f.view.update(&mut v, |r, cx| r.dispatch(Command::Scroll(0), cx));
    f.view.update(&mut v, |r, cx| r.dispatch(Command::Split(reader_core::session::Split::Columns), cx));
    settle(cx, &f, |r| r.session().state().shown(1).is_some_and(|tab| r.session().state().view(tab.id()).is_some_and(|view| view.inlay_hints().is_some())));
    let other = f.view.read_with(&v, |r, _| {
        let t = r.session().state().shown(1).unwrap();
        r.tab_rows(t, 5).0[1].text.clone()
    });
    assert_eq!(other, "    fn f() { let head: i32 = add(a: 2); }");
}

#[gpui::test]
fn hinted_lines_wrap_and_clicks_land_on_their_bytes(cx: &mut TestAppContext) {
    let text = format!("fn main() {{\n    let head = add(2); {}\n}}\nfn add(a: i32) -> i32 {{ a }}\n", "word ".repeat(20));
    let f = fixture_with(cx, None, &fake_lsp());
    fake_package(cx, &f, &[("src/main.rs", &text)], "src/main.rs");
    settle(cx, &f, hinted);
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(640.0), px(300.0))); // the explorer is shown: room for ~40 columns of text
    draw(&mut v);
    v.simulate_keystrokes("alt-z");
    draw(&mut v);
    let (rows, shown) = f.view.read_with(&v, |r, _| {
        let s = r.session().state();
        let t = s.active().unwrap();
        let view = s.view(t.id()).unwrap();
        let layout = view.layout_rows(8, s.wrap_cols(t));
        assert!(layout.pending.is_none(), "the rendered hinted fixture has complete shared rows");
        let mut cursor = view.display_start(1).unwrap();
        let projected = view.display_window(&mut cursor, reader_core::projection::WINDOW_WORK).unwrap();
        assert!(projected.complete, "the short hinted line fits a bounded projection");
        (layout.rows, projected.shown.text)
    });
    let line1: Vec<_> = rows.iter().filter(|r| r.line == 1).cloned().collect();
    assert!(line1.len() >= 2, "{rows:?}");
    assert_eq!(line1.last().unwrap().display.end, shown.len());
    let (text_b, char_w) = f.view.read_with(&v, |r, _| (r.text_bounds.unwrap(), r.char_w));
    let i = rows.iter().position(|r| r.line == 1 && r.index == 1).unwrap();
    let x = f32::from(text_b.origin.x) + num_w() + 8.0 + (line1[1].indent as f32 + 0.3) * char_w;
    v.simulate_click(point(px(x), px(f32::from(text_b.origin.y) + line_h() * (i as f32 + 0.5))), Modifiers::default());
    let start = text.find("    let head").unwrap();
    assert_eq!(f.view.read_with(&v, |r, _| r.session().state().selection().unwrap().head), line1[1].source.start);
    assert!(line1[1].source.start >= start, "shared row source ranges are absolute UTF8 bytes");
}

#[gpui::test]
fn ctrl_h_opens_the_type_hierarchy_and_tab_enter_and_menus_work(cx: &mut TestAppContext) {
    let lib = "pub trait Shape {}\npub struct Circle;\npub struct Square;\nimpl Shape for Circle {}\nimpl Shape for Square {}\n";
    let f = fixture_with(cx, None, &fake_lsp());
    let pkg = fake_package(cx, &f, &[("src/lib.rs", lib)], "src/lib.rs");
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(900.0), px(500.0)));
    let at = lib.find("Shape").unwrap();
    f.view.update(&mut v, |r, cx| r.dispatch(Command::Select(Selection { anchor: at, head: at }), cx));
    v.simulate_keystrokes("ctrl-h");
    settle(cx, &f, |r| r.session().state().results().is_some_and(|r| r.kind().starts_with("Subtypes of Shape") && r.hits().len() == 3));
    assert!(f.view.read_with(&v, |r, _| r.results_focus), "keys go to the tree");
    v.simulate_keystrokes("tab");
    settle(cx, &f, |r| r.session().state().results().is_some_and(|r| r.kind().starts_with("Supertypes of Shape")));
    v.simulate_keystrokes("tab");
    settle(cx, &f, |r| r.session().state().results().is_some_and(|r| r.kind().starts_with("Subtypes of Shape") && r.hits().len() == 3));
    v.simulate_keystrokes("down enter");
    // The first child goes to its name on an `impl Shape for …` line (lines 3 and 4).
    let line = |r: &ReaderView| r.session().state().current_source().zip(r.session().state().selection()).map(|(d, s)| d.line_of(s.head));
    settle(cx, &f, |r| matches!(line(r), Some(3 | 4)));
    // The menus offer it.
    let rows = f.view.read_with(&v, |r, _| r.session().actions().into_iter().map(|a| a.label).collect::<Vec<_>>());
    assert!(rows.contains(&"Type Hierarchy") && rows.contains(&"Type Hierarchy: Supertypes") && rows.contains(&"Type Hierarchy: Subtypes"));
    // A type without subtypes says so on its row.
    let at = lib.find("Circle;").unwrap();
    f.view.update(&mut v, |r, cx| r.dispatch(Command::Select(Selection { anchor: at, head: at }), cx));
    v.simulate_keystrokes("ctrl-h");
    settle(cx, &f, |r| r.session().state().results().is_some_and(|r| r.kind().starts_with("Subtypes of Circle") && r.call(0).is_some_and(|c| c.fold == reader_core::search::Fold::Leaf)));
    let labels = f.view.update(&mut v, |r, _| r.panel_contents(10, 0).0.into_iter().map(|row| row.label).collect::<Vec<_>>());
    assert!(labels.iter().any(|l| l.contains("Circle") && l.contains("No subtypes")), "{labels:?}");
    assert_eq!(std::fs::read_to_string(pkg.join("src/lib.rs")).unwrap(), lib);
}

/// Click modal row `row` (of `modal_rows`) at the middle of its first drawn line, from the layout paint and clicks
/// share.
fn click_modal_row(f: &Fixture, v: &mut VisualTestContext, row: usize) {
    draw(v);
    let p = f.view.read_with(v, |r, _| {
        let l = r.modal_layout().unwrap();
        let j = l.lines.iter().position(|(i, ..)| *i == row).unwrap();
        assert!((l.first..l.first + l.visible).contains(&j), "row {row} is drawn: first {}, visible {}, line {j}", l.first, l.visible);
        point(l.bounds.center().x, l.bounds.origin.y + px(4.0 + (j - l.first) as f32 * line_h() + line_h() / 2.0))
    });
    v.simulate_click(p, Modifiers::default());
    draw(v);
}

fn caret_line(f: &Fixture, v: &VisualTestContext) -> usize {
    f.view.read_with(v, |v, _| { let s = v.session().state(); s.current_source().unwrap().line_of(s.selection().unwrap().head) })
}

#[gpui::test]
fn clicks_pick_rows_in_the_command_menu_recent_folders_and_theme_chooser(cx: &mut TestAppContext) {
    let f = fixture(cx, None);
    let one = project(&f.dir.path().join("one"), &["a.rs"]);
    let two = project(&f.dir.path().join("two"), &["b.rs"]);
    for (root, name) in [(&one, "one/proj"), (&two, "two/proj")] {
        f.view.update(cx, |v, cx| v.open_root(root.clone(), cx));
        settle(cx, &f, |v| scanned(v) && v.session().state().project().is_some_and(|p| p.root().ends_with(name)));
    }
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(900.0), px(600.0)));
    // The command menu: a click on Open Recent Folder… runs it.
    v.simulate_keystrokes("f10");
    let recent = CommandId::ALL.iter().position(|&c| c == CommandId::OpenRecent).unwrap();
    click_modal_row(&f, &mut v, recent);
    assert!(matches!(f.view.read_with(&v, |v, _| v.modal.clone()), Some(Modal::Recent { at: 0 })));
    // Recent folders, newest first: a click on the older one opens it.
    assert!(f.view.read_with(&v, |v, _| v.modal_rows()[1].0.ends_with("one/proj")));
    click_modal_row(&f, &mut v, 1);
    assert!(f.view.read_with(&v, |v, _| v.modal.is_none()));
    drop(v);
    settle(cx, &f, |v| scanned(v) && v.session().state().project().is_some_and(|p| p.root().ends_with("one/proj")));
    // The theme chooser from the menu (highlighted by keys so its row is drawn), then a click on Light.
    let mut v = visual(cx, &f);
    v.simulate_keystrokes("f10");
    let theme = CommandId::ALL.iter().position(|&c| c == CommandId::ChooseTheme).unwrap();
    for _ in 0..theme {
        v.simulate_keystrokes("down");
    }
    click_modal_row(&f, &mut v, theme);
    let light = f.view.read_with(&v, |v, _| match &v.modal {
        Some(Modal::Themes { ids, .. }) => ids.iter().position(|(i, _)| i == "light").unwrap(),
        other => panic!("{other:?}"),
    });
    click_modal_row(&f, &mut v, light + THEME_HEADER_ROWS);
    assert!(f.view.read_with(&v, |v, _| v.modal.is_none()));
    assert_eq!(f.view.read_with(&v, |v, _| v.session().state().palette().bg), lenz_content::theme::LIGHT.bg);
    // Outside the box: dismissed, nothing run.
    v.simulate_keystrokes("f10");
    v.simulate_click(point(px(4.0), px(590.0)), Modifiers::default());
    assert!(f.view.read_with(&v, |v, _| v.modal.is_none()));
    assert_eq!(std::fs::read(one.join("a.rs")).unwrap(), b"a.rs");
    assert_eq!(std::fs::read(two.join("b.rs")).unwrap(), b"b.rs");
}

#[gpui::test]
fn a_click_in_a_scrolled_finder_opens_the_clicked_file(cx: &mut TestAppContext) {
    let f = fixture(cx, None);
    let files: Vec<String> = (0..60).map(|i| format!("f{i:02}.rs")).collect();
    let root = project(f.dir.path(), &files.iter().map(String::as_str).collect::<Vec<_>>());
    f.view.update(cx, |v, cx| v.open_root(root.clone(), cx));
    settle(cx, &f, scanned);
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(700.0), px(300.0)));
    v.simulate_keystrokes("ctrl-p f");
    drop(v);
    settle(cx, &f, |v| v.session().state().navigation().is_some_and(|f| !f.pending && f.query == "f"));
    let mut v = visual(cx, &f);
    v.simulate_keystrokes("end");
    draw(&mut v);
    // Scrolled: the clicked row is the one drawn there, not the one the unscrolled layout would put there.
    let first = f.view.read_with(&v, |r, _| r.navigation_drawn.unwrap().1);
    assert!(first > 0, "the finder is scrolled");
    let name = f.view.read_with(&v, |r, _| r.session.state().navigation().unwrap().rows[first].label.clone());
    click_navigation_row(&f, &mut v, first);
    drop(v);
    settle(cx, &f, |v| v.session().state().current_source().is_some_and(|d| d.source().ends_with(name.as_str())));
}

#[gpui::test]
fn clicks_pick_places_and_found_files_and_their_header_rows_do_nothing(cx: &mut TestAppContext) {
    let (f, mut v, root) = bookmarks_fixture(cx);
    v.simulate_resize(size(px(900.0), px(600.0)));
    for line in [1, 4] {
        caret_to(&f, &mut v, line);
        v.simulate_keystrokes("f11");
    }
    caret_to(&f, &mut v, 0);
    v.simulate_keystrokes("shift-f11");
    // The query and hint rows do nothing.
    click_modal_row(&f, &mut v, 0);
    click_modal_row(&f, &mut v, 1);
    assert!(matches!(f.view.read_with(&v, |v, _| v.modal.clone()), Some(Modal::Places { at: 0, .. })));
    // The second bookmark (line 5): the list closes and the caret goes there.
    click_modal_row(&f, &mut v, FINDER_HEADER_ROWS + 1);
    assert!(f.view.read_with(&v, |v, _| v.modal.is_none()));
    assert_eq!(caret_line(&f, &v), 4);
    // The finder: its query row does nothing, a result opens.
    v.simulate_keystrokes("ctrl-p b");
    drop(v);
    settle(cx, &f, |v| v.session().state().navigation().is_some_and(|f| !f.pending && f.query == "b" && !f.rows.is_empty()));
    let mut v = visual(cx, &f);
    draw(&mut v);
    let input = f.view.read_with(&v, |r, _| { let b = r.navigation_bounds(); point(b.center().x, b.origin.y + px(4.0 + line_h() * 1.5)) });
    v.simulate_click(input, Modifiers::default());
    assert!(matches!(f.view.read_with(&v, |v, _| v.modal.clone()), Some(Modal::Navigation { .. })), "the query row does nothing");
    click_navigation_row(&f, &mut v, 0);
    drop(v);
    settle(cx, &f, active_is("b.rs"));
    assert_eq!(std::fs::read_to_string(root.join("a.rs")).unwrap(), "0\n1\n2\n3\n4\n5\n");
    assert_eq!(std::fs::read_to_string(root.join("b.rs")).unwrap(), "b.rs");
}

#[gpui::test]
fn a_click_on_a_crumbs_entry_goes_there(cx: &mut TestAppContext) {
    let (f, mut v) = crumbs_fixture(cx, 900.0);
    v.simulate_click(crumb(&f, &v, "ui", true), Modifiers::default());
    draw(&mut v);
    assert_eq!(f.view.read_with(&v, |v, _| v.modal_rows().into_iter().skip(FINDER_HEADER_ROWS).map(|r| r.0).collect::<Vec<_>>()), ["a.rs", "b.rs"]);
    click_modal_row(&f, &mut v, 1);
    assert!(matches!(f.view.read_with(&v, |v, _| v.modal.clone()), Some(Modal::Siblings { .. })), "the hint row does nothing");
    click_modal_row(&f, &mut v, FINDER_HEADER_ROWS + 1);
    assert!(f.view.read_with(&v, |v, _| v.modal.is_none()));
    drop(v);
    settle(cx, &f, active_is("b.rs"));
    let root = f.dir.path().join("proj");
    assert_eq!(std::fs::read_to_string(root.join("src/ui/a.rs")).unwrap(), CRUMBS_SRC);
    assert_eq!(std::fs::read_to_string(root.join("src/ui/b.rs")).unwrap(), "src/ui/b.rs");
}

#[gpui::test]
fn search_dialog_is_centered_and_its_fields_and_toggles_take_clicks(cx: &mut TestAppContext) {
    let (f, mut v, root) = bookmarks_fixture(cx);
    v.simulate_resize(size(px(900.0), px(600.0)));
    v.simulate_keystrokes("ctrl-shift-f x");
    draw(&mut v);
    let (l, w, h) = f.view.read_with(&v, |r, _| (r.search_layout().unwrap(), r.width, r.height));
    assert!((f32::from(l.bounds.center().x) - w / 2.0).abs() < 1.0 && (f32::from(l.bounds.center().y) - h / 2.0).abs() < 1.0, "centered: {:?}", l.bounds);
    let click = |v: &mut VisualTestContext, b: Bounds<Pixels>| { v.simulate_click(b.center(), Modifiers::default()); draw(v); };
    click(&mut v, l.fields[1]);
    v.simulate_keystrokes("y");
    assert!(matches!(f.view.read_with(&v, |r, _| r.modal.clone()), Some(Modal::Search { ref query, ref files, in_files: true, .. }) if query == "x" && files == "y"));
    click(&mut v, l.fields[0]);
    assert!(matches!(f.view.read_with(&v, |r, _| r.modal.clone()), Some(Modal::Search { in_files: false, .. })));
    let (option, b) = l.toggles[2];
    assert_eq!(option, FindOption::Regex);
    click(&mut v, b);
    assert!(f.view.read_with(&v, |r, _| r.session().find_options().regex));
    click(&mut v, b);
    assert!(!f.view.read_with(&v, |r, _| r.session().find_options().regex));
    click(&mut v, Bounds { origin: l.bounds.origin + point(px(2.0), px(2.0)), size: size(px(1.0), px(1.0)) });
    assert!(matches!(f.view.read_with(&v, |r, _| r.modal.clone()), Some(Modal::Search { .. })), "padding keeps it");
    // A tiny window keeps the dialog inside it, and a click outside closes it.
    for (w, h) in [(120.0, 80.0), (40.0, 20.0)] {
        v.simulate_resize(size(px(w), px(h)));
        draw(&mut v);
        let l = f.view.read_with(&v, |r, _| r.search_layout().unwrap());
        assert!(l.bounds.origin.x >= px(0.0) && l.bounds.right() <= px(w) && l.bounds.bottom() <= px(h), "{w}x{h}: {:?}", l.bounds);
    }
    v.simulate_resize(size(px(900.0), px(600.0)));
    v.simulate_click(point(px(4.0), px(590.0)), Modifiers::default());
    assert!(f.view.read_with(&v, |r, _| r.modal.is_none()));
    assert_eq!(std::fs::read_to_string(root.join("a.rs")).unwrap(), "0\n1\n2\n3\n4\n5\n");
}

#[gpui::test]
fn clicks_inside_input_modals_keep_them_and_go_to_line_still_closes(cx: &mut TestAppContext) {
    let (f, mut v, root) = bookmarks_fixture(cx);
    v.simulate_resize(size(px(900.0), px(600.0)));
    let caret = f.view.read_with(&v, |r, _| r.session().state().selection());
    let inputs = [
        Modal::Declarations { query: "x".into(), semantic: false },
        Modal::Root { path: "x".into() },
    ];
    for modal in inputs {
        f.view.update(&mut v, |r, cx| {
            r.modal = Some(modal.clone());
            cx.notify();
        });
        for row in 0..f.view.read_with(&v, |r, _| r.modal_rows().len()) {
            click_modal_row(&f, &mut v, row);
            assert_eq!(f.view.read_with(&v, |r, _| r.modal.clone()), Some(modal.clone()), "row {row}");
        }
        v.simulate_click(point(px(4.0), px(590.0)), Modifiers::default());
        assert!(f.view.read_with(&v, |r, _| r.modal.is_none()), "outside: {modal:?} dismissed");
    }
    f.view.update(&mut v, |r, cx| {
        r.modal = Some(Modal::GoTo { input: "3".into(), error: None });
        cx.notify();
    });
    click_modal_row(&f, &mut v, 0);
    assert!(f.view.read_with(&v, |r, _| r.modal.is_none()), "Go to Line is unchanged: a click closes it");
    assert_eq!(f.view.read_with(&v, |r, _| r.session().state().selection()), caret, "and goes nowhere");
    assert_eq!(std::fs::read_to_string(root.join("a.rs")).unwrap(), "0\n1\n2\n3\n4\n5\n");
}

#[gpui::test]
fn a_click_in_the_key_chooser_picks_that_rows_key(cx: &mut TestAppContext) {
    let (f, mut v, root) = bookmarks_fixture(cx);
    v.simulate_resize(size(px(900.0), px(600.0)));
    caret_to(&f, &mut v, 2);
    v.simulate_keystrokes("ctrl-f11 k");
    caret_to(&f, &mut v, 4);
    v.simulate_keystrokes("ctrl-f11");
    click_modal_row(&f, &mut v, 0);
    assert!(matches!(f.view.read_with(&v, |v, _| v.modal.clone()), Some(Modal::Mnemonic)), "the prompt does nothing");
    click_modal_row(&f, &mut v, 1);
    assert!(f.view.read_with(&v, |v, _| v.modal.is_none()));
    assert_eq!(f.view.read_with(&v, |v, _| v.session().state().places().mnemonic('K').map(|m| m.line)), Some(4), "K moved to the caret's line");
    v.simulate_keystrokes("ctrl-f11");
    v.simulate_click(point(px(2.0), px(590.0)), Modifiers::default());
    assert!(f.view.read_with(&v, |v, _| v.modal.is_none()));
    assert_eq!(f.view.read_with(&v, |v, _| v.session().state().places().mnemonic('K').map(|m| m.line)), Some(4), "outside changes nothing");
    assert_eq!(std::fs::read_to_string(root.join("a.rs")).unwrap(), "0\n1\n2\n3\n4\n5\n");
}

#[gpui::test]
fn alt_and_an_azerty_digit_key_go_to_that_numbered_bookmark(cx: &mut TestAppContext) {
    let (f, mut v, root) = bookmarks_fixture(cx);
    // Linux names the AZERTY digit row without Shift: à & é " ' ( - è _ ç for 0–9, the accented keys by keysym.
    let keys = ["agrave", "&", "eacute", "\"", "'", "(", "-", "egrave", "_", "ccedilla"];
    for (d, key) in keys.iter().enumerate() {
        caret_to(&f, &mut v, d % 6);
        v.simulate_keystrokes(&format!("ctrl-shift-{d}"));
        caret_to(&f, &mut v, (d + 1) % 6);
        v.simulate_keystrokes(&format!("alt-{key}"));
        assert_eq!(caret_line(&f, &v), d % 6, "Alt+{key}");
    }
    assert_eq!(std::fs::read_to_string(root.join("a.rs")).unwrap(), "0\n1\n2\n3\n4\n5\n");
}

#[gpui::test]
fn the_caret_sits_after_a_parameter_hint_and_before_a_type_hint(cx: &mut TestAppContext) {
    let f = fixture_with(cx, None, &fake_lsp());
    let pkg = fake_package(cx, &f, &[("src/main.rs", HINTED)], "src/main.rs");
    settle(cx, &f, hinted);
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(900.0), px(500.0)));
    let one = HINTED.find("(1)").unwrap() + 1;
    f.view.update(&mut v, |r, cx| r.dispatch(Command::Select(Selection { anchor: one, head: one }), cx));
    draw(&mut v);
    let row = f.view.read_with(&v, |r, _| r.visible_rows()[1].clone());
    assert_eq!(row.text, "    let v: i32 = add(a: 1);");
    assert_eq!(row.caret, Some("    let v: i32 = add(a: ".len()), "a: ▮1");
    let after_v = HINTED.find("v =").unwrap() + 1;
    f.view.update(&mut v, |r, cx| r.dispatch(Command::Select(Selection { anchor: after_v, head: after_v }), cx));
    draw(&mut v);
    assert_eq!(f.view.read_with(&v, |r, _| r.visible_rows()[1].caret), Some("    let v".len()), "v▮: i32");
    assert_eq!(std::fs::read_to_string(pkg.join("src/main.rs")).unwrap(), HINTED);
}

#[gpui::test]
fn sticky_lines_carry_mutable_and_deprecated_marks(cx: &mut TestAppContext) {
    let body = "        let k = 1;\n".repeat(40);
    let text = format!("mod m {{\n    fn old_g() {{ let mut top = 1;\n{body}    }}\n}}\n");
    let f = fixture_with(cx, None, &fake_lsp());
    let pkg = fake_package(cx, &f, &[("src/main.rs", &text)], "src/main.rs");
    settle(cx, &f, |r| r.session().state().active().and_then(|tab| r.session().state().view(tab.id())).is_some_and(|view| view.tokens().is_some() && view.declarations().is_some()));
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(900.0), px(500.0)));
    draw(&mut v);
    f.view.update(&mut v, |r, cx| r.dispatch(Command::Scroll(10), cx));
    draw(&mut v);
    let (rows, marks) = f.view.read_with(&v, |r, _| r.sticky_rows(r.session().state().active().unwrap(), 5));
    let y = rows.iter().find(|(l, _)| *l == 1).map(|(_, row)| row.y).unwrap();
    let name = "    fn ".len().."    fn old_g".len();
    let top = "    fn old_g() { let mut ".len();
    assert!(marks.iter().any(|(ry, r, _, m)| *ry == y && *r == name && *m == reader_core::decor::DEPRECATED), "{marks:?}");
    assert!(marks.iter().any(|(ry, r, _, m)| *ry == y && *r == (top..top + 3) && *m == reader_core::decor::MUTABLE), "{marks:?}");
    assert_eq!(std::fs::read_to_string(pkg.join("src/main.rs")).unwrap(), text);
}

#[gpui::test]
fn sticky_lines_draw_their_fold_marks_over_the_rows_under_them(cx: &mut TestAppContext) {
    let f = fixture(cx, None);
    let path = f.dir.path().join("a.rs");
    // Lines 2, 5, 8, 11, 14, … start `if` blocks (each a fold); scrolled to line 10, line 11 lies under `fn f`.
    let text = format!("mod m {{\n    fn f() {{\n{}    }}\n}}\n", "        if a {\n            b;\n        }\n".repeat(20));
    std::fs::write(&path, &text).unwrap();
    f.view.update(cx, |v, cx| v.open(path.clone(), cx));
    settle(cx, &f, |v| v.session().state().active().is_some_and(|tab| v.session().state().view(tab.id()).is_some_and(|view| view.declarations().is_some() && view.fold_at(11).is_some())));
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(900.0), px(500.0)));
    draw(&mut v);
    f.view.update(&mut v, |v, cx| v.dispatch(Command::Scroll(10), cx));
    draw(&mut v);
    assert_eq!(sticky_lines(&f, &v), [0, 1]);
    let drawn = f.view.read_with(&v, |v, _| v.fold_marks_drawn.clone());
    assert_eq!(drawn.iter().filter(|(y, _)| *y < 2.0).copied().collect::<Vec<_>>(), [(0.0, false), (1.0, false)], "only the sticky lines' marks over the band: {drawn:?}");
    assert!(drawn.contains(&(4.0, false)), "line 14 below the band keeps its mark: {drawn:?}");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
}

#[gpui::test]
fn every_toggle_shows_on_or_off_in_the_command_menu(cx: &mut TestAppContext) {
    let f = fixture(cx, Some(("a.rs", b"fn a() {}\n")));
    settle(cx, &f, |v| v.session().state().current_source().is_some());
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(900.0), px(600.0)));
    draw(&mut v);
    let label = |v: &VisualTestContext, id: CommandId| f.view.read_with(v, |r, _| r.modal_rows()[CommandId::ALL.iter().position(|c| *c == id).unwrap()].0.clone());
    v.simulate_keystrokes("f10");
    assert_eq!(label(&v, CommandId::SoftWrap), "Soft Wrap (off)");
    assert_eq!(label(&v, CommandId::StickyScroll), "Sticky Scroll (on)");
    assert_eq!(label(&v, CommandId::Breadcrumbs), "Breadcrumbs (on)");
    assert_eq!(label(&v, CommandId::DocumentationOnHover), "Documentation on Hover (on)");
    assert_eq!(label(&v, CommandId::MatchCase), "Find: Match Case (off)");
    assert_eq!(label(&v, CommandId::WholeWord), "Find: Whole Word (off)");
    assert_eq!(label(&v, CommandId::RegularExpression), "Find: Regular Expression (off)");
    v.simulate_keystrokes("escape alt-z");
    f.view.update(&mut v, |r, cx| r.dispatch(Command::ToggleFindOption(FindOption::Case), cx));
    v.simulate_keystrokes("f10");
    assert_eq!(label(&v, CommandId::SoftWrap), "Soft Wrap (on)");
    assert_eq!(label(&v, CommandId::MatchCase), "Find: Match Case (on)");
    v.simulate_keystrokes("escape");
    assert_eq!(std::fs::read(f.dir.path().join("a.rs")).unwrap(), b"fn a() {}\n");
}

/// As `settle`, but says whether `done` held instead of panicking.
fn wait_until(cx: &mut TestAppContext, f: &Fixture, done: impl Fn(&ReaderView) -> bool) -> bool {
    for _ in 0..300 {
        cx.run_until_parked();
        f.view.update(cx, |v, cx| v.poll(cx));
        if f.view.read_with(cx, |v, _| done(v)) {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    false
}

#[gpui::test]
fn the_lower_panes_close_mark_closes_its_tab(cx: &mut TestAppContext) {
    let f = fixture(cx, None);
    let root = project(f.dir.path(), &["a.rs", "b.rs", "c.rs"]);
    f.view.update(cx, |v, cx| v.open_root(root.clone(), cx));
    settle(cx, &f, scanned);
    for name in ["a.rs", "b.rs", "c.rs"] {
        f.view.update(cx, |v, cx| v.dispatch(Command::OpenRetained(root.join(name)), cx));
        settle(cx, &f, active_is(name));
    }
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(1000.0), px(600.0)));
    draw(&mut v);
    v.simulate_keystrokes("ctrl-shift-\\");
    v.simulate_keystrokes("ctrl-shift-\\");
    draw(&mut v);
    let panes = f.view.read_with(&v, |r, _| r.panes.clone());
    assert!(panes.len() == 2 && panes[1].bar.2 > panes[0].bar.2, "precondition: one pane above the other");
    f.view.update(&mut v, |r, cx| r.dispatch(Command::FocusPane(0), cx));
    draw(&mut v);
    let (id, hit) = f.view.read_with(&v, |r, _| {
        let id = r.session().state().pane_tabs(1).next().unwrap().id();
        (id, r.tab_hits.iter().find(|h| h.2 == id).copied().unwrap())
    });
    // The lower tab's close mark, where the upper bar's first tab (starting after its buttons) also lies.
    let (upper, char_w) = f.view.read_with(&v, |r, _| (r.tab_hits[0], r.char_w));
    let x = hit.1 - char_w;
    assert!(x > upper.0 && x < upper.1, "precondition: both bars' tabs span the point");
    v.simulate_click(point(px(x), px(panes[1].bar.2 + line_h() / 2.0)), Modifiers::default());
    let lower: Vec<_> = f.view.read_with(&v, |r, _| r.session().state().pane_tabs(1).map(|t| t.id()).collect());
    assert!(!lower.contains(&id), "the lower tab closed; focused pane {}", f.view.read_with(&v, |r, _| r.session().state().focused_pane()));
}

#[gpui::test]
fn a_tree_click_selects_the_row_drawn_there(cx: &mut TestAppContext) {
    let f = fixture(cx, None);
    let root = project(f.dir.path(), &["a.rs", "b.rs", "c.rs"]);
    f.view.update(cx, |v, cx| v.open_root(root.clone(), cx));
    settle(cx, &f, scanned);
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(900.0), px(400.0)));
    draw(&mut v);
    // The tree is drawn below its title row: row i from y = (1 + i) * line_h().
    v.simulate_click(point(px(20.0), px(toolbar_h() + 1.5 * line_h())), Modifiers::default());
    drop(v);
    settle(cx, &f, active_is("a.rs"));
    let mut v = visual(cx, &f);
    draw(&mut v);
    v.simulate_click(point(px(20.0), px(toolbar_h() + 2.5 * line_h())), Modifiers::default());
    drop(v);
    assert!(wait_until(cx, &f, active_is("b.rs")), "b.rs, drawn on row 1, opens; tabs {:?}", f.view.read_with(cx, |v, _| tab_titles(v)));
}

#[gpui::test]
fn a_preview_with_collapsed_folds_scrolls(cx: &mut TestAppContext) {
    let source = (0..40).map(|i| format!("# Section {i}\n\n{}\n\n", "Some words here. ".repeat(8))).collect::<String>();
    let f = fixture(cx, Some(("a.md", source.as_bytes())));
    settle(cx, &f, |v| matches!(v.session().state().rendering(), Some(Rendering::Source)) && v.session().state().active().is_some_and(|t| v.session().state().view(t.id()).is_some_and(|view| view.folds().is_some())));
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(800.0), px(500.0)));
    show(&mut v);
    v.simulate_keystrokes("ctrl-shift--");
    show(&mut v);
    assert!(f.view.read_with(&v, |r, _| r.session().state().view(r.session().state().active().unwrap().id()).unwrap().visible().hidden(2)), "precondition: sections collapsed");
    v.simulate_keystrokes("ctrl-shift-v");
    drop(v);
    settle(cx, &f, |v| matches!(v.session().state().rendering(), Some(Rendering::Preview(p)) if p.images.is_some()));
    let mut v = visual(cx, &f);
    show(&mut v);
    show(&mut v);
    v.simulate_event(ScrollWheelEvent { position: point(px(400.0), px(250.0)), delta: ScrollDelta::Lines(point(0.0, -6.0)), ..Default::default() });
    show(&mut v);
    show(&mut v);
    let top = f.view.read_with(&v, |r, _| r.preview_state().map(|s| s.0));
    assert!(top.is_some_and(|t| t > 0.0), "the preview scrolled: {top:?}");
    v.simulate_keystrokes("pagedown");
    show(&mut v);
    v.simulate_keystrokes("pagedown");
    show(&mut v);
    let paged = f.view.read_with(&v, |r, _| r.preview_state().map(|s| s.0));
    assert!(paged > top, "PageDown scrolled further: {top:?} -> {paged:?}");
}

#[gpui::test]
fn selecting_down_through_wrapped_lines_keeps_the_caret_shown(cx: &mut TestAppContext) {
    let text = format!("{}\n", "word ".repeat(80)).repeat(40);
    let f = fixture(cx, Some(("a.txt", text.as_bytes())));
    settle(cx, &f, |v| v.session().state().current_source().is_some());
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(400.0), px(300.0)));
    draw(&mut v);
    v.simulate_keystrokes("alt-z");
    draw(&mut v);
    let rows = f.view.read_with(&v, |r, _| r.rows);
    for _ in 0..rows + 2 {
        v.simulate_keystrokes("shift-down");
        draw(&mut v);
    }
    assert!(f.view.read_with(&v, |r, _| r.session().state().selection().is_some()), "precondition: a selection");
    let shown = f.view.read_with(&v, |r, _| r.visible_rows().iter().any(|row| row.caret.is_some()));
    assert!(shown, "the caret is on a visible row");
}

/// The mouse's back and forward side buttons go Back and Forward, as Alt+Left and Alt+Right do; a dialog open over
/// the view keeps them from it.
#[gpui::test]
fn mouse_side_buttons_go_back_and_forward(cx: &mut TestAppContext) {
    let f = fixture(cx, Some(("one.txt", "1\n".repeat(80).as_bytes())));
    settle(cx, &f, active_is("one.txt"));
    let two = f.dir.path().join("two.txt");
    std::fs::write(&two, "2\n".repeat(80)).unwrap();
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(600.0), px(300.0)));
    draw(&mut v);
    f.view.update(&mut v, |view, cx| view.open(two.clone(), cx));
    drop(v);
    settle(cx, &f, active_is("two.txt"));
    let side = |cx: &mut TestAppContext, direction: gpui::NavigationDirection| {
        let mut v = visual(cx, &f);
        let (button, position) = (MouseButton::Navigate(direction), point(px(300.0), px(150.0)));
        v.simulate_event(gpui::MouseDownEvent { button, position, modifiers: Modifiers::default(), click_count: 1, first_mouse: false });
        v.simulate_event(gpui::MouseUpEvent { button, position, modifiers: Modifiers::default(), click_count: 1 });
    };
    side(cx, gpui::NavigationDirection::Back);
    settle(cx, &f, active_is("one.txt"));
    side(cx, gpui::NavigationDirection::Forward);
    settle(cx, &f, active_is("two.txt"));
    // The command menu open: the button goes nowhere and the menu stays.
    let mut v = visual(cx, &f);
    v.simulate_keystrokes("f10");
    drop(v);
    side(cx, gpui::NavigationDirection::Back);
    drain(cx, &f, 100);
    assert!(f.view.read_with(cx, |r, _| r.modal.is_some()));
    assert!(f.view.read_with(cx, |r, _| active_is("two.txt")(r)));
}

/// The pointer over a context-menu entry highlights it, as Up and Down do (Enter then runs it); a separator keeps the
/// highlight where it was.
#[gpui::test]
fn hovering_a_context_menu_entry_highlights_it(cx: &mut TestAppContext) {
    let f = fixture(cx, Some(("main.rs", b"fn main() {\n    helper();\n}\nfn helper() {}\n")));
    settle(cx, &f, active_is("main.rs"));
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(800.0), px(400.0)));
    draw(&mut v);
    let text = f.view.read_with(&v, |r, _| r.text_bounds.unwrap());
    let p = point(text.origin.x + px(num_w() + 40.0), text.origin.y + px(line_h() * 1.5));
    v.simulate_event(gpui::MouseDownEvent { button: MouseButton::Right, position: p, modifiers: Modifiers::default(), click_count: 1, first_mouse: false });
    draw(&mut v);
    let rows = f.view.read_with(&v, |r, _| r.modal_rows());
    let at = |v: &mut VisualTestContext| f.view.read_with(v, |r, _| match &r.modal { Some(Modal::Context { at, .. }) => *at, other => panic!("{other:?}") });
    let (menu, entry_y) = f.view.read_with(&v, |r, _| (r.context_bounds().unwrap(), r.context_row_y("Copy Path").unwrap()));
    v.simulate_mouse_move(point(menu.origin.x + px(10.0), px(entry_y + line_h() / 2.0)), None, Modifiers::default());
    let copy_path = rows.iter().position(|r| r.0 == "Copy Path").unwrap();
    assert_eq!(at(&mut v), copy_path);
    v.simulate_keystrokes("enter");
    assert!(v.read_from_clipboard().and_then(|c| c.text()).is_some_and(|t| t.ends_with("main.rs")), "{:?}", v.read_from_clipboard());
}

/// A right-click while the pointer's rest is still counting down (its documentation not asked yet): the menu opens
/// alone, and no documentation pops up over it when the rest would have ended.
#[gpui::test]
fn a_right_click_cancels_documentation_waiting_on_the_pointer(cx: &mut TestAppContext) {
    let text = "fn main() { helper(); }\nfn helper() {}\n";
    let f = fixture_with(cx, None, &fake_lsp());
    let _pkg = fake_package(cx, &f, &[("src/main.rs", text)], "src/main.rs");
    settle(cx, &f, |r| r.session().state().active().is_some_and(|t| r.session().state().view(t.id()).is_some_and(|view| view.highlights().is_some())) && r.session().state().provider().is_some_and(|p| p.status == reader_core::semantics::Status::Ready));
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(800.0), px(400.0)));
    draw(&mut v);
    let tb = f.view.read_with(&v, |r, _| r.text_bounds.unwrap());
    let char_w = f.view.read_with(&v, |r, _| r.char_w);
    let at = point(px(f32::from(tb.origin.x) + num_w() + 8.0 + 4.5 * char_w), tb.origin.y + px(line_h() / 2.0));
    v.simulate_click(point(px(f32::from(tb.origin.x) + num_w() + 8.0 + 14.5 * char_w), at.y), Modifiers::default());
    v.simulate_mouse_move(at, None, Modifiers::default());
    assert!(f.view.read_with(&v, |r, _| r.doc_rest.1.is_some()), "the rest counts down");
    cx.executor().advance_clock(DOC_REST / 2);
    // Straight to the view: the test platform moves the pointer (button down) before a press, which a real one
    // does not, and that move would end the rest.
    let ev = gpui::MouseDownEvent { button: MouseButton::Right, position: at, modifiers: Modifiers::default(), click_count: 1, first_mouse: false };
    f.view.update_in(&mut v, |r, window, cx| r.on_right_down(&ev, window, cx));
    cx.executor().advance_clock(DOC_REST);
    drain(cx, &f, 300);
    assert!(f.view.read_with(cx, |r, _| r.session().state().documentation().is_none()), "no documentation over the menu");
    assert!(f.view.read_with(cx, |r, _| matches!(r.modal, Some(Modal::Context { .. }))));
}

/// Documentation already asked for (the rest ended, its answer on the way) or shown when a menu or dialog opens:
/// it closes, and its answer never lands over the menu.
#[gpui::test]
fn a_menu_or_dialog_closes_documentation_shown_or_on_its_way(cx: &mut TestAppContext) {
    let text = "fn main() { helper(); }\nfn helper() {}\n";
    let f = fixture_with(cx, None, &fake_lsp());
    let _pkg = fake_package(cx, &f, &[("src/main.rs", text)], "src/main.rs");
    settle(cx, &f, |r| r.session().state().active().is_some_and(|t| r.session().state().view(t.id()).is_some_and(|view| view.highlights().is_some())) && r.session().state().provider().is_some_and(|p| p.status == reader_core::semantics::Status::Ready));
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(800.0), px(400.0)));
    draw(&mut v);
    let tb = f.view.read_with(&v, |r, _| r.text_bounds.unwrap());
    let char_w = f.view.read_with(&v, |r, _| r.char_w);
    let col = |c: f32| point(px(f32::from(tb.origin.x) + num_w() + 8.0 + c * char_w), tb.origin.y + px(line_h() / 2.0));
    v.simulate_click(col(14.5), Modifiers::default());
    // The rest ends (asked), then a right-click before the answer.
    v.simulate_mouse_move(col(4.5), None, Modifiers::default());
    cx.executor().advance_clock(DOC_REST);
    let ev = gpui::MouseDownEvent { button: MouseButton::Right, position: col(4.5), modifiers: Modifiers::default(), click_count: 1, first_mouse: false };
    f.view.update_in(&mut v, |r, window, cx| r.on_right_down(&ev, window, cx));
    draw(&mut v);
    drain(cx, &f, 300);
    draw(&mut v);
    assert!(f.view.read_with(&v, |r, _| r.session().state().documentation().is_none() && r.doc_popup_bounds().is_none()), "nothing over the menu");
    v.simulate_keystrokes("escape");
    // Shown at the caret, then the command menu opens: it closes.
    v.simulate_keystrokes("ctrl-q");
    settle(cx, &f, |r| r.session().state().documentation().is_some());
    let mut v = visual(cx, &f);
    v.simulate_keystrokes("f10");
    draw(&mut v);
    assert!(f.view.read_with(&v, |r, _| r.modal.is_some() && r.session().state().documentation().is_none()));
}

/// Hidden entries (a name starting with a dot, and everything inside a hidden folder) are greyed out in the tree, as
/// IntelliJ does; others keep the text colour.
#[gpui::test]
fn hidden_files_and_folders_are_greyed_out_in_the_tree(cx: &mut TestAppContext) {
    let f = fixture(cx, None);
    let proj = f.dir.path().join("proj");
    for (name, text) in [("src/main.rs", "fn main() {}\n"), (".gitignore", "target\n"), (".github/workflows/ci.yml", "on: push\n"), ("src/.env", "A=1\n")] {
        std::fs::create_dir_all(proj.join(name).parent().unwrap()).unwrap();
        std::fs::write(proj.join(name), text).unwrap();
    }
    f.view.update(cx, |v, cx| v.dispatch(Command::OpenRoot(proj.clone()), cx));
    settle(cx, &f, |v| v.session().state().project().is_some_and(|p| p.files() == 4));
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(800.0), px(400.0)));
    // `.github` opens down to `workflows`, its only entry.
    for dir in [".github", "src"] {
        f.view.update(&mut v, |r, cx| r.dispatch(Command::ToggleDir(dir.into()), cx));
        draw(&mut v);
    }
    let hidden: Vec<(String, bool)> = f.view.read_with(&v, |r, _| r.tree_lines(usize::MAX).into_iter().map(|l| (l.name, l.hidden)).collect());
    for (name, want) in [(".github", true), ("workflows", true), ("ci.yml", true), (".gitignore", true), (".env", true), ("src", false), ("main.rs", false)] {
        assert!(hidden.contains(&(name.to_string(), want)), "{name} hidden: {want} in {hidden:?}");
    }
}

#[gpui::test]
fn overflowing_context_menu_wheel_moves_viewport_and_keeps_keyboard_selection(cx: &mut TestAppContext) {
    let bytes = b"ordinary source\nsecond line\n";
    let f = fixture(cx, Some(("small.txt", bytes)));
    settle(cx, &f, active_is("small.txt"));
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(400.0), px(180.0)));
    draw(&mut v);
    let text = f.view.read_with(&v, |r, _| r.text_bounds.unwrap());
    v.simulate_event(gpui::MouseDownEvent { button: MouseButton::Right,
        position: point(text.origin.x + px(num_w() + 8.0), text.origin.y + px(10.0)),
        modifiers: Modifiers::default(), click_count: 1, first_mouse: false });
    draw(&mut v);
    let bounds = f.view.read_with(&v, |r, _| r.context_bounds().unwrap());
    assert!(bounds.origin.x >= px(0.0) && bounds.right() <= px(400.0));
    assert!(bounds.origin.y >= px(0.0) && bounds.bottom() <= px(180.0));
    v.simulate_keystrokes("end");
    draw(&mut v);
    f.view.read_with(&v, |r, _| {
        let Modal::Context { entries, at, .. } = r.modal.as_ref().unwrap() else { panic!("menu closed") };
        assert_eq!(*at, entries.iter().rposition(|e| matches!(e, MenuEntry::Command(_))).unwrap());
        let (bounds, rows) = r.context_layout().unwrap();
        assert!(rows[*at].0 >= f32::from(bounds.origin.y));
        assert!(rows[*at].0 + rows[*at].1 <= f32::from(bounds.bottom()));
    });
    let selected = f.view.read_with(&v, |r, _| r.modal.as_ref().unwrap().at());
    let old_scroll = f.view.read_with(&v, |r, _| match r.modal { Some(Modal::Context { scroll, .. }) => scroll, _ => unreachable!() });
    let before = f.view.read_with(&v, |r, _| r.session.state().top_line());
    v.simulate_event(gpui::ScrollWheelEvent { position: bounds.center(), delta: ScrollDelta::Lines(point(0.0, 3.0)),
        modifiers: Modifiers::default(), touch_phase: gpui::TouchPhase::Moved });
    draw(&mut v);
    f.view.read_with(&v, |r, _| {
        assert_eq!(r.session.state().top_line(), before);
        let Modal::Context { at, scroll, .. } = r.modal.as_ref().unwrap() else { panic!("menu closed") };
        assert_eq!(*at, selected, "the wheel keeps the selected command");
        assert!(*scroll < old_scroll, "the viewport moves toward the start");
    });
    let old_scroll = f.view.read_with(&v, |r, _| match r.modal { Some(Modal::Context { scroll, .. }) => scroll, _ => unreachable!() });
    for _ in 0..30 {
        v.simulate_event(gpui::ScrollWheelEvent { position: bounds.center(), delta: ScrollDelta::Pixels(point(px(0.0), px(2.0))),
            modifiers: Modifiers::default(), touch_phase: gpui::TouchPhase::Moved });
    }
    assert_eq!(f.view.read_with(&v, |r, _| r.modal.as_ref().unwrap().at()), selected);
    assert!(f.view.read_with(&v, |r, _| matches!(r.modal, Some(Modal::Context { scroll, .. }) if scroll < old_scroll)), "small trackpad deltas move the viewport");
    v.simulate_keystrokes("up");
    draw(&mut v);
    f.view.read_with(&v, |r, _| {
        let at = r.modal.as_ref().unwrap().at();
        let (b, rows) = r.context_layout().unwrap();
        assert!(rows[at].0 >= f32::from(b.origin.y) && rows[at].0 + rows[at].1 <= f32::from(b.bottom()));
    });
    v.simulate_keystrokes("escape");
    assert!(f.view.read_with(&v, |r, _| r.modal.is_none()));
    assert_eq!(std::fs::read(f.dir.path().join("small.txt")).unwrap(), bytes);
}

#[gpui::test]
#[cfg(unix)]
fn server_runtime_dialog_validates_saves_and_preserves_the_document(cx: &mut TestAppContext) {
    use std::os::unix::fs::PermissionsExt;
    let source = "class Main {}\n";
    let f = fixture(cx, Some(("Main.java", source.as_bytes())));
    settle(cx, &f, |r| r.session().state().current_source().is_some());
    let java = f.dir.path().join("java with spaces");
    std::fs::write(&java, "#!/bin/sh\ntest \"$1\" = -version\n").unwrap();
    std::fs::set_permissions(&java, std::fs::Permissions::from_mode(0o755)).unwrap();
    let settings = f.dir.path().join("config/lenz-reader/providers.json");
    let original = std::fs::read(&settings).unwrap();
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(800.0), px(400.0)));
    v.simulate_keystrokes("ctrl-a f2");
    let selected = f.view.read_with(&v, |r, _| r.session().state().selection());
    let at = f.view.read_with(&v, |r, _| r.server_rows().iter().position(|(_, action)| *action == Some(CommandId::ServerRuntime)).unwrap());
    for _ in 0..at { v.simulate_keystrokes("down"); }
    v.simulate_keystrokes("enter");
    assert!(f.view.read_with(&v, |r, _| matches!(r.modal, Some(Modal::ServerRuntime { .. }))));
    v.update(|_, cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string("/nonexistent/java".into())));
    v.simulate_keystrokes("ctrl-v enter");
    drop(v);
    settle(cx, &f, |r| r.session().state().server_runtime().unwrap().message.contains("not changed"));
    assert_eq!(std::fs::read(&settings).unwrap(), original);
    let mut v = visual(cx, &f);
    for (w, h) in [(180.0, 90.0), (1.0, 1.0), (800.0, 400.0)] {
        v.simulate_resize(size(px(w), px(h)));
        v.simulate_keystrokes("end up");
        draw(&mut v);
        assert!(f.view.read_with(&v, |r, _| !r.modal_lines().is_empty()));
    }
    v.update(|_, cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string(java.to_str().unwrap().into())));
    v.simulate_keystrokes("ctrl-a ctrl-v enter");
    drop(v);
    settle(cx, &f, |r| r.session().state().server_runtime().unwrap().message.contains("saved."));
    let mut v = visual(cx, &f);
    assert_eq!(f.view.read_with(&v, |r, _| r.session().state().server_runtime().unwrap().configured.clone()), Some(java.clone()));
    v.simulate_keystrokes("f5 escape");
    assert!(f.view.read_with(&v, |r, _| r.modal.is_none() && r.session().state().server_runtime().is_none()));
    assert_eq!(f.view.read_with(&v, |r, _| r.session().state().selection()), selected);
    let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(settings).unwrap()).unwrap();
    assert_eq!(saved["providers"]["jdtls"]["args"], serde_json::json!([format!("--java-executable={}",java.display())]));
    assert_eq!(std::fs::read_to_string(f.dir.path().join("Main.java")).unwrap(), source);
}

#[gpui::test]
fn app_menu_groups_existing_actions_and_keeps_clicks_modal(cx: &mut TestAppContext) {
    let source = b"first\nsecond\n";
    let f = fixture(cx, Some(("note.txt", source)));
    settle(cx, &f, |r| r.session().state().current_source().is_some());
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(900.0), px(500.0)));
    draw(&mut v);
    let menu = f.view.read_with(&v, |r, _| point(px(r.sidebar_w + 4.0 + line_h() / 2.0), px(line_h() / 2.0)));
    v.simulate_click(menu, Modifiers::default());
    draw(&mut v);
    assert_eq!(f.view.read_with(&v, |r, _| r.modal_rows()[0].0.clone()), "File ›");
    v.simulate_keystrokes("down enter"); // View
    draw(&mut v);
    assert!(f.view.read_with(&v, |r, _| r.modal_rows().iter().any(|(label, _)| label == "Soft Wrap (off)")));
    v.simulate_keystrokes("enter"); // Soft Wrap
    assert!(f.view.read_with(&v, |r, _| r.session().state().soft_wrap()));
    assert!(f.view.read_with(&v, |r, _| r.modal.is_none()));
    v.simulate_click(menu, Modifiers::default());
    v.simulate_keystrokes("ctrl-a ctrl-c ctrl-w");
    assert!(f.view.read_with(&v, |r, _| r.session().state().active().unwrap().selection().is_none()));
    v.simulate_click(point(px(600.0), px(200.0)), Modifiers::default());
    assert!(f.view.read_with(&v, |r, _| r.modal.is_none()));
    assert!(f.view.read_with(&v, |r, _| r.session().state().active().unwrap().selection().is_none()));
    assert_eq!(std::fs::read(f.dir.path().join("note.txt")).unwrap(), source);
}

#[gpui::test]
fn footer_commands_opens_catalog_while_left_status_keeps_server_details(cx: &mut TestAppContext) {
    let f = fixture(cx, None);
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(900.0), px(500.0)));
    draw(&mut v);
    v.simulate_click(point(px(860.0), px(490.0)), Modifiers::default());
    assert!(f.view.read_with(&v, |r, _| matches!(r.modal, Some(Modal::Navigation { .. }))));
    settle(cx, &f, |r| r.session.state().navigation().is_some_and(|s| !s.pending));
    assert_eq!(f.view.read_with(&v, |r, _| r.session.state().navigation().unwrap().scope), reader_core::navigation::Scope::Actions);
    v.simulate_keystrokes("escape");
    v.simulate_click(point(px(20.0), px(490.0)), Modifiers::default());
    assert!(f.view.read_with(&v, |r, _| matches!(r.modal, Some(Modal::LanguageServer { .. }))));
}

#[gpui::test]
fn menu_groups_support_pointer_back_disabled_actions_and_tiny_appearances(cx: &mut TestAppContext) {
    let f = fixture(cx, None);
    let mut v = visual(cx, &f);
    let click_row = |v: &mut VisualTestContext, label: &str| {
        draw(v);
        let point = f.view.read_with(v, |r, _| {
            let l = r.modal_layout().unwrap();
            let n = l.lines.iter().enumerate().skip(l.first).take(l.visible).find(|(_, (_, text, _))| text.starts_with(label)).unwrap().0;
            point(l.bounds.origin.x + px(16.0), l.bounds.origin.y + px(4.0 + (n - l.first) as f32 * line_h() + line_h() / 2.0))
        });
        v.simulate_click(point, Modifiers::default());
    };
    for dark in [true, false] {
        f.view.update(&mut v, |r, cx| r.dispatch(Command::Theme(ThemeCommand::Select(ThemeSelection::from_id(if dark { "darcula" } else { "github-light" }).unwrap())), cx));
        v.simulate_resize(size(px(800.0), px(500.0)));
        draw(&mut v);
        let button = f.view.read_with(&v, |r, _| r.app_menu_button().center());
        v.simulate_click(button, Modifiers::default());
        click_row(&mut v, "File");
        assert!(f.view.read_with(&v, |r, _| r.modal_rows().iter().any(|(s, why)| s == "Reload" && *why == Some("No file is open"))));
        click_row(&mut v, "‹ All menus");
        click_row(&mut v, "Language Servers");
        assert!(f.view.read_with(&v, |r, _| r.modal_rows().iter().any(|(s, _)| s == CommandId::ServerDetails.label())));
        click_row(&mut v, "Server Details");
        assert!(f.view.read_with(&v, |r, _| matches!(r.modal, Some(Modal::StatusDetails { title: "Server details", .. }))));
        v.simulate_keystrokes("escape");
        v.simulate_click(button, Modifiers::default());
        click_row(&mut v, "Language Servers");
        v.simulate_keystrokes("escape end enter");
        assert!(f.view.read_with(&v, |r, _| matches!(r.modal, Some(Modal::Navigation { .. }))));
        v.simulate_keystrokes("escape");
        for (w, h) in [(180.0, 90.0), (120.0, 40.0), (1.0, 1.0)] {
            v.simulate_resize(size(px(w), px(h)));
            draw(&mut v);
            f.view.update(&mut v, |r, cx| { r.modal = Some(Modal::AppMenu { at: 0 }); cx.notify(); });
            v.simulate_keystrokes("end up right end up");
            draw(&mut v);
            assert!(f.view.read_with(&v, |r, _| r.modal_layout().is_some_and(|l| !l.lines.is_empty() && l.first < l.lines.len())));
            v.simulate_keystrokes("escape escape");
            assert!(f.view.read_with(&v, |r, _| r.modal.is_none()));
        }
    }
}

#[gpui::test]
fn app_menu_wheel_reveals_long_groups_without_scrolling_the_document(cx: &mut TestAppContext) {
    let source = "reader line\n".repeat(100);
    let f = fixture(cx, Some(("note.txt", source.as_bytes())));
    settle(cx, &f, |r| r.session().state().current_source().is_some());
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(180.0), px(90.0)));
    draw(&mut v);
    v.simulate_click(point(px(12.0), px(10.0)), Modifiers::default());
    v.simulate_keystrokes("down enter");
    draw(&mut v);
    let selected = f.view.read_with(&v, |r, _| r.modal.as_ref().unwrap().at());
    v.simulate_event(ScrollWheelEvent { position: point(px(50.0), px(40.0)), delta: ScrollDelta::Lines(point(0.0, -1000.0)), ..Default::default() });
    draw(&mut v);
    assert!(f.view.read_with(&v, |r, _| r.modal.as_ref().unwrap().at() == selected && r.modal_layout().unwrap().first > 0));
    v.simulate_event(ScrollWheelEvent { position: point(px(50.0), px(40.0)), delta: ScrollDelta::Lines(point(0.0, 1.0)), ..Default::default() });
    draw(&mut v);
    assert!(f.view.read_with(&v, |r, _| r.modal.as_ref().unwrap().at() == selected));
    v.simulate_keystrokes("escape escape");
    assert!(f.view.read_with(&v, |r, _| r.session().state().active().unwrap().top_line() == 0 && r.modal.is_none()));
    assert_eq!(std::fs::read(f.dir.path().join("note.txt")).unwrap(), source.as_bytes());
}

/// Click a unified search result using the geometry and revision from its last frame.
fn click_navigation_row(f: &Fixture, v: &mut VisualTestContext, row: usize) {
    draw(v);
    let p = f.view.read_with(v, |r, _| {
        let (_, first, visible) = r.navigation_drawn.unwrap();
        assert!((first..first + visible).contains(&row));
        let b = r.navigation_bounds();
        point(b.center().x, b.origin.y + px(4.0 + (3.0 + (row - first) as f32 * 2.0 + 0.5) * line_h()))
    });
    v.simulate_click(p, Modifiers::default());
}

#[gpui::test]
fn unified_search_switches_categories_preserving_query_and_runs_actions(cx: &mut TestAppContext) {
    use reader_core::navigation::Scope;
    let source = b"fn sample() {}\n";
    let f = fixture(cx, Some(("main.rs", source)));
    settle(cx, &f, |r| r.session.state().current_source().is_some());
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(900.0), px(500.0)));
    v.simulate_keystrokes("ctrl-k s o f t");
    assert_eq!(f.view.read_with(&v, |r, _| { let n = r.session.state().navigation().unwrap(); (n.scope, n.query.clone()) }), (Scope::All, "soft".to_string()));
    for (keys, scope) in [("ctrl-p", Scope::Files), ("ctrl-shift-n", Scope::Files), ("ctrl-alt-shift-n", Scope::Symbols), ("ctrl-shift-p", Scope::Actions), ("ctrl-shift-a", Scope::Actions), ("ctrl-k", Scope::All)] {
        v.simulate_keystrokes(keys);
        assert_eq!(f.view.read_with(&v, |r, _| { let n = r.session.state().navigation().unwrap(); (n.scope, n.query.clone()) }), (scope, "soft".to_string()), "{keys} preserves the query");
    }
    v.simulate_keystrokes("tab tab tab");
    assert_eq!(f.view.read_with(&v, |r, _| { let n = r.session.state().navigation().unwrap(); (n.scope, n.query.clone()) }), (Scope::Actions, "soft".to_string()));
    v.simulate_keystrokes("shift-tab");
    assert_eq!(f.view.read_with(&v, |r, _| r.session.state().navigation().unwrap().scope), Scope::Symbols);
    v.simulate_keystrokes("tab");
    drop(v);
    settle(cx, &f, |r| r.session.state().navigation().is_some_and(|n| !n.pending && n.rows.iter().any(|r| r.label == "Soft Wrap")));
    let mut v = visual(cx, &f);
    draw(&mut v);
    let rows = f.view.read_with(&v, |r, _| r.modal_rows());
    assert!(rows.iter().any(|(r, _)| r.contains("Soft Wrap (off)")));
    v.simulate_keystrokes("enter");
    assert!(f.view.read_with(&v, |r, _| r.modal.is_none() && r.session.state().soft_wrap()));
    v.simulate_keystrokes("ctrl-shift-a ctrl-a s o f t");
    drop(v);
    settle(cx, &f, |r| r.session.state().navigation().is_some_and(|n| !n.pending && !n.rows.is_empty()));
    let mut v = visual(cx, &f);
    draw(&mut v);
    click_navigation_row(&f, &mut v, 0);
    assert!(!f.view.read_with(&v, |r, _| r.session.state().soft_wrap()));
    assert_eq!(std::fs::read(f.dir.path().join("main.rs")).unwrap(), source);
}

#[gpui::test]
fn unified_search_opens_syntactic_symbols_without_a_language_server_and_file_positions(cx: &mut TestAppContext) {
    let f = fixture(cx, None);
    let root = project(f.dir.path(), &["src/main.rs"]);
    let source = "fn first() {}\n\nfn navigate_target() {}\n";
    std::fs::write(root.join("src/main.rs"), source).unwrap();
    f.view.update(cx, |r, cx| r.open_root(root.clone(), cx));
    settle(cx, &f, scanned);
    let mut v = visual(cx, &f);
    v.simulate_keystrokes("ctrl-alt-shift-n n a v i g a t e");
    drop(v);
    settle(cx, &f, |r| r.session.state().navigation().is_some_and(|n| !n.pending && n.rows.iter().any(|r| r.label.contains("navigate_target"))));
    let mut v = visual(cx, &f);
    draw(&mut v);
    assert!(f.view.read_with(&v, |r, _| r.modal_rows().iter().any(|(s, _)| s.to_lowercase().contains("syntactic"))));
    v.simulate_keystrokes("enter");
    drop(v);
    settle(cx, &f, |r| r.modal.is_none() && r.session.state().current_source().is_some());
    let mut v = visual(cx, &f);
    assert_eq!(caret_line(&f, &v), 2);
    v.simulate_keystrokes("ctrl-shift-n m a i n : 2 : 1");
    drop(v);
    settle(cx, &f, |r| r.session.state().navigation().is_some_and(|n| !n.pending && !n.rows.is_empty()));
    let mut v = visual(cx, &f);
    v.simulate_keystrokes("enter");
    drop(v);
    settle(cx, &f, |r| r.modal.is_none() && r.session.state().selection().is_some_and(|s| s.head == source.find('\n').unwrap() + 1));
    assert_eq!(std::fs::read_to_string(root.join("src/main.rs")).unwrap(), source);
}

#[gpui::test]
fn double_shift_opens_unified_search_but_typing_and_other_modals_interrupt_it(cx: &mut TestAppContext) {
    let f = fixture(cx, Some(("note.txt", b"first\nsecond\n")));
    settle(cx, &f, |r| r.session.state().current_source().is_some());
    let mut v = visual(cx, &f);
    draw(&mut v);
    v.update(|w, _| w.activate_window());
    let shift = Modifiers { shift: true, ..Default::default() };
    for _ in 0..2 { v.simulate_modifiers_change(shift); v.simulate_modifiers_change(Modifiers::default()); }
    assert!(f.view.read_with(&v, |r, _| matches!(r.modal, Some(Modal::Navigation { .. }))));
    v.simulate_keystrokes("escape");
    v.simulate_modifiers_change(shift);
    v.simulate_modifiers_change(Modifiers::default());
    v.deactivate_window();
    v.update(|w, _| w.activate_window());
    v.simulate_modifiers_change(shift);
    v.simulate_modifiers_change(Modifiers::default());
    assert!(f.view.read_with(&v, |r, _| r.modal.is_none()), "a tap before focus loss does not count");
    v.simulate_modifiers_change(shift);
    v.simulate_keystrokes("shift-right");
    v.simulate_modifiers_change(Modifiers::default());
    v.simulate_modifiers_change(shift);
    v.simulate_modifiers_change(Modifiers::default());
    assert!(f.view.read_with(&v, |r, _| r.modal.is_none()));
    v.simulate_keystrokes("f10");
    for _ in 0..2 { v.simulate_modifiers_change(shift); v.simulate_modifiers_change(Modifiers::default()); }
    assert!(f.view.read_with(&v, |r, _| matches!(r.modal, Some(Modal::Commands { .. }))));
    v.simulate_keystrokes("escape ctrl-k");
    let before = f.view.read_with(&v, |r, _| r.session.state().selection());
    v.simulate_keystrokes("ctrl-a ctrl-w ctrl-s ctrl-f4 shift-right");
    draw(&mut v);
    v.simulate_click(point(px(5.0), px(100.0)), Modifiers::default());
    assert_eq!(f.view.read_with(&v, |r, _| r.session.state().selection()), before);
    assert_eq!(f.view.read_with(&v, |r, _| r.session.state().tabs().len()), 1);
    assert_eq!(std::fs::read(f.dir.path().join("note.txt")).unwrap(), b"first\nsecond\n");
}

#[gpui::test]
fn unified_search_empty_constructor_paste_tabs_resize_and_appearances(cx: &mut TestAppContext) {
    use reader_core::navigation::Scope;
    let f = fixture(cx, None);
    let mut v = visual(cx, &f);
    v.simulate_keystrokes("ctrl-p");
    assert_eq!(f.view.read_with(&v, |r, _| r.session.state().navigation().unwrap().scope), Scope::Files);
    assert!(f.view.read_with(&v, |r, _| r.modal_rows().iter().any(|(s, _)| s.contains("Open a folder"))));
    v.simulate_keystrokes("escape");
    for dark in [true, false] {
        f.view.update(&mut v, |r, cx| r.dispatch(Command::Theme(ThemeCommand::Select(ThemeSelection::from_id(if dark { "darcula" } else { "github-light" }).unwrap())), cx));
        for (w, h) in [(900.0, 500.0), (180.0, 90.0), (1.0, 1.0)] {
            v.simulate_resize(size(px(w), px(h)));
            v.simulate_keystrokes("ctrl-k end up pageup pagedown home");
            draw(&mut v);
            let (bounds, at, n) = f.view.read_with(&v, |r, _| (r.navigation_bounds(), r.modal.as_ref().unwrap().at(), r.session.state().navigation().unwrap().rows.len()));
            assert!(f32::from(bounds.size.width) <= w && f32::from(bounds.size.height) <= h);
            assert!(at <= n.saturating_sub(1));
            v.simulate_keystrokes("escape");
        }
    }
    v.simulate_resize(size(px(900.0), px(500.0)));
    v.update(|_, cx| cx.write_to_clipboard(ClipboardItem::new_string("Soft Wrap".into())));
    v.simulate_keystrokes("ctrl-shift-p ctrl-v");
    draw(&mut v);
    let b = f.view.read_with(&v, |r, _| r.navigation_bounds());
    v.simulate_click(point(b.origin.x + b.size.width / 8.0, b.origin.y + px(10.0)), Modifiers::default());
    assert_eq!(f.view.read_with(&v, |r, _| { let n = r.session.state().navigation().unwrap(); (n.scope, n.query.clone()) }), (Scope::All, "Soft Wrap".to_string()));
    v.simulate_keystrokes("ctrl-a");
    assert!(f.view.read_with(&v, |r, _| r.session.state().navigation().unwrap().query.is_empty()));
    let long_query = format!("{}終端", "Été/".repeat(40));
    v.update(|_, cx| cx.write_to_clipboard(ClipboardItem::new_string(long_query.clone())));
    v.simulate_keystrokes("ctrl-v");
    for (w, h) in [(180.0, 90.0), (1.0, 1.0)] {
        v.simulate_resize(size(px(w), px(h)));
        draw(&mut v);
        assert_eq!(f.view.read_with(&v, |r, _| r.session.state().navigation().unwrap().query.clone()), long_query);
    }
    v.simulate_keystrokes("escape");
}

#[gpui::test]
fn unified_search_rejects_stale_mouse_rows_and_disabled_actions(cx: &mut TestAppContext) {
    use reader_core::navigation::Scope;
    let f = fixture(cx, None);
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(900.0), px(500.0)));
    v.simulate_keystrokes("ctrl-shift-a s o f t");
    drop(v);
    settle(cx, &f, |r| r.session.state().navigation().is_some_and(|n| !n.pending && !n.rows.is_empty()));
    let mut v = visual(cx, &f);
    draw(&mut v);
    let (revision, p) = f.view.read_with(&v, |r, _| {
        let b = r.navigation_bounds();
        (r.navigation_drawn.unwrap().0, point(b.center().x, b.origin.y + px(4.0 + line_h() * 3.5)))
    });
    // A replacement snapshot arrives between paint and mouse-down. Its row zero is not the row drawn there.
    f.view.update(&mut v, |r, cx| r.dispatch(Command::SearchEverywhere { scope: Scope::Actions, query: "Sticky Scroll".into() }, cx));
    drop(v);
    settle(cx, &f, |r| r.session.state().navigation().is_some_and(|n| !n.pending && n.revision != revision));
    let mut v = visual(cx, &f);
    // Keep the previous painted hit identity while delivering a real GPUI mouse event.
    f.view.update(&mut v, |r, _| r.navigation_drawn = Some((revision, 0, 1)));
    v.simulate_event(MouseDownEvent { position: p, button: MouseButton::Left, ..Default::default() });
    assert!(f.view.read_with(&v, |r, _| matches!(r.modal, Some(Modal::Navigation { .. })) && !r.session.state().soft_wrap()));
    v.simulate_keystrokes("ctrl-a r e l o a d");
    drop(v);
    settle(cx, &f, |r| r.session.state().navigation().is_some_and(|n| !n.pending && n.query == "reload"));
    let mut v = visual(cx, &f);
    assert!(f.view.read_with(&v, |r, _| r.modal_rows().iter().any(|(s, why)| s.contains("Reload") && *why == Some("No file is open"))));
    v.simulate_keystrokes("enter");
    assert!(f.view.read_with(&v, |r, _| matches!(r.modal, Some(Modal::Navigation { .. }))));
    v.simulate_keystrokes("escape");
}

#[gpui::test]
fn navigation_wheel_keeps_enter_target_and_accumulates_trackpad_scroll(cx: &mut TestAppContext) {
    let bytes = b"reader bytes\n";
    let f = fixture(cx, Some(("note.txt", bytes)));
    settle(cx, &f, active_is("note.txt"));
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(700.0), px(260.0)));
    v.simulate_keystrokes("ctrl-shift-a");
    drop(v);
    settle(cx, &f, |r| r.session.state().navigation().is_some_and(|n| !n.pending && n.rows.len() > 10));
    let mut v = visual(cx, &f);
    draw(&mut v);
    let selected = f.view.read_with(&v, |r, _| r.session.state().navigation().unwrap().rows.iter().position(|row| row.label == "Soft Wrap").unwrap());
    for _ in 0..selected { v.simulate_keystrokes("down"); }
    draw(&mut v);
    let first = f.view.read_with(&v, |r, _| r.navigation_drawn.unwrap().1);
    let position = f.view.read_with(&v, |r, _| r.navigation_bounds().center());
    for _ in 0..30 {
        v.simulate_event(ScrollWheelEvent { position, delta: ScrollDelta::Pixels(point(px(0.0), px(-2.0))), ..Default::default() });
    }
    draw(&mut v);
    assert_eq!(f.view.read_with(&v, |r, _| r.modal.as_ref().unwrap().at()), selected);
    assert!(f.view.read_with(&v, |r, _| r.navigation_drawn.unwrap().1 > first), "small trackpad events accumulate");
    assert_eq!(f.view.read_with(&v, |r, _| r.session.state().top_line()), 0);
    v.simulate_event(ScrollWheelEvent { position, delta: ScrollDelta::Lines(point(0.0, -1000.0)), ..Default::default() });
    draw(&mut v);
    assert!(f.view.read_with(&v, |r, _| r.navigation_drawn.unwrap().1 > selected), "the selected command is offscreen");
    v.simulate_keystrokes("enter");
    assert!(f.view.read_with(&v, |r, _| r.modal.is_none() && r.session.state().soft_wrap()), "Enter runs the original selected action offscreen");
    assert_eq!(std::fs::read(f.dir.path().join("note.txt")).unwrap(), bytes);
}

#[gpui::test]
fn plain_arrows_move_unicode_caret_and_reveal_without_editing(cx: &mut TestAppContext) {
    let text = format!("é🙂{}\nsecond\n{}", "x".repeat(200), "line\n".repeat(60));
    let f = fixture(cx, Some(("arrows.txt", text.as_bytes())));
    settle(cx, &f, active_is("arrows.txt"));
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(360.0), px(240.0)));
    draw(&mut v);
    v.simulate_keystrokes("right");
    assert_eq!(f.view.read_with(&v, |r, _| r.session.state().selection()), Some(Selection { anchor: 2, head: 2 }));
    v.simulate_keystrokes("right");
    assert_eq!(f.view.read_with(&v, |r, _| r.session.state().selection()), Some(Selection { anchor: 6, head: 6 }));
    v.simulate_keystrokes("left shift-right");
    assert_eq!(f.view.read_with(&v, |r, _| r.session.state().selection()), Some(Selection { anchor: 2, head: 6 }));
    v.simulate_keystrokes("left");
    assert_eq!(f.view.read_with(&v, |r, _| r.session.state().selection()), Some(Selection { anchor: 2, head: 2 }));
    for _ in 0..100 { v.simulate_keystrokes("right"); }
    draw(&mut v);
    assert!(f.view.read_with(&v, |r, _| r.session.state().column() > 0), "horizontal motion reveals the caret");
    assert!(f.view.read_with(&v, |r, _| r.visible_rows().iter().any(|row| row.caret.is_some())));
    for _ in 0..25 { v.simulate_keystrokes("down"); }
    draw(&mut v);
    assert!(f.view.read_with(&v, |r, _| r.session.state().top_line() > 0), "vertical motion reveals the caret");
    assert!(f.view.read_with(&v, |r, _| r.session.state().selection().is_some_and(|s| s.anchor == s.head)));
    let line = caret_line(&f, &v);
    v.simulate_keystrokes("up");
    assert_eq!(caret_line(&f, &v), line - 1);
    let selected = f.view.read_with(&v, |r, _| r.session.state().selection());
    v.simulate_keystrokes("f10 down right up left escape");
    assert_eq!(f.view.read_with(&v, |r, _| r.session.state().selection()), selected, "modal navigation leaves text alone");
    assert_eq!(std::fs::read_to_string(f.dir.path().join("arrows.txt")).unwrap(), text);
}

#[gpui::test]
fn plain_arrows_follow_wrapped_and_folded_visible_rows(cx: &mut TestAppContext) {
    let text = format!("fn folded() {{\n{}\n}}\nfn next() {{}}\n", "    word word word word word word word word word word;\n".repeat(30));
    let f = fixture(cx, Some(("folded.rs", text.as_bytes())));
    settle(cx, &f, |r| r.session.state().active().is_some_and(|tab| r.session.state().view(tab.id()).is_some_and(|view| view.fold_at(0).is_some())));
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(360.0), px(240.0)));
    draw(&mut v);
    v.simulate_keystrokes("right ctrl-shift-- down");
    assert_eq!(caret_line(&f, &v), 33, "Down skips the collapsed function");
    v.simulate_keystrokes("up");
    assert_eq!(caret_line(&f, &v), 0);
    v.simulate_keystrokes("ctrl-shift-= alt-z");
    draw(&mut v);
    for _ in 0..25 { v.simulate_keystrokes("down"); draw(&mut v); }
    assert!(f.view.read_with(&v, |r, _| r.visible_rows().iter().any(|row| row.caret.is_some())), "wrapped motion keeps its caret shown");
    assert!(f.view.read_with(&v, |r, _| r.session.state().selection().is_some_and(|s| s.anchor == s.head)));
    assert_eq!(std::fs::read_to_string(f.dir.path().join("folded.rs")).unwrap(), text);
}

#[gpui::test]
fn recent_files_use_search_presentation_filter_delete_and_keep_query_between_modes(cx: &mut TestAppContext) {
    let f = fixture(cx, None);
    let root = project(f.dir.path(), &["nested/Été.rs", "other/plain.rs", "not-visited.rs"]);
    f.view.update(cx, |r, cx| r.open_root(root.clone(), cx));
    settle(cx, &f, scanned);
    for name in ["nested/Été.rs", "other/plain.rs"] {
        f.view.update(cx, |r, cx| r.open(root.join(name), cx));
        settle(cx, &f, active_is(name));
    }
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(800.0), px(500.0)));
    v.simulate_keystrokes("ctrl-e");
    drop(v);
    settle(cx, &f, |r| r.session.state().navigation().is_some_and(|n| !n.pending));
    let mut v = visual(cx, &f);
    draw(&mut v);
    assert_eq!(f.view.read_with(&v, |r, _| r.modal_rows()[0].0.clone()), "Recent Files");
    f.view.read_with(&v, |r, _| {
        let n = r.session.state().navigation().unwrap();
        assert_eq!(n.scope.label(), "Recent Files");
        assert_eq!(n.rows[0].label, "Été.rs");
        assert!(n.rows[0].detail.contains("nested/Été.rs"));
        assert_eq!(n.rows[1].label, "plain.rs");
        assert!(!n.rows.iter().any(|row| row.label == "not-visited.rs"));
    });
    v.simulate_keystrokes("tab shift-tab");
    assert_eq!(f.view.read_with(&v, |r, _| r.session.state().navigation().unwrap().scope.label()), "Recent Files");
    v.simulate_keystrokes("é r"); // subsequence in Été.rs; a substring-only filter would fail
    drop(v);
    settle(cx, &f, |r| r.session.state().navigation().is_some_and(|n| !n.pending && n.query == "ér"));
    let mut v = visual(cx, &f);
    assert_eq!(f.view.read_with(&v, |r, _| r.session.state().navigation().unwrap().rows.len()), 1);
    for keys in ["ctrl-p", "ctrl-e", "ctrl-shift-a", "ctrl-e"] {
        v.simulate_keystrokes(keys);
        assert_eq!(f.view.read_with(&v, |r, _| r.session.state().navigation().unwrap().query.clone()), "ér");
    }
    v.simulate_keystrokes("backspace");
    drop(v);
    settle(cx, &f, |r| r.session.state().navigation().is_some_and(|n| !n.pending && n.query == "é"));
    let mut v = visual(cx, &f);
    draw(&mut v);
    v.simulate_keystrokes("delete");
    drop(v);
    settle(cx, &f, |r| r.session.state().navigation().is_some_and(|n| !n.pending && n.rows.is_empty()));
    assert!(!f.view.read_with(cx, |r, _| r.session.state().places().recent_files().iter().any(|file| file.ends_with("Été.rs"))));
    let mut v = visual(cx, &f);
    v.simulate_keystrokes("ctrl-a");
    for dark in [true, false] {
        f.view.update(&mut v, |r, cx| r.dispatch(Command::Theme(ThemeCommand::Select(ThemeSelection::from_id(if dark { "darcula" } else { "github-light" }).unwrap())), cx));
        for (w, h) in [(800.0, 500.0), (180.0, 90.0), (1.0, 1.0)] {
            v.simulate_resize(size(px(w), px(h)));
            v.simulate_keystrokes("end up");
            draw(&mut v);
            let b = f.view.read_with(&v, |r, _| r.navigation_bounds());
            assert!(f32::from(b.size.width) <= w && f32::from(b.size.height) <= h);
        }
    }
    v.simulate_keystrokes("escape");
}

#[gpui::test]
fn wheel_scrolls_theme_place_and_server_dialogs_without_selecting(cx: &mut TestAppContext) {
    let text = "class Main {}\n".repeat(40);
    let f = fixture(cx, Some(("Main.java", text.as_bytes())));
    settle(cx, &f, active_is("Main.java"));
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(180.0), px(90.0)));
    draw(&mut v);
    for line in 0..20 {
        caret_to(&f, &mut v, line);
        v.simulate_keystrokes("f11");
    }
    let selection = f.view.read_with(&v, |r, _| r.session.state().selection());
    for id in [CommandId::ChooseTheme, CommandId::ShowBookmarks, CommandId::LanguageServer, CommandId::ServerRuntime] {
        f.view.update_in(&mut v, |r, w, cx| r.run(id, w, cx));
        draw(&mut v);
        let (selected, first) = f.view.read_with(&v, |r, _| (r.modal.as_ref().unwrap().at(), r.modal_layout().unwrap().first));
        v.simulate_event(ScrollWheelEvent { position: point(px(60.0), px(40.0)), delta: ScrollDelta::Lines(point(0.0, -1000.0)), ..Default::default() });
        draw(&mut v);
        assert_eq!(f.view.read_with(&v, |r, _| r.modal.as_ref().unwrap().at()), selected, "{id:?} keeps its selected option");
        assert!(f.view.read_with(&v, |r, _| r.modal_layout().unwrap().first > first), "{id:?} scrolls its content");
        assert_eq!(f.view.read_with(&v, |r, _| r.session.state().selection()), selection);
        v.simulate_keystrokes("home");
        draw(&mut v);
        assert!(f.view.read_with(&v, |r, _| {
            let l = r.modal_layout().unwrap();
            l.lines.iter().skip(l.first).take(l.visible).any(|(row, ..)| *row == r.modal.as_ref().unwrap().at())
        }), "keyboard navigation reveals the first option");
        v.simulate_event(ScrollWheelEvent { position: point(px(60.0), px(40.0)), delta: ScrollDelta::Lines(point(0.0, -1000.0)), ..Default::default() });
        v.simulate_keystrokes("escape");
        f.view.update_in(&mut v, |r, w, cx| r.run(id, w, cx));
        draw(&mut v);
        assert!(f.view.read_with(&v, |r, _| {
            let l = r.modal_layout().unwrap();
            l.lines.iter().skip(l.first).take(l.visible).any(|(row, ..)| *row == r.modal.as_ref().unwrap().at())
        }), "reopening {id:?} reveals its selected option");
        v.simulate_keystrokes("escape");
    }
    assert_eq!(std::fs::read_to_string(f.dir.path().join("Main.java")).unwrap(), text);
}

#[gpui::test]
fn recent_files_missing_rows_stay_disabled_and_delete_uses_drawn_revision(cx: &mut TestAppContext) {
    let f = fixture(cx, None);
    let root = project(f.dir.path(), &["old.rs", "current.rs"]);
    for name in ["old.rs", "current.rs"] {
        f.view.update(cx, |r, cx| r.open(root.join(name), cx));
        settle(cx, &f, active_is(name));
    }
    std::fs::remove_file(root.join("old.rs")).unwrap();
    let mut v = visual(cx, &f);
    v.simulate_keystrokes("ctrl-e");
    drop(v);
    settle(cx, &f, |r| r.session.state().navigation().is_some_and(|n| !n.pending));
    let mut v = visual(cx, &f);
    draw(&mut v);
    assert!(f.view.read_with(&v, |r, _| r.session.state().navigation().unwrap().rows[0].disabled.is_some()));
    v.simulate_keystrokes("enter");
    assert!(f.view.read_with(&v, |r, _| r.modal.is_some() && active_is("current.rs")(r)), "a missing row cannot be opened");
    f.view.update(&mut v, |r, _| {
        let (revision, first, visible) = r.navigation_drawn.unwrap();
        r.navigation_drawn = Some((revision.wrapping_sub(1), first, visible));
    });
    v.simulate_keystrokes("delete");
    assert!(f.view.read_with(&v, |r, _| r.session.state().places().recent_files().iter().any(|path| path.ends_with("old.rs"))), "stale displayed rows cannot delete current history");
    draw(&mut v);
    v.simulate_keystrokes("delete");
    assert!(!f.view.read_with(&v, |r, _| r.session.state().places().recent_files().iter().any(|path| path.ends_with("old.rs"))));
    v.simulate_keystrokes("escape");
    assert_eq!(std::fs::read_to_string(root.join("current.rs")).unwrap(), "current.rs");
}

#[gpui::test]
fn status_message_click_opens_full_snapshot_and_keeps_document_input_modal(cx: &mut TestAppContext) {
    let f = fixture(cx, Some(("latin.txt", b"caf\xe9\n")));
    settle(cx, &f, active_is("latin.txt"));
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(900.0), px(400.0)));
    draw(&mut v);
    // The message has its own target, separate from server recovery on the left.
    v.simulate_click(point(px(350.0), px(400.0 - status_h() / 2.0)), Modifiers::default());
    let rows = f.view.read_with(&v, |r, _| r.modal_rows());
    assert!(rows.iter().any(|(s, _)| s.contains("Status details")), "{rows:?}");
    assert!(rows.iter().any(|(s, _)| s.contains("windows-1252")), "{rows:?}");
    let before = f.view.read_with(&v, |r, _| r.session().state().selection());
    v.simulate_keystrokes("ctrl-c ctrl-a ctrl-s down end up");
    assert_eq!(f.view.read_with(&v, |r, _| r.session().state().selection()), before);
    assert!(f.view.read_with(&v, |r, _| r.modal.is_some()));
    assert!(v.update(|_, cx| cx.read_from_clipboard().and_then(|c| c.text())).is_some_and(|s| s.contains("windows-1252")));
    assert_eq!(std::fs::read(f.dir.path().join("latin.txt")).unwrap(), b"caf\xe9\n");
}

#[gpui::test]
fn chrome_segments_fit_and_details_scroll_copy_and_dismiss_in_both_appearances(cx: &mut TestAppContext) {
    let f = fixture(cx, Some(("note.txt", b"unchanged\n")));
    settle(cx, &f, active_is("note.txt"));
    let mut v = visual(cx, &f);
    for theme in ["darcula", "github-light"] {
        f.view.update(&mut v, |r, cx| r.dispatch(Command::Theme(ThemeCommand::Select(ThemeSelection::from_id(theme).unwrap())), cx));
        for (w, h) in [(900.0, 500.0), (480.0, 300.0), (160.0, 120.0), (1.0, 1.0)] {
            v.simulate_resize(size(px(w), px(h)));
            draw(&mut v);
            f.view.read_with(&v, |r, _| {
                let (a, b, g, c) = r.status_segments();
                assert!(a.0.right() <= b.0.left() && b.0.right() <= g.0.left() && g.0.right() <= c.0.left());
                assert!(c.0.right() <= px(w));
                for (bounds, text) in [a, b, g, c] {
                    assert!(lenz_content::wrap::width(&text) as f32 * r.char_w <= f32::from(bounds.size.width) + 0.01);
                }
                assert!(r.nav_buttons().iter().all(|b| b.0 >= 0.0 && b.1 <= w));
                assert!(r.panes.iter().all(|p| p.bar.2 >= toolbar_h()));
            });
            // The shortcut reaches the same frozen snapshot as a message click.
            v.simulate_keystrokes("shift-f2");
            assert!(f.view.read_with(&v, |r, _| matches!(r.modal, Some(Modal::StatusDetails { .. }))));
            // A long Unicode snapshot exercises the adapter's wrapping and independent scroll viewport.
            let text = format!("{}\n{}", "日本語 e\u{301} ".repeat(100), "details\n".repeat(60));
            f.view.update(&mut v, |r, cx| { r.modal = Some(Modal::StatusDetails { kind: Details::Text, title: "Status details", text: text.clone(), logo: None }); cx.notify(); });
            v.simulate_keystrokes("end up");
            draw(&mut v);
            f.view.read_with(&v, |r, _| {
                let l = r.modal_layout().unwrap();
                assert!(l.bounds.left() >= px(0.0) && l.bounds.right() <= px(w));
                assert!(l.bounds.top() >= px(0.0) && l.bounds.bottom() <= px(h));
                assert!(l.first <= l.lines.len().saturating_sub(l.visible));
                let cols = r.columns(r.modal_width() - 24.0);
                assert!(l.lines.iter().all(|(_, line, _)| lenz_content::wrap::width(line) <= cols.max(2)));
            });
            v.simulate_keystrokes("ctrl-c");
            assert_eq!(v.update(|_, cx| cx.read_from_clipboard().and_then(|c| c.text())), Some(text));
            v.simulate_keystrokes("escape");
            assert!(f.view.read_with(&v, |r, _| r.modal.is_none()));
        }
    }
    assert_eq!(std::fs::read(f.dir.path().join("note.txt")).unwrap(), b"unchanged\n");
}

/// File Icons (View menu, remembered): off, names lose their icons and tabs the icon's width; tabs still answer
/// clicks; on again, the width comes back.
#[gpui::test]
fn file_icons_toggle_from_the_menu_and_tabs_keep_their_targets(cx: &mut TestAppContext) {
    let f = fixture(cx, Some(("a.rs", b"fn a() {}\n")));
    let mut v = visual(cx, &f);
    draw(&mut v);
    let width = |v: &mut VisualTestContext| f.view.read_with(v, |r, _| r.tab_hits.first().map(|h| h.1 - h.0).unwrap());
    let with = width(&mut v);
    assert!(f.view.read_with(&v, |r, _| r.session().state().file_icons()), "on by default");
    f.view.update_in(&mut v, |r, w, cx| r.run(CommandId::FileIcons, w, cx));
    draw(&mut v);
    assert!(!f.view.read_with(&v, |r, _| r.session().state().file_icons()));
    assert_eq!(with - width(&mut v), crate::icons::SIZE + 6.0, "the icon and its gap are gone");
    let (hit, y) = f.view.read_with(&v, |r, _| (r.tab_hits[0], toolbar_h() + line_h() / 2.0));
    v.simulate_click(point(px(hit.0 + 10.0), px(y)), Modifiers::default());
    assert_eq!(f.view.read_with(&v, |r, _| r.session().state().active().map(|t| t.id())), Some(hit.2));
    f.view.update_in(&mut v, |r, w, cx| r.run(CommandId::FileIcons, w, cx));
    draw(&mut v);
    assert_eq!(width(&mut v), with);
}

/// About: the shared text and its logo, decoded only when About opens and out of the atlas (and memory) once it
/// closes; reopening keeps one texture; the dialog takes keys and a tiny window still draws.
#[gpui::test]
fn about_shows_the_shared_text_and_releases_its_logo_on_close(cx: &mut TestAppContext) {
    let f = fixture(cx, Some(("a.rs", b"fn a() {}\n")));
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(900.0), px(500.0)));
    draw(&mut v);
    assert_eq!(f.view.read_with(&v, |r, _| r.about_upload.len()), 0, "nothing decoded before About opens");
    let mut weak = None;
    for _ in 0..3 {
        f.view.update_in(&mut v, |r, w, cx| r.run(CommandId::About, w, cx));
        draw(&mut v);
        let (title, text, logo) = f.view.read_with(&v, |r, _| match &r.modal {
            Some(Modal::StatusDetails { title, text, logo, .. }) => (*title, text.clone(), logo.as_ref().map(|l| Arc::downgrade(&l.0))),
            other => panic!("{other:?}"),
        });
        assert_eq!(title, "About Lenz Code");
        assert!(text.contains(&format!("Version: {}", env!("CARGO_PKG_VERSION"))), "{text}");
        assert!(f.view.read_with(&v, |r, _| r.modal_lines().iter().any(|(_, line, _)| line.contains("Licence"))));
        assert_eq!(f.view.read_with(&v, |r, _| r.about_upload.len()), 1, "one texture while open");
        weak = logo;
        v.simulate_keystrokes("x escape");
        draw(&mut v);
        assert!(f.view.read_with(&v, |r, _| r.modal.is_none() && r.about_upload.is_empty()), "out of the atlas after closing");
    }
    assert!(weak.unwrap().upgrade().is_none(), "the pixels are freed too");
    f.view.update_in(&mut v, |r, w, cx| r.run(CommandId::About, w, cx));
    for (w, h) in [(1.0, 1.0), (300.0, 120.0)] {
        v.simulate_resize(size(px(w), px(h)));
        draw(&mut v);
    }
    v.simulate_keystrokes("end up escape");
    assert!(f.view.read_with(&v, |r, _| r.modal.is_none()));
    assert_eq!(std::fs::read(f.dir.path().join("a.rs")).unwrap(), b"fn a() {}\n");
}

#[gpui::test]
fn gui_double_and_triple_click_select_word_and_group(cx: &mut TestAppContext) {
    let src = "fn f() { g(\"hello world\", 1); }\n";
    let f = fixture(cx, Some(("a.rs", src.as_bytes())));
    settle(cx, &f, |v| v.session().state().current_source().is_some());
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(800.0), px(400.0)));
    draw(&mut v);
    let text = f.view.read_with(&v, |r, _| r.text_bounds.unwrap());
    let char_w = f.view.read_with(&v, |r, _| r.char_w);
    let at = point(px(f32::from(text.origin.x) + num_w() + 8.0 + (src.find("world").unwrap() as f32 + 2.5) * char_w), text.origin.y + px(line_h() / 2.0));
    let click = |v: &mut VisualTestContext, n: usize| {
        v.simulate_event(MouseDownEvent { button: MouseButton::Left, position: at, modifiers: Modifiers::default(), click_count: n, first_mouse: false });
        v.simulate_event(gpui::MouseUpEvent { button: MouseButton::Left, position: at, modifiers: Modifiers::default(), click_count: n });
    };
    let sel = |v: &mut VisualTestContext| f.view.read_with(v, |r, _| src[r.session().state().selection().unwrap().range()].to_string());
    click(&mut v, 1);
    assert_eq!(sel(&mut v), "");
    click(&mut v, 2);
    assert_eq!(sel(&mut v), "world");
    click(&mut v, 3);
    settle(cx, &f, |r| r.session().state().selection().is_some_and(|s| s.range().len() > 5));
    assert_eq!(sel(&mut v), "hello world");
    click(&mut v, 4);
    assert_eq!(sel(&mut v), "", "a fourth click starts over");
    assert_eq!(std::fs::read(f.dir.path().join("a.rs")).unwrap(), src.as_bytes(), "the file is unchanged");
}

/// A fixture whose folder is a git repository with `a.txt` (two lines) committed as "First line".
fn blame_fixture(cx: &mut TestAppContext) -> (Fixture, String) {
    use lenz_content::git::testing::{commit_all, git};
    let f = fixture(cx, Some(("a.txt", b"one\ntwo\n")));
    git(f.dir.path(), &["init", "-q", "-b", "main"]);
    std::fs::write(f.dir.path().join(".gitignore"), "*\n!a.txt\n!.gitignore\n").unwrap();
    commit_all(f.dir.path(), "First line");
    let sha = git(f.dir.path(), &["rev-parse", "HEAD"]).trim().to_string();
    settle(cx, &f, |v| v.session().state().current_source().is_some());
    (f, sha)
}

fn annotated(v: &ReaderView) -> bool {
    let s = v.session();
    s.state().active().is_some_and(|t| s.annotation(t.id(), 1).is_some())
}

#[gpui::test]
fn edited_blame_renders_pending_current_lines_undo_and_failure_without_saving(cx: &mut TestAppContext) {
    use lenz_content::git::testing::git;
    let (f, sha) = blame_fixture(cx);
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(900.0), px(400.0)));
    draw(&mut v);
    f.view.update(&mut v, |view, _| view.enable_draft_input_for_test());
    v.simulate_keystrokes("alt-b");
    settle(cx, &f, |view| matches!(view.session().state().active().unwrap().blame_status(), Some(reader_core::blame::Status::Ready)));
    v.simulate_keystrokes("home");
    v.update(|_, cx| cx.write_to_clipboard(ClipboardItem::new_string("new\n".into())));
    v.simulate_keystrokes("ctrl-v home");
    assert_eq!(f.view.read_with(&v, |view, _| view.session().disabled(CommandId::CopyRevision)), Some("Blame pending"));
    draw(&mut v);
    assert!(f.view.read_with(&v, |view, _| view.painted_annotations.iter().any(|label| label == "Blame pending…")));
    settle(cx, &f, |view| matches!(view.session().state().active().unwrap().blame_status(), Some(reader_core::blame::Status::Ready)));
    draw(&mut v);
    f.view.read_with(&v, |view, _| {
        let tab = view.session().state().active().unwrap().id();
        assert_eq!(view.session().annotation(tab, 0).unwrap().label, "Not committed");
        for line in [1, 2] { assert_eq!(view.session().annotation(tab, line).unwrap().commit.sha, sha); }
        assert!(view.painted_annotations.iter().any(|label| label == "Not committed"));
        assert!(view.visible_rows().iter().any(|row| row.text == "new"));
    });
    v.simulate_keystrokes("shift-right shift-right shift-right shift-right");
    assert_eq!(f.view.read_with(&v, |view, _| view.session().state().selection().unwrap().range()), 4..8);
    v.update(|_, cx| cx.write_to_clipboard(ClipboardItem::new_string("λ".into())));
    v.simulate_keystrokes("ctrl-v");
    f.view.read_with(&v, |view, _| { let source = view.session().state().current_source().unwrap(); assert_eq!(source.copy_range(0..source.len_bytes()).unwrap(), "new\nλtwo\n", "selected replacement deletes the old newline"); });
    draw(&mut v);
    assert!(f.view.read_with(&v, |view, _| view.painted_annotations.iter().any(|label| label == "Blame pending…")));
    settle(cx, &f, |view| matches!(view.session().state().active().unwrap().blame_status(), Some(reader_core::blame::Status::Ready)));
    f.view.read_with(&v, |view, _| {
        let tab = view.session().state().active().unwrap().id();
        let source = view.session().state().current_source().unwrap();
        assert_eq!(source.copy_range(0..source.len_bytes()).unwrap(), "new\nλtwo\n");
        assert_eq!(view.session().annotation(tab, 0).unwrap().label, "Not committed");
        assert_eq!(view.session().annotation(tab, 1).unwrap().label, "Not committed");
    });
    v.simulate_keystrokes("ctrl-z");
    settle(cx, &f, |view| matches!(view.session().state().active().unwrap().blame_status(), Some(reader_core::blame::Status::Ready)));
    v.simulate_keystrokes("ctrl-z");
    draw(&mut v);
    assert!(f.view.read_with(&v, |view, _| view.painted_annotations.iter().any(|label| label == "Blame pending…")));
    settle(cx, &f, |view| matches!(view.session().state().active().unwrap().blame_status(), Some(reader_core::blame::Status::Ready)));
    draw(&mut v);
    f.view.read_with(&v, |view, _| {
        let tab = view.session().state().active().unwrap().id();
        assert_eq!(view.session().annotation(tab, 0).unwrap().commit.sha, sha);
        let source = view.session().state().current_source().unwrap();
        assert_eq!(source.copy_range(0..source.len_bytes()).unwrap(), "one\ntwo\n");
    });
    std::fs::write(f.dir.path().join("bad-revisions"), "not-a-revision\n").unwrap();
    git(f.dir.path(), &["config", "blame.ignoreRevsFile", "bad-revisions"]);
    v.simulate_keystrokes("alt-b alt-b");
    settle(cx, &f, |view| matches!(view.session().state().active().unwrap().blame_status(), Some(reader_core::blame::Status::Failed(_))));
    draw(&mut v);
    assert!(f.view.read_with(&v, |view, _| view.painted_annotations.iter().any(|label| label == "Blame failed")));
    assert_eq!(f.view.read_with(&v, |view, _| view.session().disabled(CommandId::CopyRevision)), Some("Blame failed"));
    assert_eq!(std::fs::read_to_string(f.dir.path().join("a.txt")).unwrap(), "one\ntwo\n");
}

#[gpui::test]
fn gui_annotation_column_shifts_text_and_click_opens_the_commit(cx: &mut TestAppContext) {
    let (f, sha) = blame_fixture(cx);
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(900.0), px(400.0)));
    draw(&mut v);
    let before = f.view.read_with(&v, |r, _| r.text_bounds.unwrap().origin.x);
    v.simulate_keystrokes("alt-b");
    settle(cx, &f, annotated);
    draw(&mut v);
    let (g, char_w) = f.view.read_with(&v, |r, _| (r.panes[0], r.char_w));
    assert_eq!(g.ann, reader_core::blame::ANNOTATION_COLS as f32 * char_w + 12.0);
    assert_eq!(g.text.origin.x, before + px(g.ann), "the text moves right of the column");
    // A click on the first line's annotation opens its commit.
    let at = point(g.text.origin.x - px(g.ann / 2.0), g.text.origin.y + px(line_h() * 1.5));
    v.simulate_click(at, Modifiers::default());
    settle(cx, &f, |r| r.session().state().commit_popup().is_some_and(|p| p.details.is_some()));
    draw(&mut v);
    let text = f.view.read_with(&v, |r, _| match &r.modal { Some(Modal::StatusDetails { kind: Details::Commit, text, .. }) => text.clone(), _ => String::new() });
    assert!(text.starts_with(&format!("commit {sha}")), "{text}");
    assert_eq!(f.view.read_with(&v, |r, _| r.session().state().selection().unwrap().range()), 4..4, "the caret is on the clicked line");
    // Copy Revision, then Show Diff (Lenz Diff is not beside the test binary: it says so).
    let button = |v: &mut VisualTestContext, i: usize| f.view.read_with(v, |r, _| r.modal_layout().unwrap().details_buttons(r.char_w, &r.details_labels(Details::Commit))[i].center());
    let copy = button(&mut v, 1);
    v.simulate_click(copy, Modifiers::default());
    assert_eq!(cx.update(|cx| cx.read_from_clipboard().and_then(|c| c.text())), Some(sha.clone()));
    let diff = button(&mut v, 0);
    v.simulate_click(diff, Modifiers::default());
    assert!(f.view.read_with(&v, |r, _| r.modal.is_none() && r.session().state().commit_popup().is_none()), "Show Diff closes the popup");
    let status = f.view.read_with(&v, |r, _| (r.status(), r.launched.clone()));
    assert!(status.0.contains("Show Diff: Lenz Diff is not beside Lenz Code") && status.1.is_empty(), "{status:?}");
    // Escape closes a reopened popup; the file is unchanged.
    v.simulate_click(at, Modifiers::default());
    v.simulate_keystrokes("escape");
    assert!(f.view.read_with(&v, |r, _| r.modal.is_none() && r.session().state().commit_popup().is_none()));
    assert_eq!(std::fs::read(f.dir.path().join("a.txt")).unwrap(), b"one\ntwo\n");
}

#[gpui::test]
fn gui_annotation_column_hides_in_a_narrow_pane(cx: &mut TestAppContext) {
    let (f, _) = blame_fixture(cx);
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(900.0), px(400.0)));
    draw(&mut v);
    v.simulate_keystrokes("alt-b");
    settle(cx, &f, annotated);
    draw(&mut v);
    assert!(f.view.read_with(&v, |r, _| r.panes[0].ann) > 0.0);
    v.simulate_resize(size(px(220.0), px(200.0)));
    draw(&mut v);
    assert_eq!(f.view.read_with(&v, |r, _| r.panes[0].ann), 0.0, "no room: no column");
}

#[gpui::test]
fn gui_right_click_in_the_column_moves_the_caret_and_offers_the_commit(cx: &mut TestAppContext) {
    let (f, _) = blame_fixture(cx);
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(900.0), px(400.0)));
    draw(&mut v);
    v.simulate_keystrokes("alt-b");
    settle(cx, &f, annotated);
    draw(&mut v);
    let g = f.view.read_with(&v, |r, _| r.panes[0]);
    let at = point(g.text.origin.x - px(g.ann / 2.0), g.text.origin.y + px(line_h() * 1.5));
    v.simulate_event(MouseDownEvent { button: MouseButton::Right, position: at, modifiers: Modifiers::default(), click_count: 1, first_mouse: false });
    assert_eq!(f.view.read_with(&v, |r, _| r.session().state().selection().unwrap().range()), 4..4);
    let entries = f.view.read_with(&v, |r, _| match &r.modal { Some(Modal::Context { entries, .. }) => entries.clone(), _ => Vec::new() });
    for id in [CommandId::ShowCommit, CommandId::ShowCommitDiff, CommandId::CopyRevision, CommandId::CloseAnnotations] {
        assert!(entries.contains(&MenuEntry::Command(id)), "{id:?}");
    }
}

#[gpui::test]
fn gui_the_column_beside_a_sticky_line_belongs_to_that_line(cx: &mut TestAppContext) {
    use lenz_content::git::testing::{commit_all, git};
    let body: String = (0..80).map(|i| format!("    let v{i} = {i};\n")).collect();
    let src = format!("fn f() {{\n{body}}}\n");
    let f = fixture(cx, Some(("a.rs", src.as_bytes())));
    git(f.dir.path(), &["init", "-q", "-b", "main"]);
    std::fs::write(f.dir.path().join(".gitignore"), "*\n!a.rs\n!.gitignore\n").unwrap();
    commit_all(f.dir.path(), "First line");
    settle(cx, &f, |v| v.session().state().current_source().is_some_and(|d| d.language().is_some()));
    f.view.update(cx, |v, cx| v.dispatch(Command::SetStickyScroll(true), cx));
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(900.0), px(400.0)));
    draw(&mut v);
    v.simulate_keystrokes("alt-b");
    settle(cx, &f, annotated);
    f.view.update(cx, |v, cx| v.dispatch(Command::Scroll(40), cx));
    settle(cx, &f, |v| v.session().state().active().is_some_and(|t| v.session().state().view(t.id()).is_some_and(|view| view.highlights().is_some())));
    draw(&mut v);
    draw(&mut v);
    let (g, band) = f.view.read_with(&v, |r, _| (r.panes[0], r.sticky_hits.first().map(|h| (h.0, h.2))));
    let (bounds, line) = band.expect("a sticky line over the top row");
    assert_eq!(line, 0, "fn f() is pinned");
    let at = point(g.text.origin.x - px(g.ann / 2.0), bounds.origin.y + px(line_h() / 2.0));
    v.simulate_click(at, Modifiers::default());
    assert_eq!(f.view.read_with(&v, |r, _| r.session().state().selection().unwrap().range()), 0..0, "the sticky line, not the row under it");
}

#[gpui::test]
fn gui_the_hovered_annotation_follows_a_scroll(cx: &mut TestAppContext) {
    use lenz_content::git::testing::{commit_all, git};
    let text: String = (0..40).map(|i| format!("line {i}\n")).collect();
    let f = fixture(cx, Some(("a.txt", text.as_bytes())));
    git(f.dir.path(), &["init", "-q", "-b", "main"]);
    std::fs::write(f.dir.path().join(".gitignore"), "*\n!a.txt\n!.gitignore\n").unwrap();
    commit_all(f.dir.path(), "First line");
    settle(cx, &f, |v| v.session().state().current_source().is_some());
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(900.0), px(400.0)));
    draw(&mut v);
    v.simulate_keystrokes("alt-b");
    settle(cx, &f, annotated);
    draw(&mut v);
    let g = f.view.read_with(&v, |r, _| r.panes[0]);
    v.simulate_mouse_move(point(g.text.origin.x - px(g.ann / 2.0), g.text.origin.y + px(line_h() * 0.5)), None, Modifiers::default());
    assert_eq!(f.view.read_with(&v, |r, _| r.ann_hovered().map(|h| h.2)), Some(0));
    f.view.update(&mut v, |r, cx| r.dispatch(Command::Scroll(5), cx));
    draw(&mut v);
    assert_eq!(f.view.read_with(&v, |r, _| r.ann_hovered().map(|h| h.2)), Some(5), "the line now under the pointer");
}

/// A fixture on a git repository: `main` with `a.txt`, branch `other` changing it, tag `v1`, a second worktree.
fn branches_fixture(cx: &mut TestAppContext) -> (Fixture, tempfile::TempDir) {
    use lenz_content::git::testing::{commit_all, git};
    let f = fixture(cx, Some(("a.txt", b"main\n")));
    let d = f.dir.path();
    git(d, &["init", "-q", "-b", "main"]);
    git(d, &["config", "user.name", "t"]);
    git(d, &["config", "user.email", "t@t"]);
    std::fs::write(d.join(".gitignore"), "*\n!a.txt\n!.gitignore\n").unwrap();
    commit_all(d, "main");
    git(d, &["tag", "v1"]);
    git(d, &["switch", "-q", "-c", "other"]);
    std::fs::write(d.join("a.txt"), "other\n").unwrap();
    commit_all(d, "other");
    git(d, &["switch", "-q", "main"]);
    let wt = tempfile::tempdir().unwrap();
    git(d, &["worktree", "add", "-q", "-b", "side", &wt.path().join("second").to_string_lossy()]);
    f.view.update(cx, |v, cx| v.dispatch(Command::OpenRoot(d.to_path_buf()), cx));
    settle_for(cx, &f, 10, &|v| v.session().branch_label().is_some() && v.session().state().current_source().is_some());
    (f, wt)
}
fn branch_of(v: &ReaderView) -> Option<String> {
    v.session().branch_label()
}
fn type_text(v: &mut VisualTestContext, text: &str) {
    for c in text.chars() {
        v.simulate_keystrokes(&c.to_string());
    }
}

#[gpui::test]
fn gui_the_status_branch_opens_the_popup_and_enter_checks_out(cx: &mut TestAppContext) {
    let (f, _wt) = branches_fixture(cx);
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(900.0), px(400.0)));
    draw(&mut v);
    let (bounds, text) = f.view.read_with(&v, |r, _| r.status_branch());
    assert_eq!(text, "⎇ main");
    v.simulate_click(bounds.center(), Modifiers::default());
    settle_for(cx, &f, 10, &|r| r.session().state().branches().is_some_and(|b| !b.loading));
    draw(&mut v);
    assert!(f.view.read_with(&v, |r, _| matches!(r.modal, Some(Modal::Branches { .. }))));
    let rows = f.view.read_with(&v, |r, _| r.modal_rows().into_iter().map(|r| r.0).collect::<Vec<_>>());
    assert!(rows.iter().any(|r| r.contains("● main")), "{rows:?}");
    type_text(&mut v, "oth");
    let found: Vec<_> = f.view.read_with(&v, |r, _| r.session().state().branch_rows("oth").into_iter().map(|r| (r.section, r.label)).collect());
    assert_eq!(found, [("Recent", "other".to_string()), ("Local", "other".to_string())]);
    v.simulate_keystrokes("enter");
    settle_for(cx, &f, 10, &|r| !r.session().state().checking_out());
    assert_eq!(f.view.read_with(&v, |r, _| branch_of(r)).as_deref(), Some("⎇ other"));
    assert!(f.view.read_with(&v, |r, _| r.modal.is_none()));
    settle_for(cx, &f, 10, &|r| r.session().state().current_source().is_some_and(|d| d.copy_range(0..d.len_bytes()).as_deref() == Some("other\n")));
    // Narrow: no branch cell.
    v.simulate_resize(size(px(400.0), px(300.0)));
    draw(&mut v);
    assert_eq!(f.view.read_with(&v, |r, _| r.status_branch().1), "");
}

#[gpui::test]
fn gui_a_rows_menu_makes_a_branch_from_it_and_the_revision_prompt_checks_out(cx: &mut TestAppContext) {
    let (f, _wt) = branches_fixture(cx);
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(900.0), px(400.0)));
    draw(&mut v);
    v.simulate_keystrokes("ctrl-shift-`");
    settle_for(cx, &f, 10, &|r| r.session().state().branches().is_some_and(|b| !b.loading));
    draw(&mut v);
    type_text(&mut v, "v1");
    v.simulate_keystrokes("right");
    let menu = f.view.read_with(&v, |r, _| r.modal_rows().into_iter().map(|r| r.0).collect::<Vec<_>>());
    assert_eq!(&menu[1..], ["Checkout", "New Branch from Here…", "Compare with Working Tree"]);
    v.simulate_keystrokes("escape");
    assert!(f.view.read_with(&v, |r, _| matches!(&r.modal, Some(Modal::Branches { query, .. }) if query == "v1")), "back to the list");
    v.simulate_keystrokes("right down enter");
    assert!(f.view.read_with(&v, |r, _| matches!(&r.modal, Some(Modal::NewBranch { from: Some(f), .. }) if f == "refs/tags/v1")));
    type_text(&mut v, "other");
    v.simulate_keystrokes("enter");
    let refused = f.view.read_with(&v, |r, _| r.modal_rows().into_iter().map(|r| r.0).collect::<Vec<_>>());
    assert!(refused.iter().any(|r| r == "A branch named \"other\" already exists"), "{refused:?}");
    for _ in 0..5 { v.simulate_keystrokes("backspace"); }
    type_text(&mut v, "from-v1");
    v.simulate_keystrokes("enter");
    settle_for(cx, &f, 10, &|r| !r.session().state().checking_out());
    assert_eq!(f.view.read_with(&v, |r, _| branch_of(r)).as_deref(), Some("⎇ from-v1"));
    // Checkout Tag or Revision…: a bad name is refused in the prompt, a good one checks out detached.
    f.view.update_in(&mut v, |r, w, cx| r.run(CommandId::CheckoutRevision, w, cx));
    // A paste goes in (its first line).
    cx.write_to_clipboard(gpui::ClipboardItem::new_string("pasted\nsecond line".into()));
    v.simulate_keystrokes("ctrl-v");
    assert!(f.view.read_with(&v, |r, _| matches!(&r.modal, Some(Modal::Revision { input, .. }) if input == "pasted")));
    for _ in 0..6 { v.simulate_keystrokes("backspace"); }
    type_text(&mut v, "nope");
    v.simulate_keystrokes("enter");
    let refused = f.view.read_with(&v, |r, _| r.modal_rows().into_iter().map(|r| r.0).collect::<Vec<_>>());
    assert!(refused.iter().any(|r| r == "No commit named \"nope\""), "{refused:?}");
    for _ in 0..4 { v.simulate_keystrokes("backspace"); }
    type_text(&mut v, "v1");
    v.simulate_keystrokes("enter");
    settle_for(cx, &f, 10, &|r| !r.session().state().checking_out());
    let sha = lenz_content::git::testing::git(f.dir.path(), &["rev-parse", "v1"]).trim().to_string();
    assert_eq!(f.view.read_with(&v, |r, _| branch_of(r)), Some(format!("⎇ {}", &sha[..7])));
}

#[gpui::test]
fn gui_checkout_problems_offer_smart_and_a_confirmed_force(cx: &mut TestAppContext) {
    let (f, _wt) = branches_fixture(cx);
    std::fs::write(f.dir.path().join("a.txt"), "mine\n").unwrap();
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(900.0), px(400.0)));
    draw(&mut v);
    f.view.update(&mut v, |r, cx| r.dispatch(Command::Checkout(lenz_content::branches::Target::Branch("other".into())), cx));
    settle_for(cx, &f, 10, &|r| r.session().state().checkout_problem().is_some());
    f.view.update(&mut v, |r, cx| r.dispatch(Command::Notice(String::new()), cx));
    draw(&mut v);
    let dialog = f.view.read_with(&v, |r, _| match &r.modal { Some(Modal::StatusDetails { kind: Details::Checkout, text, .. }) => Some(text.clone()), _ => None });
    assert!(dialog.as_deref().is_some_and(|t| t.contains("a.txt")), "{dialog:?}");
    assert_eq!(f.view.read_with(&v, |r, _| r.details_labels(Details::Checkout)), ["Smart Checkout", "Force Checkout", "Cancel"]);
    // Force: a second step lists what is discarded; Enter there does nothing; f confirms.
    v.simulate_keystrokes("f");
    assert_eq!(f.view.read_with(&v, |r, _| r.details_labels(Details::Checkout)), ["Force Checkout", "Cancel"]);
    v.simulate_keystrokes("enter");
    v.simulate_keystrokes("f");
    assert_eq!(std::fs::read_to_string(f.dir.path().join("a.txt")).unwrap(), "mine\n", "neither Enter nor a second f discards");
    assert_eq!(f.view.read_with(&v, |r, _| r.details_labels(Details::Checkout)), ["Force Checkout", "Cancel"]);
    v.simulate_keystrokes("escape");
    assert!(f.view.read_with(&v, |r, _| r.modal.is_none() && r.session().state().checkout_problem().is_none()));
    // Again, then the Force button and its confirmation button.
    f.view.update(&mut v, |r, cx| r.dispatch(Command::Checkout(lenz_content::branches::Target::Branch("other".into())), cx));
    settle_for(cx, &f, 10, &|r| r.session().state().checkout_problem().is_some());
    f.view.update(&mut v, |r, cx| r.dispatch(Command::Notice(String::new()), cx));
    draw(&mut v);
    let button = |v: &mut VisualTestContext, i: usize| f.view.read_with(v, |r, _| r.modal_layout().unwrap().details_buttons(r.char_w, &r.details_labels(Details::Checkout))[i].center());
    let force = button(&mut v, 1);
    v.simulate_click(force, Modifiers::default());
    draw(&mut v);
    let confirm = button(&mut v, 0);
    v.simulate_click(confirm, Modifiers::default());
    settle_for(cx, &f, 10, &|r| !r.session().state().checking_out());
    assert_eq!(f.view.read_with(&v, |r, _| branch_of(r)).as_deref(), Some("⎇ other"));
    assert_eq!(std::fs::read_to_string(f.dir.path().join("a.txt")).unwrap(), "other\n");
}

#[gpui::test]
fn gui_a_worktree_opens_here_or_in_a_new_window(cx: &mut TestAppContext) {
    let (f, wt) = branches_fixture(cx);
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(900.0), px(400.0)));
    draw(&mut v);
    v.simulate_keystrokes("ctrl-shift-`");
    settle_for(cx, &f, 10, &|r| r.session().state().branches().is_some_and(|b| !b.loading));
    draw(&mut v);
    type_text(&mut v, "second");
    v.simulate_keystrokes("right");
    let menu = f.view.read_with(&v, |r, _| r.modal_rows().into_iter().map(|r| r.0).collect::<Vec<_>>());
    assert_eq!(&menu[1..], ["Open", "Open in New Window", "Compare with Working Tree"]);
    v.simulate_keystrokes("down enter");
    let second = std::fs::canonicalize(wt.path().join("second")).unwrap();
    assert_eq!(f.view.read_with(&v, |r, _| r.opened_windows.iter().map(|p| std::fs::canonicalize(p).unwrap()).collect::<Vec<_>>()), [second]);
    v.simulate_keystrokes("ctrl-shift-`");
    settle_for(cx, &f, 10, &|r| r.session().state().branches().is_some_and(|b| !b.loading));
    type_text(&mut v, "second");
    v.simulate_keystrokes("enter");
    settle_for(cx, &f, 10, &|r| branch_of(r).as_deref() == Some("⎇ side"));
}


/// A Lenz Diff (and its terminal app) beside where the view thinks Lenz Code is.
fn lenz_diff_beside(cx: &mut TestAppContext, f: &Fixture) -> tempfile::TempDir {
    let bin = tempfile::tempdir().unwrap();
    for name in ["lenz-diff", "lenz-diff-tui"] {
        std::fs::write(bin.path().join(name), "").unwrap();
    }
    let exe = bin.path().join("lenz-code");
    f.view.update(cx, |r, _| r.exe = Some(exe));
    bin
}
fn launched_args(v: &mut VisualTestContext, f: &Fixture) -> Vec<Vec<String>> {
    f.view.read_with(v, |r, _| r.launched.iter().map(|l| l.args.clone()).collect())
}

#[gpui::test]
fn gui_the_git_menu_compares_the_working_tree(cx: &mut TestAppContext) {
    let (f, _wt) = branches_fixture(cx);
    let _bin = lenz_diff_beside(cx, &f);
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(900.0), px(400.0)));
    draw(&mut v);
    f.view.update_in(&mut v, |r, w, cx| r.run(CommandId::ShowLocalChanges, w, cx));
    assert_eq!(launched_args(&mut v, &f), [vec!["HEAD".to_string()]]);
    // On main, the base branch: it says so and starts nothing.
    f.view.update_in(&mut v, |r, w, cx| r.run(CommandId::ShowBranchChanges, w, cx));
    assert!(f.view.read_with(&v, |r, _| r.status()).contains("Show Changes Since Branch Start: main is the base branch"));
    assert_eq!(launched_args(&mut v, &f).len(), 1);
    // Compare with Base Branch: main's tip.
    f.view.update_in(&mut v, |r, w, cx| r.run(CommandId::CompareWithBase, w, cx));
    assert_eq!(launched_args(&mut v, &f)[1], ["refs/heads/main"]);
    // Compare with Branch, Tag or Revision…: a pick list of branches and tags, or what is typed.
    f.view.update_in(&mut v, |r, w, cx| r.run(CommandId::CompareWithRevision, w, cx));
    settle_for(cx, &f, 10, &|r| r.session().state().branches().is_some_and(|b| !b.loading));
    draw(&mut v);
    let rows = f.view.read_with(&v, |r, _| r.modal_rows().into_iter().map(|r| r.0).collect::<Vec<_>>());
    assert!(rows[1].starts_with("Compare with Branch, Tag or Revision — type to filter or a revision"), "{rows:?}");
    assert!(rows.iter().any(|r| r.ends_with("other")) && rows.iter().any(|r| r.ends_with("v1")) && !rows.iter().any(|r| r.contains("New Branch")), "{rows:?}");
    // Narrow: the list stays inside the window.
    v.simulate_resize(size(px(260.0), px(200.0)));
    draw(&mut v);
    let bounds = f.view.read_with(&v, |r, _| r.modal_layout().map(|l| l.bounds).unwrap());
    assert!(bounds.right() <= px(260.0), "{bounds:?}");
    v.simulate_resize(size(px(900.0), px(400.0)));
    draw(&mut v);
    // A listed branch: type to find it, then Down to its row.
    type_text(&mut v, "oth");
    v.simulate_keystrokes("down enter");
    assert!(f.view.read_with(&v, |r, _| r.modal.is_none()));
    assert_eq!(launched_args(&mut v, &f)[2], ["refs/heads/other"]);
    // Typed or pasted text: the first row compares with it.
    f.view.update_in(&mut v, |r, w, cx| r.run(CommandId::CompareWithRevision, w, cx));
    settle_for(cx, &f, 10, &|r| r.session().state().branches().is_some_and(|b| !b.loading));
    cx.write_to_clipboard(gpui::ClipboardItem::new_string("HEAD\nsecond line".into()));
    v.simulate_keystrokes("ctrl-v");
    assert!(f.view.read_with(&v, |r, _| matches!(&r.modal, Some(Modal::Branches { query, .. }) if query == "HEAD")));
    v.simulate_keystrokes("enter");
    assert_eq!(launched_args(&mut v, &f)[3], ["HEAD"]);
    assert_eq!(f.view.read_with(&v, |r, _| branch_of(r)).as_deref(), Some("⎇ main"), "nothing checked out");
    assert_eq!(std::fs::read(f.dir.path().join("a.txt")).unwrap(), b"main\n");
}

#[gpui::test]
fn gui_rows_compare_with_the_working_tree_and_since_branching(cx: &mut TestAppContext) {
    let (f, wt) = branches_fixture(cx);
    let _bin = lenz_diff_beside(cx, &f);
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(900.0), px(400.0)));
    draw(&mut v);
    v.simulate_keystrokes("ctrl-shift-`");
    settle_for(cx, &f, 10, &|r| r.session().state().branches().is_some_and(|b| !b.loading));
    type_text(&mut v, "other");
    v.simulate_keystrokes("right");
    let menu = f.view.read_with(&v, |r, _| r.modal_rows().into_iter().map(|r| r.0).collect::<Vec<_>>());
    assert_eq!(&menu[1..], ["Checkout", "New Branch from Here…", "Compare with Working Tree", "Show Changes Since Branching from Here"]);
    v.simulate_keystrokes("down down enter");
    assert!(f.view.read_with(&v, |r, _| r.modal.is_none() && r.session().state().branches().is_none()));
    assert_eq!(launched_args(&mut v, &f), [vec!["refs/heads/other".to_string()]]);
    v.simulate_keystrokes("ctrl-shift-`");
    settle_for(cx, &f, 10, &|r| r.session().state().branches().is_some_and(|b| !b.loading));
    type_text(&mut v, "other");
    v.simulate_keystrokes("right down down down enter");
    let split = lenz_content::git::testing::git(f.dir.path(), &["rev-parse", "main"]).trim().to_string();
    assert_eq!(launched_args(&mut v, &f)[1], [split]);
    // Compare with Worktree…: only the other worktree; Enter compares the folders.
    f.view.update_in(&mut v, |r, w, cx| r.run(CommandId::CompareWithWorktree, w, cx));
    settle_for(cx, &f, 10, &|r| r.session().state().branches().is_some_and(|b| !b.loading));
    draw(&mut v);
    let rows = f.view.read_with(&v, |r, _| r.modal_rows().into_iter().map(|r| r.0).collect::<Vec<_>>());
    assert!(rows[1].starts_with("Compare with Worktree — type to filter; Enter compares"), "{rows:?}");
    assert_eq!(rows.len(), 3, "{rows:?}");
    v.simulate_keystrokes("enter");
    let second = wt.path().join("second").to_string_lossy().into_owned();
    assert_eq!(launched_args(&mut v, &f)[2].last(), Some(&second));
    assert!(f.view.read_with(&v, |r, _| r.modal.is_none()));
    // With the other worktree gone the popup says so, and Escape closes it.
    std::fs::remove_dir_all(wt.path().join("second")).unwrap();
    f.view.update_in(&mut v, |r, w, cx| r.run(CommandId::CompareWithWorktree, w, cx));
    settle_for(cx, &f, 10, &|r| r.session().state().branches().is_some_and(|b| !b.loading));
    v.simulate_resize(size(px(300.0), px(200.0)));
    draw(&mut v);
    let rows = f.view.read_with(&v, |r, _| r.modal_rows().into_iter().map(|r| r.0).collect::<Vec<_>>());
    assert_eq!(rows[1], "Compare with Worktree — No other worktree");
    let bounds = f.view.read_with(&v, |r, _| r.modal_layout().map(|l| l.bounds).unwrap());
    assert!(bounds.right() <= px(300.0), "{bounds:?}");
    v.simulate_keystrokes("enter escape");
    assert!(f.view.read_with(&v, |r, _| r.modal.is_none() && r.session().state().branches().is_none()));
    assert_eq!(launched_args(&mut v, &f).len(), 3);
}

#[gpui::test]
fn gui_ctrl_click_marks_two_entries_and_ctrl_d_compares_them(cx: &mut TestAppContext) {
    let f = fixture(cx, Some(("a.txt", b"one\n")));
    std::fs::write(f.dir.path().join("b.txt"), "two\n").unwrap();
    f.view.update(cx, |v, cx| v.dispatch(Command::OpenRoot(f.dir.path().to_path_buf()), cx));
    settle_for(cx, &f, 10, &|v| v.session().state().tree_rows().any(|r| &*r.path == "b.txt"));
    let _bin = lenz_diff_beside(cx, &f);
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(900.0), px(400.0)));
    draw(&mut v);
    let row_at = |v: &mut VisualTestContext, name: &str| f.view.read_with(v, |r, _| {
        let i = r.session().state().tree_rows().position(|row| &*row.path == name).unwrap() - r.tree_scroll;
        point(px(20.0), px(toolbar_h() + line_h() * (i as f32 + 1.5)))
    });
    let a = row_at(&mut v, "a.txt");
    let b = row_at(&mut v, "b.txt");
    // Nothing picked yet: the first row is drawn highlighted, and a Ctrl+click marks it too.
    let first = f.view.read_with(&v, |r, _| r.session().state().tree_rows().next().unwrap().path.clone());
    v.simulate_click(b, Modifiers { control: true, ..Default::default() });
    assert_eq!(f.view.read_with(&v, |r, _| r.session().state().tree_marks().to_vec()), [first, "b.txt".into()]);
    v.simulate_keystrokes("escape");
    v.simulate_click(a, Modifiers::default());
    v.simulate_click(b, Modifiers { control: true, ..Default::default() });
    draw(&mut v);
    assert_eq!(f.view.read_with(&v, |r, _| r.session().state().tree_marks().to_vec()), ["a.txt".into(), "b.txt".into()] as [std::sync::Arc<str>; 2]);
    assert!(f.view.read_with(&v, |r, _| r.tree_lines(usize::MAX).iter().filter(|l| l.marked).count()) == 2, "both drawn marked");
    v.simulate_keystrokes("ctrl-d");
    let args = f.view.read_with(&v, |r, _| r.launched.last().map(|l| l.args.clone())).unwrap();
    assert_eq!(args, ["--no-index".to_string(), f.dir.path().join("a.txt").to_string_lossy().into_owned(), f.dir.path().join("b.txt").to_string_lossy().into_owned()]);
    // Escape in the explorer, or a plain click, forgets the marks.
    v.simulate_keystrokes("escape");
    assert!(f.view.read_with(&v, |r, _| r.session().state().tree_marks().is_empty()));
    v.simulate_click(b, Modifiers { control: true, ..Default::default() });
    v.simulate_click(a, Modifiers::default());
    assert!(f.view.read_with(&v, |r, _| r.session().state().tree_marks().is_empty()));
    // Space marks the highlighted entry, as in the terminal.
    v.simulate_click(a, Modifiers::default());
    v.simulate_keystrokes("space down space");
    assert_eq!(f.view.read_with(&v, |r, _| r.session().state().tree_marks().len()), 2);
    v.simulate_keystrokes("escape");
    // Ctrl+D with nothing marked starts nothing (its menu entry says why).
    v.simulate_keystrokes("ctrl-d");
    assert_eq!(f.view.read_with(&v, |r, _| r.launched.len()), 1);
    assert_eq!(f.view.read_with(&v, |r, _| r.session().disabled(CommandId::CompareSelected)), Some("Mark two files or folders in the explorer (Ctrl+click, or Space)"));
}


fn ime_handler(f: &Fixture, v: &mut VisualTestContext) -> gpui::PlatformInputHandler {
    let view = f.view.clone(); let bounds = f.view.read_with(v, |view, _| view.text_bounds.unwrap());
    v.update(|window, cx| gpui::PlatformInputHandler::new(window.to_async(cx), Box::new(gpui::ElementInputHandler::new(bounds, view))))
}

#[gpui::test]
fn native_composition_projects_unicode_without_mutating_the_buffer_then_unmark_accepts_once(cx: &mut TestAppContext) {
    for wrapped in [false, true] {
    let original = format!("{}😀suffix\r\nlast\r\n", "a".repeat(12_000));
    let f = fixture(cx, Some(("composition.txt", original.as_bytes())));
    settle(cx, &f, |r| r.session.state().current_source().is_some());
    let mut v = visual(cx, &f); draw(&mut v);
    f.view.update(&mut v, |view, _| view.enable_draft_input_for_test());
    if wrapped { v.simulate_keystrokes("alt-z"); }
    v.simulate_keystrokes("ctrl-g 1 : 1 2 0 0 1 enter ctrl-\\ shift-right"); draw(&mut v);
    let (tab, base, selected) = f.view.read_with(&v, |r, _| (r.session.state().active().unwrap().id(), r.session.state().current_source().unwrap().clone(), r.session.state().selection()));
    let mut input = ime_handler(&f, &mut v);
    let conversions = reader_core::text::TextSnapshot::conversion_calls_on_thread_for_test();
    input.replace_and_mark_text_in_range(None, "界😀\nβ", Some(0..1));
    assert_eq!(input.marked_text_range(), Some(12_000..12_005));
    assert_eq!(input.selected_text_range(false).unwrap().range, 12_000..12_001);
    f.view.read_with(&v, |r, _| {
        assert!(Arc::ptr_eq(r.session.state().current_source().unwrap(), &base));
        assert_eq!(r.session.state().selection(), selected);
        assert!(!r.session.state().view(tab).unwrap().can_undo());
        assert!(Arc::ptr_eq(r.session.state().presentation_view(r.session.state().shown(0).unwrap().id()).unwrap().snapshot(), &base));
    });
    let mut adjusted = None;
    let length = input.text_length_utf16().unwrap();
    let around = input.text_for_range(0..length, &mut adjusted).unwrap();
    assert!(around.len() <= 4096); assert!(around.contains("界😀\nβ")); assert!(adjusted.unwrap().start > 9_000);
    let mut unchanged = None;
    assert!(input.text_for_range(12_002..12_003, &mut unchanged).is_none(), "a surrogate interior must be rejected before clipping");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let caret = loop {
        cx.run_until_parked(); f.view.update(&mut v, |r, cx| r.poll(cx)); draw(&mut v);
        if let Some(caret) = input.bounds_for_range(12_000..12_000) { break caret; }
        assert!(std::time::Instant::now() < deadline, "candidate geometry must use the projected row, wrapped={wrapped}: {:?}", f.view.read_with(&v, |r, _| {
            let state=r.session.state();let tab=state.active().unwrap();let shown=state.presentation_view(tab.id()).unwrap();
            (shown.tab().top_line(),shown.tab().top_row(),shown.tab().column(),state.text_size(tab.pane()),r.session.native_reveal(tab.pane()).map(|goal|(goal.source,goal.context)),r.visible_rows().iter().map(|row|(row.y,row.text.find('界'))).collect::<Vec<_>>())
        }));
        std::thread::yield_now();
    };
    assert_eq!(input.character_index_for_point(caret.origin), Some(12_000));
    assert_eq!(reader_core::text::TextSnapshot::conversion_calls_on_thread_for_test(), conversions);
    input.unmark_text(); input.unmark_text();
    let expected = format!("{}界😀\nβsuffix\r\nlast\r\n", "a".repeat(12_000));
    f.view.read_with(&v, |r, _| {
        let source = r.session.state().current_source().unwrap(); assert_eq!(source.copy_range(0..source.len_bytes()).unwrap(), expected);
        assert!(r.session.state().view(tab).unwrap().can_undo());
    });
    v.simulate_keystrokes("ctrl-1 ctrl-z");
    f.view.read_with(&v, |r, _| { let source = r.session.state().current_source().unwrap(); assert_eq!(source.copy_range(0..source.len_bytes()).unwrap(), original); });
    assert_eq!(std::fs::read(f.dir.path().join("composition.txt")).unwrap(), original.as_bytes());
    }
}

#[gpui::test]
fn native_composition_partial_replacement_cancel_and_painted_commits_use_one_coordinate_frame(cx: &mut TestAppContext) {
    let original = "a😀suffix\r\n";
    let f = fixture(cx, Some(("partial.txt", original.as_bytes()))); settle(cx, &f, |r| r.session.state().current_source().is_some());
    let mut v = visual(cx, &f); draw(&mut v); f.view.update(&mut v, |r, _| r.enable_draft_input_for_test());
    v.simulate_keystrokes("right shift-right"); draw(&mut v);
    let base = f.view.read_with(&v, |r, _| r.session.state().current_source().unwrap().clone());
    let mut input = ime_handler(&f, &mut v);
    input.replace_and_mark_text_in_range(None, "界😀tail", None);
    let relative = 1..3;
    #[cfg(target_os = "macos")] let replacement = relative;
    #[cfg(not(target_os = "macos"))] let replacement = relative.start + 1..relative.end + 1;
    input.replace_and_mark_text_in_range(Some(replacement), "β", Some(1..1));
    assert_eq!(f.view.read_with(&v, |r, _| r.marked_text_for_test().unwrap().1), "界βtail");
    assert_eq!(input.selected_text_range(false).unwrap().range, 3..3);
    // Invalid selected surrogate endpoints and unsupported replacement expansion are atomic refusals.
    input.replace_and_mark_text_in_range(None, "😀", Some(1..1));
    input.replace_and_mark_text_in_range(Some(100..101), "bad", None);
    assert_eq!(f.view.read_with(&v, |r, _| r.marked_text_for_test().unwrap().1), "界βtail");
    // Native XKB emits this non-character event after setMarkedText; it must preserve preedit.
    let mut dead = gpui::Keystroke::parse("a").unwrap(); dead.key = "dead_acute".into(); dead.key_char = None;
    v.simulate_event(gpui::KeyDownEvent { keystroke: dead, is_held: false, prefer_character_input: false });
    assert!(input.marked_text_range().is_some());
    v.simulate_keystrokes("escape");
    assert!(input.marked_text_range().is_none()); assert!(f.view.read_with(&v, |r, _| r.marked_text_for_test().is_none()));
    input.replace_text_in_range(None, "late",); // an old terminal callback cannot become ordinary typing
    f.view.read_with(&v, |r, _| { assert!(Arc::ptr_eq(r.session.state().current_source().unwrap(), &base)); assert!(!r.session.state().view(r.session.state().active().unwrap().id()).unwrap().can_undo()); });
    // A fresh real key establishes a new input boundary. After paint, use the platform fallback,
    // proving registration rather than only constructing the callback wrapper above.
    draw(&mut v); v.simulate_input("é");
    f.view.read_with(&v, |r, _| { let source = r.session.state().current_source().unwrap(); assert_eq!(source.copy_range(0..source.len_bytes()).unwrap(), "aésuffix\r\n"); });
    v.simulate_keystrokes("ctrl-z");
    f.view.read_with(&v, |r, _| { let source = r.session.state().current_source().unwrap(); assert_eq!(source.copy_range(0..source.len_bytes()).unwrap(), original); });
    assert_eq!(std::fs::read(f.dir.path().join("partial.txt")).unwrap(), original.as_bytes());
}

#[gpui::test]
fn native_composition_rejects_delayed_commit_after_source_selection_permission_modal_or_resize_changes(cx: &mut TestAppContext) {
    for change in [0, 1, 2, 3, 4] {
        let original = "a😀suffix\r\n"; let f = fixture(cx, Some(("stale-ime.txt", original.as_bytes())));
        settle(cx, &f, |r| r.session.state().current_source().is_some()); let mut v = visual(cx, &f); draw(&mut v);
        f.view.update(&mut v, |r, _| r.enable_draft_input_for_test()); v.simulate_keystrokes("right shift-right"); draw(&mut v);
        let tab = f.view.read_with(&v, |r, _| r.session.state().active().unwrap().id()); let mut input = ime_handler(&f, &mut v);
        input.replace_and_mark_text_in_range(None, "pending", None);
        match change {
            0 => f.view.update(&mut v, |r, cx| {
                let expected = r.session.state().current_source().unwrap().key();
                r.dispatch(Command::Edit { tab, expected, edits: vec![reader_core::text::TextEdit { range: 0..0, text: "changed ".into() }], kind: reader_core::text::EditKind::Paste }, cx);
                r.dispatch(Command::Undo { tab }, cx);
            }),
            1 => f.view.update(&mut v, |r, cx| r.dispatch(Command::Select(Selection { anchor: 0, head: 0 }), cx)),
            2 => f.view.update(&mut v, |r, cx| r.dispatch(Command::SetReadOnly(true), cx)),
            3 => { v.simulate_keystrokes("f10"); },
            4 => { v.simulate_resize(size(px(500.0), px(200.0))); draw(&mut v); },
            _ => unreachable!(),
        }
        let before = f.view.read_with(&v, |r, _| (r.session.state().current_source().unwrap().clone(), r.session.state().selection()));
        input.replace_text_in_range(None, "obsolete"); input.unmark_text();
        f.view.read_with(&v, |r, _| {
            assert!(Arc::ptr_eq(r.session.state().current_source().unwrap(), &before.0), "stale composition case {change}");
            assert_eq!(r.session.state().selection(), before.1); assert!(r.marked_text_for_test().is_none());
        });
        assert_eq!(std::fs::read(f.dir.path().join("stale-ime.txt")).unwrap(), original.as_bytes());
    }
}

#[gpui::test]
fn native_composition_explicit_partial_commit_and_linux_cancel_preserve_the_pending_splice(cx: &mut TestAppContext) {
    for cancel in [false, true] {
        let original = "a😀suffix\r\n"; let f = fixture(cx, Some(("terminal-ime.txt", original.as_bytes())));
        settle(cx, &f, |r| r.session.state().current_source().is_some()); let mut v = visual(cx, &f); draw(&mut v);
        f.view.update(&mut v, |r, _| r.enable_draft_input_for_test()); v.simulate_keystrokes("right shift-right"); draw(&mut v);
        let base = f.view.read_with(&v, |r, _| r.session.state().current_source().unwrap().clone()); let mut input = ime_handler(&f, &mut v);
        input.replace_and_mark_text_in_range(None, "界😀tail", None);
        if cancel {
            #[cfg(not(target_os = "macos"))] { let range = input.marked_text_range().unwrap(); input.replace_text_in_range(Some(range), ""); }
            #[cfg(target_os = "macos")] v.simulate_keystrokes("escape");
            input.unmark_text(); input.replace_text_in_range(None, "late");
            f.view.read_with(&v, |r, _| { assert!(Arc::ptr_eq(r.session.state().current_source().unwrap(), &base)); assert!(!r.session.state().view(r.session.state().active().unwrap().id()).unwrap().can_undo()); });
        } else {
            #[cfg(not(target_os = "macos"))] let range = 2..4;
            #[cfg(target_os = "macos")] let range = 1..3;
            input.replace_text_in_range(Some(range), "終😀"); input.unmark_text();
            #[cfg(not(target_os = "macos"))] let expected = "a界終😀tailsuffix\r\n";
            #[cfg(target_os = "macos")] let expected = "a終😀suffix\r\n";
            f.view.read_with(&v, |r, _| { let source = r.session.state().current_source().unwrap(); assert_eq!(source.copy_range(0..source.len_bytes()).unwrap(), expected); });
            v.simulate_keystrokes("ctrl-z");
            f.view.read_with(&v, |r, _| { let source = r.session.state().current_source().unwrap(); assert_eq!(source.copy_range(0..source.len_bytes()).unwrap(), original); });
        }
        assert_eq!(std::fs::read(f.dir.path().join("terminal-ime.txt")).unwrap(), original.as_bytes());
    }
}

#[gpui::test]
fn native_composition_surrounding_text_rejects_surrogates_before_clipping_a_distant_query(cx: &mut TestAppContext) {
    let original = "😀".to_owned() + &"a".repeat(20_000);
    let f = fixture(cx, Some(("surrounding.txt", original.as_bytes()))); settle(cx, &f, |r| r.session.state().current_source().is_some());
    let mut v = visual(cx, &f); draw(&mut v); f.view.update(&mut v, |r, _| r.enable_draft_input_for_test());
    v.simulate_keystrokes("ctrl-g 1 : 2 0 0 0 2 enter"); draw(&mut v);
    let mut input = ime_handler(&f, &mut v); let length = input.text_length_utf16().unwrap();
    let conversions = reader_core::text::TextSnapshot::conversion_calls_on_thread_for_test();
    let mut actual = None; let text = input.text_for_range(0..length, &mut actual).unwrap();
    assert!(text.len() <= 4096); assert!(actual.unwrap().start > 16_000);
    let mut rejected = None; assert!(input.text_for_range(1..length, &mut rejected).is_none()); assert!(rejected.is_none());
    input.replace_and_mark_text_in_range(Some(1..2), "invalid", None);
    assert!(input.marked_text_range().is_none());
    assert_eq!(reader_core::text::TextSnapshot::conversion_calls_on_thread_for_test(), conversions);
    f.view.read_with(&v, |r, _| { let source = r.session.state().current_source().unwrap(); assert_eq!(source.copy_range(0..source.len_bytes()).unwrap(), original); });
    assert_eq!(std::fs::read(f.dir.path().join("surrounding.txt")).unwrap(), original.as_bytes());
}
