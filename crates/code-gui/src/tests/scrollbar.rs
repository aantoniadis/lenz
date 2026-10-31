//! The scrollbar at each pane's right edge: thumb drag, track paging, mark clicks, previews and tables, through the
//! real pointer path.
use super::*;
use lenz_content::scrollbar::MarkKind;

fn lines(n: usize, needles: &[usize]) -> String {
    (0..n).map(|i| if needles.contains(&i) { format!("needle {i}\n") } else { format!("line {i}\n") }).collect()
}

fn opened(cx: &mut TestAppContext, text: &str, w: f32, h: f32) -> (Fixture, VisualTestContext) {
    let f = fixture(cx, Some(("long.txt", text.as_bytes())));
    settle(cx, &f, |v| v.session().state().current_source().is_some());
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(w), px(h)));
    draw(&mut v);
    (f, v)
}

fn strip(v: &VisualTestContext, f: &Fixture, pane: usize) -> Bounds<Pixels> { f.view.read_with(v, |r, _| r.panes[pane].bar_strip) }
fn painted(v: &VisualTestContext, f: &Fixture) -> Vec<PaintedScrollbar> { f.view.read_with(v, |r, _| r.painted_scrollbars.clone()) }
fn top_line(v: &VisualTestContext, f: &Fixture) -> usize { f.view.read_with(v, |r, _| r.session().state().active().unwrap().top_line()) }
fn caret_line(v: &VisualTestContext, f: &Fixture) -> usize {
    f.view.read_with(v, |r, _| { let s = r.session().state(); let t = s.active().unwrap(); s.view(t.id()).unwrap().snapshot().line_of(t.selection().unwrap().head) })
}
fn click(v: &mut VisualTestContext, x: Pixels, y: f32) {
    v.simulate_click(point(x, px(y)), Modifiers::default());
    draw(v);
}
fn finding(cx: &mut TestAppContext, f: &Fixture, v: VisualTestContext) -> VisualTestContext {
    let mut v = v;
    v.simulate_keystrokes("ctrl-f n e e d l e");
    drop(v);
    settle(cx, f, find_settled);
    let mut v = visual(cx, f);
    draw(&mut v);
    v
}

/// The window y over the jumpable mark for source `line` in the first pane's strip, as the model places it.
fn mark_y(v: &VisualTestContext, f: &Fixture, line: usize) -> f32 {
    f.view.read_with(v, |r, _| {
        let s = r.panes[0].bar_strip;
        let (bar, marks) = r.session().state().scrollbar(0).unwrap();
        let len = f32::from(s.size.height);
        let cells = (len / 3.0).floor() as usize;
        let placed = lenz_content::scrollbar::place(&marks, bar.total, cells);
        let cell = placed.iter().position(|p| p.is_some_and(|i| marks[i].target == line && marks[i].kind.jumps())).unwrap();
        f32::from(s.origin.y) + (cell as f32 + 0.5) * len / cells as f32
    })
}

#[gpui::test]
fn scrollbar_thumb_drag_scrolls_and_marks_are_painted(cx: &mut TestAppContext) {
    let (f, v) = opened(cx, &lines(300, &[100, 250]), 800.0, 400.0);
    let mut v = finding(cx, &f, v);
    let painted = painted(&v, &f);
    assert_eq!(painted.len(), 1);
    let (pane, thumb, kinds) = &painted[0];
    assert_eq!(*pane, 0);
    assert!(kinds.contains(&MarkKind::Find) && kinds.contains(&MarkKind::FindCurrent), "{kinds:?}");
    let (start, size) = thumb.expect("a thumb");
    let s = strip(&v, &f, 0);
    let (y0, len) = (f32::from(s.origin.y), f32::from(s.size.height));
    let bar = f.view.read_with(&v, |r, _| r.session().state().scrollbar(0).unwrap().0);
    let x = f32::from(s.center().x);
    let mid = y0 + len / 2.0;
    drag(&mut v, (x, y0 + start + size / 2.0), (x, mid));
    // No folds: a shown row is its line.
    let want = bar.top_at(mid - y0 - size / 2.0, len, 16.0).round() as usize;
    let got = top_line(&v, &f);
    assert!(got.abs_diff(want) <= 1 && got > 50, "{got} vs {want}");
    assert_eq!(f.view.read_with(&v, |r, _| r.drag), None);
}

#[gpui::test]
fn scrollbar_track_click_pages_and_mark_click_goes_to_the_line(cx: &mut TestAppContext) {
    let (f, mut v) = opened(cx, &lines(300, &[120, 250]), 800.0, 400.0);
    let s = strip(&v, &f, 0);
    let rows = f.view.read_with(&v, |r, _| r.panes[0].rows);
    click(&mut v, s.center().x, f32::from(s.origin.y) + f32::from(s.size.height) * 0.6);
    assert_eq!(top_line(&v, &f), rows, "one page down");
    click(&mut v, s.center().x, f32::from(s.origin.y) + 2.0);
    assert_eq!(top_line(&v, &f), 0, "and back up");
    let mut v = finding(cx, &f, v);
    let before = caret_line(&v, &f);
    assert_ne!(before, 250);
    let y = mark_y(&v, &f, 250);
    click(&mut v, s.center().x, y);
    assert_eq!(caret_line(&v, &f), 250, "the mark's line");
    v.simulate_keystrokes("alt-left");
    assert_eq!(caret_line(&v, &f), before, "Back returns");
    // A click on the thumb that does not move goes to the mark under it.
    f.view.update(&mut v, |r, cx| r.dispatch(Command::Scroll(240), cx));
    draw(&mut v);
    let (y, (start, size)) = (mark_y(&v, &f, 250), painted(&v, &f)[0].1.unwrap());
    assert!((start..start + size).contains(&(y - f32::from(s.origin.y))), "the mark is under the thumb");
    click(&mut v, s.center().x, y);
    assert_eq!((caret_line(&v, &f), f.view.read_with(&v, |r, _| r.drag)), (250, None));
}

#[gpui::test]
fn split_strip_does_not_take_the_divider(cx: &mut TestAppContext) {
    let (f, mut v) = opened(cx, &lines(300, &[]), 1000.0, 600.0);
    v.simulate_keystrokes("ctrl-\\");
    draw(&mut v);
    let (s0, s1, split) = f.view.read_with(&v, |r, _| (r.panes[0].bar_strip, r.panes[1].bar_strip, r.split_at));
    assert!(s0.size.width > px(0.0) && f32::from(s0.right()) <= split, "{s0:?} {split}");
    assert!(s1.size.width > px(0.0) && f32::from(s1.right()) <= 1000.0 && f32::from(s1.left()) > split, "{s1:?}");
    let y = s0.center().y;
    v.simulate_mouse_down(point(px(split), y), MouseButton::Left, Modifiers::default());
    assert_eq!(f.view.read_with(&v, |r, _| r.drag), Some(Drag::Split));
    v.simulate_mouse_up(point(px(split), y), MouseButton::Left, Modifiers::default());
    // The first pane's strip, left of the divider's reach, is the scrollbar's; it focuses that pane.
    v.simulate_mouse_down(point(s0.left() + px(2.0), y), MouseButton::Left, Modifiers::default());
    assert!(f.view.read_with(&v, |r, _| r.drag != Some(Drag::Split) && r.session().state().focused_pane() == 0));
    v.simulate_mouse_up(point(s0.left() + px(2.0), y), MouseButton::Left, Modifiers::default());
}

#[gpui::test]
fn preview_and_csv_get_a_thumb_without_marks(cx: &mut TestAppContext) {
    let md = format!("# Top\n\n{}", (0..120).map(|i| format!("Paragraph {i}.\n\n")).collect::<String>());
    let f = fixture(cx, Some(("doc.md", md.as_bytes())));
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
    let p = painted(&v, &f);
    assert!(p.len() == 1 && p[0].1.is_some() && p[0].2.is_empty(), "{p:?}");
    let s = strip(&v, &f, 0);
    click(&mut v, s.center().x, f32::from(s.origin.y) + f32::from(s.size.height) * 0.8);
    assert!(f.view.read_with(&v, |r, _| r.markdown[0].as_ref().unwrap().top) > 0.0, "a page down the preview");

    let rows: String = std::iter::once("a,b\n".to_string()).chain((0..200).map(|i| format!("{i},x\n"))).collect();
    let f = fixture(cx, Some(("data.csv", rows.as_bytes())));
    settle(cx, &f, active_is("data.csv"));
    let mut v = visual(cx, &f);
    v.simulate_resize(size(px(800.0), px(400.0)));
    draw(&mut v);
    click_preview(&mut v, &f, 0);
    drop(v);
    let ready = |r: &ReaderView| r.session().state().active().and_then(|t| r.session().state().csv(t.id())).is_some_and(|v| !v.pending);
    settle(cx, &f, ready);
    let mut v = visual(cx, &f);
    show(&mut v);
    let p = painted(&v, &f);
    assert!(p.len() == 1 && p[0].1.is_some() && p[0].2.is_empty(), "{p:?}");
    let s = strip(&v, &f, 0);
    click(&mut v, s.center().x, f32::from(s.origin.y) + f32::from(s.size.height) * 0.8);
    let (top, page) = f.view.read_with(&v, |r, _| { let c = r.session().state().csv(r.session().state().active().unwrap().id()).unwrap(); (c.top, c.rows) });
    assert_eq!(top, page, "a page down the table");
}

#[gpui::test]
fn pointer_over_the_scrollbar_is_the_default_arrow(cx: &mut TestAppContext) {
    let (f, mut v) = opened(cx, &lines(300, &[]), 800.0, 400.0);
    let (s, text) = f.view.read_with(&v, |r, _| (r.panes[0].bar_strip, r.panes[0].text));
    assert!(s.size.width > px(0.0) && s.left() >= text.right(), "the text gives the strip its width");
    let pointer = |v: &mut VisualTestContext| f.view.read_with(v, |r, _| r.pointer());
    v.simulate_mouse_move(point(text.right() - px(4.0), s.center().y), None, Modifiers::default());
    assert_eq!(pointer(&mut v), Some(gpui::CursorStyle::IBeam), "over the text");
    v.simulate_mouse_move(s.center(), None, Modifiers::default());
    assert_eq!(pointer(&mut v), None, "over the scrollbar");
}

#[gpui::test]
fn scrollbar_wheel_over_it_scrolls_the_pane(cx: &mut TestAppContext) {
    let (f, mut v) = opened(cx, &lines(300, &[]), 1000.0, 400.0);
    v.simulate_keystrokes("ctrl-\\");
    draw(&mut v);
    let s = strip(&v, &f, 0);
    assert!(s.size.width > px(0.0) && f.view.read_with(&v, |r, _| r.session().state().focused_pane()) == 1);
    v.simulate_event(ScrollEvent { position: s.center(), delta: ScrollDelta::Lines(point(0.0, -2.0)), ..Default::default() });
    draw(&mut v);
    assert_eq!(f.view.read_with(&v, |r, _| r.session().state().focused_pane()), 0, "the strip's pane, as its text would");
    assert!(top_line(&v, &f) > 0);
}
