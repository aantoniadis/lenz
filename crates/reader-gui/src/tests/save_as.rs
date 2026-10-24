use super::*;

fn text(v: &VisualTestContext, f: &Fixture) -> String {
    f.view.read_with(v, |view, _| { let s = view.session().state().current_source().unwrap(); s.copy_range(0..s.len_bytes()).unwrap() })
}

#[gpui::test]
fn new_file_and_save_as_use_real_keys_dialogs_and_the_core_replace_decision(cx: &mut TestAppContext) {
    let f = fixture(cx, None);
    let existing = f.dir.path().join("existing.txt");
    std::fs::write(&existing, b"keep\n").unwrap();
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(800.0), px(400.0)));
    draw(&mut v);
    f.view.update(&mut v, |view, _| view.enable_draft_input_for_test());
    v.simulate_keystrokes("ctrl-n");
    let tab = f.view.read_with(&v, |view, _| view.session().state().active().unwrap().id());
    assert!(f.view.read_with(&v, |view, _| view.session().state().active().unwrap().untitled()));
    v.update(|_, cx| cx.write_to_clipboard(ClipboardItem::new_string("draft λ\n".into())));
    v.simulate_keystrokes("ctrl-v");
    draw(&mut v);
    assert_eq!(text(&v, &f), "draft λ\n");
    assert!(f.view.read_with(&v, |view, _| view.tab_hits.iter().any(|h| h.2 == tab)));
    assert!(f.view.read_with(&v, |view, _| view.status().starts_with("Untitled-1")), "{}", f.view.read_with(&v, |view, _| view.status()));
    // Plain Save of an untitled document asks where; the platform dialog is cancelled without writing.
    v.simulate_keystrokes("ctrl-s");
    assert!(cx.did_prompt_for_new_path());
    cx.simulate_new_path_selection(|dir| { assert!(dir.is_absolute()); None });
    cx.run_until_parked();
    assert!(f.view.read_with(&v, |view, _| view.session().state().active().unwrap().untitled()));
    // An existing target opens the core Replace dialog; Escape keeps both files and the draft.
    v.simulate_keystrokes("ctrl-shift-s");
    let chosen = existing.clone();
    cx.simulate_new_path_selection(move |_| Some(chosen));
    cx.run_until_parked();
    assert!(f.view.read_with(&v, |view, _| matches!(view.modal, Some(Modal::StatusDetails { kind: Details::Replace, .. }))));
    assert_eq!(f.view.read_with(&v, |view, _| view.details_labels(Details::Replace)), ["Replace", "Cancel"]);
    for (w, h) in [(1.0, 1.0), (220.0, 100.0), (800.0, 400.0)] { v.simulate_resize(size(px(w), px(h))); draw(&mut v); }
    v.simulate_keystrokes("ctrl-v");
    assert_eq!(text(&v, &f), "draft λ\n", "the dialog owns input");
    v.simulate_keystrokes("escape");
    assert!(f.view.read_with(&v, |view, _| view.modal.is_none() && view.session().save_as_prompt().is_none()));
    assert_eq!(std::fs::read(&existing).unwrap(), b"keep\n");
    // Replace through the dialog's key moves the document to the confirmed file.
    v.simulate_keystrokes("ctrl-shift-s");
    let chosen = existing.clone();
    cx.simulate_new_path_selection(move |_| Some(chosen));
    cx.run_until_parked();
    v.simulate_keystrokes("r");
    settle(cx, &f, |view| !view.session().state().tab_dirty(tab));
    draw(&mut v);
    assert_eq!(std::fs::read(&existing).unwrap(), "draft λ\n".as_bytes());
    assert!(f.view.read_with(&v, |view, _| { let t = view.session().state().active().unwrap(); !t.untitled() && t.source() == existing && t.title() == "existing.txt" }));
    // A new path from the menu command writes a new file and keeps history.
    choose_menu_command(&mut v, &f, CommandId::SaveAs);
    let fresh = f.dir.path().join("fresh.txt");
    let (chosen, start) = (fresh.clone(), f.dir.path().to_owned());
    cx.simulate_new_path_selection(move |dir| { assert_eq!(dir, start); Some(chosen) });
    settle(cx, &f, |view| view.session().state().active().unwrap().source() == fresh);
    assert_eq!(std::fs::read(&fresh).unwrap(), "draft λ\n".as_bytes());
    assert_eq!(std::fs::read(&existing).unwrap(), "draft λ\n".as_bytes());
    v.simulate_keystrokes("ctrl-z");
    assert_eq!(text(&v, &f), "");
    choose_menu_command(&mut v, &f, CommandId::NewDocument);
    assert_eq!(f.view.read_with(&v, |view, _| view.session().state().active().unwrap().title()), "Untitled-2");
}
