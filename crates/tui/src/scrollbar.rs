//! The scrollbar column at each Diff text pane's right edge, in cells: a Comparison's Sides (or its unified pane), the
//! three merge panes and the Markdown preview. The shared model places the thumb and marks and classifies a press;
//! this adapter paints the column and turns a press or a held thumb into a scroll or a mark's target.
use lenz::scrollbar::{self, Hit, Mark, Scrollbar};
use ratatui::{buffer::Buffer, layout::Rect};

/// The column a pane whose text is `text` columns wide gives its strip: one, unless the text would be left narrower
/// than 12 columns (the strip goes before the text does).
pub(crate) fn strip_w(text: u16) -> u16 { u16::from(text > 12) }

/// A held thumb: what it scrolls (with what was shown when it was grabbed: the drag ends once that changes), where
/// in the thumb it was grabbed, the row pressed, and whether it has left that row.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Drag<T> { pub target: T, grab: f32, row: u16, moved: bool }

/// What a press or a held thumb asks of its pane: a scrollbar top, or a mark's target.
pub(crate) enum Act { Scroll(f64), Mark(usize) }

/// A press at `row` on `strip`, as [`scrollbar::hit`] classifies a press at the cell's centre.
pub(crate) enum Press<T> { Hold(Drag<T>), Act(Act) }

fn at(strip: Rect, row: u16) -> (f32, f32) { (f32::from(row.saturating_sub(strip.y)) + 0.5, f32::from(strip.height)) }

/// A press at `row` on `strip`: the thumb is held, a mark goes to its target, the track pages toward the press.
pub(crate) fn press<T>(target: T, strip: Rect, bar: &Scrollbar, marks: &[Mark], row: u16) -> Press<T> {
    let (at, len) = at(strip, row);
    let placed = scrollbar::place(marks, bar.total, usize::from(strip.height));
    match scrollbar::hit(bar, &placed, marks, at, len, 1.0) {
        Hit::Thumb => Press::Hold(Drag { target, grab: at - bar.thumb(len, 1.0).map_or(0.0, |t| t.0), row, moved: false }),
        Hit::Mark(i) => Press::Act(Act::Mark(marks[i].target)),
        Hit::Above => Press::Act(Act::Scroll(bar.page(false))),
        Hit::Below => Press::Act(Act::Scroll(bar.page(true))),
    }
}

/// The held thumb at pointer `row`: the drag while it is held, and what to do. Once off its row it follows the
/// pointer in whole rows; released on its row it is a click on the mark under it, if any.
pub(crate) fn held<T: Copy>(drag: Drag<T>, strip: Rect, bar: &Scrollbar, marks: &[Mark], row: u16, up: bool) -> (Option<Drag<T>>, Option<Act>) {
    let (at, len) = at(strip, row);
    let moved = drag.moved || row != drag.row;
    if up {
        let placed = scrollbar::place(marks, bar.total, usize::from(strip.height));
        let mark = (!moved).then(|| scrollbar::mark_at(&placed, marks, at, len)).flatten();
        return (None, mark.map(|i| Act::Mark(marks[i].target)));
    }
    (Some(Drag { moved, ..drag }), moved.then(|| Act::Scroll(bar.top_at(at - drag.grab, len, 1.0).round())))
}

/// Paint `strip`: the track `│`, the thumb's cells in its colour, and each marked cell `━` in its mark's colour over
/// the cell's background, as Lenz Code's terminal app does. Bounded by the marks and the column's cells.
pub(crate) fn draw(buf: &mut Buffer, strip: Rect, bar: Option<&(Scrollbar, Vec<Mark>)>, p: &lenz::theme::Palette) {
    let (thumb, placed) = bar.map_or((None, Vec::new()), |(bar, marks)| (bar.thumb(f32::from(strip.height), 1.0), scrollbar::place(marks, bar.total, usize::from(strip.height))));
    let (thumb_bg, track_fg) = (scrollbar::thumb_color(p), lenz::theme::mix(p.gutter_bg, p.border, 0.5));
    let base = crate::chrome::style_with(p);
    for cell in 0..strip.height {
        let on = thumb.is_some_and(|(start, size)| (start..=start + size).contains(&(f32::from(cell) + 0.5)));
        let (symbol, fg) = match (placed.get(usize::from(cell)).copied().flatten(), bar) {
            (Some(i), Some((_, marks))) => ("━", lenz::theme::scrollbar_mark_color(marks[i].kind, p)),
            _ if on => (" ", thumb_bg),
            _ => ("│", track_fg),
        };
        let bg = if on { thumb_bg } else { scrollbar::track_color(p) };
        buf[(strip.x, strip.y + cell)].set_symbol(symbol).set_style(base.fg(crate::chrome::color(fg)).bg(crate::chrome::color(bg)));
    }
}

#[cfg(test)]
mod tests {
    use crate::app::{App, tests::{key, pair, ready, screen}};
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    use lenz::{commands::Command, scrollbar::{Mark, MarkKind, Scrollbar}, view::{ScrollMode, Side}};
    use ratatui::{Terminal, backend::TestBackend, buffer::Buffer, layout::Rect};

    /// The window's fixture (crates/app/src/ui/scrollbar_tests.rs `two_way`): 300 lines, line 50 modified, line 150
    /// deleted, a line added after line 250.
    fn two_way() -> (tempfile::TempDir, App) {
        let left: String = (0..300).map(|i| format!("line {i}\n")).collect();
        let right: String = (0..300).filter(|&i| i != 150).map(|i| match i {
            50 => "changed 50\n".to_string(),
            250 => "line 250\nadded\n".to_string(),
            _ => format!("line {i}\n"),
        }).collect();
        let (dir, mut app) = pair(&left, &right);
        // the window's fixture shows every line: no Collapsed Regions
        app.session.execute(Command::ToggleContext);
        ready(&mut app);
        for side in [Side::Left, Side::Right] { app.session.scroll_to(side, 0.0); }
        screen(&mut app, 120, 30);
        assert!(app.session.view().scrollbar(Side::Left).total >= 300.0, "every line shown");
        (dir, app)
    }
    /// The terminal's default path: 600 lines, every 40th from line 25 changed, so Collapsed Regions fold each unchanged
    /// run; lines 300 and 450 read `needle`, each inside a folded run.
    fn collapsed() -> (tempfile::TempDir, App) {
        let line = |i: usize| if i == 300 || i == 450 { format!("needle {i}\n") } else { format!("line {i}\n") };
        let left: String = (0..600).map(line).collect();
        let right: String = (0..600).map(|i| if i % 40 == 25 { format!("changed {i}\n") } else { line(i) }).collect();
        let (dir, mut app) = pair(&left, &right);
        for side in [Side::Left, Side::Right] { app.session.scroll_to(side, 0.0); }
        screen(&mut app, 120, 30);
        let total = app.session.view().scrollbar(Side::Right).total;
        assert!(total < 300.0, "Collapsed Regions are on: {total} rows");
        (dir, app)
    }
    /// A row on `side`'s track above its thumb and off any mark.
    fn track_above(app: &App, side: Side) -> u16 {
        let (s, bar, marks) = (strip(app, side), app.session.view().scrollbar(side), app.session.scrollbar_marks(side));
        let placed = lenz::scrollbar::place(&marks, bar.total, usize::from(s.height));
        let (start, _) = bar.thumb(f32::from(s.height), 1.0).unwrap();
        s.y + (0..start.floor() as u16).rev().find(|&r| lenz::scrollbar::mark_at(&placed, &marks, f32::from(r) + 0.5, f32::from(s.height)).is_none()).expect("track off any mark")
    }
    fn draw(app: &mut App, width: u16, height: u16) -> Buffer {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|f| app.render(f)).unwrap();
        terminal.backend().buffer().clone()
    }
    fn mouse(app: &mut App, kind: MouseEventKind, x: u16, y: u16) {
        app.handle(Event::Mouse(MouseEvent { kind, column: x, row: y, modifiers: KeyModifiers::NONE }));
    }
    fn press(app: &mut App, x: u16, y: u16) {
        mouse(app, MouseEventKind::Down(MouseButton::Left), x, y);
        mouse(app, MouseEventKind::Up(MouseButton::Left), x, y);
        screen(app, 120, 30);
    }
    fn drag(app: &mut App, x: u16, from: u16, to: u16) {
        mouse(app, MouseEventKind::Down(MouseButton::Left), x, from);
        mouse(app, MouseEventKind::Drag(MouseButton::Left), x, (from + to) / 2);
        mouse(app, MouseEventKind::Drag(MouseButton::Left), x, to);
        mouse(app, MouseEventKind::Up(MouseButton::Left), x, to);
        screen(app, 120, 30);
    }
    fn typed(app: &mut App, text: &str) { for c in text.chars() { app.handle(Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))); } }
    fn strip(app: &App, side: Side) -> Rect { app.scrollbar_strips().into_iter().find(|s| s.0 == side).unwrap().1 }
    fn top(app: &App, side: Side) -> f64 { app.session.view().scrollbar(side).top }
    /// The row over the cell holding a mark that `want` accepts, as the model places it, and that mark.
    fn mark_row(s: Rect, bar: Scrollbar, marks: &[Mark], want: impl Fn(&Mark) -> bool) -> (u16, Mark) {
        let placed = lenz::scrollbar::place(marks, bar.total, usize::from(s.height));
        let cell = placed.iter().rposition(|p| p.is_some_and(|i| want(&marks[i]))).expect("a placed mark");
        (s.y + cell as u16, marks[placed[cell].unwrap()])
    }
    fn side_mark(app: &App, side: Side, want: impl Fn(&Mark) -> bool) -> (u16, Mark) {
        mark_row(strip(app, side), app.session.view().scrollbar(side), &app.session.scrollbar_marks(side), want)
    }
    /// The thumb's middle row on `s` and where it is grabbed there.
    fn thumb_row(s: Rect, bar: Scrollbar) -> (u16, f32) {
        let (start, size) = bar.thumb(f32::from(s.height), 1.0).expect("a thumb");
        let row = (start + size / 2.0).floor() as u16;
        (s.y + row, f32::from(row) + 0.5 - start)
    }
    fn marked(buf: &Buffer, s: Rect, kind: MarkKind, app: &App) -> bool {
        let fg = crate::chrome::color(lenz::theme::scrollbar_mark_color(kind, &app.palette));
        (s.y..s.bottom()).any(|y| buf[(s.x, y)].symbol() == "━" && buf[(s.x, y)].fg == fg)
    }
    fn thumbed(buf: &Buffer, s: Rect, app: &App) -> bool {
        let bg = crate::chrome::color(lenz::scrollbar::thumb_color(&app.palette));
        (s.y..s.bottom()).any(|y| buf[(s.x, y)].bg == bg)
    }

    #[test]
    fn scrollbar_columns_show_blocks_and_find_hits() {
        use MarkKind::*;
        let (_dir, mut app) = two_way();
        let buf = draw(&mut app, 120, 30);
        let strips = app.scrollbar_strips();
        assert_eq!(strips.iter().map(|s| s.0).collect::<Vec<_>>(), [Side::Left, Side::Right]);
        let (area, left) = (app.diff_area, app.left_width());
        assert_eq!(strips[0].1.x, area.x + left - 1, "the left Side's last text column, before the divider");
        assert_eq!(strips[1].1.right(), area.right(), "the right Side's at the area's right edge");
        assert_eq!(buf[(area.x + left, strips[0].1.y)].symbol(), "│", "the divider keeps its column");
        let p = &app.palette;
        let track = crate::chrome::color(lenz::scrollbar::track_color(p));
        for &(side, s) in &strips {
            assert_eq!(app.source_rect(side).right(), s.x, "{side:?}: text stops before the strip");
            assert_eq!(s.height, app.session.view().viewport_rows() as u16, "{side:?}: as tall as the viewport");
            assert!(thumbed(&buf, s, &app), "{side:?}: 300 lines overflow the pane");
            assert!((s.y..s.bottom()).any(|y| buf[(s.x, y)].symbol() == "│" && buf[(s.x, y)].bg == track), "{side:?}: track cells");
            for kind in [Added, Deleted, Modified] { assert!(marked(&buf, s, kind, &app), "{side:?}: {kind:?}"); }
            assert!(!marked(&buf, s, Current, &app), "{side:?}: nothing is current yet");
        }
        app.handle(Event::Key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE)));
        let buf = draw(&mut app, 120, 30);
        assert!(strips.iter().all(|&(_, s)| marked(&buf, s, Current, &app)), "Next Change makes a block current on both Sides");
        typed(&mut app, "/29");
        let side = app.session.find().unwrap().side();
        let buf = draw(&mut app, 120, 30);
        let (shown, other) = if side == Side::Left { (strips[0].1, strips[1].1) } else { (strips[1].1, strips[0].1) };
        assert!(marked(&buf, shown, Find, &app) && marked(&buf, shown, FindCurrent, &app), "find hits on their Side");
        assert!(!marked(&buf, other, Find, &app), "find hits mark only their Side");
        // Unified: one pane, one strip, with both Sides' blocks and the find hits
        app.session.execute(Command::SetDiffLayout(lenz::commands::DiffLayout::Unified));
        let buf = draw(&mut app, 120, 30);
        let strips = app.scrollbar_strips();
        assert_eq!(strips.iter().map(|s| s.0).collect::<Vec<_>>(), [Side::Right]);
        assert_eq!(strips[0].1.right(), area.right());
        for kind in [Current, Find, FindCurrent] { assert!(marked(&buf, strips[0].1, kind, &app), "unified {kind:?}"); }
    }

    #[test]
    fn narrow_terminal_keeps_text_and_drops_strips() {
        let (_dir, mut app) = two_way();
        for (w, h) in [(120, 30), (69, 10), (40, 6), (24, 8), (20, 8), (12, 4), (1, 1)] {
            app.handle(Event::Resize(w, h));
            draw(&mut app, w, h);
            let strips = app.scrollbar_strips();
            assert!(strips.len() <= if w < 70 { 1 } else { 2 }, "{w}x{h}: one Side shown");
            for (side, s) in &strips {
                let text = app.source_rect(*side);
                assert!(text.width >= 12 && text.right() == s.x && s.width == 1 && s.height > 0, "{w}x{h}: text {text:?} beside strip {s:?}");
                assert!(s.right() <= app.diff_area.right());
            }
            let body = app.diff_area.height.saturating_sub(4 + app.applied_panel_height());
            let side = if w < 70 { app.focus } else { Side::Right };
            let text = app.source_rect(side).width;
            if body > 0 && text >= 12 { assert!(strips.iter().any(|s| s.0 == side), "{w}x{h}: {text} columns of text keep a strip"); }
            if text < 12 { assert!(strips.is_empty(), "{w}x{h}: the strip goes before the text"); }
            // a press where the strip would be never panics
            let (x, y) = (app.diff_area.right().saturating_sub(1), app.diff_area.y + 2);
            mouse(&mut app, MouseEventKind::Down(MouseButton::Left), x, y);
            mouse(&mut app, MouseEventKind::Up(MouseButton::Left), x, y);
        }
        app.handle(Event::Resize(40, 6));
        assert!(draw(&mut app, 40, 6).content.iter().any(|c| c.symbol() == "l"), "40x6 still shows text");
    }

    #[test]
    fn scrollbar_drag_page_mark_in_each_scroll_mode() {
        // every line shown (the window's fixture), then the terminal's default Collapsed Regions
        for (fixture, mode) in [two_way as fn() -> (tempfile::TempDir, App), collapsed].into_iter().flat_map(|f| [ScrollMode::Shared, ScrollMode::Aligned, ScrollMode::Padded, ScrollMode::Independent].map(|m| (f, m))) {
            let (_dir, mut app) = fixture();
            app.session.execute(Command::SetScroll(mode));
            screen(&mut app, 120, 30);
            for side in [Side::Left, Side::Right] {
                app.session.scroll_to(side, 0.0);
                app.session.scroll_to(Side::Left, 0.0);
                screen(&mut app, 120, 30);
                let s = strip(&app, side);
                let bar = app.session.view().scrollbar(side);
                let (from, grab) = thumb_row(s, bar);
                let to = s.y + s.height / 2;
                drag(&mut app, s.x, from, to);
                let want = bar.top_at(f32::from(to - s.y) + 0.5 - grab, f32::from(s.height), 1.0).round();
                let got = top(&app, side);
                assert!((got - want).abs() <= 1.0 && got >= bar.visible, "{mode:?} {side:?}: {got} vs {want}");
                if mode == ScrollMode::Independent && side == Side::Right { assert_eq!(top(&app, Side::Left), 0.0, "Independent: the left Side stays"); }
                // the track above the thumb, off any mark, pages up one viewport
                let bar = app.session.view().scrollbar(side);
                let y = track_above(&app, side);
                press(&mut app, s.x, y);
                assert!((top(&app, side) - bar.page(false)).abs() < 0.01, "{mode:?} {side:?}: a page up: {} vs {}", top(&app, side), bar.page(false));
                // a block's mark makes it current
                let target = side as usize + 1;
                let (y, _) = side_mark(&app, side, |m| m.target == target);
                press(&mut app, s.x, y);
                assert_eq!(app.session.view().current_change(), Some(target), "{mode:?} {side:?}: the mark's block");
            }
        }
    }

    /// The parity scenario on the terminal's default path, Collapsed Regions on: a drag lands within a row of `top_at`,
    /// the track pages, a block's mark makes it current, and a find hit's mark inside a folded run makes that hit
    /// current and selected, reveals its line, and leaves the find prompt open.
    #[test]
    fn collapsed_regions_keep_drag_page_mark_and_reveal_find_hits() {
        let (_dir, mut app) = collapsed();
        typed(&mut app, "/needle");
        screen(&mut app, 120, 30);
        let find = app.session.find().unwrap();
        let (side, at) = (find.side(), find.at());
        let i = find.hits().iter().position(|h| h.0 == 450).unwrap();
        assert_ne!(at, Some(i), "the search shows line 300 first");
        assert!(!app.session.view().line_visible(side, 450), "line 450 is folded away");
        let (y, mark) = side_mark(&app, side, |m| m.target == lenz::view::FIND_TARGET + i);
        assert_eq!(mark.kind, MarkKind::Find);
        let x = strip(&app, side).x;
        press(&mut app, x, y);
        assert_eq!(app.session.find().map(|f| f.at()), Some(Some(i)), "the hit is current");
        assert!(app.session.view().line_visible(side, 450), "its folded run opens to show it");
        let sel = app.selection.unwrap();
        assert_eq!((sel.side, sel.anchor.line), (side, 450));
        assert_eq!(app.prompt, Some(crate::app::Prompt::Search), "the find prompt stays");
        key(&mut app, KeyCode::Esc);
        // drag, page and a block's mark, as `diff_scrollbars_replay_the_same_outcomes_as_the_window` on all lines
        for side in [Side::Left, Side::Right] { app.session.scroll_to(side, 0.0); }
        screen(&mut app, 120, 30);
        assert!(app.session.view().scrollbar(Side::Right).total < 300.0, "still collapsed");
        let s = strip(&app, Side::Right);
        let bar = app.session.view().scrollbar(Side::Right);
        let (from, grab) = thumb_row(s, bar);
        let mid = s.y + s.height / 2;
        drag(&mut app, s.x, from, mid);
        let want = bar.top_at(f32::from(mid - s.y) + 0.5 - grab, f32::from(s.height), 1.0);
        let got = top(&app, Side::Right);
        assert!((got - want).abs() <= 1.0 && got >= bar.visible, "{got} vs {want}");
        let bar = app.session.view().scrollbar(Side::Right);
        let y = track_above(&app, Side::Right);
        press(&mut app, s.x, y);
        assert!((top(&app, Side::Right) - bar.page(false)).abs() < 0.01, "{} vs {}", top(&app, Side::Right), bar.page(false));
        let (y, _) = side_mark(&app, Side::Right, |m| m.target == 9);
        press(&mut app, s.x, y);
        assert_eq!(app.session.view().current_change(), Some(9));
    }

    /// A page from the track disarms "press again for the next file" as any scroll does, in the terminal as in the
    /// window (`gui_scrollbar_page_disarms_press_again`): `n` at the last change then arms again rather than opening the
    /// next file.
    #[test]
    fn strip_page_disarms_press_again_as_in_the_window() {
        let dir = tempfile::tempdir().unwrap();
        let left: String = (0..300).map(|i| format!("line {i}\n")).collect();
        let right = left.replacen("line 250\n", "changed 250\n", 1);
        for (side, text) in [("left", &left), ("right", &right)] {
            std::fs::create_dir(dir.path().join(side)).unwrap();
            std::fs::write(dir.path().join(side).join("a.txt"), text).unwrap();
            std::fs::write(dir.path().join(side).join("b.txt"), format!("{side}\n")).unwrap();
        }
        let args = ["--no-index".to_string(), "left".into(), "right".into()];
        let mut app = App::open(lenz::cli::parse(&args, dir.path()).unwrap(), dir.path().into());
        ready(&mut app);
        app.session.execute(Command::ToggleContext);
        ready(&mut app);
        screen(&mut app, 120, 30);
        assert_eq!(app.session.current(), 0);
        app.session.go_to_change(0);
        key(&mut app, KeyCode::Char('n'));
        assert!(app.session.notice().is_some_and(|n| n.contains("again")), "the last change arms: {:?}", app.session.notice());
        screen(&mut app, 120, 30);
        // the track's last cell, below the thumb and off the mark, pages down
        let s = strip(&app, Side::Right);
        let bar = app.session.view().scrollbar(Side::Right);
        let (start, size) = bar.thumb(f32::from(s.height), 1.0).unwrap();
        assert!(start + size < f32::from(s.height) - 1.0 && side_mark(&app, Side::Right, |_| true).0 < s.bottom() - 1);
        press(&mut app, s.x, s.bottom() - 1);
        assert!(top(&app, Side::Right) > bar.top, "a page down");
        key(&mut app, KeyCode::Char('n'));
        ready(&mut app);
        assert_eq!(app.session.current(), 0, "disarmed: n arms again instead of opening b.txt");
        key(&mut app, KeyCode::Char('n'));
        ready(&mut app);
        assert_eq!(app.session.current(), 1, "armed, n opens the next file");
    }

    #[test]
    fn preview_scrollbar_has_no_marks() {
        let body: String = (0..120).map(|i| format!("Paragraph {i}.\n\n")).collect();
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("left.md"), format!("# Old\n\n{body}")).unwrap();
        std::fs::write(dir.path().join("right.md"), format!("# New\n\n{body}")).unwrap();
        let req = lenz::cli::parse(&["--no-index".into(), "left.md".into(), "right.md".into()], dir.path()).unwrap();
        let mut app = App::open(req, dir.path().into());
        ready(&mut app);
        for (layout, n) in [(lenz::markdown::MarkdownLayout::Inline, 1), (lenz::markdown::MarkdownLayout::SideBySide, 2)] {
            app.run_action(lenz::catalog::Action::Comparison(Command::Preview(layout)));
            ready(&mut app);
            screen(&mut app, 120, 30);
            let buf = draw(&mut app, 120, 30);
            let strips = app.scrollbar_strips();
            assert_eq!(strips.len(), n, "{layout:?}");
            for &(side, s) in &strips {
                assert!(thumbed(&buf, s, &app), "{layout:?} {side:?}: a thumb");
                assert!((s.y..s.bottom()).all(|y| buf[(s.x, y)].symbol() != "━"), "{layout:?} {side:?}: no marks");
                assert!(app.markdown_scrollbar(side).is_some());
            }
            let (side, s) = strips[n - 1];
            let bar = app.markdown_scrollbar(side).unwrap();
            let (from, _) = thumb_row(s, bar);
            // toward whichever end is farther
            let (start, size) = bar.thumb(f32::from(s.height), 1.0).unwrap();
            drag(&mut app, s.x, from, if start > (f32::from(s.height) - size) / 2.0 { s.y } else { s.bottom() - 1 });
            let after = app.markdown_scrollbar(side).unwrap().top;
            assert!((after - bar.top).abs() > bar.visible, "{layout:?}: the preview scrolled: {} -> {after}", bar.top);
            // the wheel over the strip scrolls that pane, as over its text
            mouse(&mut app, if after > 0.0 { MouseEventKind::ScrollUp } else { MouseEventKind::ScrollDown }, s.x, s.y);
            assert_ne!(app.markdown_scrollbar(side).unwrap().top, after, "{layout:?}: the wheel over the strip");
        }
    }

    /// The Diff window's scrollbar scenarios replayed through the terminal's pointer path, asserting the same core
    /// outcomes on the same fixture: `gui_scrollbar_drag_and_page_move_the_side` (a drag of the right Side's thumb in
    /// Independent lands within a row of `Scrollbar::top_at`, the left Side stays; the track pages to `page(..)`; the
    /// wheel over the strip scrolls that Side) and `gui_scrollbar_mark_click_makes_the_change_current` (a mark makes
    /// its block current and takes the keys back to the diff; a still thumb press goes to the mark under it; a find
    /// hit's mark makes that hit current and selects it, the find prompt open) in crates/app/src/ui/scrollbar_tests.rs.
    #[test]
    fn diff_scrollbars_replay_the_same_outcomes_as_the_window() {
        let (dir, mut app) = two_way();
        app.session.execute(Command::SetScroll(ScrollMode::Independent));
        screen(&mut app, 120, 30);
        let s = strip(&app, Side::Right);
        let bar = app.session.view().scrollbar(Side::Right);
        let (from, grab) = thumb_row(s, bar);
        let mid = s.y + s.height / 2;
        drag(&mut app, s.x, from, mid);
        let want = bar.top_at(f32::from(mid - s.y) + 0.5 - grab, f32::from(s.height), 1.0);
        let got = top(&app, Side::Right);
        assert!((got - want).abs() <= 1.0 && got > 50.0, "{got} vs {want}");
        assert_eq!(top(&app, Side::Left), 0.0, "Independent: the left Side stays");
        assert!(app.scrollbar_drag.is_none(), "the release ends the drag");
        let bar = app.session.view().scrollbar(Side::Right);
        press(&mut app, s.x, s.y);
        assert!((top(&app, Side::Right) - bar.page(false)).abs() < 0.01, "{} vs {}", top(&app, Side::Right), bar.page(false));
        assert_eq!(top(&app, Side::Left), 0.0);
        let before = top(&app, Side::Right);
        mouse(&mut app, MouseEventKind::ScrollDown, s.x, mid);
        assert!(top(&app, Side::Right) > before, "the wheel over the strip scrolls that Side");
        assert_eq!(top(&app, Side::Left), 0.0);

        let (_dir, mut app) = two_way();
        let (y, mark) = side_mark(&app, Side::Right, |m| m.target == 0);
        assert_eq!(mark.kind, MarkKind::Modified);
        app.navigator.focused = true;
        press(&mut app, s.x, y);
        assert_eq!((app.session.view().current_change(), app.navigator.focused), (Some(0), false), "the keys go to the diff");
        // a press on the thumb released without moving goes to the mark under it
        app.session.execute(Command::SetScroll(ScrollMode::Independent));
        app.session.scroll_to(Side::Right, 240.0);
        screen(&mut app, 120, 30);
        let bar = app.session.view().scrollbar(Side::Right);
        let (y, mark) = side_mark(&app, Side::Right, |m| m.target == 2);
        assert_eq!(mark.kind, MarkKind::Added);
        let (start, size) = bar.thumb(f32::from(s.height), 1.0).unwrap();
        assert!((start..=start + size).contains(&(f32::from(y - s.y) + 0.5)), "the mark is under the thumb");
        press(&mut app, s.x, y);
        assert_eq!((app.session.view().current_change(), app.scrollbar_drag.is_none()), (Some(2), true));
        // a find hit's mark makes that hit current and selects it; the find prompt stays
        typed(&mut app, "/29");
        screen(&mut app, 120, 30);
        let side = app.session.find().unwrap().side();
        let (y, mark) = side_mark(&app, side, |m| m.kind == MarkKind::Find);
        let i = mark.target - lenz::view::FIND_TARGET;
        let line = app.session.find().unwrap().hits()[i].0;
        let x = strip(&app, side).x;
        press(&mut app, x, y);
        assert_eq!(app.session.find().map(|f| f.at()), Some(Some(i)), "the hit is current, the find open");
        assert_eq!(app.prompt, Some(crate::app::Prompt::Search));
        let sel = app.selection.unwrap();
        assert_eq!((sel.side, sel.anchor.line), (side, line));
        drop(dir);
    }
}
