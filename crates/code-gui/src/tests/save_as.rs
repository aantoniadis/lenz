use super::*;

#[gpui::test]
fn gui_scratch_files_filter_edit_clear_and_open_the_filtered_path(cx: &mut TestAppContext) {
    let f = fixture(cx, Some(("keep.txt", b"keep\n")));
    settle(cx, &f, active_is("keep.txt"));
    f.view.update(cx, |r, _| r.enable_draft_input_for_test());
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(900.0), px(500.0)));
    draw(&mut v);
    choose_menu_command(&mut v, &f, CommandId::NewScratch);
    type_text(&mut v, "ruby");
    v.simulate_keystrokes("enter");
    settle(cx, &f, |r| r.session().state().active().is_some_and(|t| t.source().extension().is_some_and(|e| e == "rb")));
    let ruby = f.view.read_with(&v, |r, _| r.session().state().active().unwrap().source().to_owned());
    choose_menu_command(&mut v, &f, CommandId::NewScratch);
    type_text(&mut v, "json");
    v.simulate_keystrokes("enter");
    settle(cx, &f, |r| r.session().state().active().is_some_and(|t| t.source().extension().is_some_and(|e| e == "json")));
    let json = f.view.read_with(&v, |r, _| r.session().state().active().unwrap().source().to_owned());
    // Opening a scratch file must return from a previously focused results pane to the editor.
    f.view.update(&mut v, |r, _| r.results_focus = true);
    choose_menu_command(&mut v, &f, CommandId::ScratchFiles);
    type_text(&mut v, "no-such-scratch");
    assert!(f.view.read_with(&v, |r, _| r.modal_rows().iter().any(|row| row.0 == "No matching scratch files")));
    v.simulate_keystrokes("enter");
    assert!(f.view.read_with(&v, |r, _| matches!(&r.modal, Some(Modal::ScratchFiles { query, .. }) if query == "no-such-scratch")));
    assert_eq!(f.view.read_with(&v, |r, _| r.session().state().active().unwrap().source().to_owned()), json);
    v.simulate_keystrokes("ctrl-a backspace");
    let rows = f.view.read_with(&v, |r, _| r.modal_rows().into_iter().skip(FINDER_HEADER_ROWS).map(|row| row.0).collect::<Vec<_>>());
    assert!(rows.contains(&ruby.file_name().unwrap().to_string_lossy().into_owned()) && rows.contains(&json.file_name().unwrap().to_string_lossy().into_owned()));
    type_text(&mut v, ".RB");
    assert_eq!(f.view.read_with(&v, |r, _| r.modal_rows().len()), FINDER_HEADER_ROWS + 1);
    v.simulate_resize(size(px(220.0), px(180.0)));
    draw(&mut v);
    assert!(f.view.read_with(&v, |r, _| r.modal_layout().is_some_and(|layout| layout.visible > 0 && layout.query.as_deref() == Some(".RB"))));
    v.simulate_keystrokes("enter");
    settle(cx, &f, |r| r.session().state().active().is_some_and(|t| t.source() == ruby)
        && r.session().state().current_source().is_some());
    assert!(f.view.read_with(&v, |r, _| r.modal.is_none() && !r.results_focus && !r.tree_focus));
    v.simulate_keystrokes("x");
    assert_eq!(f.view.read_with(&v, |r, _| r.session().state().current_source().unwrap().copy_range(0..1)), Some("x".into()));
    assert_eq!(std::fs::read(&ruby).unwrap(), b"");
    assert_eq!(std::fs::read(&json).unwrap(), b"");
    assert_eq!(std::fs::read(f.dir.path().join("keep.txt")).unwrap(), b"keep\n");
}

fn text(v: &VisualTestContext, f: &Fixture) -> String {
    f.view.read_with(v, |view, _| { let s = view.session().state().current_source().unwrap(); s.copy_range(0..s.len_bytes()).unwrap() })
}

#[gpui::test]
fn gui_save_as_keeps_exact_path_whitespace_and_first_pasted_line(cx: &mut TestAppContext) {
    let f = fixture(cx, None);
    let sibling = f.dir.path().join("result");
    std::fs::write(&sibling, b"keep sibling").unwrap();
    let mut v = visual(cx, &f);
    draw(&mut v);
    f.view.update(&mut v, |r, _| r.enable_draft_input_for_test());
    v.simulate_keystrokes("ctrl-n");
    v.update(|_, cx| cx.write_to_clipboard(ClipboardItem::new_string("draft contents".into())));
    v.simulate_keystrokes("ctrl-v");
    for name in ["result ", " result", " "] {
        let target = f.dir.path().join(name);
        v.simulate_keystrokes("ctrl-shift-s ctrl-u enter");
        assert!(f.view.read_with(&v, |r, _| matches!(&r.modal, Some(Modal::SaveAs { error: Some(_), .. }))));
        let pasted = format!("{}\nignored", target.display());
        v.update(|_, cx| cx.write_to_clipboard(ClipboardItem::new_string(pasted)));
        v.simulate_keystrokes("ctrl-v");
        assert!(f.view.read_with(&v, |r, _| matches!(&r.modal, Some(Modal::SaveAs { input, .. }) if input == &target.display().to_string())));
        v.simulate_keystrokes("enter");
        assert!(f.view.read_with(&v, |r, _| r.session().save_as_prompt().is_none()), "the trimmed sibling must not request replacement");
        settle(cx, &f, |r| r.session().state().active().is_some_and(|t| t.source() == target));
        assert_eq!(std::fs::read(&target).unwrap(), b"draft contents");
        assert_eq!(std::fs::read(&sibling).unwrap(), b"keep sibling");
    }
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
    // The kinds, pinned ones first; typing filters, Enter creates the next numbered scratch file of the kind.
    let rows = f.view.read_with(&v, |r, _| r.modal_rows().into_iter().map(|r| r.0).collect::<Vec<_>>());
    assert!(rows[1].contains("New Scratch File"), "{rows:?}");
    assert_eq!(&rows[2..6], ["Plain text  .txt", "JSON  .json", "Ruby  .rb", "PostgreSQL  .sql"]);
    type_text(&mut v, "rub");
    assert_eq!(f.view.read_with(&v, |r, _| r.modal_rows().get(2).map(|r| r.0.clone())).as_deref(), Some("Ruby  .rb"));
    v.simulate_keystrokes("enter");
    settle(cx, &f, active_is("scratch_1.rb"));
    assert_eq!(f.view.read_with(&v, |r, _| r.session().state().current_source().and_then(|d| d.language())), Some(lenz_content::highlight::Language::Ruby));
    let scratch = f.view.read_with(&v, |r, _| r.session().state().active().unwrap().source().to_owned());
    assert!(scratch.exists());
    choose_menu_command(&mut v, &f, CommandId::Close);
    choose_menu_command(&mut v, &f, CommandId::ScratchFiles);
    assert!(f.view.read_with(&v, |r, _| r.modal_rows().iter().any(|r| r.0 == "scratch_1.rb")));
    v.simulate_keystrokes("enter");
    settle(cx, &f, active_is("scratch_1.rb"));
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

    // The kind list owns its keys, at any size; a filter with no kind creates nothing, and Escape cancels.
    choose_menu_command(&mut v, &f, CommandId::NewScratch);
    type_text(&mut v, "zzz"); v.simulate_keystrokes("enter");
    assert!(f.view.read_with(&v, |r, _| matches!(&r.modal, Some(Modal::NewScratch { query, .. }) if query == "zzz")));
    for (w, h) in [(1.0, 1.0), (220.0, 100.0), (800.0, 400.0)] { v.simulate_resize(size(px(w), px(h))); draw(&mut v); v.simulate_keystrokes("pagedown up ctrl-z ctrl-s"); }
    assert_eq!(text(&v, &f), "");
    v.simulate_keystrokes("escape");
    assert!(f.view.read_with(&v, |r, _| r.modal.is_none()));
    assert!(f.view.read_with(&v, |r, _| r.session().scratch_files().unwrap().is_empty()), "nothing was created");
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
