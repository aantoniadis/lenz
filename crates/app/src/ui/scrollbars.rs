//! The scrollbar strip at each text pane's right edge: a Comparison's Sides (or its unified pane), the three Merge
//! View panes and the Markdown preview. The shared model places the thumb and marks and classifies a press; this
//! adapter lays the strips out, paints them and routes the pointer to the cores' scroll and navigation.
use super::*;
use crate::scrollbar::{self, Hit, Mark, MarkKind, Scrollbar};
use crate::view::FIND_TARGET;

const THUMB_MIN: f32 = 16.0;

/// The strip's width on a pane `w` wide: 12 px at the default font size, none once the pane's text would be under two
/// strips wide (the strip goes before the text does).
pub(super) fn strip_w(w: f32) -> f32 {
    let s = scale_of(12.0);
    if w - num_w() - 16.0 < 2.0 * s { 0.0 } else { s }
}

/// One track cell per 3 px.
fn cells(len: f32) -> usize { (len / 3.0).floor().max(0.0) as usize }

/// What a strip scrolls: a Comparison Side (Unified paints `Right`), a merge pane, or a preview pane.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Target { Side(Side), Merge(usize), Preview(Side) }

/// A strip's window rectangle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Strip { pub key: &'static str, pub target: Target, pub x: f32, pub y: f32, pub w: f32, pub h: f32 }
impl Strip {
    fn contains(&self, x: f32, y: f32) -> bool { x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h }
}

/// What was shown when a thumb was grabbed: the Comparison, its diff, and the merge (a drag ends when it changes).
type Shown = (usize, u64, usize);

/// A held thumb: its strip, where in the thumb it was grabbed, the press, and whether it has moved.
#[derive(Clone, Copy)]
pub(super) struct BarDrag { target: Target, grab: f32, y: f32, moved: bool, shown: Shown }

/// What a frame painted on one strip: its pane key, the thumb, and the kind of each marked cell.
#[cfg(test)]
pub(super) type PaintedScrollbar = (&'static str, Option<(f32, f32)>, Vec<MarkKind>);

/// A strip ready to paint: the thumb and each marked cell's winning kind.
pub(super) struct PaintedBar { strip: Strip, thumb: Option<(f32, f32)>, marks: Vec<(usize, MarkKind)> }

/// Paint a strip: its track, the thumb, and over both each marked cell (the caret a 1 px line).
pub(super) fn paint(palette: &theme::Palette, window: &mut Window, b: &PaintedBar) {
    let s = b.strip;
    fill(window, s.x, s.y, s.w, s.h, scrollbar::track_color(palette));
    if let Some((start, size)) = b.thumb { fill(window, s.x + 2.0, s.y + start, s.w - 4.0, size, scrollbar::thumb_color(palette)); }
    let cell_h = s.h / cells(s.h).max(1) as f32;
    for &(cell, kind) in &b.marks {
        let (y, h) = if kind == MarkKind::Caret { (s.y + cell as f32 * cell_h + 1.0, 1.0) } else { (s.y + cell as f32 * cell_h, 3.0) };
        fill(window, s.x + 1.0, y, s.w - 2.0, h, theme::scrollbar_mark_color(kind, palette));
    }
}

impl Viewer {
    /// The strips on screen, left to right: none over an image, a details tab, a Folder Summary or a placeholder.
    pub(super) fn scrollbar_strips(&self, window: &Window) -> Vec<Strip> {
        let (x0, y) = (self.diff_x(), self.top());
        let width = f32::from(window.viewport_size().width) - x0;
        let h = (self.content_bottom() - y - self.panel_h() - self.picker_v_height()).max(0.0);
        let strip = |key, target, x: f32, w: f32| {
            let sw = strip_w(w);
            (sw > 0.0 && h > 0.0).then_some(Strip { key, target, x: x + w - sw, y, w: sw, h })
        };
        if self.merge.is_some() {
            let w = self.merge_widths(width);
            let xs = [x0, x0 + w[0] + GUTTER_W, x0 + w[0] + w[1] + 2.0 * GUTTER_W];
            return ["ours", "middle", "theirs"].into_iter().enumerate().filter_map(|(i, key)| strip(key, Target::Merge(i), xs[i], w[i])).collect();
        }
        if self.image_doc().is_some() || self.pr_details_shown() || self.summary_shown() || self.doc().is_none_or(|d| d.placeholder.is_some()) {
            return Vec::new();
        }
        let w = self.comparison_widths(width);
        let rx = x0 + w[0] + GUTTER_W;
        let (left, right) = match () {
            _ if self.markdown_active() => (("preview", Target::Preview(Side::Left)), ("preview", Target::Preview(Side::Right))),
            _ if self.comparison.view.unified() => (("", Target::Side(Side::Left)), ("unified", Target::Side(Side::Right))),
            _ => (("left", Target::Side(Side::Left)), ("right", Target::Side(Side::Right))),
        };
        // Inline and Unified are one pane: the left one is 0 wide
        [strip(left.0, left.1, x0, w[0]), strip(right.0, right.1, rx, w[1])].into_iter().flatten().collect()
    }

    /// `target`'s scroll position: the core's for text, the laid-out pixels for a preview.
    fn bar(&self, target: Target, window: &Window) -> Option<Scrollbar> {
        match target {
            Target::Side(side) => Some(self.comparison.view.scrollbar(side)),
            Target::Merge(pane) => self.merge.as_ref().map(|s| s.scrollbar(pane)),
            Target::Preview(side) => self.markdown_scrollbar(side, window),
        }
    }

    /// `target`'s marks: Change Blocks (or merge blocks) and find hits; none on a preview.
    fn marks(&self, target: Target) -> Vec<Mark> {
        match target {
            Target::Side(side) => self.comparison.scrollbar_marks(side),
            Target::Merge(pane) => self.merge.as_ref().map_or_else(Vec::new, |s| s.scrollbar_marks(pane)),
            Target::Preview(_) => Vec::new(),
        }
    }

    /// Scroll so `target`'s scrollbar top is `top`, through each core's own scroll (the Scroll Mode moves the rest).
    fn bar_scroll_to(&mut self, target: Target, top: f64, window: &Window) {
        match target {
            Target::Side(side) => self.comparison.view.scroll_to(side, top),
            Target::Merge(pane) => if let Some(s) = self.merge.as_mut() { s.scroll_to(pane, top) },
            Target::Preview(side) => self.markdown_scroll_to(side, top as f32, window),
        }
    }

    /// A mark's place: its Change Block (or merge block) made current, or its find hit made current and shown.
    fn bar_mark(&mut self, target: Target, mark: usize) {
        match (target, mark.checked_sub(FIND_TARGET)) {
            (Target::Merge(_), Some(hit)) => if let Some(s) = self.merge.as_mut() { s.go_to_find_hit(hit) },
            (Target::Merge(_), None) => if let Some(s) = self.merge.as_mut() { s.go_to_block(mark) },
            (_, Some(hit)) => if let Some((side, line, range)) = self.comparison.go_to_find_hit(hit) { self.select_hit(side, line, range) },
            (_, None) => self.comparison.view.go_to_change(mark),
        }
    }

    fn bar_shown(&self) -> Shown {
        (self.comparison.current, self.comparison.revision(), self.merge.as_ref().map_or(0, |s| Arc::as_ptr(&s.parse_id) as usize))
    }

    /// Whether window `(x, y)` is on a strip.
    pub(super) fn on_scrollbar(&self, x: f32, y: f32, window: &Window) -> bool {
        self.scrollbar_strips(window).iter().any(|s| s.contains(x, y))
    }

    /// A press at window `(x, y)`: on the thumb it starts a drag, on a mark it goes to the mark's place, elsewhere on
    /// the track it pages toward the press. Whether a strip took it.
    pub(super) fn scrollbar_press(&mut self, x: f32, y: f32, window: &Window, cx: &mut Context<Self>) -> bool {
        let Some(s) = self.scrollbar_strips(window).into_iter().find(|s| s.contains(x, y)) else { return false };
        let Some(bar) = self.bar(s.target, window) else { return true };
        let marks = self.marks(s.target);
        let at = y - s.y;
        let placed = scrollbar::place(&marks, bar.total, cells(s.h));
        match scrollbar::hit(&bar, &placed, &marks, at, s.h, THUMB_MIN) {
            Hit::Thumb => {
                let start = bar.thumb(s.h, THUMB_MIN).map_or(0.0, |t| t.0);
                self.panes.bar = Some(BarDrag { target: s.target, grab: at - start, y, moved: false, shown: self.bar_shown() });
            }
            hit => {
                // the keys act on the diff after a mark or a page, as after a click on its text
                (self.nav_focused, self.panel_focused) = (false, false);
                match hit {
                    Hit::Mark(i) => self.bar_mark(s.target, marks[i].target),
                    Hit::Above => self.bar_scroll_to(s.target, bar.page(false), window),
                    _ => self.bar_scroll_to(s.target, bar.page(true), window),
                }
            }
        }
        cx.notify();
        true
    }

    /// The pointer moved: a held thumb follows it. The drag ends once the button is up or another view is shown.
    pub(super) fn scrollbar_move(&mut self, y: f32, pressed: bool, window: &Window, cx: &mut Context<Self>) -> bool {
        let Some(mut d) = self.panes.bar else { return false };
        if !pressed || d.shown != self.bar_shown() { self.panes.bar = None; return false; }
        d.moved |= (y - d.y).abs() > 2.0;
        self.panes.bar = Some(d);
        let strip = self.scrollbar_strips(window).into_iter().find(|s| s.target == d.target);
        if let (true, Some(s), Some(bar)) = (d.moved, strip, self.bar(d.target, window)) {
            self.bar_scroll_to(d.target, bar.top_at(y - s.y - d.grab, s.h, THUMB_MIN), window);
            cx.notify();
        }
        true
    }

    /// The button came up: a thumb released where it was pressed goes to the mark under it, as a click on the mark.
    pub(super) fn scrollbar_release(&mut self, y: f32, window: &Window, cx: &mut Context<Self>) {
        let Some(d) = self.panes.bar.take() else { return };
        if d.moved || d.shown != self.bar_shown() || !matches!(self.modal(), None | Some(Modal::Find | Modal::MergeFind)) { return; }
        let (Some(s), Some(bar)) = (self.scrollbar_strips(window).into_iter().find(|s| s.target == d.target), self.bar(d.target, window)) else { return };
        let marks = self.marks(d.target);
        let placed = scrollbar::place(&marks, bar.total, cells(s.h));
        if let Some(i) = scrollbar::mark_at(&placed, &marks, y - s.y, s.h) {
            self.bar_mark(d.target, marks[i].target);
            cx.notify();
        }
    }

    /// This frame's strips: one scrollbar, one mark list and one placement each. Under test, also what was painted.
    pub(super) fn scrollbar_frame(&mut self, window: &Window) -> Vec<PaintedBar> {
        #[cfg(test)]
        self.painted_scrollbars.clear();
        let strips = self.scrollbar_strips(window);
        strips.into_iter().map(|strip| {
            let (thumb, marks) = self.bar(strip.target, window).map_or((None, Vec::new()), |bar| {
                let marks = self.marks(strip.target);
                let placed = scrollbar::place(&marks, bar.total, cells(strip.h));
                (bar.thumb(strip.h, THUMB_MIN), placed.iter().enumerate().filter_map(|(cell, p)| p.map(|i| (cell, marks[i].kind))).collect::<Vec<_>>())
            });
            #[cfg(test)]
            self.painted_scrollbars.push((strip.key, thumb, marks.iter().map(|m| m.1).collect()));
            PaintedBar { strip, thumb, marks }
        }).collect()
    }
}
