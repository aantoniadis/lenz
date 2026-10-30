use super::*;

const JAVA: &str = "interface PgWireAuthenticator {\n    enum Trust implements PgWireAuthenticator {\n        INSTANCE;\n        public static void main(String[] args) {\n            System.out.println(\"Hello Lenz!\");\n            [1,2,3];\n        }\n    }\n}\n";

fn ready(cx: &mut TestAppContext, f: &Fixture) {
    settle(cx, f, |r| r.session.state().active().is_some_and(|tab| r.session.state().diagnostics(tab.id()).is_some_and(|d| !d.items.is_empty()) && r.session.state().highlights().is_some()));
}
fn draw_inline(cx: &mut TestAppContext, f: &Fixture, v: &mut VisualTestContext, line: usize) {
    for _ in 0..400 {
        cx.run_until_parked(); f.view.update(v, |r, cx| r.poll(cx)); draw(v);
        if f.view.read_with(v, |r, _| r.painted_inline.iter().any(|(_, annotation)| annotation.line == line)) { return; }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    panic!("inline problem line {} absent: {}", line + 1, f.view.read_with(v, |r, _| r.status()));
}
fn assert_array_painted(f: &Fixture, v: &mut VisualTestContext, point: gpui::Point<Pixels>, line: usize) {
    f.view.read_with(v, |r, _| {
        let state = r.session.state(); let tab = state.active().unwrap();
        let base = state.view(tab.id()).unwrap(); let shown = state.presentation_view(tab.id()).unwrap();
        assert_eq!(base.snapshot().key(), shown.snapshot().key());
        assert!(Arc::ptr_eq(base.snapshot().source_meta(), shown.snapshot().source_meta()));
        let annotations: Vec<_> = r.painted_inline.iter().filter(|(_, annotation)| annotation.line == line).collect();
        assert_eq!(annotations.len(), 1, "one suffix for the diagnostic's logical start line");
        let (_, annotation) = annotations[0];
        assert!(annotation.text.starts_with("Error:") && annotation.text.contains("(+1)"), "{}", annotation.text);
        assert_eq!(annotation.color, state.palette().error);
        assert!(annotation.x >= num_w() + 8.0 && annotation.width >= 0.0);
        assert!(r.visible_rows().iter().all(|row| !row.text.contains("Error:")), "messages never enter source projections");
        let painted = r.painted_problem_bounds.borrow();
        assert!(painted.iter().any(|(_, bounds, color)| *color == state.palette().error
            && (f32::from(bounds.origin.x) - f32::from(point.x)).abs() < 1.0
            && (f32::from(bounds.origin.y) - f32::from(point.y) - line_h() / 2.0 + 2.0).abs() < 1.0
            && f32::from(bounds.size.width) >= 2.0), "actual underline rectangles must start at the '[' glyph: {point:?}; {painted:?}");
    });
}

#[gpui::test]
fn gui_inline_java_array_errors_keep_source_geometry_copy_and_file_bytes(cx: &mut TestAppContext) {
    let f = fixture(cx, Some(("PgWireAuthenticator.java", JAVA.as_bytes()))); ready(cx, &f);
    let mut v = visual(cx, &f); v.simulate_resize(size(px(900.0), px(500.0)));
    for dark in [true, false] {
        for wrapped in [false, true] {
            f.view.update(&mut v, |r, cx| {
                r.dispatch(Command::Theme(ThemeCommand::Select(ThemeSelection::from_id(if dark { "darcula" } else { "github-light" }).unwrap())), cx);
                r.dispatch(Command::SetSoftWrap(wrapped), cx);
                r.dispatch(Command::Select(Selection { anchor: 0, head: 0 }), cx);
            });
            v.simulate_keystrokes("f8");
            let (point, _) = settle_native_literal(cx, &f, &mut v, "[1,2,3]");
            draw_inline(cx, &f, &mut v, 5);
            assert_array_painted(&f, &mut v, point, 5);
            assert_eq!(f.view.read_with(&v, |r, _| r.session.state().current_source().unwrap().line_of(r.session.state().selection().unwrap().head)), 5);
            v.simulate_keystrokes("ctrl-a ctrl-c");
            assert_eq!(v.read_from_clipboard().and_then(|item| item.text()).as_deref(), Some(JAVA));
        }
    }
    v.simulate_keystrokes("f8");
    f.view.update(&mut v, |r, cx| { r.dispatch(Command::SetStickyScroll(true), cx); r.dispatch(Command::Scroll(2), cx); });
    draw_inline(cx, &f, &mut v, 5);
    assert_eq!(f.view.read_with(&v, |r, _| r.painted_inline.iter().filter(|(_, a)| a.line == 5).count()), 1);
    choose_menu_command(&mut v, &f, CommandId::CollapseAll); draw(&mut v);
    assert!(f.view.read_with(&v, |r, _| r.painted_inline.iter().all(|(_, a)| a.line != 5)), "folded source does not leave an unrelated suffix");
    choose_menu_command(&mut v, &f, CommandId::ExpandAll);
    f.view.update(&mut v, |r, cx| { r.dispatch(Command::SetStickyScroll(false), cx); r.dispatch(Command::Scroll(0), cx); });
    v.simulate_resize(size(px(220.0), px(180.0))); v.simulate_keystrokes("f8");
    let _ = settle_native_literal(cx, &f, &mut v, "[1"); draw(&mut v);
    assert!(f.view.read_with(&v, |r, _| !r.painted_problem_bounds.borrow().is_empty()), "underlines remain when a narrow row has no suffix space");
    assert!(f.view.read_with(&v, |r, _| r.painted_inline.iter().all(|(_, a)| a.x + a.width <= f32::from(r.panes[0].text.size.width) - 3.9)));
    assert_eq!(std::fs::read(f.dir.path().join("PgWireAuthenticator.java")).unwrap(), JAVA.as_bytes());
}

#[gpui::test]
fn gui_inline_eof_problems_paint_an_anchor_on_empty_source_rows(cx: &mut TestAppContext) {
    for source in ["", "<root>\n"] {
        let f = fixture(cx, Some(("empty.xml", source.as_bytes()))); ready(cx, &f);
        let line = source.matches('\n').count();
        let mut v = visual(cx, &f); v.simulate_resize(size(px(600.0), px(220.0)));
        v.simulate_keystrokes("f8"); draw_inline(cx, &f, &mut v, line);
        f.view.read_with(&v, |r, _| {
            assert!(r.painted_inline.iter().any(|(_, a)| a.line == line && a.text.starts_with("Error:")));
            assert!(r.painted_problem_bounds.borrow().iter().any(|(_, b, c)| f32::from(b.size.width) >= 2.0 && *c == r.session.state().palette().error));
            assert_eq!(r.max_hscroll(), 0.0, "the message does not extend source scrolling");
        });
        v.simulate_resize(size(px(180.0), px(150.0))); v.simulate_keystrokes("f8"); draw_inline(cx, &f, &mut v, line);
        assert!(f.view.read_with(&v, |r, _| r.painted_inline.iter().all(|(_, a)| a.x + a.width <= f32::from(r.panes[0].text.size.width) - 3.9)));
        assert_eq!(std::fs::read(f.dir.path().join("empty.xml")).unwrap(), source.as_bytes());
    }
}

#[gpui::test]
#[ignore = "read-only exact-file GUI probe: LENZ_TEST_JAVA_SOURCE must name an owned fixture copy"]
fn gui_inline_full_java_file_underlines_array_literal_with_soft_wrap(cx: &mut TestAppContext) {
    let path = std::path::PathBuf::from(std::env::var_os("LENZ_TEST_JAVA_SOURCE").expect("owned fixture copy"));
    let bytes = std::fs::read(&path).unwrap(); let source = std::str::from_utf8(&bytes).unwrap();
    let line = source[..source.find("[1,2,3]").unwrap()].matches('\n').count();
    let f = fixture(cx, Some(("PgWireAuthenticator.java", &bytes))); ready(cx, &f);
    f.view.update(cx, |r, cx| r.dispatch(Command::SetSoftWrap(true), cx));
    let mut v = visual(cx, &f); v.simulate_resize(size(px(1000.0), px(600.0)));
    v.simulate_keystrokes("f8"); let (point, _) = settle_native_literal(cx, &f, &mut v, "[1,2,3]");
    draw_inline(cx, &f, &mut v, line); assert_array_painted(&f, &mut v, point, line);
    let evidence = f.view.read_with(&v, |r, _| {
        let state = r.session.state(); let tab = state.active().unwrap();
        let base = state.view(tab.id()).unwrap(); let shown = state.presentation_view(tab.id()).unwrap();
        serde_json::json!({"scope":"GPUI TestPlatform; no native display claim", "source_line":line + 1,
            "current_version":base.snapshot().key().version, "presentation_version":shown.snapshot().key().version,
            "same_source_metadata":Arc::ptr_eq(base.snapshot().source_meta(), shown.snapshot().source_meta()),
            "diagnostic_ranges":state.diagnostics(tab.id()).unwrap().items.iter().map(|d| (d.range.start,d.range.end)).collect::<Vec<_>>(),
            "expected_glyph_x":f32::from(point.x), "expected_glyph_mid_y":f32::from(point.y),
            "actual_underlines":r.painted_problem_bounds.borrow().iter().map(|(_, b,c)| (f32::from(b.origin.x),f32::from(b.origin.y),f32::from(b.size.width),*c)).collect::<Vec<_>>(),
            "inline":r.inline_problem_labels()})
    });
    eprintln!("GUI_JAVA_DIAGNOSTICS {evidence}");
    assert_eq!(std::fs::read(path).unwrap(), bytes);
    assert_eq!(std::fs::read(f.dir.path().join("PgWireAuthenticator.java")).unwrap(), bytes);
}
