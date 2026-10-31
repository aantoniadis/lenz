//! Scrollbar model shared by both apps and their four adapters (two GPUI windows, two terminal UIs): thumb
//! geometry, the priority-merged marks on the track and hit testing. Pure arithmetic over scroll rows; each
//! adapter supplies its own track length (pixels or cells) and draws the result.

/// What a track mark stands for. Declaration order is priority: an earlier kind wins a shared cell, except that
/// `Added | Deleted | Modified` tie and the first placed wins.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MarkKind { Error, Conflict, Warning, Current, FindCurrent, Find, Added, Deleted, Modified, Bookmark, Caret }

impl MarkKind {
    /// Whether clicking the mark scrolls to it; the caret is drawn but never a target.
    pub fn jumps(self) -> bool { self != MarkKind::Caret }

    fn rank(self) -> MarkKind {
        match self { MarkKind::Deleted | MarkKind::Modified => MarkKind::Added, kind => kind }
    }
}

/// A mark at `row` spanning `rows` scroll rows. `target` is opaque to the model: the adapter's own jump argument.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mark { pub row: f64, pub rows: f64, pub kind: MarkKind, pub target: usize }

/// A scroll position in the pane's scroll rows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scrollbar { pub total: f64, pub top: f64, pub visible: f64 }

impl Scrollbar {
    fn max_top(&self) -> f64 { (self.total - self.visible).max(0.0) }

    fn thumb_size(&self, len: f32, min: f32) -> Option<f64> {
        if self.total <= self.visible || len < min || len <= 0.0 { return None; }
        let len = f64::from(len);
        Some((len * self.visible / self.total).max(f64::from(min)).min(len))
    }

    /// `(start, size)` of the thumb along a track of `len`, or `None` when everything is visible or the track is
    /// shorter than `min`.
    pub fn thumb(&self, len: f32, min: f32) -> Option<(f32, f32)> {
        let size = self.thumb_size(len, min)?;
        let start = ((f64::from(len) - size) * self.top / self.max_top()).clamp(0.0, f64::from(len) - size);
        Some((start as f32, size as f32))
    }

    /// The scroll top for a thumb starting at `thumb_start`; the inverse of [`Scrollbar::thumb`].
    pub fn top_at(&self, thumb_start: f32, len: f32, min: f32) -> f64 {
        let Some(size) = self.thumb_size(len, min) else { return 0.0 };
        let room = f64::from(len) - size;
        if room <= 0.0 { return 0.0; }
        (f64::from(thumb_start) / room * self.max_top()).clamp(0.0, self.max_top())
    }

    /// The scroll top one viewport down or up, clamped.
    pub fn page(&self, down: bool) -> f64 {
        let step = if down { self.visible } else { -self.visible };
        (self.top + step).clamp(0.0, self.max_top())
    }
}

/// The track's colour: the gutter's, so a strip reads as part of the pane's edge.
pub fn track_color(p: &crate::theme::Palette) -> u32 { p.gutter_bg }

/// The thumb's colour: a border tone a little toward the track, so marks over it keep their contrast.
pub fn thumb_color(p: &crate::theme::Palette) -> u32 { crate::theme::mix(p.gutter_bg, p.border, 0.8) }

fn cell_of(x: f64, total: f64, cells: usize) -> usize {
    ((x / total * cells as f64).floor().max(0.0) as usize).min(cells.saturating_sub(1))
}

/// One entry per track cell: the index into `marks` of the highest-priority mark covering it.
pub fn place(marks: &[Mark], total: f64, cells: usize) -> Vec<Option<usize>> {
    let mut placed = vec![None; cells];
    if total <= 0.0 || cells == 0 { return placed; }
    for (index, mark) in marks.iter().enumerate() {
        let first = cell_of(mark.row, total, cells);
        let last = cell_of(mark.row + mark.rows, total, cells).max(first);
        for slot in &mut placed[first..=last] {
            if slot.is_none_or(|held| mark.kind.rank() < marks[held].kind.rank()) { *slot = Some(index); }
        }
    }
    placed
}

/// A value kept until its key changes: the cores' mark rows, so a frame reuses them instead of mapping every mark's
/// line again. The key holds every input the value is built from. A session holding one stays `Send` and `Sync`.
pub struct Cached<K, V>(std::sync::Mutex<Option<(K, std::sync::Arc<V>)>>);

impl<K, V> Default for Cached<K, V> {
    fn default() -> Self { Self(Default::default()) }
}

impl<K: PartialEq, V> Cached<K, V> {
    /// The value for `key`, built by `build` only when `key` differs from the last one.
    pub fn get(&self, key: K, build: impl FnOnce() -> V) -> std::sync::Arc<V> {
        let mut held = self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        match &*held {
            Some((k, value)) if *k == key => value.clone(),
            _ => {
                let value = std::sync::Arc::new(build());
                *held = Some((key, value.clone()));
                value
            }
        }
    }
}

/// What a press on the track lands on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit { Thumb, Mark(usize), Above, Below }

/// The jumpable mark in the cell at `at` on a track of `len`.
pub fn mark_at(placed: &[Option<usize>], marks: &[Mark], at: f32, len: f32) -> Option<usize> {
    if placed.is_empty() || len <= 0.0 { return None; }
    let cell = cell_of(f64::from(at), f64::from(len), placed.len());
    placed[cell].filter(|&index| marks[index].kind.jumps())
}

/// Classifies a press at `at`: the thumb, else a jumpable mark, else a page toward the press.
pub fn hit(bar: &Scrollbar, placed: &[Option<usize>], marks: &[Mark], at: f32, len: f32, min: f32) -> Hit {
    let thumb = bar.thumb(len, min);
    if let Some((start, size)) = thumb && at >= start && at <= start + size { return Hit::Thumb; }
    if let Some(index) = mark_at(placed, marks, at, len) { return Hit::Mark(index); }
    let edge = thumb.map_or(len / 2.0, |(start, _)| start);
    if at < edge { Hit::Above } else { Hit::Below }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test] fn thumb_spans_the_visible_share_and_inverts() {
        let bar = Scrollbar { total: 1000.0, top: 0.0, visible: 100.0 };
        assert_eq!(bar.thumb(200.0, 16.0), Some((0.0, 20.0)));
        let end = Scrollbar { top: 900.0, ..bar };
        assert_eq!(end.thumb(200.0, 16.0), Some((180.0, 20.0)));
        assert_eq!(end.top_at(180.0, 200.0, 16.0), 900.0);
        assert_eq!(bar.top_at(90.0, 200.0, 16.0), 450.0);
        assert_eq!(Scrollbar { total: 50.0, top: 0.0, visible: 100.0 }.thumb(200.0, 16.0), None);
    }
    #[test] fn tiny_tracks_keep_the_thumb_inside() {
        let bar = Scrollbar { total: 1_000_000.0, top: 500_000.0, visible: 3.0 };
        let (start, size) = bar.thumb(3.0, 1.0).unwrap();
        assert!(size >= 1.0 && start + size <= 3.0);
        assert_eq!(Scrollbar { total: 10.0, top: 0.0, visible: 3.0 }.thumb(0.0, 1.0), None);
    }
    #[test] fn paging_moves_one_viewport_and_clamps() {
        let bar = Scrollbar { total: 1000.0, top: 950.0, visible: 100.0 };
        assert_eq!(bar.page(true), 900.0);
        assert_eq!(bar.page(false), 850.0);
    }
    #[test] fn place_keeps_the_most_important_mark_per_cell() {
        let m = |row, kind, target| Mark { row, rows: 0.0, kind, target };
        let marks = [m(10.0, MarkKind::Find, 0), m(10.0, MarkKind::Error, 1), m(500.0, MarkKind::Caret, 2), m(500.0, MarkKind::Added, 3)];
        let placed = place(&marks, 1000.0, 100);
        assert_eq!(placed[1], Some(1));
        assert_eq!(placed[50], Some(3));
        assert_eq!(placed.iter().flatten().count(), 2);
    }
    #[test] fn hit_prefers_the_thumb_then_marks_then_paging() {
        let bar = Scrollbar { total: 1000.0, top: 0.0, visible: 100.0 };
        let marks = [Mark { row: 500.0, rows: 0.0, kind: MarkKind::Warning, target: 7 }, Mark { row: 10.0, rows: 0.0, kind: MarkKind::Error, target: 8 }];
        let placed = place(&marks, 1000.0, 200);
        assert_eq!(hit(&bar, &placed, &marks, 5.0, 200.0, 16.0), Hit::Thumb);
        assert_eq!(hit(&bar, &placed, &marks, 100.5, 200.0, 16.0), Hit::Mark(0));
        assert_eq!(hit(&bar, &placed, &marks, 150.0, 200.0, 16.0), Hit::Below);
        assert_eq!(mark_at(&placed, &marks, 2.5, 200.0), Some(1), "a mark under the thumb, for a release that did not move");
    }
    #[test] fn cached_rebuilds_only_when_its_key_changes() {
        let (cache, built) = (Cached::default(), std::cell::Cell::new(0));
        let get = |key: u32| *cache.get(key, || { built.set(built.get() + 1); key * 10 });
        assert_eq!((get(1), get(1), built.get()), (10, 10, 1));
        assert_eq!((get(2), get(1), built.get()), (20, 10, 3));
    }
    #[test] fn caret_marks_are_drawn_but_never_hit() {
        let bar = Scrollbar { total: 1000.0, top: 0.0, visible: 100.0 };
        let marks = [Mark { row: 600.0, rows: 0.0, kind: MarkKind::Caret, target: 0 }];
        let placed = place(&marks, 1000.0, 100);
        assert_eq!(hit(&bar, &placed, &marks, 60.5, 100.0, 16.0), Hit::Below);
    }
}
