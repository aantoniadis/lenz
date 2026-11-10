use super::*;

#[gpui::test]
fn gui_native_red_close_dismisses_a_picker_and_preserves_the_dirty_decision(cx: &mut TestAppContext) {
    let f = fixture_setup(cx, Some(("keep.txt", b"saved\n")), &[], true);
    settle(cx, &f, active_is("keep.txt"));
    f.view.update(cx, |view, _| view.enable_draft_input_for_test());
    let mut v = visual(cx, &f); draw(&mut v);
    v.write_to_clipboard(ClipboardItem::new_string("draft λ".into()));
    v.simulate_keystrokes("ctrl-a ctrl-v");
    choose_menu_command(&mut v, &f, CommandId::NewScratch);
    assert!(f.view.read_with(&v, |view, _| matches!(view.modal, Some(Modal::NewScratch { .. }))));
    assert!(!f.view.update(&mut v, |view, cx| view.window_should_close(cx)));
    cx.run_until_parked();
    let ticket = f.view.read_with(&v, |view, _| {
        assert!(matches!(view.modal, Some(Modal::StatusDetails { kind: Details::Close, .. })));
        view.session.close_prompt().unwrap().ticket
    });
    // Another native close preserves this exact decision instead of cancelling/replacing it.
    assert!(!f.view.update(&mut v, |view, cx| view.window_should_close(cx)));
    cx.run_until_parked();
    assert_eq!(f.view.read_with(&v, |view, _| view.session.close_prompt().unwrap().ticket), ticket);
    assert_eq!(std::fs::read(f.dir.path().join("keep.txt")).unwrap(), b"saved\n");
    v.simulate_keystrokes("escape"); cx.run_until_parked();
    assert!(f.view.read_with(&v, |view, _| view.modal.is_none() && !view.session.closed()));
    assert_eq!(f.view.read_with(&v, |view, _| view.session.state().current_source().unwrap().copy_range(0..8)), Some("draft λ".into()));
}

#[gpui::test]
fn gui_native_quit_action_and_command_q_use_all_window_consent(cx: &mut TestAppContext) {
    let a = fixture_setup(cx, Some(("clean.txt", b"clean\n")), &[], true);
    let b = fixture_setup(cx, Some(("dirty.txt", b"saved\n")), &[], true);
    settle(cx, &a, active_is("clean.txt")); settle(cx, &b, active_is("dirty.txt"));
    b.view.update(cx, |view, _| view.enable_draft_input_for_test());
    let mut av = visual(cx, &a); let mut bv = visual(cx, &b); draw(&mut av); draw(&mut bv);
    bv.write_to_clipboard(ClipboardItem::new_string("draft".into())); bv.simulate_keystrokes("ctrl-a ctrl-v");
    cx.update(close::install_native_commands);
    choose_menu_command(&mut av, &a, CommandId::NewScratch);
    av.update(|window, cx| window.dispatch_action(Box::new(close::Quit), cx));
    cx.run_until_parked();
    assert_eq!(cx.update(|cx| cx.windows().len()), 2);
    assert!(a.view.read_with(&av, |view, _| view.modal.is_none()));
    assert!(b.view.read_with(&bv, |view, _| view.session.close_prompt().is_some()));
    bv.simulate_keystrokes("escape"); cx.run_until_parked();
    assert_eq!(cx.update(|cx| cx.windows().len()), 2);
    assert_eq!(std::fs::read(b.dir.path().join("dirty.txt")).unwrap(), b"saved\n");
    // Exercise the raw platform-key route too, while the picker owns a native text field.
    a.view.update(&mut av, |view, _| view.mac_keys = true);
    choose_menu_command(&mut av, &a, CommandId::NewScratch);
    av.simulate_keystrokes("cmd-q"); cx.run_until_parked();
    assert!(b.view.read_with(&bv, |view, _| view.session.close_prompt().is_some()));
    bv.simulate_keystrokes("d"); cx.run_until_parked();
    assert!(cx.update(|cx| cx.windows().is_empty()));
    assert_eq!(std::fs::read(a.dir.path().join("clean.txt")).unwrap(), b"clean\n");
    assert_eq!(std::fs::read(b.dir.path().join("dirty.txt")).unwrap(), b"saved\n");
}

#[gpui::test]
#[cfg(target_os = "linux")]
fn gui_native_close_shows_an_idle_terminal_question_without_waiting_for_a_tick(cx: &mut TestAppContext) {
    let f = fixture_setup(cx, Some(("keep.txt", b"keep\n")), &[], true);
    settle(cx, &f, active_is("keep.txt"));
    let mut v = visual(cx, &f); draw(&mut v);
    v.simulate_keystrokes("ctrl-alt-t"); drop(v);
    settle(cx, &f, |view| view.session.state().terminal().active().is_some_and(|tab| tab.running() && tab.frame().is_some()));
    let mut v = visual(cx, &f); draw(&mut v);
    assert!(!f.view.update(&mut v, |view, cx| view.window_should_close(cx)));
    cx.run_until_parked();
    // Do not poll terminal/timer completions: preparation itself must present this question.
    assert!(f.view.read_with(&v, |view, _| view.session.state().terminal().prompt().is_some()
        && matches!(view.modal, Some(Modal::StatusDetails { kind: Details::Terminal { .. }, .. }))));
    v.simulate_keystrokes("escape"); cx.run_until_parked();
    assert!(f.view.read_with(&v, |view, _| !view.session.closed() && view.session.state().terminal().active().unwrap().running()));
    assert!(!f.view.update(&mut v, |view, cx| view.window_should_close(cx)));
    cx.run_until_parked();
    assert!(f.view.read_with(&v, |view, _| matches!(view.modal, Some(Modal::StatusDetails { kind: Details::Terminal { .. }, .. }))));
    v.simulate_keystrokes("enter"); drop(v);
    settle(cx, &f, |view| view.session.closed());
    cx.run_until_parked();
    assert!(cx.update(|cx| cx.windows().is_empty()));
    assert_eq!(std::fs::read(f.dir.path().join("keep.txt")).unwrap(), b"keep\n");
}
