use super::*;

fn menu(v: &mut VisualTestContext, f: &Fixture, command: CommandId) {
    v.simulate_keystrokes("f10");
    let row = f.view.read_with(v, |view, _| view.modal_rows().iter().position(|r| r.0 == command.label()).unwrap());
    for _ in 0..row { v.simulate_keystrokes("down"); }
    v.simulate_keystrokes("enter");
}

/// A saved file with the draft `draft λ\r\n`, then the file changed on disk and the conflict observed.
fn conflict(cx: &mut TestAppContext, name: &str) -> (Fixture, PathBuf, VisualTestContext) {
    let f = fixture(cx, Some((name, b"saved\r\n")));
    let path = f.dir.path().join(name);
    settle(cx, &f, |view| view.session.state().current_source().is_some());
    let mut v = visual(cx, &f);
    f.view.update(&mut v, |view, _| view.enable_draft_input_for_test());
    v.write_to_clipboard(ClipboardItem::new_string("draft λ\r\n".into()));
    v.simulate_keystrokes("ctrl-a ctrl-v");
    draw(&mut v);
    std::fs::write(&path, b"external replacement\r\n").unwrap();
    f.view.update(&mut v, |view, cx| view.dispatch(Command::Recheck, cx));
    settle(cx, &f, |view| view.session.state().active().unwrap().stale_on_disk());
    (f, path, v)
}

fn open_dialog(cx: &mut TestAppContext, f: &Fixture, v: &mut VisualTestContext) {
    menu(v, f, CommandId::ResolveExternalChange);
    settle(cx, f, |view| view.session.reconcile_prompt().is_some_and(|prompt| prompt.disk.is_some()));
    draw(v);
}

fn dialog_kind(f: &Fixture, v: &VisualTestContext) -> Option<Details> {
    f.view.read_with(v, |view, _| match &view.modal { Some(Modal::StatusDetails { kind, .. }) => Some(*kind), _ => None })
}

fn draft_text(f: &Fixture, v: &VisualTestContext) -> String {
    f.view.read_with(v, |view, _| view.session.state().current_source().unwrap().copy_range(0..view.session.state().current_source().unwrap().len_bytes()).unwrap())
}

fn dirty(f: &Fixture, v: &VisualTestContext) -> bool {
    f.view.read_with(v, |view, _| view.session.state().view(view.session.state().active().unwrap().id()).unwrap().dirty())
}

#[gpui::test]
fn the_conflict_dialog_uses_real_keys_and_keeps_the_underlying_draft_safe(cx: &mut TestAppContext) {
    let (f, path, mut v) = conflict(cx, "reconcile.txt");
    open_dialog(cx, &f, &mut v);
    assert_eq!(dialog_kind(&f, &v), Some(Details::Reconcile));
    for theme in ["darcula", "github-light"] {
        f.view.update(&mut v, |r, cx| r.dispatch(Command::Theme(ThemeCommand::Select(ThemeSelection::from_id(theme).unwrap())), cx));
        v.simulate_resize(size(px(360.0), px(240.0))); draw(&mut v);
        f.view.read_with(&v, |r, _| {
            let l = r.modal_layout().unwrap();
            assert!(l.visible > 0, "consequences and disabled reasons stay scrollable");
            let buttons = l.details_buttons(r.char_w, &r.details_labels(Details::Reconcile));
            assert_eq!(buttons.len(), 4);
            let bottom = l.bounds.top() + px(l.body_top + l.visible as f32 * line_h());
            assert!(buttons.iter().all(|b| b.top() >= bottom && b.size.height > px(0.0)));
        });
    }
    v.simulate_resize(size(px(900.0), px(500.0))); draw(&mut v);

    let labels = f.view.read_with(&v, |view, _| match &view.modal { Some(Modal::StatusDetails { kind, .. }) => view.details_labels(*kind), _ => panic!("expected the external-change dialog") });
    assert_eq!(labels, ["Compare Disk, Draft and Baseline…", "Take Disk Version", "Save Draft over Disk", "Keep Draft"]);
    let text = f.view.read_with(&v, |view, _| view.session.reconcile_prompt().unwrap().text());
    for word in ["Disk:", "Current draft:", "Confirmed baseline"] { assert!(text.contains(word), "{text}"); }
    // Modal precedence: nothing typed or saved reaches the draft, and the file is not written.
    v.write_to_clipboard(ClipboardItem::new_string("must not paste".into()));
    v.simulate_keystrokes("ctrl-v ctrl-s ctrl-z x");
    for (w, h) in [(180., 90.), (1., 1.), (800., 500.)] {
        v.simulate_resize(size(px(w), px(h))); draw(&mut v);
        v.simulate_keystrokes("end up");
    }
    assert_eq!(draft_text(&f, &v), "draft λ\r\n");
    assert_eq!(std::fs::read(&path).unwrap(), b"external replacement\r\n");
    assert_eq!(dialog_kind(&f, &v), Some(Details::Reconcile), "the dialog survives resizing and stray keys");

    // Compare opens the three sides; the dialog returns when the comparison closes.
    v.simulate_keystrokes("c");
    draw(&mut v);
    settle(cx, &f, |view| view.session.state().saved_comparison().is_some());
    assert!(f.view.read_with(&v, |view, _| view.modal.is_none()));
    let sides = f.view.read_with(&v, |view, _| {
        let comparison = view.session.state().saved_comparison().unwrap();
        assert_eq!(comparison.kind(), code_core::session::ComparisonKind::Conflict);
        let title = |id| view.session.state().view(id).unwrap().tab().title();
        (title(comparison.saved_tab()), title(comparison.current_tab()), title(comparison.context_tab().unwrap()))
    });
    assert_eq!(sides, ("Disk (read-only)".to_string(), "Current draft".to_string(), "Confirmed baseline (read-only)".to_string()));
    assert_eq!(std::fs::read(&path).unwrap(), b"external replacement\r\n");
    menu(&mut v, &f, CommandId::CloseSavedComparison);
    draw(&mut v);
    assert!(f.view.read_with(&v, |view, _| view.session.state().saved_comparison().is_none()));
    assert_eq!(dialog_kind(&f, &v), Some(Details::Reconcile), "the undecided dialog returns");

    // S saves the draft over the version the dialog showed, after one more check.
    v.simulate_keystrokes("s");
    settle(cx, &f, |view| !view.session.state().view(view.session.state().active().unwrap().id()).unwrap().dirty());
    assert_eq!(std::fs::read(&path).unwrap(), "draft λ\r\n".as_bytes());
    assert!(!dirty(&f, &v) && f.view.read_with(&v, |view, _| view.session.state().view(view.session.state().active().unwrap().id()).unwrap().can_undo()));
    v.simulate_keystrokes("ctrl-z");
    assert!(dirty(&f, &v));
    assert_eq!(std::fs::read(&path).unwrap(), "draft λ\r\n".as_bytes(), "undo changes the draft, not the file");
    assert_eq!(f.view.read_with(&v, |view, _| view.session.disabled(CommandId::ResolveExternalChange)), Some(code_core::reconcile::NO_CONFLICT));
}

#[gpui::test]
fn taking_disk_and_keeping_the_draft_are_separate_keyboard_decisions(cx: &mut TestAppContext) {
    let (f, path, mut v) = conflict(cx, "take.txt");
    open_dialog(cx, &f, &mut v);
    v.simulate_keystrokes("escape");
    assert!(f.view.read_with(&v, |view, _| view.session.reconcile_prompt().is_none() && view.modal.is_none()));
    assert_eq!(draft_text(&f, &v), "draft λ\r\n");
    assert!(dirty(&f, &v), "keeping the draft does not make it clean");
    assert_eq!(std::fs::read(&path).unwrap(), b"external replacement\r\n");
    // Compared with its last saved version, the draft still shows the baseline it was edited from.
    menu(&mut v, &f, CommandId::CompareSaved);
    draw(&mut v);
    settle(cx, &f, |view| view.session.state().saved_comparison().is_some());
    let baseline = f.view.read_with(&v, |view, _| {
        let comparison = view.session.state().saved_comparison().unwrap();
        let text = view.session.state().view(comparison.saved_tab()).unwrap().snapshot();
        text.copy_range(0..text.len_bytes()).unwrap()
    });
    assert_eq!(baseline, "saved\r\n", "the new disk version never becomes the baseline silently");
    menu(&mut v, &f, CommandId::CloseSavedComparison);

    open_dialog(cx, &f, &mut v);
    v.simulate_keystrokes("d");
    settle(cx, &f, |view| view.session.state().current_source().unwrap().copy_range(0..view.session.state().current_source().unwrap().len_bytes()).unwrap() == "external replacement\r\n");
    assert_eq!(draft_text(&f, &v), "external replacement\r\n");
    assert!(!dirty(&f, &v));
    assert_eq!(std::fs::read(&path).unwrap(), b"external replacement\r\n", "taking disk never writes");
    v.simulate_keystrokes("ctrl-z");
    assert_eq!(draft_text(&f, &v), "draft λ\r\n");
    assert!(dirty(&f, &v));
}
