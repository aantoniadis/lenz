use super::*;

#[gpui::test]
fn gui_terminal_rename_menu_edits_only_the_captured_tab(cx: &mut TestAppContext) {
    let f = fixture(cx, Some(("keep.txt", b"keep\n")));
    settle(cx, &f, active_is("keep.txt"));
    let mut v = visual(cx, &f); draw(&mut v);
    v.simulate_keystrokes("ctrl-alt-t ctrl-alt-n");
    let open_rename = |v: &mut VisualTestContext| f.view.update(v, |r, cx| {
        let group = CommandGroup::Terminal;
        let at = r.session.menu(Some(group)).iter().position(|row| row.target == MenuTarget::Run(CommandId::RenameTerminal)).unwrap();
        r.modal = Some(Modal::CommandGroup { group, at }); cx.notify();
    });
    open_rename(&mut v); v.simulate_keystrokes("enter");
    assert!(f.view.read_with(&v, |r,_| matches!(&r.modal, Some(Modal::RenameTerminal { input, selected: true, .. }) if input == "Terminal 2")));
    v.simulate_keystrokes("backspace enter");
    assert!(f.view.read_with(&v, |r,_| matches!(&r.modal, Some(Modal::RenameTerminal { error: Some(_), .. }))));
    f.view.update(&mut v, |r,cx| {
        let first = r.session.state().terminal().tabs()[0].id();
        r.dispatch(Command::Terminal(code_core::terminal::Command::Activate(first)),cx);
    });
    v.simulate_keystrokes("c o d e x enter"); draw(&mut v);
    assert!(f.view.read_with(&v, |r,_| r.modal.is_none() && r.session.state().terminal().tabs()[1].label() == "codex" && r.session.state().terminal().active().unwrap().label() == "Terminal 1"));
    f.view.update(&mut v, |r,cx| {
        let second = r.session.state().terminal().tabs()[1].id();
        r.dispatch(Command::Terminal(code_core::terminal::Command::Activate(second)),cx);
    });
    open_rename(&mut v); v.simulate_keystrokes("enter");
    for (w,h) in [(900.0,500.0),(180.0,140.0),(1.0,1.0),(900.0,500.0)] {
        v.simulate_resize(size(px(w),px(h))); draw(&mut v);
        assert!(f.view.read_with(&v, |r,_| r.form_layout().is_some()));
    }
    v.simulate_keystrokes("x escape");
    assert!(f.view.read_with(&v, |r,_| r.session.state().terminal().active().unwrap().label() == "codex"));
    f.view.update(&mut v, |r,cx| {
        let id = r.session.state().terminal().active().unwrap().id();
        r.dispatch(Command::Terminal(code_core::terminal::Command::Rename { id, label: "界界".into() }),cx);
        let layout = r.terminal_layout().unwrap();
        let (bounds,_,label,_) = layout.tabs.iter().find(|(_,tab,_,_)| *tab == id).unwrap();
        assert!(f32::from(bounds.size.width) >= (lenz_content::wrap::width(label) + 2) as f32 * r.char_w, "wide names fit in their tab without clipping");
    });
    assert_eq!(f.view.read_with(&v, |r,_| source_text(r)), "keep\n");
    assert_eq!(std::fs::read(f.dir.path().join("keep.txt")).unwrap(), b"keep\n");
}

fn source_text(r: &ReaderView) -> String {
    let s = r.session.state().current_source().unwrap();
    s.copy_range(0..s.len_bytes()).unwrap()
}

#[gpui::test]
fn gui_terminal_keys_native_input_and_modals_preserve_source(cx: &mut TestAppContext) {
    let f = fixture(cx, Some(("keep.txt", b"keep\n")));
    settle(cx, &f, active_is("keep.txt"));
    f.view.update(cx, |r, _| r.enable_draft_input_for_test());
    let mut v = visual(cx, &f); draw(&mut v);
    v.simulate_keystrokes("home"); v.simulate_input("draft");
    assert_eq!(f.view.read_with(&v, |r,_| source_text(r)), "draftkeep\n");
    v.simulate_keystrokes("ctrl-alt-t"); draw(&mut v);
    assert!(f.view.read_with(&v, |r, _| r.session.state().terminal().focused()));
    for _ in 0..2 {v.simulate_modifiers_change(Modifiers {shift:true,..Default::default()});v.simulate_modifiers_change(Modifiers::default());}
    assert!(f.view.read_with(&v,|r,_| r.modal.is_none()),"terminal Shift input does not open source navigation");
    v.simulate_keystrokes("ctrl-s ctrl-q ctrl-z ctrl-c ctrl-d tab escape");
    let mut input = ime_handler(&f, &mut v);
    input.replace_and_mark_text_in_range(None, "界", None);
    input.replace_text_in_range(None, "Écho");
    assert_eq!(f.view.read_with(&v, |r, _| source_text(r)), "draftkeep\n");
    v.simulate_keystrokes("f10");
    assert!(f.view.read_with(&v, |r, _| r.modal.is_some()));
    input.replace_text_in_range(None, "must stay modal");
    v.simulate_keystrokes("ctrl-s ctrl-z escape");
    assert_eq!(f.view.read_with(&v, |r, _| source_text(r)), "draftkeep\n");
    assert_eq!(std::fs::read(f.dir.path().join("keep.txt")).unwrap(), b"keep\n");
    v.simulate_keystrokes("ctrl-alt-0");
    assert!(f.view.read_with(&v, |r, _| !r.session.state().terminal().focused()));
}

#[gpui::test]
fn gui_terminal_multiple_tabs_themes_tiny_layout_and_scoped_drops(cx: &mut TestAppContext) {
    let f = fixture(cx, Some(("a ' quoted.txt", b"unchanged\n")));
    settle(cx, &f, active_is("a ' quoted.txt"));
    let mut v = visual(cx, &f); draw(&mut v);
    v.simulate_keystrokes("ctrl-alt-t ctrl-alt-n"); draw(&mut v);
    assert_eq!(f.view.read_with(&v, |r, _| r.session.state().terminal().tabs().len()), 2);
    let second = f.view.read_with(&v, |r, _| r.session.state().terminal().active().unwrap().id());
    v.simulate_keystrokes("ctrl-alt-pageup");
    assert_ne!(f.view.read_with(&v, |r, _| r.session.state().terminal().active().unwrap().id()), second);
    v.simulate_keystrokes("ctrl-alt-pagedown");
    assert_eq!(f.view.read_with(&v, |r, _| r.session.state().terminal().active().unwrap().id()), second);
    for theme in ["darcula", "github-light"] {
        f.view.update(&mut v, |r, cx| r.dispatch(Command::Theme(ThemeCommand::Select(ThemeSelection::from_id(theme).unwrap())), cx));
        for (w,h) in [(900.0,500.0),(180.0,140.0),(1.0,1.0),(900.0,500.0)] {
            v.simulate_resize(size(px(w),px(h))); draw(&mut v);
            f.view.update(&mut v, |r, _| {
                if let Some(l) = r.terminal_layout() {
                    assert!(l.bounds.top() >= px(0.0) && l.bounds.bottom() <= px(h));
                    assert!(l.grid.size.width >= px(0.0) && l.grid.size.height >= px(0.0));
                    assert!(r.body_bottom() <= f32::from(l.bounds.top()));
                }
            });
        }
    }
    let p = f.dir.path().join("a ' quoted.txt");
    let outside = point(px(2.0),px(2.0));
    f.view.update(&mut v, |r,cx| assert!(!r.terminal_drop_paths(std::slice::from_ref(&p), outside, cx)));
    let inside = f.view.read_with(&v, |r,_| r.terminal_layout().unwrap().grid.center());
    f.view.update(&mut v, |r,cx| {
        assert!(r.terminal_drop_paths(std::slice::from_ref(&p), inside, cx));
        r.modal = Some(Modal::Commands { at: 0 });
        assert!(!r.terminal_drop_paths(&[p], inside, cx));
        assert_eq!(source_text(r), "unchanged\n");
    });
    assert_eq!(std::fs::read_to_string(f.dir.path().join("a ' quoted.txt")).unwrap(), "unchanged\n");
}

#[gpui::test]
fn gui_terminal_external_file_drop_uses_actual_grid_and_respects_modal(cx: &mut TestAppContext) {
    use gpui::InputEvent;
    let f = fixture(cx, Some(("drop.txt", b"keep\n")));
    settle(cx, &f, active_is("drop.txt"));
    let mut v = visual(cx, &f); draw(&mut v);
    v.simulate_keystrokes("ctrl-alt-t"); draw(&mut v);
    f.view.update(&mut v, |r,cx| r.dispatch(Command::Terminal(code_core::terminal::Command::Focus(false)),cx));
    draw(&mut v);
    let at = f.view.read_with(&v, |r,_| r.terminal_layout().unwrap().grid.center());
    let path = f.dir.path().join("drop.txt");
    v.update(|w,cx| {
        w.dispatch_event(gpui::FileDropEvent::Entered { position: at, paths: gpui::ExternalPaths([path.clone()].into_iter().collect()) }.to_platform_input(),cx);
        w.dispatch_event(gpui::FileDropEvent::Submit { position: at }.to_platform_input(),cx);
    });
    assert!(f.view.read_with(&v, |r,_| r.session.state().terminal().focused()));
    f.view.update(&mut v, |r,cx| {
        r.dispatch(Command::Terminal(code_core::terminal::Command::Focus(false)),cx);
        r.modal = Some(Modal::Commands { at: 0 });
    });
    draw(&mut v);
    v.update(|w,cx| {
        w.dispatch_event(gpui::FileDropEvent::Entered { position: at, paths: gpui::ExternalPaths([path].into_iter().collect()) }.to_platform_input(),cx);
        w.dispatch_event(gpui::FileDropEvent::Submit { position: at }.to_platform_input(),cx);
    });
    assert!(f.view.read_with(&v, |r,_| !r.session.state().terminal().focused() && r.modal.is_some()));
    assert_eq!(f.view.read_with(&v, |r,_| source_text(r)), "keep\n");
    assert_eq!(std::fs::read_to_string(f.dir.path().join("drop.txt")).unwrap(), "keep\n");
}

#[gpui::test]
fn gui_terminal_native_commits_clipboard_and_cell_output_reach_child_only(cx: &mut TestAppContext) {
    let f = fixture(cx, Some(("source.txt", b"source\n")));
    settle(cx, &f, active_is("source.txt"));
    f.view.update(cx, |r,_| r.enable_draft_input_for_test());
    let mut v = visual(cx, &f); draw(&mut v);
    v.simulate_keystrokes("ctrl-alt-t");
    drop(v);
    settle(cx, &f, |r| r.session.state().terminal().active().is_some_and(|t| t.running() && t.frame().is_some()));
    let mut v = visual(cx, &f); draw(&mut v);
    let mut input = ime_handler(&f, &mut v);
    input.replace_and_mark_text_in_range(None, "temporary", None);
    input.replace_text_in_range(None, "printf '%s' 'É界😀' > terminal-output.txt");
    v.simulate_keystrokes("enter");
    drop(v);
    settle(cx, &f, |_| std::fs::read(f.dir.path().join("terminal-output.txt")).is_ok_and(|v| v == "É界😀".as_bytes()));
    let mut v = visual(cx, &f); draw(&mut v);
    v.update(|_,cx| cx.write_to_clipboard(ClipboardItem::new_string("printf '\\033[31mGUI_TERMINAL_RED\\033[0m\\n'".into())));
    v.simulate_keystrokes("ctrl-shift-v enter");
    drop(v);
    settle(cx, &f, |r| r.session.state().terminal().active().and_then(|t| t.frame()).is_some_and(|f| {
        f.cells.iter().any(|c| c.fg.is_some() && c.text == "G")
    }));
    let mut v = visual(cx, &f); draw(&mut v);
    assert_eq!(f.view.read_with(&v, |r,_| source_text(r)), "source\n");
    assert_eq!(std::fs::read_to_string(f.dir.path().join("source.txt")).unwrap(), "source\n");
    assert_eq!(std::fs::read_to_string(f.dir.path().join("terminal-output.txt")).unwrap(), "É界😀");
}

#[gpui::test]
fn gui_terminal_tab_and_explorer_drags_quote_saved_paths_without_execution(cx: &mut TestAppContext) {
    let f = fixture(cx, None);
    let root = f.dir.path().join("project");
    std::fs::create_dir(&root).unwrap();
    let path = root.join("a ' quoted.txt");
    std::fs::write(&path,b"saved\n").unwrap();
    let other = root.join("other.txt"); std::fs::write(&other,b"saved\n").unwrap();
    f.view.update(cx,|r,cx| r.open_root(root.clone(),cx));
    settle(cx,&f,scanned);
    f.view.update(cx,|r,cx| r.dispatch(Command::OpenRetained(path.clone()),cx));
    settle(cx,&f,active_is("a ' quoted.txt"));
    let mut v = visual(cx,&f); draw(&mut v);
    v.simulate_keystrokes("ctrl-alt-t");
    drop(v);
    settle(cx,&f,|r| r.session.state().terminal().active().is_some_and(|t| t.running() && t.frame().is_some()));
    let mut v = visual(cx,&f); draw(&mut v);
    for (name,tree,menu) in [("tab-result",false,false),("tree-result",true,false),("tab-menu-result",false,true),("tree-menu-result",true,true)] {
        v.update(|_,cx| cx.write_to_clipboard(ClipboardItem::new_string(format!("printf '%s' > {name} "))));
        v.simulate_keystrokes("ctrl-shift-v"); draw(&mut v);
        let (from,to) = f.view.read_with(&v,|r,_| {
            let from = if tree {
                let index = r.session.state().tree_rows().position(|row| &*row.path == "a ' quoted.txt").unwrap();
                (80.0,toolbar_h()+line_h()*(index.saturating_sub(r.tree_scroll) as f32+1.5))
            } else {
                let (a,b,_) = r.tab_hits[0]; ((a+b)*0.5,toolbar_h()+line_h()*0.5)
            };
            let to = r.terminal_layout().unwrap().grid.center();
            (from,(f32::from(to.x),f32::from(to.y)))
        });
        if menu {
            v.simulate_event(gpui::MouseDownEvent { button:MouseButton::Right,position:point(px(from.0),px(from.1)),..Default::default() });
            let before = f.view.read_with(&v,|r,_| match &r.modal {
                Some(Modal::Context {entries,..}) => {
                    let at = entries.iter().position(|e| *e == MenuEntry::Command(CommandId::PasteTerminalPath)).unwrap();
                    entries.iter().take(at).filter(|e| matches!(e,MenuEntry::Command(_))).count()
                },_ => panic!("path insertion stays in its context menu"),
            });
            f.view.update(&mut v,|r,cx| r.dispatch(Command::OpenRetained(other.clone()),cx));
            drop(v); settle(cx,&f,active_is("other.txt"));
            v = visual(cx,&f); draw(&mut v);
            assert!(f.view.read_with(&v,|r,_| matches!(r.modal,Some(Modal::Context {..}))),"the context remains open while its originating file loses focus");
            for _ in 0..before {v.simulate_keystrokes("down");}
            v.simulate_keystrokes("enter");
        } else { drag(&mut v,from,to); }
        assert!(f.view.read_with(&v,|r,_| r.session.state().terminal().focused()));
        assert!(!root.join(name).exists(),"dropping only inserts a quoted path");
        v.simulate_keystrokes("enter");
        drop(v);
        settle(cx,&f,|_| std::fs::read(root.join(name)).is_ok_and(|v| v == path.to_string_lossy().as_bytes()));
        v = visual(cx,&f); draw(&mut v);
    }
    assert_eq!(std::fs::read(&path).unwrap(),b"saved\n");
    assert_eq!(f.view.read_with(&v,|r,_| source_text(r)),"saved\n");
}

#[gpui::test]
fn gui_terminal_header_controls_and_ticket_prompt_are_modal(cx: &mut TestAppContext) {
    let f = fixture(cx,Some(("keep.txt",b"keep\n")));
    settle(cx,&f,active_is("keep.txt"));
    let mut v = visual(cx,&f); draw(&mut v);
    v.simulate_keystrokes("ctrl-alt-t");
    drop(v);
    settle(cx,&f,|r| r.session.state().terminal().active().is_some_and(|t| t.running()));
    let mut v = visual(cx,&f); draw(&mut v);
    let button = |v: &VisualTestContext,id| f.view.read_with(v,|r,_| r.terminal_layout().unwrap().buttons.iter().find(|(_,value,_)| *value == id).unwrap().0.center());
    v.simulate_click(button(&v,CommandId::StopTerminal),Modifiers::default());
    let first = f.view.read_with(&v,|r,_| match r.modal { Some(Modal::StatusDetails {kind:Details::Terminal {ticket},..}) => ticket,_ => panic!("terminal owns its confirmation") });
    v.simulate_keystrokes("ctrl-s ctrl-z");
    assert!(f.view.read_with(&v,|r,_| r.session.state().terminal().active().unwrap().running()));
    v.simulate_keystrokes("escape");
    assert!(f.view.read_with(&v,|r,_| r.modal.is_none() && r.session.state().terminal().prompt().is_none()));
    v.simulate_click(button(&v,CommandId::StopTerminal),Modifiers::default());
    assert!(f.view.read_with(&v,|r,_| r.session.state().terminal().prompt().unwrap().ticket > first));
    v.simulate_keystrokes("enter"); draw(&mut v);
    assert!(f.view.read_with(&v,|r,_| !r.session.state().terminal().active().unwrap().running() && r.modal.is_none()));
    v.simulate_click(button(&v,CommandId::NewTerminal),Modifiers::default()); draw(&mut v);
    assert_eq!(f.view.read_with(&v,|r,_| r.session.state().terminal().tabs().len()),2);
    v.simulate_click(button(&v,CommandId::ShowTerminal),Modifiers::default()); draw(&mut v);
    assert!(f.view.read_with(&v,|r,_| !r.session.state().terminal().shown()));
    assert_eq!(f.view.read_with(&v,|r,_| source_text(r)),"keep\n");
    assert_eq!(std::fs::read(f.dir.path().join("keep.txt")).unwrap(),b"keep\n");
}

#[gpui::test]
fn gui_terminal_cell_selection_copy_and_rejection_are_visible(cx: &mut TestAppContext) {
    let f = fixture(cx,Some(("keep.txt",b"keep\n")));
    settle(cx,&f,active_is("keep.txt"));
    let mut v = visual(cx,&f); draw(&mut v);
    v.simulate_keystrokes("ctrl-alt-t");
    drop(v);
    settle(cx,&f,|r| r.session.state().terminal().active().is_some_and(|t| t.running() && t.frame().is_some()));
    let mut v = visual(cx,&f); draw(&mut v);
    v.update(|_,cx| cx.write_to_clipboard(ClipboardItem::new_string("printf 'SELECT_ME\\n'".into())));
    v.simulate_keystrokes("ctrl-shift-v enter");
    drop(v);
    let row = |r: &ReaderView| {
        let frame = r.session.state().terminal().active()?.frame()?;
        frame.cells.chunks(frame.columns).position(|row| row.iter().take(9).map(|c| c.text.as_str()).collect::<String>() == "SELECT_ME")
    };
    settle(cx,&f,|r| row(r).is_some());
    let mut v = visual(cx,&f); draw(&mut v);
    let (from,to) = f.view.read_with(&v,|r,_| {
        let grid = r.terminal_layout().unwrap().grid;
        let y = grid.top()+px((row(r).unwrap() as f32+0.5)*line_h());
        (point(grid.left()+px(r.char_w*0.1),y),point(grid.left()+px(r.char_w*8.9),y))
    });
    let modifiers = Modifiers {shift:true,..Default::default()};
    v.simulate_mouse_down(from,MouseButton::Left,modifiers);
    v.simulate_mouse_move(to,MouseButton::Left,modifiers);
    v.simulate_mouse_up(to,MouseButton::Left,modifiers);
    drop(v);
    settle(cx,&f,|r| r.session.state().terminal().active().and_then(|t| t.frame()).is_some_and(|f| f.cells.iter().any(|c| c.selected)));
    let mut v = visual(cx,&f); draw(&mut v);
    v.simulate_keystrokes("ctrl-shift-c");
    drop(v);
    for _ in 0..100 {
        cx.run_until_parked(); f.view.update(cx,|r,cx| r.poll(cx));
        if cx.update(|cx| cx.read_from_clipboard().and_then(|item| item.text())).as_deref() == Some("SELECT_ME") { break; }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert_eq!(cx.update(|cx| cx.read_from_clipboard().and_then(|item| item.text())).as_deref(),Some("SELECT_ME"));
    f.view.update(cx,|r,cx| r.dispatch(Command::Terminal(code_core::terminal::Command::Paste("invalid\u{1b}paste".into())),cx));
    let mut v = visual(cx,&f); draw(&mut v);
    f.view.read_with(&v,|r,_| {
        assert!(r.session.state().terminal().active().unwrap().frame().is_some());
        let paint = r.terminal_paint().unwrap();
        assert!(paint.error && !paint.notice.is_empty(),"rejected input is visible alongside existing output");
        assert_eq!(source_text(r),"keep\n");
    });
    assert_eq!(std::fs::read(f.dir.path().join("keep.txt")).unwrap(),b"keep\n");
}

#[gpui::test]
fn gui_terminal_empty_constructor_can_return_to_editor_and_render_tiny(cx: &mut TestAppContext) {
    let f = fixture(cx,None);
    f.view.update(cx,|r,cx| r.open_root(f.dir.path().to_owned(),cx));
    settle(cx,&f,scanned);
    let mut v = visual(cx,&f); draw(&mut v);
    v.simulate_keystrokes("ctrl-alt-t");
    for (width,height) in [(900.0,500.0),(120.0,100.0),(1.0,1.0),(900.0,500.0)] {
        v.simulate_resize(size(px(width),px(height)));draw(&mut v);
        v.simulate_keystrokes("ctrl-z ctrl-s escape tab");
        assert!(f.view.read_with(&v,|r,_| r.session.state().active().is_none()));
    }
    v.simulate_keystrokes("ctrl-alt-0 ctrl-z f5 f10 escape");draw(&mut v);
    assert!(f.view.read_with(&v,|r,_| r.session.state().active().is_none() && r.modal.is_none() && !r.session.state().terminal().focused()));
}

fn terminal_text(r: &ReaderView) -> String {
    r.session.state().terminal().active().and_then(|tab| tab.frame()).map_or_else(String::new, |frame| {
        frame.cells.chunks(frame.columns).map(|row| row.iter().map(|cell| cell.text.as_str()).collect::<String>()).collect::<Vec<_>>().join("\n")
    })
}

#[gpui::test]
fn gui_terminal_shift_history_keys_scroll_locally_without_child_input(cx: &mut TestAppContext) {
    let f = fixture(cx,Some(("keep.txt",b"keep\n")));
    settle(cx,&f,active_is("keep.txt"));
    let mut v = visual(cx,&f); draw(&mut v);
    v.simulate_keystrokes("ctrl-alt-t"); drop(v);
    settle(cx,&f,|r| r.session.state().terminal().active().is_some_and(|t| t.running() && t.frame().is_some()));
    let mut v = visual(cx,&f); draw(&mut v);
    v.update(|_,cx| cx.write_to_clipboard(ClipboardItem::new_string("i=0; while [ $i -lt 120 ]; do printf 'HISTORY_%03d\\n' \"$i\"; i=$((i+1)); done; stty raw -echo; dd bs=1 count=1 of=history-input 2>/dev/null; stty sane".into())));
    v.simulate_keystrokes("ctrl-shift-v enter"); drop(v);
    let input = f.dir.path().join("history-input");
    settle(cx,&f,|r| input.exists() && terminal_text(r).contains("HISTORY_119") && r.session.state().terminal().active().and_then(|t| t.frame()).is_some_and(|f| f.scrollback > 20));
    let bottom = f.view.read_with(cx,|r,_| terminal_text(r));
    let mut v = visual(cx,&f); draw(&mut v);
    v.simulate_keystrokes("shift-pageup"); drop(v);
    settle(cx,&f,|r| terminal_text(r) != bottom);
    assert_eq!(std::fs::read(&input).unwrap(),b"","Shift+PageUp scrolls history without sending an escape to the child");
    let page = f.view.read_with(cx,|r,_| terminal_text(r));
    let mut v = visual(cx,&f); draw(&mut v);
    v.simulate_keystrokes("shift-pagedown"); drop(v);
    settle(cx,&f,|r| terminal_text(r) == bottom);
    let mut v = visual(cx,&f); draw(&mut v);
    v.simulate_keystrokes("shift-home"); drop(v);
    settle(cx,&f,|r| terminal_text(r).contains("HISTORY_000") && terminal_text(r) != page);
    let mut v = visual(cx,&f); draw(&mut v);
    v.simulate_keystrokes("shift-end"); drop(v);
    settle(cx,&f,|r| terminal_text(r) == bottom);
    assert_eq!(std::fs::read(&input).unwrap(),b"","local history keys send no escapes to the waiting child");
    let mut v = visual(cx,&f); draw(&mut v); v.simulate_keystrokes("z"); drop(v);
    settle(cx,&f,|_| std::fs::read(&input).is_ok_and(|bytes| bytes == b"z"));
    assert_eq!(f.view.read_with(cx,|r,_| source_text(r)),"keep\n");
    assert_eq!(std::fs::read(f.dir.path().join("keep.txt")).unwrap(),b"keep\n");
}

#[gpui::test]
fn gui_terminal_sgr_mouse_tracks_hover_and_all_buttons_and_releases_outside(cx: &mut TestAppContext) {
    let f = fixture(cx,Some(("keep.txt",b"keep\n")));
    settle(cx,&f,active_is("keep.txt"));
    let mut v = visual(cx,&f); draw(&mut v);
    v.simulate_keystrokes("ctrl-alt-t"); drop(v);
    settle(cx,&f,|r| r.session.state().terminal().active().is_some_and(|t| t.running() && t.frame().is_some()));
    let mut v = visual(cx,&f); draw(&mut v); drop(v);
    settle(cx,&f,|r| {
        let Some(layout) = r.terminal_layout() else {return false};
        r.session.state().terminal().active().and_then(|tab| tab.frame()).is_some_and(|frame| frame.columns == r.columns(f32::from(layout.grid.size.width)).clamp(2,512) && frame.rows == ((f32::from(layout.grid.size.height)/line_h()).floor() as usize).clamp(1,256))
    });
    let mut v = visual(cx,&f); draw(&mut v);
    let (a,b,outside,rows,columns) = f.view.read_with(&v,|r,_| {
        let grid = r.terminal_layout().unwrap().grid;
        let frame = r.session.state().terminal().active().unwrap().frame().unwrap();
        (point(grid.left()+px(r.char_w*0.5),grid.top()+px(line_h()*0.5)),
         point(grid.left()+px(r.char_w*2.5),grid.top()+px(line_h()*1.5)),
         point(px(r.width+50.0),px(r.height+50.0)),frame.rows,frame.columns)
    });
    let expected = format!("\u{1b}[<35;3;2M\u{1b}[<1;1;1M\u{1b}[<33;3;2M\u{1b}[<1;1;1m\u{1b}[<2;1;1M\u{1b}[<34;3;2M\u{1b}[<2;{columns};{rows}m");
    v.update(|_,cx| cx.write_to_clipboard(ClipboardItem::new_string(format!("printf '\\033[?1003h\\033[?1006hMOUSE_READY'; stty raw -echo; dd bs=1 count={} of=mouse-input 2>/dev/null; stty sane; printf '\\033[?1003l\\033[?1006l'",expected.len()))));
    v.simulate_keystrokes("ctrl-shift-v enter"); drop(v);
    let input = f.dir.path().join("mouse-input");
    settle(cx,&f,|r| input.exists() && terminal_text(r).contains("MOUSE_READY"));
    let mut v = visual(cx,&f); draw(&mut v);
    v.simulate_keystrokes("f10");
    v.simulate_mouse_move(b,None,Modifiers::default());
    v.simulate_mouse_down(a,MouseButton::Middle,Modifiers::default());
    v.simulate_mouse_up(outside,MouseButton::Middle,Modifiers::default());
    assert!(f.view.read_with(&v,|r,_| r.modal.is_some()));
    v.simulate_keystrokes("escape");
    v.simulate_mouse_move(b,None,Modifiers::default());
    v.simulate_mouse_down(a,MouseButton::Middle,Modifiers::default());
    v.simulate_mouse_move(b,MouseButton::Middle,Modifiers::default());
    v.simulate_mouse_up(point(px(-50.0),px(-50.0)),MouseButton::Middle,Modifiers::default());
    v.simulate_mouse_down(a,MouseButton::Right,Modifiers::default());
    v.simulate_mouse_move(b,MouseButton::Right,Modifiers::default());
    v.simulate_mouse_up(outside,MouseButton::Right,Modifiers::default());
    drop(v);
    settle(cx,&f,|_| std::fs::read(&input).is_ok_and(|bytes| bytes == expected.as_bytes()));
    assert_eq!(f.view.read_with(cx,|r,_| source_text(r)),"keep\n");
    assert_eq!(std::fs::read(f.dir.path().join("keep.txt")).unwrap(),b"keep\n");
}

#[gpui::test]
fn gui_terminal_find_owns_native_input_reveals_matches_and_restores_shell_focus(cx: &mut TestAppContext) {
    use code_core::terminal::Command as T;
    let f = fixture(cx, Some(("keep.txt", b"keep\n")));
    settle(cx, &f, active_is("keep.txt"));
    let mut v = visual(cx, &f); draw(&mut v);
    v.simulate_keystrokes("ctrl-alt-t"); drop(v);
    settle(cx,&f,|r|r.session.state().terminal().active().is_some_and(|t|t.running() && t.frame().is_some()));
    f.view.update(cx,|r,cx| r.dispatch(Command::Terminal(T::Text("stty -echo; printf 'needle.*é界\\r\\n'; printf 'last\\r\\n'\n".into())),cx));
    settle(cx,&f,|r|r.session.state().terminal().active().unwrap().frame().unwrap().cells.iter().any(|c|c.text == "界"));
    let mut v = visual(cx,&f); draw(&mut v);
    v.simulate_keystrokes("ctrl-f"); draw(&mut v);
    assert!(f.view.read_with(&v,|r,_| r.session.state().terminal().find() == Some("") && r.find_focus.is_none()));
    // Actual native text commits go to the shared query, never the PTY or source draft.
    let mut input = ime_handler(&f,&mut v);
    input.replace_text_in_range(None,"needle.*é界");
    v.simulate_keystrokes("enter"); drop(v);
    settle(cx,&f,|r|r.session.state().terminal().find_status() == "Match");
    let mut v = visual(cx,&f); draw(&mut v);
    assert!(f.view.read_with(&v,|r,_| r.session.state().terminal().active().unwrap().frame().unwrap().cells.iter().any(|c|c.search_match)));
    // A real file drop is refused at the shared boundary while Find owns input.
    let drop_at = f.view.read_with(&v,|r,_|r.terminal_layout().unwrap().grid.center());
    f.view.update(&mut v,|r,cx| assert!(r.terminal_drop_paths(&[f.dir.path().join("keep.txt")],drop_at,cx)));
    assert_eq!(f.view.read_with(&v,|r,_|r.session.state().terminal().find().map(str::to_owned)),Some("needle.*é界".into()));
    v.simulate_keystrokes("f10");
    let mut blocked = ime_handler(&f,&mut v); blocked.replace_text_in_range(None,"modal-only");
    v.simulate_keystrokes("escape");
    assert_eq!(f.view.read_with(&v,|r,_|r.session.state().terminal().find().map(str::to_owned)),Some("needle.*é界".into()));
    v.simulate_keystrokes("shift-enter ctrl-a");
    v.update(|_,cx| cx.write_to_clipboard(ClipboardItem::new_string("printf bad > find-must-not-execute".into())));
    v.simulate_keystrokes("ctrl-shift-v enter"); drop(v);
    settle(cx,&f,|r|r.session.state().terminal().find_status() == "No matches");
    let mut v = visual(cx,&f); draw(&mut v);
    for theme in ["darcula","github-light"] {
        f.view.update(&mut v,|r,cx| r.dispatch(Command::Theme(ThemeCommand::Select(ThemeSelection::from_id(theme).unwrap())),cx));
        for (w,h) in [(180.0,140.0),(1.0,1.0),(900.0,500.0)] { v.simulate_resize(size(px(w),px(h))); draw(&mut v); }
    }
    assert!(!f.dir.path().join("find-must-not-execute").exists());
    v.simulate_keystrokes("escape");
    assert!(f.view.read_with(&v,|r,_| r.session.state().terminal().find().is_none() && r.session.state().terminal().focused()));
    v.update(|_,cx| cx.write_to_clipboard(ClipboardItem::new_string("printf restored > find-shell-restored".into())));
    v.simulate_keystrokes("ctrl-shift-v enter"); drop(v);
    settle(cx,&f,|_| std::fs::read(f.dir.path().join("find-shell-restored")).is_ok_and(|b|b == b"restored"));
    let mut v = visual(cx,&f); draw(&mut v);
    v.simulate_keystrokes("ctrl-f x ctrl-alt-n");
    assert!(f.view.read_with(&v,|r,_| r.session.state().terminal().find().is_none()));
    assert_eq!(f.view.read_with(&v,|r,_|source_text(r)),"keep\n");
    assert_eq!(std::fs::read(f.dir.path().join("keep.txt")).unwrap(),b"keep\n");
}

#[gpui::test]
fn gui_terminal_divider_drag_released_over_cells_still_commits_the_layout(cx: &mut TestAppContext) {
    let f = fixture(cx, None);
    let root = project(f.dir.path(), &["a.rs"]);
    f.view.update(cx, |v, cx| v.open_root(root.clone(), cx));
    settle(cx, &f, scanned);
    f.view.update(cx, |v, cx| v.dispatch(Command::OpenRetained(root.join("a.rs")), cx));
    settle(cx, &f, active_is("a.rs"));
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(1000.0), px(600.0)));
    draw(&mut v);
    v.simulate_keystrokes("ctrl-alt-t"); draw(&mut v);
    let (sidebar, mid, below) = f.view.read_with(&v, |r, _| {
        let grid = r.terminal_layout().unwrap().grid;
        (r.sidebar_w, f32::from(r.panes[0].text.origin.y) + 20.0, f32::from(grid.top()) + line_h() * 1.5)
    });
    drag(&mut v, (sidebar, mid), (350.0, below));
    assert_eq!(f.view.read_with(&v, |r, _| (r.sidebar_w, r.session().state().layout().explorer_width, r.drag.is_none())), (350.0, Some(350), true));
}

#[gpui::test]
fn gui_terminal_plain_ctrl_v_reaches_the_program_and_ctrl_shift_v_pastes(cx: &mut TestAppContext) {
    let f = fixture(cx, Some(("keep.txt", b"keep\n")));
    settle(cx, &f, active_is("keep.txt"));
    let mut v = visual(cx, &f); draw(&mut v);
    v.simulate_keystrokes("ctrl-alt-t"); drop(v);
    settle(cx, &f, |r| r.session.state().terminal().active().is_some_and(|t| t.running() && t.frame().is_some()));
    let mut v = visual(cx, &f); draw(&mut v);
    v.update(|_,cx| cx.write_to_clipboard(ClipboardItem::new_string("stty raw -echo; printf CTRLV_READY; dd bs=1 count=1 of=ctrl-v 2>/dev/null; stty sane".into())));
    v.simulate_keystrokes("ctrl-shift-v enter"); drop(v);
    settle(cx, &f, |r| terminal_text(r).contains("CTRLV_READY"));
    let mut v = visual(cx, &f); draw(&mut v);
    v.update(|_,cx| cx.write_to_clipboard(ClipboardItem::new_string("must not paste".into())));
    v.simulate_keystrokes("ctrl-v"); drop(v);
    let out = f.dir.path().join("ctrl-v");
    settle(cx, &f, |_| std::fs::read(&out).is_ok_and(|bytes| bytes == [0x16]));
    assert_eq!(std::fs::read(f.dir.path().join("keep.txt")).unwrap(), b"keep\n");
}

#[gpui::test]
fn gui_terminal_close_and_stop_ask_their_own_question_and_a_newer_one_replaces_it(cx: &mut TestAppContext) {
    let f = fixture(cx,Some(("keep.txt",b"keep\n")));
    settle(cx,&f,active_is("keep.txt"));
    let mut v = visual(cx,&f); draw(&mut v);
    v.simulate_keystrokes("ctrl-alt-t");
    drop(v);
    settle(cx,&f,|r| r.session.state().terminal().active().is_some_and(|t| t.running()));
    let mut v = visual(cx,&f); draw(&mut v);
    let button = |v: &VisualTestContext,id| f.view.read_with(v,|r,_| r.terminal_layout().unwrap().buttons.iter().find(|(_,value,_)| *value == id).unwrap().0.center());
    let dialog = |v: &VisualTestContext| f.view.read_with(v,|r,_| match &r.modal {
        Some(Modal::StatusDetails { title, kind: kind @ Details::Terminal { ticket }, .. }) => (title.to_string(), r.details_labels(*kind), *ticket),
        _ => panic!("terminal confirmation expected"),
    });
    // Closing a running tab asks about closing it, not stopping it.
    v.simulate_click(button(&v,CommandId::CloseTerminal),Modifiers::default());
    let (title, labels, first) = dialog(&v);
    assert_eq!((title.as_str(), labels), ("Close terminal tab?", vec!["Close tab", "Cancel"]));
    // A newer question replaces it entirely, ticket included.
    f.view.update(&mut v,|r,cx| r.dispatch(Command::Terminal(code_core::terminal::Command::Stop),cx)); draw(&mut v);
    let (title, labels, second) = dialog(&v);
    assert_eq!((title.as_str(), labels), ("Stop terminal?", vec!["Stop", "Cancel"]));
    assert!(second > first);
    v.simulate_keystrokes("escape");
    assert!(f.view.read_with(&v,|r,_| r.modal.is_none() && r.session.state().terminal().active().unwrap().running()));
    v.simulate_click(button(&v,CommandId::CloseTerminal),Modifiers::default());
    v.simulate_keystrokes("enter"); draw(&mut v);
    assert!(f.view.read_with(&v,|r,_| r.session.state().terminal().tabs().is_empty() && r.modal.is_none()));
    assert_eq!(std::fs::read(f.dir.path().join("keep.txt")).unwrap(),b"keep\n");
}

#[gpui::test]
fn gui_terminal_wheel_scrolls_history_during_find(cx: &mut TestAppContext) {
    let f = fixture(cx,Some(("keep.txt",b"keep\n")));
    settle(cx,&f,active_is("keep.txt"));
    let mut v = visual(cx,&f); draw(&mut v);
    v.simulate_keystrokes("ctrl-alt-t");
    drop(v);
    settle(cx,&f,|r| r.session.state().terminal().active().is_some_and(|t| t.running() && t.frame().is_some()));
    f.view.update(cx,|r,cx| r.dispatch(Command::Terminal(code_core::terminal::Command::Text("stty -echo; seq -f 'LINE_%03g' 1 200\n".into())),cx));
    settle(cx,&f,|r| terminal_text(r).contains("LINE_200"));
    let mut v = visual(cx,&f); draw(&mut v);
    v.simulate_keystrokes("ctrl-f");
    let at = f.view.read_with(&v,|r,_| r.terminal_layout().unwrap().grid.center());
    for _ in 0..10 {
        v.simulate_event(ScrollEvent { position: at, delta: ScrollDelta::Lines(point(0.0, 1.0)), ..Default::default() });
    }
    drop(v);
    settle(cx,&f,|r| !terminal_text(r).contains("LINE_200"));
    assert!(f.view.read_with(cx,|r,_| r.session.state().terminal().find().is_some()), "Find stays open");
}
