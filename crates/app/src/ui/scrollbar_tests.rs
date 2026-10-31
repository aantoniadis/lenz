//! The scrollbar at each Diff pane's right edge, through the real pointer path: marks, thumb drag, track paging,
//! mark clicks, the merge panes and the Markdown preview.
use super::*;
use crate::scrollbar::{Mark, MarkKind, Scrollbar};

/// 300 lines: line 50 modified, line 150 deleted, a line added after line 250.
fn two_way(cx: &mut TestAppContext) -> Fixture {
    let left: String = (0..300).map(|i| format!("line {i}\n")).collect();
    let right: String = (0..300).filter(|&i| i != 150).map(|i| match i {
        50 => "changed 50\n".to_string(),
        250 => "line 250\nadded\n".to_string(),
        _ => format!("line {i}\n"),
    }).collect();
    let f = fixture(cx, &left, &right);
    sized(cx, &f);
    f
}
fn sized(cx: &mut TestAppContext, f: &Fixture) {
    cx.simulate_window_resize(f.window.into(), size(px(1200.0), px(600.0)));
    cx.executor().run_until_parked();
    render(cx, f);
}
fn render(cx: &mut TestAppContext, f: &Fixture) { f.window.update(cx, |v, w, cx| { let _ = v.render(w, cx); }).unwrap(); }
type Painted = Vec<super::super::scrollbars::PaintedScrollbar>;
fn painted(cx: &mut TestAppContext, f: &Fixture) -> Painted {
    render(cx, f);
    f.viewer.read_with(cx, |v, _| v.painted_scrollbars.clone())
}
fn strip(cx: &mut TestAppContext, f: &Fixture, key: &str, n: usize) -> super::super::scrollbars::Strip {
    f.window.update(cx, |v, w, _| v.scrollbar_strips(w).into_iter().filter(|s| s.key == key).nth(n).unwrap()).unwrap()
}
fn mouse(cx: &mut TestAppContext, f: &Fixture, x: f32, y: f32, phase: u8) {
    let position = point(px(x), px(y));
    f.window.update(cx, |v, w, cx| match phase {
        0 => v.on_mouse_down(&MouseDownEvent { button: MouseButton::Left, position, modifiers: Default::default(), click_count: 1, first_mouse: false }, w, cx),
        1 => v.on_mouse_move(&MouseMoveEvent { position, pressed_button: Some(MouseButton::Left), modifiers: Default::default() }, w, cx),
        _ => v.on_mouse_up(&MouseUpEvent { button: MouseButton::Left, position, modifiers: Default::default(), click_count: 1 }, w, cx),
    }).unwrap();
    cx.executor().run_until_parked();
}
fn press(cx: &mut TestAppContext, f: &Fixture, x: f32, y: f32) { mouse(cx, f, x, y, 0); mouse(cx, f, x, y, 2); render(cx, f); }
fn drag(cx: &mut TestAppContext, f: &Fixture, x: f32, from: f32, to: f32) {
    mouse(cx, f, x, from, 0);
    mouse(cx, f, x, (from + to) / 2.0, 1);
    mouse(cx, f, x, to, 1);
    mouse(cx, f, x, to, 2);
    render(cx, f);
}
/// The window y over the cell holding a mark that `want` accepts, as the model places it, and that mark.
fn mark_y(s: &super::super::scrollbars::Strip, bar: Scrollbar, marks: &[Mark], want: impl Fn(&Mark) -> bool) -> (f32, Mark) {
    let cells = (s.h / 3.0).floor() as usize;
    let placed = crate::scrollbar::place(marks, bar.total, cells);
    let cell = placed.iter().rposition(|p| p.is_some_and(|i| want(&marks[i]))).expect("a placed mark");
    (s.y + (cell as f32 + 0.5) * s.h / cells as f32, marks[placed[cell].unwrap()])
}
fn top(cx: &mut TestAppContext, f: &Fixture, side: Side) -> f64 { f.viewer.read_with(cx, |v, _| v.comparison.view.scrollbar(side).top) }

#[gpui::test]
fn gui_scrollbars_mark_change_blocks_and_find_hits(cx: &mut TestAppContext) {
    use MarkKind::*;
    let f = two_way(cx);
    let p = painted(cx, &f);
    assert_eq!(p.iter().map(|b| b.0).collect::<Vec<_>>(), ["left", "right"]);
    for (key, thumb, kinds) in &p {
        assert!(thumb.is_some(), "{key}: 300 lines overflow the pane");
        for kind in [Added, Deleted, Modified] { assert!(kinds.contains(&kind), "{key} {kinds:?}"); }
        assert!(!kinds.contains(&Current), "{key}: nothing is current yet");
    }
    f.viewer.update(cx, |v, cx| v.comparison_command(Command::NextChange, cx));
    assert!(painted(cx, &f).iter().all(|b| b.2.contains(&Current)), "Next Change makes a block current on both Sides");
    key(cx, &f, "f", true, false, false);
    for c in ["2", "9"] { key(cx, &f, c, false, false, false); }
    let side = f.viewer.read_with(cx, |v, _| v.comparison.find.as_ref().unwrap().side());
    let p = painted(cx, &f);
    let (shown, other) = if side == Side::Left { (&p[0], &p[1]) } else { (&p[1], &p[0]) };
    assert!(shown.2.contains(&Find) && shown.2.contains(&FindCurrent), "{:?}", shown.2);
    assert!(!other.2.contains(&Find), "find hits mark only their Side: {:?}", other.2);
    // Unified: one pane, one strip, with both Sides' blocks and the find hits
    f.viewer.update(cx, |v, _| v.comparison.view.set_unified(true));
    let p = painted(cx, &f);
    assert_eq!(p.iter().map(|b| b.0).collect::<Vec<_>>(), ["unified"]);
    for kind in [Current, Find, FindCurrent] { assert!(p[0].2.contains(&kind), "{kind:?} {:?}", p[0].2); }
}

#[gpui::test]
fn gui_scrollbar_drag_and_page_move_the_side(cx: &mut TestAppContext) {
    let f = two_way(cx);
    f.viewer.update(cx, |v, _| v.comparison.view.set_mode(ScrollMode::Independent));
    let s = strip(cx, &f, "right", 0);
    let (start, size) = painted(cx, &f)[1].1.unwrap();
    let bar = f.viewer.read_with(cx, |v, _| v.comparison.view.scrollbar(Side::Right));
    let (x, mid) = (s.x + s.w / 2.0, s.y + s.h / 2.0);
    drag(cx, &f, x, s.y + start + size / 2.0, mid);
    let want = bar.top_at(mid - s.y - size / 2.0, s.h, 16.0);
    let got = top(cx, &f, Side::Right);
    assert!((got - want).abs() <= 1.0 && got > 50.0, "{got} vs {want}");
    assert_eq!(top(cx, &f, Side::Left), 0.0, "Independent: the left Side stays");
    assert!(f.viewer.read_with(cx, |v, _| v.panes.bar.is_none()), "the release ends the drag");
    // the track above the thumb (no mark before line 50) pages up one viewport
    let bar = f.viewer.read_with(cx, |v, _| v.comparison.view.scrollbar(Side::Right));
    press(cx, &f, x, s.y + 2.0);
    assert!((top(cx, &f, Side::Right) - bar.page(false)).abs() < 0.01, "{} vs {}", top(cx, &f, Side::Right), bar.page(false));
    assert_eq!(top(cx, &f, Side::Left), 0.0);
    // the wheel over the strip scrolls that Side, as over its text
    let before = top(cx, &f, Side::Right);
    f.window.update(cx, |v, w, cx| v.on_wheel(&ScrollEvent { position: point(px(x), px(mid)), delta: ScrollDelta::Lines(point(0.0, -3.0)), ..Default::default() }, w, cx)).unwrap();
    assert!(top(cx, &f, Side::Right) > before);
    assert_eq!(top(cx, &f, Side::Left), 0.0);
}

#[gpui::test]
fn gui_scrollbar_mark_click_makes_the_change_current(cx: &mut TestAppContext) {
    let f = two_way(cx);
    let s = strip(cx, &f, "right", 0);
    let at = |cx: &mut TestAppContext, target: usize| f.viewer.read_with(cx, |v, _| mark_y(&s, v.comparison.view.scrollbar(Side::Right), &v.comparison.scrollbar_marks(Side::Right), |m| m.target == target));
    let (y, mark) = at(cx, 0);
    assert_eq!(mark.kind, MarkKind::Modified);
    f.viewer.update(cx, |v, _| v.nav_focused = true);
    press(cx, &f, s.x + s.w / 2.0, y);
    assert_eq!(f.viewer.read_with(cx, |v, _| (v.comparison.view.current_change(), v.nav_focused)), (Some(0), false), "the keys go to the diff");
    // a press on the thumb released without moving goes to the mark under it
    f.viewer.update(cx, |v, _| { v.comparison.view.set_mode(ScrollMode::Independent); v.comparison.view.scroll_to(Side::Right, 240.0); });
    let (start, size) = painted(cx, &f)[1].1.unwrap();
    let (y, mark) = at(cx, 2);
    assert_eq!(mark.kind, MarkKind::Added);
    assert!((start..start + size).contains(&(y - s.y)), "the mark is under the thumb");
    press(cx, &f, s.x + s.w / 2.0, y);
    assert_eq!(f.viewer.read_with(cx, |v, _| (v.comparison.view.current_change(), v.panes.bar.is_none())), (Some(2), true));
    // a find hit's mark makes that hit current and selects it; the find bar stays
    key(cx, &f, "f", true, false, false);
    for c in ["2", "9"] { key(cx, &f, c, false, false, false); }
    let side = f.viewer.read_with(cx, |v, _| v.comparison.find.as_ref().unwrap().side());
    let s = strip(cx, &f, if side == Side::Left { "left" } else { "right" }, 0);
    let (y, mark) = f.viewer.read_with(cx, |v, _| mark_y(&s, v.comparison.view.scrollbar(side), &v.comparison.scrollbar_marks(side), |m| m.kind == MarkKind::Find));
    let i = mark.target - crate::view::FIND_TARGET;
    let line = f.viewer.read_with(cx, |v, _| v.comparison.find.as_ref().unwrap().hits()[i].0);
    press(cx, &f, s.x + s.w / 2.0, y);
    f.viewer.read_with(cx, |v, _| {
        assert_eq!(v.comparison.find.as_ref().map(|f| f.at()), Some(Some(i)), "the hit is current, the bar open");
        let sel = v.sel.unwrap();
        assert_eq!((sel.side, sel.anchor.line), (side, line));
    });
}

/// A 200-line merge with Conflicts at lines 5 and 150.
fn merge(cx: &mut TestAppContext) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    let text = |who: &str| (0..200).map(|i| if i == 5 || i == 150 { format!("{who} {i}\n") } else { format!("line {i}\n") }).collect::<String>();
    for (name, who) in [("base", "line"), ("ours", "ours"), ("theirs", "theirs"), ("result", "ours")] { std::fs::write(d.join(name), text(who)).unwrap(); }
    let opened = merge_view::open_files(&d.join("base"), &d.join("ours"), &d.join("theirs"), &d.join("result"), d).unwrap();
    let f = fixture(cx, "old\n", "new\n");
    f.viewer.update(cx, |v, cx| v.start_merge(opened, false, cx));
    let f = Fixture { dir, ..f };
    sized(cx, &f);
    f
}

#[gpui::test]
fn gui_merge_scrollbars_mark_conflicts(cx: &mut TestAppContext) {
    let f = merge(cx);
    let p = painted(cx, &f);
    assert_eq!(p.iter().map(|b| b.0).collect::<Vec<_>>(), ["ours", "middle", "theirs"]);
    assert!(p.iter().all(|b| b.1.is_some() && b.2.contains(&MarkKind::Conflict)), "{p:?}");
    let disk = std::fs::read(f.dir.path().join("result")).unwrap();
    f.viewer.update(cx, |v, _| v.merge.as_mut().unwrap().current = None);
    // the mark of the Conflict at line 150 sits beside text far above it
    let s = strip(cx, &f, "middle", 0);
    let (y, mark) = f.viewer.read_with(cx, |v, _| { let m = v.merge.as_ref().unwrap(); mark_y(&s, m.scrollbar(1), &m.scrollbar_marks(1), |m| m.target == 1) });
    assert_eq!(mark.kind, MarkKind::Conflict);
    press(cx, &f, s.x + s.w / 2.0, y);
    assert_eq!(f.viewer.read_with(cx, |v, _| v.merge.as_ref().unwrap().current), Some(1));
    assert_eq!(std::fs::read(f.dir.path().join("result")).unwrap(), disk);
}

/// Each merge pane's thumb drags it and its track pages it, through each pane's own scroll path.
#[gpui::test]
fn gui_merge_scrollbar_drag_and_page_move_each_pane(cx: &mut TestAppContext) {
    let f = merge(cx);
    let disk = std::fs::read(f.dir.path().join("result")).unwrap();
    let bar = |cx: &mut TestAppContext, pane: usize| f.viewer.read_with(cx, |v, _| v.merge.as_ref().unwrap().scrollbar(pane));
    for (pane, key) in [(0, "ours"), (1, "middle"), (2, "theirs")] {
        f.viewer.update(cx, |v, _| v.merge.as_mut().unwrap().scroll_to(pane, 0.0));
        let s = strip(cx, &f, key, 0);
        let (start, size) = painted(cx, &f)[pane].1.unwrap();
        let before = bar(cx, pane);
        let (x, mid, grab) = (s.x + s.w / 2.0, s.y + s.h / 2.0, size / 2.0);
        drag(cx, &f, x, s.y + start + grab, mid);
        let want = before.top_at(mid - s.y - grab, s.h, 16.0);
        let got = bar(cx, pane).top;
        assert!((got - want).abs() <= 1.0 && got > 50.0, "{key}: {got} vs {want}");
        assert!(f.viewer.read_with(cx, |v, _| v.panes.bar.is_none()));
        // the track between the line-5 Conflict's mark and the thumb pages up one viewport
        let now = bar(cx, pane);
        let (start, _) = painted(cx, &f)[pane].1.unwrap();
        let y = s.y + start / 2.0;
        let on_mark = f.viewer.read_with(cx, |v, _| {
            let marks = v.merge.as_ref().unwrap().scrollbar_marks(pane);
            let placed = crate::scrollbar::place(&marks, now.total, (s.h / 3.0).floor() as usize);
            crate::scrollbar::mark_at(&placed, &marks, y - s.y, s.h)
        });
        assert_eq!(on_mark, None, "{key}: off any mark");
        press(cx, &f, x, y);
        let got = bar(cx, pane).top;
        assert!((got - now.page(false)).abs() < 0.01, "{key}: {got} vs {}", now.page(false));
    }
    assert_eq!(std::fs::read(f.dir.path().join("result")).unwrap(), disk);
}

/// Narrowing the window drops each pane's strip before its text gets under two strips wide (a preview has no
/// gutter); text and strip always fit the pane.
#[gpui::test]
fn gui_scrollbars_leave_narrow_panes_before_their_text(cx: &mut TestAppContext) {
    let s = scale_of(12.0);
    let check = |cx: &mut TestAppContext, f: &Fixture, what: &str, gutter: f32| {
        for width in [1200.0, 900.0, 700.0, 560.0, 480.0, 420.0, 360.0, 300.0, 240.0, 120.0, 1.0] {
            cx.simulate_window_resize(f.window.into(), size(px(width), px(400.0)));
            cx.executor().run_until_parked();
            render(cx, f);
            let (panes, strips) = f.window.update(cx, |v, w, _| {
                let x0 = v.diff_x();
                let avail = f32::from(w.viewport_size().width) - x0;
                let panes: Vec<(f32, f32)> = if v.merge.is_some() {
                    let m = v.merge_widths(avail);
                    vec![(x0, m[0]), (x0 + m[0] + GUTTER_W, m[1]), (x0 + m[0] + m[1] + 2.0 * GUTTER_W, m[2])]
                } else {
                    let c = v.comparison_widths(avail);
                    vec![(x0, c[0]), (x0 + c[0] + GUTTER_W, c[1])]
                };
                (panes, v.scrollbar_strips(w))
            }).unwrap();
            for (px0, pw) in panes {
                let strip = strips.iter().find(|b| b.x >= px0 && b.x + b.w <= px0 + pw + 0.01 && pw > 0.0);
                let text = pw - gutter - 16.0;
                if text < 2.0 * s {
                    assert!(strip.is_none(), "{what} at {width}: a {pw} px pane keeps no strip");
                } else {
                    let b = strip.unwrap_or_else(|| panic!("{what} at {width}: a {pw} px pane has a strip"));
                    assert!((b.x + b.w - (px0 + pw)).abs() < 0.01, "{what} at {width}: at the pane's right edge");
                }
                let text_w = if gutter > 0.0 { text_width(pw) } else { (pw - strip.map_or(0.0, |b| b.w)).max(0.0) };
                assert!(text_w + strip.map_or(0.0, |b| b.w) <= pw.max(0.0) + 0.01, "{what} at {width}: text and strip fit");
            }
        }
    };
    let f = two_way(cx);
    check(cx, &f, "two-way", num_w());
    f.viewer.update(cx, |v, _| v.comparison.view.set_unified(true));
    check(cx, &f, "unified", num_w());
    let f = merge(cx);
    check(cx, &f, "merge", num_w());
    let body: String = (0..40).map(|i| format!("Paragraph {i}.\n\n")).collect();
    let f = fixture_named(cx, &format!("# Old\n\n{body}"), &format!("# New\n\n{body}"), "md");
    sized(cx, &f);
    run(cx, &f, crate::catalog::Action::Comparison(Command::Preview(crate::markdown::MarkdownLayout::SideBySide)));
    markdown_ready(cx, &f);
    check(cx, &f, "preview", 0.0);
}

#[gpui::test]
fn gui_markdown_preview_scrolls_from_its_scrollbar(cx: &mut TestAppContext) {
    let body: String = (0..120).map(|i| format!("Paragraph {i}.\n\n")).collect();
    let f = fixture_named(cx, &format!("# Old\n\n{body}"), &format!("# New\n\n{body}"), "md");
    sized(cx, &f);
    for (layout, n) in [(crate::markdown::MarkdownLayout::Inline, 1), (crate::markdown::MarkdownLayout::SideBySide, 2)] {
        run(cx, &f, crate::catalog::Action::Comparison(Command::Preview(layout)));
        markdown_ready(cx, &f);
        let previews: Painted = painted(cx, &f).into_iter().filter(|b| b.0 == "preview").collect();
        assert_eq!(previews.len(), n, "{layout:?} {previews:?}");
        assert!(previews.iter().all(|b| b.1.is_some() && b.2.is_empty()), "{layout:?}: a thumb, no marks: {previews:?}");
        let s = strip(cx, &f, "preview", n - 1);
        let (start, size) = previews[n - 1].1.unwrap();
        // toward whichever end is farther
        let to = if start > (s.h - size) / 2.0 { s.y + size / 2.0 } else { s.y + s.h - size / 2.0 };
        drag(cx, &f, s.x + s.w / 2.0, s.y + start + size / 2.0, to);
        let after = painted(cx, &f).into_iter().filter(|b| b.0 == "preview").nth(n - 1).unwrap().1.unwrap().0;
        assert!((after - start).abs() > size, "{layout:?}: the preview scrolled: {start} -> {after}");
    }
}

/// A page from the track disarms "press again for the next file", as any scroll does; the terminal asserts the same
/// (crates/tui/src/scrollbar.rs `strip_page_disarms_press_again_as_in_the_window`).
#[gpui::test]
fn gui_scrollbar_page_disarms_press_again(cx: &mut TestAppContext) {
    let f = two_way(cx);
    let s = strip(cx, &f, "right", 0);
    f.viewer.update(cx, |v, _| {
        v.comparison.go_to_change(2);
        assert!(!v.comparison.press_again.at_edge(true) && v.comparison.press_again.at_edge(true), "an edge press arms, the next crosses");
        v.comparison.press_again.at_edge(true);
    });
    let top = top(cx, &f, Side::Right);
    let (start, size) = painted(cx, &f)[1].1.unwrap();
    assert!(start + size < s.h - 6.0, "the track's end is below the thumb");
    press(cx, &f, s.x + s.w / 2.0, s.y + s.h - 1.0);
    assert!(f.viewer.read_with(cx, |v, _| v.comparison.view.scrollbar(Side::Right).top) > top, "a page down");
    assert!(f.viewer.update(cx, |v, _| !v.comparison.press_again.at_edge(true)), "the page disarmed it");
}
