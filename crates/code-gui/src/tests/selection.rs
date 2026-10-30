use super::*;

#[gpui::test]
fn selected_empty_lines_show_their_line_endings_without_changing_text(cx: &mut TestAppContext) {
    for original in ["\n\nend\n", "\r\n\r\nend\r\n"] {
        let f = fixture(cx, Some(("selection.txt", original.as_bytes())));
        settle(cx, &f, |r| r.session.state().current_source().is_some());
        let mut v = visual(cx, &f);
        f.view.update(&mut v, |r, _| r.enable_draft_input_for_test());
        draw(&mut v);
        v.simulate_keystrokes("ctrl-home shift-down shift-down");
        assert_eq!(f.view.read_with(&v, |r, _| copied_selection(r.session.state())), Some(original[..original.find('e').unwrap()].to_owned()));
        for id in ["darcula", "light"] {
            f.view.update(&mut v, |r, cx| r.dispatch(Command::Theme(ThemeCommand::Select(ThemeSelection::from_id(id).unwrap())), cx));
            for width in [900.0, 240.0] {
                v.simulate_resize(size(px(width), px(300.0))); show(&mut v);
                f.view.read_with(&v, |r, _| {
                    let rows = r.visible_rows();
                    for row in &rows[..2] {
                        assert_eq!(row.text, " ", "the selected newline occupies one display cell");
                        assert_eq!(row.selected, Some(0..1));
                    }
                    assert_eq!(rows[2].text, "end");
                    assert_eq!(rows[2].selected, None, "the exclusive endpoint does not select the next line");
                    assert_eq!(rows[3].text, "", "the final empty line has no newline to select");
                    assert_eq!(rows[3].selected, None);
                });
            }
        }
        v.simulate_keystrokes("right shift-right"); show(&mut v);
        f.view.read_with(&v, |r, _| {
            let rows = r.visible_rows();
            assert_eq!(rows[2].text, "end");
            assert_eq!(rows[2].selected, Some(0..1), "ordinary selection keeps its glyph range");
        });
        v.simulate_keystrokes("ctrl-home down down end shift-down"); show(&mut v);
        f.view.read_with(&v, |r, _| {
            let rows = r.visible_rows();
            assert_eq!(rows[2].text, "end ");
            assert_eq!(rows[2].selected, Some(3..4), "selecting only a newline leaves the preceding text alone");
            assert_eq!(rows[3].selected, None);
        });
        v.simulate_keystrokes("ctrl-home shift-down alt-j"); show(&mut v);
        f.view.read_with(&v, |r, _| {
            let rows = r.visible_rows();
            assert_eq!(rows[0].extra, vec![(0..1, None)], "a secondary selection also shows its newline");
            assert_eq!(rows[1].selected, Some(0..1));
        });
        v.simulate_keystrokes("ctrl-a"); show(&mut v);
        f.view.read_with(&v, |r, _| {
            let rows = r.visible_rows();
            assert_eq!(rows[2].text, "end ");
            assert_eq!(rows[2].selected, Some(0..4));
            assert_eq!(rows[3].selected, None);
            assert_eq!(copied_selection(r.session.state()).as_deref(), Some(original));
        });
        assert_eq!(std::fs::read(f.dir.path().join("selection.txt")).unwrap(), original.as_bytes());
    }
}
