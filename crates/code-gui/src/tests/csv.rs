use super::*;

fn choose_csv_command(v: &mut VisualTestContext, f: &Fixture, id: CommandId) {
    v.simulate_keystrokes("f10");
    let at = f.view.read_with(v, |r, _| r.modal_rows().iter().position(|row| row.0 == id.label() || row.0.starts_with(&format!("{} (", id.label()))).unwrap());
    for _ in 0..at { v.simulate_keystrokes("down"); }
    v.simulate_keystrokes("enter");
}

fn csv_ready(r: &ReaderView) -> bool { r.session.state().active().and_then(|t| r.session.state().csv(t.id())).is_some_and(|v| !v.pending) }

#[gpui::test]
fn gui_csv_table_opt_in_cells_edit_filter_sort_and_column_commands(cx: &mut TestAppContext) {
    let original = b"name,note\r\nbeta,\"a,b\"\r\nalpha,\"two\nlines\"\r\n";
    let f = fixture(cx, Some(("data.csv", original)));
    settle(cx, &f, active_is("data.csv"));
    assert!(f.view.read_with(cx, |r, _| r.session.state().csv(r.session.state().active().unwrap().id()).is_none()));
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(900.0), px(400.0)));
    draw(&mut v);
    choose_csv_command(&mut v, &f, CommandId::CsvTable);
    drop(v); settle(cx, &f, csv_ready); let mut v = visual(cx, &f); draw(&mut v);
    assert!(f.view.read_with(&v, |r, _| !r.draft_context() && r.draft_target().is_none()), "native source input is disabled in the table");
    f.view.update(&mut v, |r, _| r.enable_draft_input_for_test());
    v.simulate_keystrokes("right enter ctrl-a");
    v.simulate_input("λ,changed");
    v.simulate_keystrokes("enter");
    drop(v); settle(cx, &f, csv_ready); let mut v = visual(cx, &f); draw(&mut v);
    let content = f.view.read_with(&v, |r, _| { let s = r.session.state().current_source().unwrap(); s.copy_range(0..s.len_bytes()).unwrap() });
    assert_eq!(content, "name,note\r\nbeta,\"λ,changed\"\r\nalpha,\"two\nlines\"\r\n");
    v.simulate_keystrokes("ctrl-z");
    drop(v); settle(cx, &f, csv_ready); let mut v = visual(cx, &f);
    assert_eq!(f.view.read_with(&v, |r, _| { let s = r.session.state().current_source().unwrap(); s.copy_range(0..s.len_bytes()).unwrap().into_bytes() }), original, "Undo restores the exact cell bytes before further table commands");
    choose_csv_command(&mut v, &f, CommandId::CsvSort);
    drop(v); settle(cx, &f, csv_ready); let mut v = visual(cx, &f);
    choose_csv_command(&mut v, &f, CommandId::CsvFilter);
    v.simulate_input("two"); v.simulate_keystrokes("enter");
    drop(v); settle(cx, &f, csv_ready); let mut v = visual(cx, &f);
    assert_eq!(f.view.read_with(&v, |r, _| r.session.state().csv(r.session.state().active().unwrap().id()).unwrap().row_count()), 1);
    choose_csv_command(&mut v, &f, CommandId::CsvClearFilter);
    drop(v); settle(cx, &f, csv_ready); let mut v = visual(cx, &f);
    choose_csv_command(&mut v, &f, CommandId::CsvMoveLeft);
    drop(v); settle(cx, &f, csv_ready); let mut v = visual(cx, &f);
    let content = f.view.read_with(&v, |r, _| { let s = r.session.state().current_source().unwrap(); s.copy_range(0..s.len_bytes()).unwrap() });
    assert!(content.starts_with("note,name\r\n"));
    choose_csv_command(&mut v, &f, CommandId::CsvDropColumn);
    drop(v); settle(cx, &f, csv_ready); let mut v = visual(cx, &f);
    assert_eq!(f.view.read_with(&v, |r, _| r.session.state().csv(r.session.state().active().unwrap().id()).unwrap().column_count()), 1);
    draw(&mut v);
    let source_button = f.view.read_with(&v, |r, _| {
        let (bounds, action) = r.preview_button(r.panes[0]).unwrap();
        assert_eq!((action.id, action.label), (CommandId::CsvTable, "Source"));
        bounds.center()
    });
    v.simulate_click(source_button, Modifiers::default());
    assert!(f.view.read_with(&v, |r, _| r.session.state().csv(r.session.state().active().unwrap().id()).is_none()));
    assert_eq!(std::fs::read(f.dir.path().join("data.csv")).unwrap(), original);
}

#[gpui::test]
fn gui_csv_mouse_native_composition_and_modal_safety_at_narrow_sizes(cx: &mut TestAppContext) {
    let original = "name,note\nfirst,unchanged\nsecond,other\n";
    let f = fixture(cx, Some(("data.csv", original.as_bytes())));
    settle(cx, &f, active_is("data.csv"));
    let mut v = visual(cx, &f); draw(&mut v);
    choose_csv_command(&mut v, &f, CommandId::CsvTable);
    drop(v); settle(cx, &f, csv_ready); let mut v = visual(cx, &f);
    f.view.update(&mut v, |r, _| r.enable_draft_input_for_test());
    for theme in ["darcula", "github-light"] {
        f.view.update(&mut v, |r, cx| r.dispatch(Command::Theme(ThemeCommand::Select(ThemeSelection::from_id(theme).unwrap())), cx));
        v.simulate_resize(size(px(800.0), px(300.0))); draw(&mut v);
        let at = f.view.read_with(&v, |r, _| {
            let b = r.panes[0].text;
            point(b.origin.x + px(5.0 * r.char_w + 18.0 * r.char_w + 8.0), b.origin.y + px(line_h() * 2.5))
        });
        v.simulate_click(at, Modifiers::default());
        assert_eq!(f.view.read_with(&v, |r, _| { let t = r.session.state().csv(r.session.state().active().unwrap().id()).unwrap(); (t.row, t.column) }), (1, 1));
        v.simulate_keystrokes("enter ctrl-a"); draw(&mut v);
        let mut input = ime_handler(&f, &mut v);
        input.replace_and_mark_text_in_range(None, "界😀", Some(0..1));
        assert_eq!(input.marked_text_range(), Some(0..3));
        assert_eq!(input.selected_text_range(false).unwrap().range, 0..1);
        input.replace_text_in_range(None, "é,committed");
        assert!(input.marked_text_range().is_none());
        draw(&mut v);
        f.view.update(&mut v, |r, _| {
            let frame = r.csv_frame(r.panes[0]).unwrap();
            let cell = frame.cells.iter().find(|cell| cell.editing).unwrap();
            assert_eq!(cell.text, "é,committed", "the pending edit is shown inside its table cell");
            assert_eq!(r.csv_ui.input_bounds, Some(cell.bounds), "native input geometry follows the inline cell");
            assert!(r.caret_blink.is_none(), "CSV does not keep the hidden source caret timer alive");
        });
        assert_eq!(f.view.read_with(&v, |r, _| { let s = r.session.state().current_source().unwrap(); s.copy_range(0..s.len_bytes()).unwrap() }), original, "pending cell input cannot modify the document");
        v.simulate_resize(size(px(180.0),px(240.0))); draw(&mut v);
        f.view.update(&mut v, |r, _| {
            let frame = r.csv_frame(r.panes[0]).unwrap();
            assert!(frame.cells.iter().any(|cell| cell.editing), "resizing keeps the pending cell visible");
            assert!(r.csv_ui.input_bounds.is_some());
        });
        v.simulate_keystrokes("escape");
        for (w,h) in [(180.0,100.0), (1.0,1.0), (800.0,300.0)] { v.simulate_resize(size(px(w),px(h))); draw(&mut v); }
    }
    v.simulate_keystrokes("enter ctrl-a"); draw(&mut v);
    let mut input = ime_handler(&f, &mut v);
    input.replace_and_mark_text_in_range(None, "pending", None);
    let mark = input.marked_text_range().unwrap();
    input.replace_text_in_range(Some(mark), "");
    assert_eq!(f.view.read_with(&v, |r, _| r.csv_field().unwrap().2.to_owned()), "other", "native cancellation restores the pending field");
    v.simulate_keystrokes("escape f10");
    input.replace_text_in_range(None, "modal must consume");
    assert_eq!(f.view.read_with(&v, |r, _| { let s = r.session.state().current_source().unwrap(); s.copy_range(0..s.len_bytes()).unwrap() }), original);
    assert_eq!(std::fs::read_to_string(f.dir.path().join("data.csv")).unwrap(), original);
}

#[gpui::test]
fn gui_csv_pixel_wheel_accumulates_and_native_rejections_preserve_input(cx: &mut TestAppContext) {
    let source = format!("a,b,c,d,e,f,g,h\n{}", "0,1,2,3,4,5,6,7\n".repeat(200));
    let f = fixture(cx, Some(("data.csv", source.as_bytes())));
    settle(cx, &f, active_is("data.csv"));
    let mut v = visual(cx, &f); draw(&mut v);
    choose_csv_command(&mut v, &f, CommandId::CsvTable);
    drop(v); settle(cx, &f, csv_ready); let mut v = visual(cx, &f);
    v.simulate_resize(size(px(600.0),px(300.0))); draw(&mut v);
    let point = f.view.read_with(&v, |r, _| r.panes[0].text.center());
    for _ in 0..10 { v.simulate_event(ScrollEvent { position: point, delta: ScrollDelta::Pixels(gpui::point(px(0.0),px(-2.0))), ..Default::default() }); }
    assert_eq!(f.view.read_with(&v, |r, _| r.session.state().csv(r.session.state().active().unwrap().id()).unwrap().top), 1);
    let col_w = f.view.read_with(&v, |r, _| 18.0 * r.char_w);
    for _ in 0..10 { v.simulate_event(ScrollEvent { position: point, delta: ScrollDelta::Pixels(gpui::point(px(-col_w / 10.0),px(0.0))), ..Default::default() }); }
    assert_eq!(f.view.read_with(&v, |r, _| r.session.state().csv(r.session.state().active().unwrap().id()).unwrap().left), 1);
    f.view.update(&mut v, |r, _| r.enable_draft_input_for_test());
    v.simulate_keystrokes("enter ctrl-a"); draw(&mut v);
    let mut input = ime_handler(&f, &mut v);
    input.replace_and_mark_text_in_range(None, "😀", Some(2..2));
    let selected = input.selected_text_range(false).unwrap().range;
    input.replace_text_in_range(Some(1..1), "");
    assert_eq!(input.marked_text_range(), Some(0..2), "invalid UTF-16 cancellation cannot clear a native mark");
    input.replace_text_in_range(Some(1..1), "invalid surrogate");
    assert_eq!(input.marked_text_range(), Some(0..2));
    assert_eq!(input.selected_text_range(false).unwrap().range, selected);
    input.replace_and_mark_text_in_range(None, "x", Some(4..4));
    assert_eq!(input.marked_text_range(), Some(0..2), "invalid mark selection is refused");
    input.replace_text_in_range(None, &"x".repeat(code_core::csv::MAX_INPUT + 1));
    assert_eq!(input.marked_text_range(), Some(0..2), "oversized replacement preserves the native mark");
    assert_eq!(f.view.read_with(&v, |r, _| r.csv_field().unwrap().2.to_owned()), "😀");
    input.unmark_text(); input.unmark_text();
    input.replace_text_in_range(None, "😀");
    assert_eq!(f.view.read_with(&v, |r, _| r.csv_field().unwrap().2.to_owned()), "😀", "terminal IME callbacks cannot duplicate an accepted composition");
    v.simulate_keystrokes("escape escape");
    assert_eq!(std::fs::read_to_string(f.dir.path().join("data.csv")).unwrap(), source);
}

#[gpui::test]
fn gui_csv_external_change_error_is_visible_while_pending_text_is_preserved(cx: &mut TestAppContext) {
    let f = fixture(cx, Some(("changed.csv", b"h,v\na,old\n")));
    settle(cx, &f, active_is("changed.csv"));
    let mut v = visual(cx, &f); draw(&mut v); choose_csv_command(&mut v, &f, CommandId::CsvTable);
    drop(v); settle(cx, &f, csv_ready); let mut v = visual(cx, &f);
    f.view.update(&mut v, |r, _| r.enable_draft_input_for_test());
    v.simulate_keystrokes("right enter ctrl-a"); v.simulate_input("pending field");
    std::fs::write(f.dir.path().join("changed.csv"), b"h,v\nb,external replacement\n").unwrap();
    f.view.update(&mut v, |r, cx| r.dispatch(Command::Recheck, cx));
    drop(v); settle(cx, &f, |r| r.session.state().active().and_then(|t| r.session.state().csv(t.id())).is_some_and(|v| v.error.is_some()));
    let mut v = visual(cx, &f); draw(&mut v);
    f.view.update(&mut v, |r, _| {
        let frame = r.csv_frame(r.panes[0]).unwrap();
        assert!(frame.input.is_none() && frame.status.contains("source changed"), "stale-source error takes priority over ordinary field help");
        assert_eq!(r.csv_field().unwrap().2, "pending field");
    });
    v.simulate_keystrokes("escape");
    assert_eq!(std::fs::read(f.dir.path().join("changed.csv")).unwrap(), b"h,v\nb,external replacement\n");
}

#[gpui::test]
fn gui_csv_history_shortcuts_undo_and_redo_exact_saved_cell_bytes_and_refuse_pending_fields(cx: &mut TestAppContext) {
    let original = "name,note\r\nbeta,\"a,b\"\r\nalpha,last\r\n";
    let edited = "name,note\r\nbeta,\"λ,changed\"\r\nalpha,last\r\n";
    let f = fixture(cx, Some(("history.csv", original.as_bytes())));
    settle(cx, &f, active_is("history.csv"));
    let mut v = visual(cx, &f); draw(&mut v); choose_csv_command(&mut v, &f, CommandId::CsvTable);
    drop(v); settle(cx, &f, csv_ready); let mut v = visual(cx, &f);
    f.view.update(&mut v, |r, _| r.enable_draft_input_for_test());
    v.simulate_keystrokes("right enter ctrl-a"); v.simulate_input("λ,changed"); v.simulate_keystrokes("enter");
    drop(v); settle(cx, &f, csv_ready); let mut v = visual(cx, &f);
    v.simulate_keystrokes("ctrl-s");
    drop(v); settle(cx, &f, |r| csv_ready(r) && r.session.state().active().is_some_and(|t| !r.session.state().tab_dirty(t.id()))); let mut v = visual(cx, &f);
    assert_eq!(std::fs::read_to_string(f.dir.path().join("history.csv")).unwrap(), edited);
    let text = |v: &VisualTestContext| f.view.read_with(v, |r, _| { let s = r.session.state().current_source().unwrap(); s.copy_range(0..s.len_bytes()).unwrap() });
    assert_eq!(text(&v), edited);
    for (keys, expected) in [("ctrl-z", original), ("ctrl-shift-z", edited), ("ctrl-z", original), ("ctrl-y", edited)] {
        v.simulate_keystrokes(keys);
        drop(v); settle(cx, &f, csv_ready); v = visual(cx, &f);
        assert_eq!(text(&v), expected, "{keys} operates the table's shared draft history");
        assert_eq!(std::fs::read_to_string(f.dir.path().join("history.csv")).unwrap(), edited, "history shortcuts never save implicitly");
    }
    v.simulate_keystrokes("enter ctrl-a"); v.simulate_input("still pending");
    v.simulate_keystrokes("ctrl-z ctrl-shift-z ctrl-y");
    f.view.read_with(&v, |r, _| {
        assert_eq!(r.csv_field().unwrap().2, "still pending");
        assert_eq!(r.session.disabled(CommandId::Undo), Some("Finish the CSV field with Enter or Escape first"));
        assert!(r.session.state().notices().iter().any(|n| n.contains("Enter or Escape")));
    });
    assert_eq!(text(&v), edited);
    assert_eq!(std::fs::read_to_string(f.dir.path().join("history.csv")).unwrap(), edited);
}
