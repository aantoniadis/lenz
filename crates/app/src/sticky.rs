//! Which header lines each comparison pane pins at its top (sticky scroll), from each side's declarations.

use lenz_content::highlight::Declarations;

use crate::view::{Content, Frame, Row, Side};

/// Headers pinned at most, innermost kept.
pub const MAX: usize = 5;

/// One pinned header: the side's source line (0-based) and the row (0 = top) it is drawn on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StickyLine {
    pub side: Side,
    pub line: u32,
    pub row: u32,
}

/// Rows a pane of `rows` rows gives to sticky lines: none below 6 rows, at most a third, at most [`MAX`].
pub fn cap(rows: usize) -> usize {
    if rows < 6 { 0 } else { (rows / 3).min(MAX) }
}

/// The sticky lines of each pane: index 0 the left pane, 1 the right; in Unified everything is in index 1, from the
/// side whose line is at the top. `max` is the pane's [`cap`].
pub fn lines(frame: &Frame, unified: bool, decls: [Option<&Declarations>; 2], max: usize) -> [Vec<StickyLine>; 2] {
    if unified {
        let top_row = |rows: &[Row]| rows.iter().filter(|r| r.y + 1.0 > 0.0 && matches!(r.content, Content::Line(_))).map(|r| r.y).reduce(f32::min);
        let deleted_on_top = match (top_row(&frame.left), top_row(&frame.right)) {
            (Some(l), Some(r)) => l < r,
            (l, _) => l.is_some(),
        };
        let one = if deleted_on_top { side(&frame.left, frame.left_top, decls[0], Side::Left, max) } else { side(&frame.right, frame.right_top, decls[1], Side::Right, max) };
        return [Vec::new(), one];
    }
    [side(&frame.left, frame.left_top, decls[0], Side::Left, max), side(&frame.right, frame.right_top, decls[1], Side::Right, max)]
}

/// `top` is the side's line position at the viewport top (a gap maps to the line above it); a fraction means only
/// the rest of that line shows. The band stops above the first Collapsed Region or Folded Change Block row, which
/// stays visible and clickable; a header slides up to the row its scope's last line is on.
fn side(rows: &[Row], top: f32, decls: Option<&Declarations>, side: Side, max: usize) -> Vec<StickyLine> {
    let Some(d) = decls.filter(|_| max > 0) else { return Vec::new() };
    let shown = |r: &&Row| r.y + 1.0 > 0.01;
    let fold = rows.iter().filter(shown).filter(|r| matches!(r.content, Content::Fold { .. } | Content::Folded { .. })).map(|r| r.y.max(0.0).floor() as u32).min();
    let max = max.min(fold.map_or(usize::MAX, |f| f as usize));
    if max == 0 {
        return Vec::new();
    }
    let row_of = |line: u32| {
        rows.iter().filter(shown).filter_map(|r| match r.content {
            Content::Line(l) if l >= line => Some(r.y.max(0.0).floor() as u32),
            _ => None,
        }).min().unwrap_or(u32::MAX)
    };
    // Scroll positions carry float noise (2.9999995 is line 3); snap to a thousandth of a row first.
    let top = (top.max(0.0) * 1000.0).round() / 1000.0;
    let line = top.floor();
    d.sticky_rows(line as u32, top > line, max, row_of).into_iter().map(|s| StickyLine { side, line: s.line, row: s.row }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::View;
    use lenz_content::highlight::{parse_with, Language, Want};
    use lenz_engine::ChangeBlock;

    fn decls(src: &str) -> Option<Declarations> {
        parse_with(Language::Rust, src.into(), Want { folds: false, declarations: true }, || true)?.declarations
    }

    /// A comparison of `left` and `right` (blocks given), scrolled `rows` down on the right, as (frame, declarations).
    fn scrolled(left: &str, right: &str, blocks: &[ChangeBlock], rows: f32, unified: bool) -> (Frame, [Option<Declarations>; 2]) {
        scrolled_in(crate::view::ScrollMode::Shared, left, right, blocks, rows, unified)
    }

    fn scrolled_in(mode: crate::view::ScrollMode, left: &str, right: &str, blocks: &[ChangeBlock], rows: f32, unified: bool) -> (Frame, [Option<Declarations>; 2]) {
        let mut v = View::new(left.lines().count() as u32, right.lines().count() as u32, blocks, 20.0);
        v.set_mode(mode);
        v.set_unified(unified);
        v.scroll_by(rows, Side::Right);
        (v.frame(), [decls(left), decls(right)])
    }

    fn block(left: std::ops::Range<u32>, right: std::ops::Range<u32>) -> ChangeBlock {
        ChangeBlock { left, right, ignored: false, inline: None }
    }

    #[test]
    fn each_side_sticks_from_its_own_text() {
        let left = "fn old() {\n    a;\n    b;\n    c;\n}\n";
        let right = "fn new() {\n    a;\n    b;\n    c;\n}\n";
        let (frame, d) = scrolled(left, right, &[block(0..1, 0..1)], 2.0, false);
        let [l, r] = lines(&frame, false, [d[0].as_ref(), d[1].as_ref()], MAX);
        assert_eq!(l, vec![StickyLine { side: Side::Left, line: 0, row: 0 }]);
        assert_eq!(r, vec![StickyLine { side: Side::Right, line: 0, row: 0 }]);
    }

    #[test]
    fn a_side_whose_top_is_a_gap_uses_its_line_above() {
        // Padded: the left side shows filler rows beside the right's inserted x, y, z; at the top, f still sticks.
        let tail = "    b;\n".repeat(30);
        let left = format!("fn f() {{\n    a;\n{tail}}}\n");
        let right = format!("fn f() {{\n    a;\n    x;\n    y;\n    z;\n{tail}}}\n");
        let (frame, d) = scrolled_in(crate::view::ScrollMode::Padded, &left, &right, &[block(2..2, 2..5)], 3.0, false);
        assert!(frame.left.iter().any(|r| r.y.abs() < 0.01 && r.content == crate::view::Content::Filler), "the left top row is filler: {frame:?}");
        let [l, _] = lines(&frame, false, [d[0].as_ref(), d[1].as_ref()], MAX);
        assert_eq!(l, vec![StickyLine { side: Side::Left, line: 0, row: 0 }], "{frame:?}");
    }

    #[test]
    fn unified_takes_the_old_side_for_a_deleted_top_line() {
        let left = "fn f() {\n    gone;\n    more;\n    a;\n}\n";
        let right = "fn f() {\n    a;\n}\n";
        let (frame, d) = scrolled(left, right, &[block(1..3, 1..1)], 1.0, true);
        let [l, u] = lines(&frame, true, [d[0].as_ref(), d[1].as_ref()], MAX);
        assert!(l.is_empty());
        assert_eq!(u, vec![StickyLine { side: Side::Left, line: 0, row: 0 }], "{frame:?}");
    }

    #[test]
    fn a_collapsed_region_on_top_pins_nothing() {
        // The only change is the first line; the 40 unchanged lines below collapse into one row, scrolled to the top.
        let body = "    a;\n".repeat(40);
        let (left, right) = (format!("fn old() {{\n{body}}}\n"), format!("fn new() {{\n{body}}}\n"));
        let mut v = View::new(42, 42, &[block(0..1, 0..1)], 20.0);
        v.set_collapsed(true);
        v.scroll_by(5.0, Side::Right); // past the change and its context rows
        let frame = v.frame();
        assert!(matches!(frame.right.iter().find(|r| r.y.abs() < 0.01).map(|r| &r.content), Some(crate::view::Content::Fold { .. })), "{frame:?}");
        let d = [decls(&left), decls(&right)];
        assert_eq!(lines(&frame, false, [d[0].as_ref(), d[1].as_ref()], MAX), [vec![], vec![]], "the region's row stays visible and clickable");
    }

    #[test]
    fn the_band_stops_above_a_collapsed_region_row() {
        let body = "            a;\n".repeat(50);
        let left = format!("mod m {{\n    impl A {{\n        fn f() {{\n            x;\n{body}        }}\n    }}\n}}\n");
        let right = left.replacen("            x;", "            y;", 1);
        let n = left.lines().count() as u32;
        for rows in [6.0, 7.0] {
            let mut v = View::new(n, n, &[block(3..4, 3..4)], 20.0);
            v.set_collapsed(true);
            v.scroll_by(rows, Side::Right);
            let frame = v.frame();
            let fold = frame.right.iter().filter(|r| matches!(r.content, crate::view::Content::Fold { .. }) && r.y + 1.0 > 0.01).map(|r| r.y).reduce(f32::min);
            let Some(fold) = fold else { panic!("a Collapsed Region row is shown: {frame:?}") };
            let d = [decls(&left), decls(&right)];
            let [_, r] = lines(&frame, false, [d[0].as_ref(), d[1].as_ref()], MAX);
            assert!(r.iter().all(|s| (s.row as f32) < fold), "scrolled {rows}: the band {r:?} covers the region's row at {fold}");
        }
    }

    #[test]
    fn tiny_panes_pin_less() {
        assert_eq!((cap(5), cap(6), cap(12), cap(60)), (0, 2, 4, 5));
    }
}
