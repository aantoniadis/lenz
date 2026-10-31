//! Which source lines a tab shows: every line except those inside its collapsed fold regions. Rows map to source
//! lines through this; line numbers, history and positions stay source lines.

use std::{collections::BTreeMap, ops::Range};

use lenz_content::highlight::{FoldKind, Folds};

/// A tab's collapsed regions by first line and kind, with a generation that every change bumps (so what is derived
/// from them can be kept until they change).
#[derive(Clone, Debug, Default)]
pub struct Collapsed {
    map: BTreeMap<usize, FoldKind>,
    generation: u64,
}

impl Collapsed {
    pub fn map(&self) -> &BTreeMap<usize, FoldKind> {
        &self.map
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn get(&self, first: usize) -> Option<FoldKind> {
        self.map.get(&first).copied()
    }
    pub fn insert(&mut self, first: usize, kind: FoldKind) {
        if self.map.insert(first, kind) != Some(kind) {
            self.generation += 1;
        }
    }
    /// Whether it was collapsed.
    pub fn remove(&mut self, first: usize) -> bool {
        let was = self.map.remove(&first).is_some();
        self.generation += u64::from(was);
        was
    }
    /// Whether any was collapsed.
    pub fn clear(&mut self) -> bool {
        let any = !self.map.is_empty();
        self.map.clear();
        self.generation += u64::from(any);
        any
    }
    pub fn retain(&mut self, keep: impl Fn(usize, FoldKind) -> bool) {
        let n = self.map.len();
        self.map.retain(|&first, kind| keep(first, *kind));
        self.generation += u64::from(self.map.len() != n);
    }
}

/// A tab's shown lines: the hidden ranges (each collapsed region's lines after its first), sorted and merged.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Visible {
    hidden: Vec<Range<usize>>,
    lines: usize,
}

impl Visible {
    /// `collapsed` holds fold first lines; one without a region of that kind in `folds` hides nothing.
    pub fn new(folds: Option<&Folds>, collapsed: &BTreeMap<usize, FoldKind>, lines: usize) -> Self {
        let mut hidden: Vec<Range<usize>> = Vec::new();
        if let Some(folds) = folds {
            for (&first, &kind) in collapsed {
                let Some(f) = folds.starting(first).filter(|f| f.kind == kind) else { continue };
                let r = first + 1..(f.last as usize + 1).min(lines);
                match hidden.last_mut() {
                    Some(h) if r.start <= h.end => h.end = h.end.max(r.end),
                    _ if !r.is_empty() => hidden.push(r),
                    _ => {}
                }
            }
        }
        Self { hidden, lines }
    }

    /// Every one of `lines` lines shown.
    pub fn all(lines: usize) -> Self {
        Self { hidden: Vec::new(), lines }
    }

    fn range_of(&self, line: usize) -> Option<&Range<usize>> {
        let i = self.hidden.partition_point(|h| h.end <= line);
        self.hidden.get(i).filter(|h| h.start <= line)
    }

    pub fn hidden(&self, line: usize) -> bool {
        self.range_of(line).is_some()
    }

    /// The line whose row shows `line`: itself, or its collapsed fold's first line.
    pub fn shown(&self, line: usize) -> usize {
        self.range_of(line).map_or(line, |h| h.start - 1)
    }

    fn next(&self, line: usize) -> Option<usize> {
        let n = line + 1;
        let n = self.range_of(n).map_or(n, |h| h.end);
        (n < self.lines).then_some(n)
    }

    fn previous(&self, line: usize) -> Option<usize> {
        line.checked_sub(1).map(|p| self.shown(p))
    }

    /// `rows` rows down (up when negative) from `from`'s row, stopping at the first and last shown lines.
    pub fn step(&self, from: usize, rows: isize) -> usize {
        let mut at = self.shown(from.min(self.lines.saturating_sub(1)));
        for _ in 0..rows.unsigned_abs() {
            match if rows > 0 { self.next(at) } else { self.previous(at) } {
                Some(l) => at = l,
                None => break,
            }
        }
        at
    }

    /// The source lines of up to `n` rows from `top`'s row.
    pub fn lines(&self, top: usize, n: usize) -> Vec<usize> {
        if self.lines == 0 {
            return Vec::new();
        }
        let mut out = Vec::with_capacity(n.min(self.lines));
        let mut at = Some(self.shown(top.min(self.lines - 1)));
        while let Some(l) = at.filter(|_| out.len() < n) {
            out.push(l);
            at = self.next(l);
        }
        out
    }

    /// Rows from `a`'s row down to `b`'s row (0 when `b`'s is not below).
    pub fn rows_between(&self, a: usize, b: usize) -> usize {
        let (a, b) = (self.shown(a), self.shown(b));
        if b <= a {
            return 0;
        }
        let hidden: usize = self.hidden.iter().map(|h| h.start.max(a)..h.end.min(b + 1)).map(|r| r.len()).sum();
        b - a - hidden
    }

    /// The shown line `row` rows below the first (the last shown line past the end): `step(0, row)` in O(folds), the
    /// inverse of `rows_between(0, line)`.
    pub fn line_at_row(&self, row: usize) -> usize {
        let mut line = row;
        for h in &self.hidden {
            if h.start > line { break; }
            line += h.len();
        }
        self.shown(line.min(self.lines.saturating_sub(1)))
    }

    /// The last shown line.
    pub fn last(&self) -> usize {
        self.shown(self.lines.saturating_sub(1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lenz_content::highlight::{Fold, FoldKind::*, Folds};
    use std::collections::BTreeMap;

    // Lines 0..20; regions 2..=6 (inner 3..=5) and 10..=12.
    fn folds() -> Folds {
        let f = |first, last| Fold { first, last, kind: Block, closes: true };
        Folds { regions: vec![f(2, 6), f(3, 5), f(10, 12)] }
    }

    fn view(collapsed: &[usize]) -> Visible {
        let c: BTreeMap<usize, _> = collapsed.iter().map(|&l| (l, Block)).collect();
        Visible::new(Some(&folds()), &c, 20)
    }

    #[test]
    fn collapsed_regions_hide_all_but_their_first_line() {
        let v = view(&[2, 10]);
        assert!(!v.hidden(2) && v.hidden(3) && v.hidden(6) && !v.hidden(7));
        assert_eq!(v.lines(0, 6), vec![0, 1, 2, 7, 8, 9]);
        assert_eq!(v.shown(5), 2);
        assert_eq!(v.last(), 19);
    }

    #[test]
    fn a_collapsed_region_inside_a_collapsed_one_changes_nothing() {
        assert_eq!(view(&[2, 3]).lines(0, 4), view(&[2]).lines(0, 4));
        assert_eq!(view(&[3]).lines(2, 3), vec![2, 3, 6]);
    }

    #[test]
    fn steps_and_row_counts_skip_hidden_lines() {
        let v = view(&[2, 10]);
        assert_eq!(v.step(1, 2), 7);
        assert_eq!(v.step(7, -1), 2);
        assert_eq!(v.step(0, 1000), 19);
        assert_eq!(v.step(5, 0), 2, "a hidden line steps from its fold's first line");
        assert_eq!(v.rows_between(0, 11), 6, "rows 0 1 2 7 8 9 then 10, which shows 11");
    }

    #[test]
    fn line_at_row_matches_stepping_from_the_top() {
        let f = |first, last| Fold { first, last, kind: Block, closes: true };
        // Nested (2 and 3) and chained (6 inside 2's end) regions merge; 12 and 17 stand alone, 17 at the end.
        let folds = Folds { regions: vec![f(2, 6), f(3, 5), f(6, 9), f(12, 14), f(17, 19)] };
        for collapsed in [&[][..], &[2], &[3], &[2, 3, 6], &[2, 6, 12], &[2, 3, 6, 12, 17], &[12, 17]] {
            let c: BTreeMap<usize, _> = collapsed.iter().map(|&l| (l, Block)).collect();
            let v = Visible::new(Some(&folds), &c, 20);
            for row in 0..30 {
                assert_eq!(v.line_at_row(row), v.step(0, row as isize), "row {row} with {collapsed:?} collapsed");
            }
            assert_eq!(v.line_at_row(v.rows_between(0, 13)), v.shown(13));
        }
        assert_eq!(Visible::all(0).line_at_row(5), Visible::all(0).step(0, 5));
    }

    #[test]
    fn end_and_scroll_stop_at_the_last_visible_line() {
        let f = Folds { regions: vec![Fold { first: 17, last: 19, kind: Block, closes: true }] };
        let v = Visible::new(Some(&f), &BTreeMap::from([(17, Block)]), 20);
        assert_eq!(v.last(), 17);
        assert_eq!(v.step(0, 1000), 17);
        assert_eq!(v.lines(16, 5), vec![16, 17]);
    }

    #[test]
    fn stale_collapsed_lines_and_no_folds_hide_nothing() {
        let v = Visible::new(None, &BTreeMap::from([(2, Block)]), 20);
        assert_eq!(v.lines(0, 3), vec![0, 1, 2]);
        let v = Visible::new(Some(&folds()), &BTreeMap::from([(4, Block), (99, Block)]), 20);
        assert!(!v.hidden(5));
        assert_eq!(Visible::all(0).lines(0, 5), Vec::<usize>::new());
    }
}
