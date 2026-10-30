use super::*;

fn text(v: &VisualTestContext, f: &Fixture) -> String {
    f.view.read_with(v, |view, _| { let s = view.session().state().current_source().unwrap(); s.copy_range(0..s.len_bytes()).unwrap() })
}

#[gpui::test]
fn gui_new_file_and_save_as_use_real_keys_dialogs_and_the_core_replace_decision(cx: &mut TestAppContext) {
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
    // Plain Save asks in the same window; Escape leaves the untitled draft untouched.
    v.simulate_keystrokes("ctrl-s");
    assert!(!cx.did_prompt_for_new_path());
    assert!(f.view.read_with(&v, |r, _| r.modal_rows().iter().any(|r| r.0.contains("Save As"))));
    v.simulate_keystrokes("escape");
    assert!(f.view.read_with(&v, |view, _| view.session().state().active().unwrap().untitled()));
    // An existing target opens the core Replace dialog; Escape keeps both files and the draft.
    v.simulate_keystrokes("ctrl-shift-s");
    v.simulate_keystrokes("ctrl-a");
    type_text(&mut v, &existing.display().to_string());
    v.simulate_keystrokes("enter");
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
    v.simulate_keystrokes("ctrl-a");
    type_text(&mut v, &existing.display().to_string());
    v.simulate_keystrokes("enter");
    v.simulate_keystrokes("r");
    settle(cx, &f, |view| !view.session().state().tab_dirty(tab));
    draw(&mut v);
    assert_eq!(std::fs::read(&existing).unwrap(), "draft λ\n".as_bytes());
    assert!(f.view.read_with(&v, |view, _| { let t = view.session().state().active().unwrap(); !t.untitled() && t.source() == existing && t.title() == "existing.txt" }));
    // A new path from the menu command writes a new file and keeps history.
    choose_menu_command(&mut v, &f, CommandId::SaveAs);
    let fresh = f.dir.path().join("fresh.txt");
    v.simulate_keystrokes("ctrl-a");
    type_text(&mut v, &fresh.display().to_string());
    v.simulate_keystrokes("enter");
    settle(cx, &f, |view| view.session().state().active().unwrap().source() == fresh);
    assert_eq!(std::fs::read(&fresh).unwrap(), "draft λ\n".as_bytes());
    assert_eq!(std::fs::read(&existing).unwrap(), "draft λ\n".as_bytes());
    v.simulate_keystrokes("ctrl-z");
    assert_eq!(text(&v, &f), "");
    choose_menu_command(&mut v, &f, CommandId::NewDocument);
    assert_eq!(f.view.read_with(&v, |view, _| view.session().state().active().unwrap().title()), "Untitled-2");
}

#[gpui::test]
fn gui_scratch_and_bulk_close_use_menus_and_tab_right_clicks(cx: &mut TestAppContext) {
    let f = fixture(cx, None);
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(900.0), px(500.0)));
    draw(&mut v);
    choose_menu_command(&mut v, &f, CommandId::ScratchFiles);
    assert!(f.view.read_with(&v, |r, _| r.modal_rows().iter().any(|r| r.0.contains("No scratch files"))));
    for (w, h) in [(1.0, 1.0), (220.0, 100.0), (900.0, 500.0)] { v.simulate_resize(size(px(w), px(h))); draw(&mut v); v.simulate_keystrokes("end up"); }
    v.simulate_keystrokes("escape");
    choose_menu_command(&mut v, &f, CommandId::NewScratch);
    assert!(!cx.did_prompt_for_new_path());
    assert!(f.view.read_with(&v, |r, _| r.modal_rows().iter().any(|r| r.0.contains("New scratch file"))));
    v.simulate_keystrokes("ctrl-a");
    type_text(&mut v, "personal.rs");
    v.simulate_keystrokes("enter");
    settle(cx, &f, active_is("personal.rs"));
    let scratch = f.view.read_with(&v, |r, _| r.session().state().active().unwrap().source().to_owned());
    assert!(scratch.exists());
    choose_menu_command(&mut v, &f, CommandId::Close);
    choose_menu_command(&mut v, &f, CommandId::ScratchFiles);
    assert!(f.view.read_with(&v, |r, _| r.modal_rows().iter().any(|r| r.0 == "personal.rs")));
    v.simulate_keystrokes("enter");
    settle(cx, &f, active_is("personal.rs"));
    v.simulate_keystrokes("ctrl-n");
    draw(&mut v);
    let p = f.view.read_with(&v, |r, _| { let (x0, x1, _) = r.tab_hits[0]; point(px((x0 + x1) / 2.0), px(r.panes[0].bar.2 + line_h() / 2.0)) });
    v.simulate_event(MouseDownEvent { button: MouseButton::Right, position: p, modifiers: Modifiers::default(), click_count: 1, first_mouse: false });
    assert_eq!(f.view.read_with(&v, |r, _| r.session().state().active().unwrap().source().to_owned()), scratch);
    assert!(f.view.read_with(&v, |r, _| r.modal_rows().iter().any(|r| r.0.contains("Close Other Tabs"))));
    v.simulate_keystrokes("down down enter");
    assert_eq!(f.view.read_with(&v, |r, _| r.session().state().tabs().len()), 1);
    assert_eq!(f.view.read_with(&v, |r, _| r.session().state().active().unwrap().source().to_owned()), scratch);
}

#[gpui::test]
fn gui_new_file_key_names_a_file_in_the_highlighted_project_folder(cx: &mut TestAppContext) {
    let f = fixture(cx, None);
    let root = f.dir.path().join("project");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/keep.txt"), b"keep\n").unwrap();
    f.view.update(cx, |v, cx| v.open_root(root.clone(), cx));
    settle(cx, &f, scanned);
    f.view.update(cx, |v, _| { let at = v.session().state().tree_rows().position(|r| &*r.path == "src").unwrap(); v.tree_move(at); });
    let mut v = visual(cx, &f);
    draw(&mut v);
    v.simulate_keystrokes("ctrl-n");
    assert!(!cx.did_prompt_for_new_path());
    assert!(f.view.read_with(&v, |r, _| matches!(&r.modal, Some(Modal::Create { kind: code_core::create::Kind::File, dir, .. }) if dir == "src")));
    type_text(&mut v, "named.rs");
    for (w, h) in [(1.0, 1.0), (220.0, 100.0), (800.0, 400.0)] { v.simulate_resize(size(px(w), px(h))); draw(&mut v); v.simulate_keystrokes("end up"); }
    assert!(!root.join("src/named.rs").exists());
    v.simulate_keystrokes("enter");
    settle(cx, &f, |r| r.session().state().active().is_some_and(|t| t.source() == root.join("src/named.rs")));
    assert_eq!(std::fs::read(root.join("src/named.rs")).unwrap(), b"");
    v.simulate_keystrokes("ctrl-n");
    type_text(&mut v, "cancel.txt");
    v.simulate_keystrokes("escape");
    assert!(!root.join("src/cancel.txt").exists());
}

#[gpui::test]
fn gui_file_prompts_capture_input_and_save_the_originating_tab(cx: &mut TestAppContext) {
    let f = fixture(cx, None);
    let target = f.dir.path().join("origin.rs");
    std::fs::write(&target, b"keep\n").unwrap();
    let mut v = visual(cx, &f);
    f.view.update(&mut v, |r, _| r.enable_draft_input_for_test());
    v.simulate_keystrokes("ctrl-n");
    v.update(|_, cx| cx.write_to_clipboard(ClipboardItem::new_string("origin draft".into())));
    v.simulate_keystrokes("ctrl-v");
    let origin = f.view.read_with(&v, |r, _| r.session().state().active().unwrap().id());
    v.simulate_keystrokes("ctrl-s ctrl-a");
    type_text(&mut v, &target.display().to_string());
    for (w, h) in [(1.0, 1.0), (220.0, 100.0), (800.0, 400.0)] {
        v.simulate_resize(size(px(w), px(h))); draw(&mut v); v.simulate_keystrokes("end up ctrl-z ctrl-s");
    }
    assert_eq!(text(&v, &f), "origin draft", "the path prompt consumes document actions");
    assert_eq!(std::fs::read(&target).unwrap(), b"keep\n");
    f.view.update(&mut v, |r, cx| r.dispatch(Command::NewDocument, cx));
    let other = f.view.read_with(&v, |r, _| r.session().state().active().unwrap().id());
    assert_ne!(origin, other);
    v.simulate_keystrokes("enter");
    assert!(f.view.read_with(&v, |r, _| matches!(r.modal, Some(Modal::StatusDetails { kind: Details::Replace, .. }))));
    v.simulate_keystrokes("r");
    settle(cx, &f, |r| r.session().state().tabs().iter().any(|t| t.id() == origin && t.source() == target));
    assert_eq!(std::fs::read(&target).unwrap(), b"origin draft");
    assert!(f.view.read_with(&v, |r, _| r.session().state().active().unwrap().id() == other && r.session().state().active().unwrap().untitled()));
    assert_eq!(text(&v, &f), "");

    choose_menu_command(&mut v, &f, CommandId::NewScratch);
    v.simulate_keystrokes("ctrl-a"); type_text(&mut v, "../refused.rs"); v.simulate_keystrokes("enter");
    assert!(f.view.read_with(&v, |r, _| matches!(&r.modal, Some(Modal::NewScratch { error: Some(e), .. }) if e.contains("path separator"))));
    for (w, h) in [(1.0, 1.0), (220.0, 100.0), (800.0, 400.0)] { v.simulate_resize(size(px(w), px(h))); draw(&mut v); v.simulate_keystrokes("end up ctrl-z ctrl-s"); }
    assert_eq!(text(&v, &f), "");
    v.simulate_keystrokes("ctrl-a"); type_text(&mut v, "cancel.rs"); v.simulate_keystrokes("escape");
    assert!(!f.view.read_with(&v, |r, _| r.session().scratch_directory().join("cancel.rs").exists()));
    assert!(!cx.did_prompt_for_new_path());
}

fn save_as_blocks_popup_pointer_input(cx: &mut TestAppContext, quick_fix: bool) {
    for redraw in [false, true] {
        let original = "fn main() {\n    hel;\n    let unused_x = bad_name;\n}\nfn hello() {}\nfn helper() {}\n";
        let f = fixture_with(cx, None, &fake_lsp());
        let root = fake_package(cx, &f, &[("src/main.rs", original)], "src/main.rs");
        let mut v = visual(cx, &f);
        v.simulate_resize(size(px(800.0), px(500.0)));
        f.view.update(&mut v, |r, cx| {
            r.enable_draft_input_for_test();
            let at = if quick_fix { original.find("bad_name").unwrap() } else { original.find("hel;").unwrap() + 3 };
            r.dispatch(Command::Select(Selection { anchor: at, head: at }), cx);
        });
        if quick_fix {
            settle(cx, &f, |r| r.session().state().problem_at_caret().is_some());
            v.simulate_keystrokes("alt-enter");
            settle(cx, &f, |r| r.session().state().code_actions().is_some());
        } else {
            v.simulate_keystrokes("ctrl-space");
            settle(cx, &f, |r| r.session().state().completion().is_some());
        }
        draw(&mut v);
        let bounds = f.view.read_with(&v, |r, _| if quick_fix { r.code_actions_bounds() } else { r.completion_bounds() }).expect("the underlying popup was drawn");
        let at = bounds.origin + point(px(8.0), px(4.0));
        let snapshot = |v: &VisualTestContext| f.view.read_with(v, |r, _| {
            let s = r.session().state(); let tab = s.active().unwrap(); let source = s.current_source().unwrap();
            (tab.id(), source.copy_range(0..source.len_bytes()).unwrap(), s.selection(), s.tab_dirty(tab.id()))
        });
        let before = snapshot(&v);
        v.simulate_keystrokes("ctrl-shift-s");
        assert!(f.view.read_with(&v, |r, _| matches!(r.modal, Some(Modal::SaveAs { .. }))));
        if redraw { draw(&mut v); }
        v.simulate_event(MouseDownEvent { button: MouseButton::Left, position: at, modifiers: Modifiers::default(), click_count: 1, first_mouse: false });
        assert_eq!(snapshot(&v), before, "Save As must receive the click before the old popup; quick_fix={quick_fix}, redraw={redraw}");
        assert_eq!(std::fs::read(root.join("src/main.rs")).unwrap(), original.as_bytes());
        if f.view.read_with(&v, |r, _| r.modal.is_some()) { v.simulate_keystrokes("escape"); }
        // A fresh Save As render hides the old popup, including in a narrow layout.
        v.simulate_keystrokes("ctrl-shift-s");
        for (w, h) in [(220.0, 100.0), (800.0, 500.0)] {
            v.simulate_resize(size(px(w), px(h))); draw(&mut v);
            assert!(f.view.read_with(&v, |r, _| r.completion_bounds().is_none() && r.code_actions_bounds().is_none()));
        }
        v.simulate_keystrokes("escape");
        assert_eq!(snapshot(&v), before);
        assert_eq!(std::fs::read(root.join("src/main.rs")).unwrap(), original.as_bytes());
    }
}

#[gpui::test]
fn gui_save_as_blocks_an_existing_completion_popup_before_and_after_render(cx: &mut TestAppContext) {
    save_as_blocks_popup_pointer_input(cx, false);
}

#[gpui::test]
fn gui_save_as_blocks_an_existing_quick_fix_popup_before_and_after_render(cx: &mut TestAppContext) {
    save_as_blocks_popup_pointer_input(cx, true);
}
