//! gpui-free view model: input actions in, frames out.

use std::{collections::HashSet, ops::Range};

use lenz_engine::{ChangeBlock, Refinement};

use crate::scrollbar::{Mark, MarkKind, Scrollbar};

/// A position a sliver past `x`: the first top after a run the leading Side lacks, whose own lines map back to the
/// run's start. The sliver scales with `x` (a few dozen f32 steps) so it survives the rounding of the maps it goes
/// through, at any file size.
pub(crate) fn past_run(x: f32) -> f32 {
    x + x.abs().max(1.0) * f32::EPSILON * 16.0
}

/// A fresh value per call, process-wide: a View's [`View::layout_stamp`] and a find's identity, so caches keyed on
/// them never mistake a rebuilt or replaced one for the one they were built from.
pub(crate) fn next_stamp() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// Scrollbar mark targets at or above this are find hits (`FIND_TARGET + hit index`); below it, Change Block indices.
pub const FIND_TARGET: usize = 1 << 31;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Side {
    Left,
    Right,
}

/// The kind of Change Block a line belongs to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    Inserted,
    Deleted,
    Modified,
    /// An unresolved Conflict in the Merge View (never produced by a 2-way diff).
    Conflict,
}

/// How the two Sides move together.
#[derive(Debug, Clone, Copy, PartialEq, Default, Eq)]
pub enum ScrollMode {
    #[default]
    Shared,
    /// Each Side scrolls its own lines; the other follows so matching lines stay level.
    Aligned,
    Padded,
    /// Each Side scrolls alone; the other never moves.
    Independent,
}

impl ScrollMode {
    /// Each Side keeps its own top (Aligned and Independent), rather than one shared position.
    pub fn separate_tops(self) -> bool { matches!(self, ScrollMode::Aligned | ScrollMode::Independent) }

    /// Both Sides scroll sideways together, except in Independent, where each keeps its own position.
    pub fn shares_horizontal(self) -> bool { self != ScrollMode::Independent }

    pub fn next(self) -> Self {
        match self {
            ScrollMode::Shared => ScrollMode::Aligned,
            ScrollMode::Aligned => ScrollMode::Padded,
            ScrollMode::Padded => ScrollMode::Independent,
            ScrollMode::Independent => ScrollMode::Shared,
        }
    }
}

/// What occupies a painted row on one Side.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Content {
    /// 0-based line index.
    Line(u32),
    /// Padded mode spacer lining a Change Block up with its longer counterpart.
    Filler,
    /// A Collapsed Region: `hidden` unchanged lines behind one row; `id` expands it.
    Fold { hidden: u32, id: u32 },
    /// A Folded Change Block: Change Block `block`, `left`/`right` lines long, behind one row.
    Folded { block: u32, left: u32, right: u32 },
}

/// Context lines kept around each Change Block when unchanged runs collapse.
const CONTEXT: u32 = 4;

/// A display row on one Side: a line, a Collapsed Region or a Folded Change Block starting at raw line `start`.
#[derive(Debug, Clone, Copy, PartialEq)]
enum DRow {
    Line(u32),
    Fold { hidden: u32, id: u32, start: u32 },
    Folded { block: u32, start: u32 },
}

impl DRow {
    fn raw_start(&self) -> u32 {
        match *self {
            DRow::Line(l) => l,
            DRow::Fold { start, .. } | DRow::Folded { start, .. } => start,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    /// Row position from the viewport top, in rows (fractional while scrolling).
    pub y: f32,
    pub content: Content,
    /// Change Block kind and whether it is an Ignored Change.
    pub tint: Option<(Kind, bool)>,
    /// Inline Changes on this line, as byte ranges within the line.
    pub inline: Vec<Range<usize>>,
}

/// Band linking a Change Block on the left to its counterpart on the right.
/// Positions are in rows from the viewport top; an empty Side has `top == bottom`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Connector {
    pub left: (f32, f32),
    pub right: (f32, f32),
    pub kind: Kind,
    pub ignored: bool,
    /// The Change Block's index, and whether it is folded.
    pub block: usize,
    pub folded: bool,
    /// Whether folding the original block would hide more than one line on either Side.
    pub foldable: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    /// Line position at each Side's viewport top.
    pub left_top: f32,
    pub right_top: f32,
    pub left: Vec<Row>,
    pub right: Vec<Row>,
    pub connectors: Vec<Connector>,
}

/// An equal run or a Change Block, positioned on each Side and in combined space
/// (combined length = the longer Side's length).
#[derive(Debug, Clone, Copy)]
struct Seg {
    l0: f32,
    ln: f32,
    r0: f32,
    rn: f32,
    c0: f32,
    cn: f32,
    change: Option<(Kind, bool)>,
    /// Position and length in Unified rows: a Change Block's left then right rows (a folded one is one row).
    u0: f32,
    un: f32,
}

impl Seg {
    fn side(&self, side: Side) -> (f32, f32) {
        match side {
            Side::Left => (self.l0, self.ln),
            Side::Right => (self.r0, self.rn),
        }
    }
}

/// The display row of raw line position `raw`, given `after`: how many of `rows` start at or before it.
fn display_at(rows: &[DRow], after: usize, raw: f32) -> f32 {
    let i = after.saturating_sub(1);
    match rows.get(i) {
        Some(DRow::Line(l)) => i as f32 + (raw - *l as f32).clamp(0.0, 1.0),
        Some(DRow::Fold { .. } | DRow::Folded { .. }) => i as f32,
        None => raw,
    }
}

fn kind_of(b: &ChangeBlock) -> Kind {
    match (b.left.is_empty(), b.right.is_empty()) {
        (true, _) => Kind::Inserted,
        (_, true) => Kind::Deleted,
        _ => Kind::Modified,
    }
}

/// Whether a segment `(start, len)` on one Side lies wholly before position `f`. A run the Side
/// lacks (`len == 0`) at `f` counts as containing it, so positions map to the start of that run.
fn ends_before((a, n): (f32, f32), f: f32) -> bool {
    a + n < f || (a + n == f && n > 0.0)
}

pub(crate) struct RecomparePosition {
    presentation:Presentation, geometry:(usize,u64), raw:[u32;2], mode:ScrollMode,
    tops: [f32; 3],
    collapse: bool, folded: HashSet<u32>,
}
pub struct View {
    /// Segments in display rows (a Collapsed Region is one row).
    segs: Vec<Seg>,
    combined: f32,
    /// Display row counts.
    left_lines: f32,
    right_lines: f32,
    raw_left: u32,
    raw_right: u32,
    rows_left: Vec<DRow>,
    rows_right: Vec<DRow>,
    collapse: bool,
    expanded: HashSet<u32>,
    /// Folded Change Blocks, by block index.
    folded: HashSet<u32>,
    /// The Change Block last jumped to and the scroll position it left, while untouched.
    nav: Option<(usize, f32)>,
    viewport_rows: f32,
    kinds_left: Vec<Option<(Kind, bool)>>,
    kinds_right: Vec<Option<(Kind, bool)>>,
    blocks: Vec<ChangeBlock>,
    /// Per block: refinement was tried (Inline Changes found, or judged too costly).
    refined: Vec<bool>,
    lens_left: Vec<usize>,
    lens_right: Vec<usize>,
    inline_left: Vec<Vec<Range<usize>>>,
    inline_right: Vec<Vec<Range<usize>>>,
    mode: ScrollMode,
    /// Shared/Padded position: combined-space row at the top of the viewport.
    top: f32,
    /// Independent position: each Side's line at its viewport top.
    left_top: f32,
    right_top: f32,
    /// One column: each Change Block's left rows, then its right rows.
    unified: bool,
    unified_rows: f32,
    /// New whenever what [`Self::display_row`] reads changes: the rows, the Scroll Mode or Unified.
    layout: u64,
}

#[derive(Clone)]
pub(crate) struct Presentation { expanded:HashSet<u32>, anchors:[f32;2], nav:Option<usize> }

impl View {
    pub fn new(left_lines: u32, right_lines: u32, blocks: &[ChangeBlock], viewport_rows: f32) -> Self {
        let mut view = View {
            segs: Vec::new(),
            combined: 0.0,
            left_lines: 0.0,
            right_lines: 0.0,
            raw_left: left_lines,
            raw_right: right_lines,
            rows_left: Vec::new(),
            rows_right: Vec::new(),
            collapse: false,
            expanded: HashSet::new(),
            folded: HashSet::new(),
            nav: None,
            viewport_rows,
            kinds_left: Vec::new(),
            kinds_right: Vec::new(),
            blocks: blocks.to_vec(),
            refined: blocks.iter().map(|b| b.inline.is_some()).collect(),
            lens_left: Vec::new(),
            lens_right: Vec::new(),
            inline_left: Vec::new(),
            inline_right: Vec::new(),
            mode: ScrollMode::Shared,
            top: 0.0,
            left_top: 0.0,
            right_top: 0.0,
            unified: false,
            unified_rows: 0.0,
            layout: 0,
        };
        view.tint_lines();
        view.rebuild();
        view
    }

    fn tint_lines(&mut self) {
        self.kinds_left = vec![None; self.raw_left as usize];
        self.kinds_right = vec![None; self.raw_right as usize];
        for b in &self.blocks {
            let tint = Some((kind_of(b), b.ignored));
            b.left.clone().for_each(|i| self.kinds_left[i as usize] = tint);
            b.right.clone().for_each(|i| self.kinds_right[i as usize] = tint);
        }
    }

    /// Unrefined Change Blocks on or near the screen, by index, for lazy Inline Change refinement.
    pub fn blocks_to_refine(&self) -> Vec<(usize, ChangeBlock)> {
        let margin = self.viewport_rows / 2.0;
        // The immutable viewport is shared by every candidate in this scan.
        // Avoid a segment binary search per unrefined block and side.
        let tops = [self.side_top(Side::Left), self.side_top(Side::Right)];
        let near = |side: Side, (a, n): (f32, f32)| {
            let top = tops[side as usize];
            a + n >= top - margin && a <= top + self.viewport_rows + margin
        };
        self.change_segs()
            .enumerate()
            .filter(|(i, s)| !self.refined[*i] && (near(Side::Left, s.side(Side::Left)) || near(Side::Right, s.side(Side::Right))))
            .map(|(i, _)| (i, self.blocks[i].clone()))
            .collect()
    }

    /// Up to `limit` blocks still waiting for refinement, wherever they are: the Markdown Preview shows them all.
    pub fn unrefined_blocks(&self, limit: usize) -> Vec<(usize, ChangeBlock)> {
        (0..self.blocks.len())
            .filter(|&i| !self.refined[i])
            .take(limit)
            .map(|i| (i, self.blocks[i].clone()))
            .collect()
    }

    /// Fold in refinement results for the blocks [`Self::blocks_to_refine`] returned.
    pub fn apply_refinements(&mut self, mut results: Vec<(usize, Refinement)>) {
        // highest index first, so a dropped block doesn't shift the ones still to apply
        results.sort_by_key(|r| std::cmp::Reverse(r.0));
        let mut reshaped = false;
        for (i, r) in results {
            if i >= self.blocks.len() {
                continue;
            }
            match r {
                Refinement::Refined(b) => {
                    reshaped |= b.left != self.blocks[i].left || b.right != self.blocks[i].right;
                    self.blocks[i] = b;
                    if !self.can_fold(i) {
                        self.folded.remove(&(i as u32));
                    }
                    self.refined[i] = true;
                }
                Refinement::TooBig => self.refined[i] = true,
                Refinement::Dropped => {
                    self.blocks.remove(i);
                    self.refined.remove(i);
                    let i = i as u32;
                    self.folded = self.folded.iter().filter(|&&f| f != i).map(|&f| if f > i { f - 1 } else { f }).collect();
                    reshaped = true;
                }
            }
        }
        self.tint_lines();
        if !self.lens_left.is_empty() || !self.lens_right.is_empty() {
            let (l, r) = (std::mem::take(&mut self.lens_left), std::mem::take(&mut self.lens_right));
            self.attach_line_lengths(&l, &r);
        }
        if reshaped {
            let anchor = self.anchor_line();
            self.rebuild();
            self.place_left_at(anchor);
        }
    }

    /// Lay out display rows and segments from the Change Blocks, collapsing long unchanged runs.
    fn rebuild(&mut self) {
        // raw segments: (left start, left len, right start, right len, change)
        let mut raw = Vec::new();
        let (mut l, mut r) = (0u32, 0u32);
        for b in &self.blocks {
            raw.push((l, b.left.start - l, r, b.left.start - l, None));
            raw.push((b.left.start, b.left.len() as u32, b.right.start, b.right.len() as u32, Some((kind_of(b), b.ignored))));
            l = b.left.end;
            r = b.right.end;
        }
        raw.push((l, self.raw_left - l, r, self.raw_right - r, None));

        let (mut segs, mut rows_l, mut rows_r) = (Vec::new(), Vec::new(), Vec::new());
        let mut c = 0u32;
        let mut push = |rows_l: &mut Vec<DRow>, rows_r: &mut Vec<DRow>, left: Vec<DRow>, right: Vec<DRow>, change: Option<(Kind, bool)>| {
            let (ln, rn) = (left.len() as u32, right.len() as u32);
            let cn = ln.max(rn);
            if cn == 0 && change.is_none() {
                return;
            }
            if cn > 0 {
                segs.push(Seg {
                    l0: rows_l.len() as f32,
                    ln: ln as f32,
                    r0: rows_r.len() as f32,
                    rn: rn as f32,
                    c0: c as f32,
                    cn: cn as f32,
                    change,
                    u0: 0.0,
                    un: 0.0,
                });
            }
            rows_l.extend(left);
            rows_r.extend(right);
            c += cn;
        };
        let lines = |a: u32, n: u32| (a..a + n).map(DRow::Line).collect::<Vec<_>>();
        let last = raw.len() - 1;
        for (id, &(l0, ln, r0, rn, change)) in raw.iter().enumerate() {
            let head = if id == 0 { 0 } else { CONTEXT };
            let tail = if id == last { 0 } else { CONTEXT };
            let hidden = ln.saturating_sub(head + tail);
            let id = id as u32;
            // raw segments alternate unchanged run, Change Block, …
            let block = id / 2;
            if change.is_some() && self.folded.contains(&block) {
                push(&mut rows_l, &mut rows_r, vec![DRow::Folded { block, start: l0 }], vec![DRow::Folded { block, start: r0 }], change);
            } else if change.is_none() && ln == rn && self.collapse && hidden > 1 && !self.expanded.contains(&id) {
                push(&mut rows_l, &mut rows_r, lines(l0, head), lines(r0, head), None);
                push(
                    &mut rows_l,
                    &mut rows_r,
                    vec![DRow::Fold { hidden, id, start: l0 + head }],
                    vec![DRow::Fold { hidden, id, start: r0 + head }],
                    None,
                );
                push(&mut rows_l, &mut rows_r, lines(l0 + head + hidden, tail), lines(r0 + head + hidden, tail), None);
            } else {
                push(&mut rows_l, &mut rows_r, lines(l0, ln), lines(r0, rn), change);
            }
        }
        let mut u = 0.0;
        for s in &mut segs {
            s.u0 = u;
            s.un = match s.change {
                None => s.cn,
                Some(_) if matches!(rows_l.get(s.l0 as usize), Some(DRow::Folded { .. })) => 1.0,
                Some(_) => s.ln + s.rn,
            };
            u += s.un;
        }
        self.unified_rows = u;
        self.left_lines = rows_l.len() as f32;
        self.right_lines = rows_r.len() as f32;
        self.rows_left = rows_l;
        self.rows_right = rows_r;
        self.segs = segs;
        self.combined = c as f32;
        self.nav = None;
        self.layout = next_stamp();
    }

    fn rows(&self, side: Side) -> &[DRow] {
        if side == Side::Left { &self.rows_left } else { &self.rows_right }
    }

    /// Display row position of a raw line position (a hidden line maps to its Collapsed Region).
    fn display_of(&self, side: Side, raw: f32) -> f32 {
        let rows = self.rows(side);
        display_at(rows, rows.partition_point(|r| r.raw_start() as f32 <= raw), raw)
    }

    /// Raw line position of a display row position.
    fn raw_of(&self, side: Side, d: f32) -> f32 {
        let rows = self.rows(side);
        if rows.is_empty() {
            return d;
        }
        let i = (d.max(0.0).floor() as usize).min(rows.len() - 1);
        match rows[i] {
            DRow::Line(l) => l as f32 + (d - i as f32).clamp(0.0, 1.0),
            DRow::Fold { start, .. } | DRow::Folded { start, .. } => start as f32,
        }
    }

    // Geometry excludes inline styles and ignored-change colors. Keep captures allocation-free
    // for ordinary views rather than allocating a range vector on the UI thread per option.
    fn geometry(&self)->(usize,u64){
        use std::hash::{DefaultHasher,Hasher};
        let mut hash=DefaultHasher::new();
        for b in &self.blocks{for n in [b.left.start,b.left.end,b.right.start,b.right.end]{hash.write_u32(n);}}
        (self.blocks.len(),hash.finish())
    }

    /// Compact layout state for an option recompare, without copying rows or text.
    pub(crate) fn recompare_position(&self)->RecomparePosition{RecomparePosition{presentation:self.presentation(),geometry:self.geometry(),raw:[self.raw_left,self.raw_right],mode:self.mode,tops:[self.top,self.left_top,self.right_top],collapse:self.collapse,folded:self.folded.clone()}}
    pub(crate) fn retain_recompare_position(&mut self,saved:&RecomparePosition){
        if saved.raw==[self.raw_left,self.raw_right]&&saved.mode==self.mode&&saved.geometry==self.geometry(){
            if self.collapse!=saved.collapse||self.folded!=saved.folded||self.expanded!=saved.presentation.expanded{
                self.collapse=saved.collapse;self.folded=saved.folded.clone();self.expanded=saved.presentation.expanded.clone();self.rebuild();
            }
            [self.top,self.left_top,self.right_top]=saved.tops;
            self.nav=saved.presentation.nav.filter(|i|*i<self.blocks.len()).map(|i|(i,self.scroll_key()));
        }
    }
    pub(crate) fn presentation(&self)->Presentation{Presentation{expanded:self.expanded.clone(),anchors:[self.anchor_line_on(Side::Left),self.anchor_line_on(Side::Right)],nav:self.nav.map(|n|n.0)}}
    pub(crate) fn restore_presentation(&mut self,saved:&Presentation){self.expanded=saved.expanded.clone();self.rebuild();self.place_line_at(Side::Left,saved.anchors[0]);if matches!(self.mode,ScrollMode::Aligned|ScrollMode::Independent){self.place_line_at(Side::Right,saved.anchors[1]);}
        if let Some(i)=saved.nav.filter(|i|*i<self.blocks.len()){self.nav=Some((i,self.top));}}

    /// Turn Collapsed Regions on or off, keeping the anchor line.
    pub fn set_collapsed(&mut self, on: bool) {
        let anchor = self.anchor_line();
        self.collapse = on;
        self.expanded.clear();
        self.rebuild();
        self.place_left_at(anchor);
    }

    /// Expand Collapsed Region `id`, keeping the anchor line.
    pub fn expand(&mut self, id: u32) {
        let anchor = self.anchor_line();
        self.expanded.insert(id);
        self.rebuild();
        self.place_left_at(anchor);
    }

    fn tint(&self, side: Side, line: u32) -> Option<(Kind, bool)> {
        let kinds = if side == Side::Left { &self.kinds_left } else { &self.kinds_right };
        kinds.get(line as usize).copied().flatten()
    }

    /// Split each Change Block's Inline Changes (offsets into the block's lines joined with
    /// `\n`) onto the lines they cover, given every line's byte length.
    pub fn attach_line_lengths(&mut self, left: &[usize], right: &[usize]) {
        fn split(lines: Range<u32>, lens: &[usize], ranges: impl Iterator<Item = Range<usize>>, out: &mut [Vec<Range<usize>>]) {
            let mut starts = Vec::new();
            let mut at = 0;
            for line in lines.clone() {
                starts.push(at);
                at += lens[line as usize] + 1;
            }
            for r in ranges {
                for (k, line) in lines.clone().enumerate() {
                    let (start, end) = (starts[k], starts[k] + lens[line as usize]);
                    let (a, b) = (r.start.max(start), r.end.min(end));
                    if a < b {
                        out[line as usize].push(a - start..b - start);
                    }
                }
            }
        }
        self.lens_left = left.to_vec();
        self.lens_right = right.to_vec();
        self.inline_left = vec![Vec::new(); left.len()];
        self.inline_right = vec![Vec::new(); right.len()];
        for b in &self.blocks {
            let Some(inline) = &b.inline else { continue };
            split(b.left.clone(), left, inline.iter().map(|c| c.left.clone()), &mut self.inline_left);
            split(b.right.clone(), right, inline.iter().map(|c| c.right.clone()), &mut self.inline_right);
        }
    }

    /// Inline Changes on `side`'s `line`, as byte ranges within the line.
    pub fn inline_on(&self, side: Side, line: u32) -> Vec<Range<usize>> {
        self.inline(side, line)
    }

    fn inline(&self, side: Side, line: u32) -> Vec<Range<usize>> {
        let v = if side == Side::Left { &self.inline_left } else { &self.inline_right };
        v.get(line as usize).cloned().unwrap_or_default()
    }

    pub fn mode(&self) -> ScrollMode {
        self.mode
    }

    /// Use `mode` for a freshly built view, which stays at the top of the file.
    pub fn start_in(&mut self, mode: ScrollMode) {
        self.mode = mode;
        self.layout = next_stamp();
    }

    /// Switch Scroll Mode, keeping the left Side's line at the anchor in place.
    pub fn set_mode(&mut self, mode: ScrollMode) {
        let anchor_line = self.anchor_line();
        self.mode = mode;
        self.layout = next_stamp();
        self.place_left_at(anchor_line);
    }

    pub fn set_viewport_rows(&mut self, rows: f32) {
        self.viewport_rows = rows;
    }

    pub fn viewport_rows(&self) -> f32 {
        self.viewport_rows
    }

    pub fn line_visible(&self, side: Side, line: u32) -> bool {
        let frame = self.frame();
        let rows = if side == Side::Left { frame.left } else { frame.right };
        rows.iter().any(|row| row.content == Content::Line(line) && row.y >= 0.0 && row.y <= self.viewport_rows - 1.0)
    }

    /// Left Side line at the anchor (stable across re-diffs and mode switches).
    pub fn anchor_line(&self) -> f32 {
        self.anchor_line_on(Side::Left)
    }

    /// Source line at the visible top, without moving navigation or selection.
    pub fn top_line_on(&self, side: Side) -> f32 {
        self.raw_of(side, self.side_top(side))
    }

    /// `side`'s line at the anchor.
    pub fn anchor_line_on(&self, side: Side) -> f32 {
        // measured before clamping at the file start, so it survives a round trip
        let display = match self.mode {
            ScrollMode::Aligned | ScrollMode::Independent => (if side == Side::Left { self.left_top } else { self.right_top }) + self.anchor(),
            _ => self.combined_to_side(side, self.top + self.anchor()),
        };
        self.raw_of(side, display)
    }

    /// Put the given left Side line at the anchor.
    pub fn place_left_at(&mut self, line: f32) {
        self.place_line_at(Side::Left, line);
    }

    /// Put `side`'s line at the anchor; the other Side follows.
    pub fn place_line_at(&mut self, side: Side, line: f32) {
        let d = self.display_of(side, line);
        self.place_at(side, d);
    }

    /// Put 1-based `line` of `side` at the anchor, expanding a Collapsed Region hiding it;
    /// the other Side follows.
    pub fn go_to_line(&mut self, side: Side, line: u32) {
        let raw_count = if side == Side::Left { self.raw_left } else { self.raw_right };
        let line = line.saturating_sub(1).min(raw_count.saturating_sub(1));
        let rows = self.rows(side);
        let i = rows.partition_point(|r| r.raw_start() <= line).saturating_sub(1);
        match rows.get(i).copied() {
            Some(DRow::Fold { id, .. }) => {
                self.expanded.insert(id);
                self.rebuild();
            }
            Some(DRow::Folded { block, .. }) => {
                self.folded.remove(&block);
                self.rebuild();
            }
            _ => {}
        }
        let d = self.display_of(side, line as f32);
        self.place_at(side, d);
    }

    /// The raw line `delta` rows from `line` on `side`, stepping over Collapsed Regions
    /// (the caret's ↑/↓ and page moves); stays put when only a region lies that way.
    pub fn step_line(&self, side: Side, line: u32, delta: i32) -> u32 {
        let rows = self.rows(side);
        let i = rows.partition_point(|r| r.raw_start() <= line).saturating_sub(1) as i64;
        if rows.is_empty() { return line; } // a Side with no rows at all
        let j = (i + delta as i64).clamp(0, rows.len() as i64 - 1) as usize;
        let line_at = |k: usize| match rows[k] {
            DRow::Line(l) => Some(l),
            DRow::Fold { .. } | DRow::Folded { .. } => None,
        };
        // past a region in the direction of travel, else back towards where the caret was
        let found = if delta < 0 {
            (0..=j).rev().find_map(line_at).or_else(|| (j..rows.len()).find_map(line_at))
        } else {
            (j..rows.len()).find_map(line_at).or_else(|| (0..=j).rev().find_map(line_at))
        };
        found.unwrap_or(line)
    }

    /// Scroll just enough to show `line` of `side`, expanding a Collapsed Region hiding it.
    pub fn reveal(&mut self, side: Side, line: u32) {
        let rows = self.rows(side);
        let i = rows.partition_point(|r| r.raw_start() <= line).saturating_sub(1);
        let unfold = match rows.get(i).copied() {
            Some(DRow::Fold { id, .. }) => self.expanded.insert(id),
            Some(DRow::Folded { block, .. }) => self.folded.remove(&block),
            _ => false,
        };
        if unfold {
            let anchor = self.anchor_line();
            self.rebuild();
            self.place_left_at(anchor);
        }
        let d = self.display_of(side, line as f32);
        // the mapping between Sides is piecewise linear, so a couple of rounds settle it
        for _ in 0..4 {
            let top = self.side_top(side);
            let delta = if d < top {
                d - top
            } else if d + 1.0 > top + self.viewport_rows {
                d + 1.0 - top - self.viewport_rows
            } else {
                return;
            };
            self.scroll_by(delta, side);
        }
    }

    /// Fold or unfold Change Block `i`, keeping the anchor line.
    pub fn toggle_fold(&mut self, i: usize) {
        if !self.can_fold(i) {
            return;
        }
        let i = i as u32;
        if !self.folded.remove(&i) {
            self.folded.insert(i);
        }
        self.refold();
    }

    pub fn folded(&self, i: usize) -> bool {
        self.folded.contains(&(i as u32))
    }

    /// Folding is useful only when at least one Side has multiple lines to hide.
    pub fn can_fold(&self, i: usize) -> bool {
        self.blocks.get(i).is_some_and(|b| b.left.len() > 1 || b.right.len() > 1)
    }

    pub fn unfold_all(&mut self) {
        self.folded.clear();
        self.refold();
    }

    /// Fold these blocks (by index), e.g. after a re-diff kept them unchanged.
    pub fn fold_blocks(&mut self, blocks: &[usize]) {
        for &i in blocks {
            if self.can_fold(i) {
                self.folded.insert(i as u32);
            }
        }
        self.refold();
    }

    /// The folded blocks' line ranges (left, right), to refold them after a re-diff.
    pub fn folded_blocks(&self) -> Vec<(Range<u32>, Range<u32>)> {
        let mut v: Vec<usize> = self.folded.iter().map(|&i| i as usize).collect();
        v.sort_unstable();
        v.into_iter().map(|i| (self.blocks[i].left.clone(), self.blocks[i].right.clone())).collect()
    }

    /// Every Change Block's line ranges (left, right), in order.
    pub fn block_ranges(&self) -> Vec<(Range<u32>, Range<u32>)> {
        self.blocks.iter().map(|b| (b.left.clone(), b.right.clone())).collect()
    }

    pub fn block_range(&self, index: usize) -> Option<(Range<u32>, Range<u32>)> {
        self.blocks.get(index).map(|b| (b.left.clone(), b.right.clone()))
    }

    fn refold(&mut self) {
        let anchor = self.anchor_line();
        let nav = self.nav.map(|(i, _)| i);
        self.rebuild();
        self.place_left_at(anchor);
        // stay on the change n/p landed on
        if let Some(i) = nav {
            self.nav = Some((i, self.scroll_key()));
        }
    }

    /// Index of the Change Block covering `line` of `side`.
    pub fn block_index(&self, side: Side, line: u32) -> Option<usize> {
        self.blocks.iter().position(|b| if side == Side::Left { b.left.contains(&line) } else { b.right.contains(&line) })
    }

    /// The Change Block covering `line` of `side`, as (left lines, right lines).
    pub fn block_at(&self, side: Side, line: u32) -> Option<(Range<u32>, Range<u32>)> {
        self.blocks
            .iter()
            .find(|b| if side == Side::Left { b.left.contains(&line) } else { b.right.contains(&line) })
            .map(|b| (b.left.clone(), b.right.clone()))
    }

    pub fn change_count(&self) -> usize {
        self.segs.iter().filter(|s| s.change.is_some()).count()
    }

    /// Borrow original source ranges and current inline refinements for a compact snapshot.
    pub(crate) fn source_changes(&self) -> &[ChangeBlock] {
        &self.blocks
    }

    fn change_segs(&self) -> impl Iterator<Item = &Seg> {
        self.segs.iter().filter(|s| s.change.is_some())
    }

    /// Combined-space row at the viewport top, whatever the mode.
    fn top_position(&self) -> f32 {
        match self.mode {
            ScrollMode::Aligned | ScrollMode::Independent => self.side_to_combined(Side::Left, self.left_top),
            _ => self.top,
        }
    }

    fn scroll_key(&self) -> f32 {
        match self.mode {
            ScrollMode::Aligned | ScrollMode::Independent => self.left_top,
            _ => self.top,
        }
    }

    /// The Change Block last jumped to (while not scrolled away), else the last one at or above the anchor.
    pub fn current_change(&self) -> Option<usize> {
        if let Some((i, key)) = self.nav
            && key == self.scroll_key()
        {
            return Some(i);
        }
        let at = self.top_position() + self.anchor();
        self.change_segs().filter(|s| s.c0 <= at).count().checked_sub(1)
    }

    /// Jump to the next/previous Change Block; false at the edge of the file (the view stays).
    pub fn step_change(&mut self, forward: bool) -> bool {
        let count = self.change_count();
        let target = match (self.nav.filter(|&(_, key)| key == self.scroll_key()), forward) {
            (Some((i, _)), true) => (i + 1 < count).then_some(i + 1),
            (Some((i, _)), false) => i.checked_sub(1),
            // not navigating: the first change at or below the viewport top / the last above it
            (None, true) => {
                let top = self.top_position();
                self.change_segs().position(|s| s.c0 >= top - 1e-3)
            }
            (None, false) => {
                let top = self.top_position();
                self.change_segs().filter(|s| s.c0 < top - 1e-3).count().checked_sub(1)
            }
        };
        match target {
            Some(i) => {
                self.go_to_change(i);
                true
            }
            None => false,
        }
    }

    pub fn jump_to_first_change(&mut self) {
        if self.change_count() > 0 {
            self.go_to_change(0);
        }
    }

    pub fn jump_to_last_change(&mut self) {
        if let Some(last) = self.change_count().checked_sub(1) {
            self.go_to_change(last);
        }
    }

    /// Make Change Block `i` current and show it at the anchor, as Next Change does; nothing past the last block.
    pub fn go_to_change(&mut self, i: usize) {
        let Some(s) = self.change_segs().nth(i).copied() else { return };
        let a = self.anchor();
        match self.mode {
            ScrollMode::Aligned | ScrollMode::Independent => {
                self.left_top = (s.l0 - a).clamp(0.0, Self::last_top(self.left_lines));
                // a jump lines both Sides up; only scrolling is independent
                self.follow(Side::Left);
            }
            _ => self.top = (s.c0 - a).clamp(0.0, Self::last_top(self.combined)),
        }
        self.nav = Some((i, self.scroll_key()));
    }

    fn place_at(&mut self, side: Side, line: f32) {
        let a = self.anchor();
        match self.mode {
            ScrollMode::Aligned | ScrollMode::Independent => {
                let max = Self::last_top(self.lines(side));
                match side {
                    Side::Left => self.left_top = (line - a).clamp(0.0, max),
                    Side::Right => self.right_top = (line - a).clamp(0.0, max),
                }
                self.follow(side);
            }
            _ => self.top = (self.side_to_combined(side, line) - a).clamp(0.0, Self::last_top(self.combined)),
        }
    }

    /// Scroll by `rows`; in Aligned and Independent modes `lead` is the Side being scrolled, and only Aligned moves
    /// the other Side with it.
    pub fn scroll_by(&mut self, rows: f32, lead: Side) {
        match self.mode {
            ScrollMode::Aligned | ScrollMode::Independent => {
                let (top, max) = match lead {
                    Side::Left => (&mut self.left_top, Self::last_top(self.left_lines)),
                    Side::Right => (&mut self.right_top, Self::last_top(self.right_lines)),
                };
                *top = (*top + rows).clamp(0.0, max);
                if self.mode == ScrollMode::Aligned { self.follow(lead); }
            }
            _ => self.top = (self.top + rows).clamp(0.0, Self::last_top(self.combined)),
        }
    }

    /// `side`'s scrollbar in the rows it paints: its display rows (a Collapsed Region or Folded Change Block is one
    /// row), plus its Filler Rows in Padded. Unified paints one column (adapters call it `Side::Right`): `side` is
    /// ignored and the rows are the Unified rows. The view may scroll its last row up to the top, so `total` grows
    /// to cover the rows past the end then.
    pub fn scrollbar(&self, side: Side) -> Scrollbar {
        let (total, top) = if self.unified {
            (self.unified_rows, self.unified_top())
        } else if self.mode == ScrollMode::Padded {
            (self.combined, self.top)
        } else {
            (self.lines(side), self.side_top(side))
        };
        let visible = f64::from(self.viewport_rows);
        Scrollbar { total: f64::from(total).max(f64::from(top) + visible), top: f64::from(top), visible }
    }

    /// Scroll `side`'s scrollbar top to `top` with one [`Self::scroll_by`], so the Scroll Mode moves the other Side.
    /// Unified in Aligned and Independent follows the left Side's lines, so a top inside lines it lacks lands at
    /// their start, or just past their end when that start is no move down.
    pub fn scroll_to(&mut self, side: Side, top: f64) {
        let (rows, lead) = self.scroll_units_for(side, top);
        self.scroll_by(rows, lead);
    }

    /// The [`Self::scroll_by`] rows, and the Side to lead with, that put `side`'s scrollbar top at `top`.
    pub(crate) fn scroll_units_for(&self, side: Side, top: f64) -> (f32, Side) {
        let (t, a) = (top as f32, self.anchor());
        if self.unified {
            // the combined row whose Unified row is `t`: the inverse of `unified_top`
            let i = self.segs.partition_point(|s| s.u0 + s.un <= t);
            let c = match self.segs.get(i) {
                Some(s) => s.c0 + ((t - s.u0) / s.un).clamp(0.0, 1.0) * s.cn,
                None => self.combined + (t - self.unified_rows),
            };
            return match self.mode {
                ScrollMode::Aligned | ScrollMode::Independent => {
                    // the left Side's lines place the top: one that left lacks lands at the run's start, so moving
                    // down from there steps just past the run's end instead
                    let left = self.combined_to_side(Side::Left, c);
                    let left = if t > self.unified_top() && left <= self.left_top { past_run(self.left_top) } else { left };
                    (left - self.left_top, Side::Left)
                }
                _ => (c - self.top, side),
            };
        }
        let rows = match self.mode {
            // the inverse of `side_top`: a run `side` lacks maps to its start
            ScrollMode::Shared => self.side_to_combined(side, t + a) - a - self.top,
            ScrollMode::Padded => t - self.top,
            ScrollMode::Aligned | ScrollMode::Independent => t - self.side_top(side),
        };
        (rows, side)
    }

    /// Changes whenever [`Self::display_row`] might map a line elsewhere, never otherwise: a key for cached rows.
    pub fn layout_stamp(&self) -> u64 { self.layout }

    /// `side`'s raw line in [`Self::scrollbar`] rows (a hidden line is its region's or folded block's row).
    pub fn display_row(&self, side: Side, raw_line: u32) -> f64 {
        let d = self.display_of(side, raw_line as f32);
        if !self.unified && self.mode != ScrollMode::Padded { return f64::from(d); }
        // the segment holding display row `d`, past runs `side` lacks
        let i = self.segs.partition_point(|s| { let (a, n) = s.side(side); a + n <= d });
        let Some(s) = self.segs.get(i) else {
            let end = if self.unified { self.unified_rows } else { self.combined };
            return f64::from(end + d - self.lines(side));
        };
        let k = d - s.side(side).0;
        f64::from(match s.change {
            _ if !self.unified => s.c0 + k,
            Some(_) if matches!(self.rows_left.get(s.l0 as usize), Some(DRow::Folded { .. })) => s.u0,
            // Unified lists a Change Block's left rows, then its right rows
            Some(_) if side == Side::Right => s.u0 + s.ln + k,
            _ => s.u0 + k,
        })
    }

    /// [`Self::display_row`] of each of `side`'s raw `lines`: one pass over the rows for lines in order (as Change
    /// Block bounds and find hits are), rather than a search each.
    pub fn display_rows(&self, side: Side, lines: impl IntoIterator<Item = u32>) -> Vec<f64> {
        let lines = lines.into_iter();
        if self.unified || self.mode == ScrollMode::Padded { return lines.map(|line| self.display_row(side, line)).collect(); }
        let rows = self.rows(side);
        // rows[..after] start at or before the last line
        let (mut after, mut last) = (0, 0.0);
        lines.map(|line| {
            let raw = line as f32;
            if raw < last { after = 0; }
            // a walk from the last line: lines a few rows apart make it faster than galloping or a search
            while after < rows.len() && rows[after].raw_start() as f32 <= raw { after += 1; }
            last = raw;
            f64::from(display_at(rows, after, raw))
        }).collect()
    }

    /// One scrollbar mark per Change Block on `side`, in [`Self::scrollbar`] rows, its target the block index: the
    /// current block as `Current`, the others by kind. Unified and Padded span each block's rows whatever `side`.
    pub fn block_marks(&self, side: Side) -> Vec<Mark> {
        let current = self.current_change();
        self.change_segs()
            .enumerate()
            .filter_map(|(i, s)| {
                let (row, rows) = if self.unified {
                    (s.u0, s.un)
                } else if self.mode == ScrollMode::Padded {
                    (s.c0, s.cn)
                } else {
                    s.side(side)
                };
                let kind = match s.change?.0 {
                    _ if current == Some(i) => MarkKind::Current,
                    Kind::Inserted => MarkKind::Added,
                    Kind::Deleted => MarkKind::Deleted,
                    Kind::Modified => MarkKind::Modified,
                    Kind::Conflict => MarkKind::Conflict,
                };
                Some(Mark { row: f64::from(row), rows: f64::from(rows), kind, target: i })
            })
            .collect()
    }

    // Retain trailing space for anchor navigation, but never scroll beyond the last row.
    fn last_top(rows: f32) -> f32 { (rows - 1.0).max(0.0) }

    fn anchor(&self) -> f32 {
        self.viewport_rows / 3.0
    }

    /// Aligned mode (and any jump): move the other Side so its anchor line maps to the lead's.
    fn follow(&mut self, lead: Side) {
        let a = self.anchor();
        let lead_top = if lead == Side::Left { self.left_top } else { self.right_top };
        // at the very top both Sides start at line 0 rather than mid-insertion
        let other = if lead_top <= 0.0 { 0.0 } else { (self.map(lead, lead_top + a) - a).max(0.0) };
        match lead {
            Side::Left => self.right_top = other.min(Self::last_top(self.right_lines)),
            Side::Right => self.left_top = other.min(Self::last_top(self.left_lines)),
        }
    }

    fn other(side: Side) -> Side {
        if side == Side::Left { Side::Right } else { Side::Left }
    }

    fn lines(&self, side: Side) -> f32 {
        if side == Side::Left { self.left_lines } else { self.right_lines }
    }

    /// Line position on `from` mapped piecewise-linearly to the other Side.
    fn map(&self, from: Side, f: f32) -> f32 {
        let to = Self::other(from);
        let i = self.segs.partition_point(|s| ends_before(s.side(from), f));
        match self.segs.get(i) {
            Some(s) => {
                let (a, n) = s.side(from);
                let (b, m) = s.side(to);
                b + ((f - a) / n.max(1.0)).clamp(0.0, 1.0) * m
            }
            None => self.lines(to) + (f - self.lines(from)),
        }
    }

    fn combined_to_side(&self, side: Side, c: f32) -> f32 {
        let i = self.segs.partition_point(|s| s.c0 + s.cn <= c);
        match self.segs.get(i) {
            Some(s) => {
                let (a, n) = s.side(side);
                a + ((c - s.c0) / s.cn).clamp(0.0, 1.0) * n
            }
            None => self.lines(side) + (c - self.combined),
        }
    }

    fn side_to_combined(&self, side: Side, f: f32) -> f32 {
        let i = self.segs.partition_point(|s| ends_before(s.side(side), f));
        match self.segs.get(i) {
            Some(s) => {
                let (a, n) = s.side(side);
                s.c0 + ((f - a) / n.max(1.0)).clamp(0.0, 1.0) * s.cn
            }
            None => self.combined + (f - self.lines(side)),
        }
    }

    /// Line position at a Side's viewport top in the current mode.
    fn side_top(&self, side: Side) -> f32 {
        match self.mode {
            ScrollMode::Aligned | ScrollMode::Independent => {
                if side == Side::Left { self.left_top } else { self.right_top }
            }
            ScrollMode::Shared => {
                let a = self.anchor();
                (self.combined_to_side(side, self.top + a) - a).max(0.0)
            }
            ScrollMode::Padded => self.combined_to_side(side, self.top),
        }
    }

    /// The painted row for display row `i` on `side`.
    fn row(&self, side: Side, i: u32, y: f32) -> Row {
        match self.rows(side)[i as usize] {
            DRow::Line(l) => Row { y, content: Content::Line(l), tint: self.tint(side, l), inline: self.inline(side, l) },
            DRow::Fold { hidden, id, .. } => Row { y, content: Content::Fold { hidden, id }, tint: None, inline: Vec::new() },
            DRow::Folded { block, .. } => {
                let b = &self.blocks[block as usize];
                let content = Content::Folded { block, left: b.left.len() as u32, right: b.right.len() as u32 };
                Row { y, content, tint: Some((kind_of(b), b.ignored)), inline: Vec::new() }
            }
        }
    }

    /// Show one column (Unified) or the two Sides (Split); the scroll position is shared.
    pub fn set_unified(&mut self, on: bool) { self.unified = on; self.layout = next_stamp(); }
    pub fn unified(&self) -> bool { self.unified }

    pub fn frame(&self) -> Frame {
        if self.unified { return self.unified_frame(); }
        match self.mode {
            ScrollMode::Padded => self.padded_frame(),
            _ => self.line_frame(),
        }
    }

    /// The Unified row at the viewport top: the scroll position in combined space, mapped segment by segment.
    fn unified_top(&self) -> f32 {
        let c = self.top_position();
        let i = self.segs.partition_point(|s| s.c0 + s.cn <= c);
        let u = match self.segs.get(i) {
            Some(s) if s.cn > 0.0 => s.u0 + ((c - s.c0) / s.cn).clamp(0.0, 1.0) * s.un,
            _ => self.unified_rows,
        };
        u.min(Self::last_top(self.unified_rows))
    }

    /// Unified: deleted rows in `left`, unchanged and inserted rows in `right`, in one sequence of rows; no connectors.
    fn unified_frame(&self) -> Frame {
        let top = self.unified_top();
        let (first, last) = (top.floor(), (top + self.viewport_rows).ceil().min(self.unified_rows));
        let (mut left, mut right) = (Vec::new(), Vec::new());
        let put = |out: &mut Vec<Row>, side: Side, i: f32, u: f32, kind: Option<Kind>| {
            if u + 1.0 <= first || u >= last { return; }
            let mut row = self.row(side, i as u32, u - top);
            if let (Some(k), Some((_, ignored))) = (kind, row.tint) { row.tint = Some((k, ignored)); }
            out.push(row);
        };
        // the rows of a run of `n` starting at unified row `u0` that can be on screen: a long run costs its visible part
        let visible = |u0: f32, n: f32| ((first - u0).max(0.0) as u32)..((last - u0).max(0.0).min(n) as u32);
        let start = self.segs.partition_point(|s| s.u0 + s.un <= first);
        for s in &self.segs[start..] {
            if s.u0 >= last { break; }
            match s.change {
                None => for k in visible(s.u0, s.cn) { put(&mut right, Side::Right, s.r0 + k as f32, s.u0 + k as f32, None) },
                Some(_) if matches!(self.rows_left.get(s.l0 as usize), Some(DRow::Folded { .. })) => put(&mut left, Side::Left, s.l0, s.u0, None),
                Some(_) => {
                    for k in visible(s.u0, s.ln) { put(&mut left, Side::Left, s.l0 + k as f32, s.u0 + k as f32, Some(Kind::Deleted)); }
                    for k in visible(s.u0 + s.ln, s.rn) { put(&mut right, Side::Right, s.r0 + k as f32, s.u0 + s.ln + k as f32, Some(Kind::Inserted)); }
                }
            }
        }
        Frame {
            left_top: self.raw_of(Side::Left, self.side_top(Side::Left)),
            right_top: self.raw_of(Side::Right, self.side_top(Side::Right)),
            left,
            right,
            connectors: Vec::new(),
        }
    }

    /// Shared, Aligned and Independent: each Side paints its own lines from its own top.
    fn line_frame(&self) -> Frame {
        let (lt, rt) = (self.side_top(Side::Left), self.side_top(Side::Right));
        let rows = |side: Side, top: f32| -> Vec<Row> {
            let last = (top + self.viewport_rows).ceil().min(self.lines(side));
            (top.floor() as u32..last as u32).map(|i| self.row(side, i, i as f32 - top)).collect()
        };
        let connectors = self
            .change_segs()
            .enumerate()
            .filter_map(|(block, s)| {
                let (kind, ignored) = s.change?;
                let left = (s.l0 - lt, s.l0 + s.ln - lt);
                let right = (s.r0 - rt, s.r0 + s.rn - rt);
                let visible = |(t, b): (f32, f32)| b >= 0.0 && t <= self.viewport_rows;
                let folded = self.folded.contains(&(block as u32));
                (visible(left) || visible(right)).then_some(Connector { left, right, kind, ignored, block, folded, foldable: self.can_fold(block) })
            })
            .collect();
        Frame {
            left_top: self.raw_of(Side::Left, lt),
            right_top: self.raw_of(Side::Right, rt),
            left: rows(Side::Left, lt),
            right: rows(Side::Right, rt),
            connectors,
        }
    }

    /// Padded: one scroll in combined space; Filler Rows pad the shorter side of each Change Block.
    fn padded_frame(&self) -> Frame {
        let top = self.top;
        let first = top.floor();
        let last = (top + self.viewport_rows).ceil().min(self.combined);
        let (mut left, mut right, mut connectors) = (Vec::new(), Vec::new(), Vec::new());
        let start = self.segs.partition_point(|s| s.c0 + s.cn <= first);
        let mut block = self.segs[..start].iter().filter(|x| x.change.is_some()).count();
        for s in &self.segs[start..] {
            if s.c0 >= last {
                break;
            }
            for (side, out) in [(Side::Left, &mut left), (Side::Right, &mut right)] {
                let (a, n) = s.side(side);
                for k in ((first - s.c0).max(0.0) as u32)..((last - s.c0).max(0.0).min(s.cn) as u32) {
                    let row = s.c0 + k as f32;
                    if row + 1.0 <= first || row >= last {
                        continue;
                    }
                    if (k as f32) < n {
                        out.push(self.row(side, a as u32 + k, row - top));
                    } else {
                        out.push(Row { y: row - top, content: Content::Filler, tint: s.change, inline: Vec::new() });
                    }
                }
            }
            if let Some((kind, ignored)) = s.change {
                let y = s.c0 - top;
                let folded = self.folded.contains(&(block as u32));
                connectors.push(Connector { left: (y, y + s.ln), right: (y, y + s.rn), kind, ignored, block, folded, foldable: self.can_fold(block) });
                block += 1;
            }
        }
        Frame {
            left_top: self.raw_of(Side::Left, self.side_top(Side::Left)),
            right_top: self.raw_of(Side::Right, self.side_top(Side::Right)),
            left,
            right,
            connectors,
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn block(left: std::ops::Range<u32>, right: std::ops::Range<u32>) -> ChangeBlock {
        ChangeBlock { left, right, ignored: false, inline: None }
    }

    /// The unified frame as (marker, line): '-' deleted (left), '+' inserted, ' ' unchanged (right line), '~' a
    /// Collapsed Region, '#' a Folded Change Block; in screen order.
    fn unified(v: &View) -> Vec<(char, u32)> {
        let f = v.frame();
        assert!(f.connectors.is_empty(), "one column: no connectors");
        let mark = |r: &Row, side: Side| match r.content {
            Content::Fold { .. } => ('~', 0),
            Content::Folded { block, .. } => ('#', block),
            Content::Line(l) => match (side, r.tint) {
                (Side::Left, Some((Kind::Deleted, _))) => ('-', l),
                (Side::Right, Some((Kind::Inserted, _))) => ('+', l),
                (Side::Right, None) => (' ', l),
                other => panic!("unexpected row {other:?} on line {l}"),
            },
            Content::Filler => panic!("no filler rows in Unified"),
        };
        let mut rows: Vec<(f32, (char, u32))> = f.left.iter().map(|r| (r.y, mark(r, Side::Left))).chain(f.right.iter().map(|r| (r.y, mark(r, Side::Right)))).collect();
        rows.sort_by(|a, b| a.0.total_cmp(&b.0));
        let ys: Vec<f32> = rows.iter().map(|r| r.0).collect();
        assert!(ys.windows(2).all(|w| (w[1] - w[0] - 1.0).abs() < 1e-4), "one row each, no gaps or overlaps: {ys:?}");
        rows.into_iter().map(|r| r.1).collect()
    }

    #[test]
    fn unified_lists_deleted_then_inserted_lines_and_unchanged_once() {
        // left: a b c d; right: a B c x d
        let mut v = View::new(4, 5, &[block(1..2, 1..2), block(3..3, 3..4)], 20.0);
        v.set_unified(true);
        assert!(v.unified());
        assert_eq!(unified(&v), [(' ', 0), ('-', 1), ('+', 1), (' ', 2), ('+', 3), (' ', 4)]);
        v.set_unified(false);
        assert!(!v.frame().connectors.is_empty(), "Split again");
    }

    #[test]
    fn unified_empty_and_one_sided_files() {
        let mut empty = View::new(0, 0, &[], 10.0);
        empty.set_unified(true);
        assert!(unified(&empty).is_empty());
        let mut added = View::new(0, 3, &[block(0..0, 0..3)], 10.0);
        added.set_unified(true);
        assert_eq!(unified(&added), [('+', 0), ('+', 1), ('+', 2)]);
        let mut deleted = View::new(2, 0, &[block(0..2, 0..0)], 10.0);
        deleted.set_unified(true);
        assert_eq!(unified(&deleted), [('-', 0), ('-', 1)]);
    }

    #[test]
    fn unified_shows_folds_and_collapsed_regions_once() {
        let mut v = View::new(40, 40, &[block(20..23, 20..23)], 60.0);
        v.set_collapsed(true);
        v.toggle_fold(0);
        v.set_unified(true);
        let rows = unified(&v);
        assert_eq!(rows.iter().filter(|r| r.0 == '#').count(), 1, "{rows:?}");
        assert_eq!(rows.iter().filter(|r| r.0 == '~').count(), 2, "{rows:?}");
        v.toggle_fold(0);
        assert_eq!(unified(&v).iter().filter(|r| r.0 == '-').count(), 3);
    }

    #[test]
    fn unified_scrolls_to_its_last_row_and_to_each_change() {
        let mut v = View::new(200, 200, &[block(150..160, 150..160)], 10.0);
        v.set_unified(true);
        v.step_change(true);
        assert!(unified(&v).contains(&('-', 150)), "next change shows the block's first deleted line");
        v.scroll_by(f32::INFINITY, Side::Left);
        assert_eq!(unified(&v).last(), Some(&(' ', 199)), "the end shows the last line");
        v.scroll_by(f32::NEG_INFINITY, Side::Left);
        assert_eq!(unified(&v).first(), Some(&(' ', 0)));
    }

    fn lines(rows: &[Row]) -> Vec<Content> {
        rows.iter().map(|r| r.content).collect()
    }

    #[test]
    fn pending_text_frames_keep_both_unequal_sides_in_bounds() {
        for (left, right) in [(12, 2), (2, 12), (0, 12), (12, 0)] {
            for mode in [
                ScrollMode::Shared,
                ScrollMode::Aligned,
                ScrollMode::Padded,
            ] {
                for collapsed in [false, true] {
                    let mut view = View::new(left, right, &[], 30.0);
                    view.start_in(mode);
                    view.set_collapsed(collapsed);
                    let frame = view.frame();
                    for (rows, count) in [(&frame.left, left), (&frame.right, right)] {
                        let actual = rows
                            .iter()
                            .filter_map(|r| match r.content {
                                Content::Line(line) => Some(line),
                                _ => None,
                            })
                            .collect::<Vec<_>>();
                        assert_eq!(
                            actual,
                            (0..count).collect::<Vec<_>>(),
                            "{left}/{right} {mode:?}, collapsed={collapsed}"
                        );
                    }
                    for height in [1.0, 0.0, 30.0] {
                        view.set_viewport_rows(height);
                        for side in [Side::Left, Side::Right] {
                            view.scroll_by(f32::INFINITY, side);
                            view.scroll_by(-1.0, side);
                            let frame = view.frame();
                            for (rows, count) in [(&frame.left, left), (&frame.right, right)] {
                                assert!(rows.iter().all(
                                    |r| !matches!(r.content, Content::Line(line) if line >= count)
                                ));
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn scrolling_past_end_keeps_a_display_row_and_up_moves_back() {
        for mode in [ScrollMode::Shared, ScrollMode::Aligned, ScrollMode::Padded] {
            for side in [Side::Left, Side::Right] {
                let mut view = View::new(2000, 2000, &[block(799..800, 799..800)], 30.0);
                view.start_in(mode);
                view.set_collapsed(true);
                view.scroll_by(10000.0, side);
                let frame = view.frame();
                assert!(!frame.left.is_empty() && !frame.right.is_empty(), "{mode:?} {side:?}");
                view.scroll_by(-1.0, side);
                assert!(view.frame().left_top < frame.left_top, "{mode:?} {side:?}");
                view.set_viewport_rows(1.0);
                assert!(!view.frame().left.is_empty());
                view.set_viewport_rows(100.0);
                assert!(!view.frame().right.is_empty());
            }
        }
        let mut empty = View::new(0, 0, &[], 1.0);
        empty.scroll_by(10000.0, Side::Right);
        assert_eq!(empty.frame().left_top, 0.0);
    }

    #[test]
    fn recompare_preserves_exact_display_position_at_a_collapsed_tail() {
        for mode in [ScrollMode::Shared, ScrollMode::Aligned, ScrollMode::Padded] {
            let blocks = [block(799..800, 799..800)];
            let mut view = View::new(2000, 2000, &blocks, 30.0);
            view.start_in(mode);
            view.set_collapsed(true);
            view.expand(0);
            view.scroll_by(f32::INFINITY, Side::Right);
            let mut replacement = View::new(2000, 2000, &blocks, 30.0);
            replacement.start_in(mode);
            replacement.set_collapsed(true);
            replacement.retain_recompare_position(&view.recompare_position());
            assert_eq!(replacement.frame(), view.frame(), "{mode:?}");
        }
    }

    /// Tint of a line as painted in the frame.
    fn tint_of(rows: &[Row], line: u32) -> Option<(Kind, bool)> {
        rows.iter().find(|r| r.content == Content::Line(line)).and_then(|r| r.tint)
    }

    #[test]
    fn lines_are_tinted_by_change_block_kind() {
        let blocks = [block(1..2, 1..2), block(3..3, 3..5), block(4..5, 6..6)];
        let f = View::new(6, 7, &blocks, 10.0).frame();
        assert_eq!(tint_of(&f.left, 0), None);
        assert_eq!(tint_of(&f.left, 1), Some((Kind::Modified, false)));
        assert_eq!(tint_of(&f.right, 3), Some((Kind::Inserted, false)));
        assert_eq!(tint_of(&f.left, 4), Some((Kind::Deleted, false)));
        assert_eq!(tint_of(&f.right, 6), None);
    }

    #[test]
    fn lines_of_an_ignored_change_are_faded() {
        let blocks = [ChangeBlock { left: 1..1, right: 1..2, ignored: true, inline: None }, block(3..4, 4..5)];
        let f = View::new(5, 6, &blocks, 10.0).frame();
        assert_eq!(tint_of(&f.right, 1), Some((Kind::Inserted, true)));
        assert_eq!(tint_of(&f.left, 3), Some((Kind::Modified, false)));
        assert_eq!(tint_of(&f.left, 0), None);
    }

    #[test]
    fn shared_scroll_pauses_left_side_over_inserted_lines() {
        // right inserts "x","y" after line 0; viewport of 3 rows puts the anchor 1 row down
        let mut view = View::new(2, 4, &[block(1..1, 1..3)], 3.0);
        view.scroll_by(1.0, Side::Left);
        let f = view.frame();
        assert_eq!((f.left_top, f.right_top), (0.0, 1.0));
    }

    #[test]
    fn aligned_follower_jumps_over_lines_the_leader_lacks() {
        let mut view = View::new(2, 4, &[block(1..1, 1..3)], 3.0);
        view.set_mode(ScrollMode::Aligned);
        view.scroll_by(1.0, Side::Left);
        let f = view.frame();
        assert_eq!((f.left_top, f.right_top), (1.0, 3.0));
    }

    #[test]
    fn aligned_scrolling_the_right_side_leads_the_left() {
        let mut view = View::new(2, 4, &[block(1..1, 1..3)], 3.0);
        view.set_mode(ScrollMode::Aligned);
        view.scroll_by(3.0, Side::Right);
        let f = view.frame();
        assert_eq!((f.left_top, f.right_top), (1.0, 3.0));
    }

    #[test]
    fn independent_scrolling_leaves_the_other_side_still() {
        let mut view = View::new(2, 4, &[block(1..1, 1..3)], 3.0);
        view.set_mode(ScrollMode::Independent);
        view.scroll_by(1.0, Side::Left);
        assert_eq!((view.frame().left_top, view.frame().right_top), (1.0, 0.0));
        view.scroll_by(2.0, Side::Right);
        assert_eq!((view.frame().left_top, view.frame().right_top), (1.0, 2.0));
    }

    #[test]
    fn independent_jumps_line_both_sides_up() {
        let mut view = View::new(20, 22, &[block(10..10, 10..12)], 6.0);
        view.set_mode(ScrollMode::Independent);
        view.scroll_by(15.0, Side::Right);
        view.jump_to_first_change();
        let (aligned, _) = {
            let mut other = View::new(20, 22, &[block(10..10, 10..12)], 6.0);
            other.set_mode(ScrollMode::Aligned);
            other.jump_to_first_change();
            (other.frame(), ())
        };
        assert_eq!((view.frame().left_top, view.frame().right_top), (aligned.left_top, aligned.right_top));
    }

    #[test]
    fn scroll_modes_cycle_through_all_four() {
        let mut mode = ScrollMode::Shared;
        let mut seen = Vec::new();
        for _ in 0..4 { seen.push(mode); mode = mode.next(); }
        assert_eq!(seen, [ScrollMode::Shared, ScrollMode::Aligned, ScrollMode::Padded, ScrollMode::Independent]);
        assert_eq!(mode, ScrollMode::Shared);
    }

    #[test]
    fn padded_mode_fills_the_shorter_side_with_filler_rows() {
        let mut view = View::new(2, 4, &[block(1..1, 1..3)], 10.0);
        view.set_mode(ScrollMode::Padded);
        let f = view.frame();
        assert_eq!(lines(&f.left), vec![Content::Line(0), Content::Filler, Content::Filler, Content::Line(1)]);
        assert_eq!(lines(&f.right), (0..4).map(Content::Line).collect::<Vec<_>>());
        assert_eq!(f.left[1].tint, Some((Kind::Inserted, false)));
        assert_eq!(f.left.iter().map(|r| r.y).collect::<Vec<_>>(), vec![0.0, 1.0, 2.0, 3.0]);
    }

    #[test]
    fn connector_links_a_change_block_to_its_counterpart() {
        let view = View::new(2, 4, &[block(1..1, 1..3)], 10.0);
        let f = view.frame();
        assert_eq!(f.connectors, vec![Connector { left: (1.0, 1.0), right: (1.0, 3.0), kind: Kind::Inserted, ignored: false, block: 0, folded: false, foldable: true }]);
    }

    #[test]
    fn connectors_outside_the_viewport_are_left_out() {
        let view = View::new(100, 100, &[block(80..81, 80..81)], 10.0);
        assert!(view.frame().connectors.is_empty());
    }

    #[test]
    fn switching_scroll_mode_keeps_the_anchor_line() {
        let blocks = [block(5..5, 5..25), block(40..60, 60..60)];
        let mut view = View::new(100, 100, &blocks, 30.0);
        view.scroll_by(50.0, Side::Left);
        let before = view.anchor_line();
        for mode in [ScrollMode::Aligned, ScrollMode::Padded, ScrollMode::Shared] {
            view.set_mode(mode);
            assert!((view.anchor_line() - before).abs() < 1e-3, "{mode:?}");
        }
    }

    #[test]
    fn a_new_view_starts_at_the_top_in_any_mode() {
        // an added file: one empty left line against 50 right lines
        let blocks = [block(0..0, 0..49)];
        for mode in [ScrollMode::Shared, ScrollMode::Aligned, ScrollMode::Padded] {
            let mut view = View::new(1, 50, &blocks, 30.0);
            view.start_in(mode);
            let f = view.frame();
            assert_eq!((f.left_top, f.right_top), (0.0, 0.0), "{mode:?}");
        }
    }

    #[test]
    fn switching_mode_inside_an_insertion_does_not_jump_past_it() {
        let blocks = [block(10..10, 10..60)];
        let mut view = View::new(40, 90, &blocks, 30.0);
        view.scroll_by(20.0, Side::Left); // anchor now inside the inserted lines
        let before = view.frame().right_top;
        view.set_mode(ScrollMode::Padded);
        assert!(view.frame().right_top <= before + 1.0, "jumped from {before} to {}", view.frame().right_top);
    }

    #[test]
    #[allow(clippy::single_range_in_vec_init)] // a list holding one byte range is the point
    fn inline_changes_are_split_onto_the_lines_they_cover() {
        // left lines 1..3 are "ab", "cd"; an Inline Change spans "b\nc" in the block text "ab\ncd"
        let blocks = [ChangeBlock {
            left: 1..3,
            right: 1..2,
            ignored: false,
            inline: Some(vec![lenz_engine::InlineChange { left: 1..4, right: 0..2 }]),
        }];
        let mut view = View::new(4, 3, &blocks, 10.0);
        view.attach_line_lengths(&[1, 2, 2, 1], &[1, 3, 1]);
        let f = view.frame();
        let inline_of = |rows: &[Row], line: u32| rows.iter().find(|r| r.content == Content::Line(line)).map(|r| r.inline.clone());
        assert_eq!(inline_of(&f.left, 1), Some(vec![1..2]));
        assert_eq!(inline_of(&f.left, 2), Some(vec![0..1]));
        assert_eq!(inline_of(&f.right, 1), Some(vec![0..2]));
        assert_eq!(inline_of(&f.left, 0), Some(vec![]));
    }

    #[test]
    fn going_to_a_line_puts_it_at_the_anchor_on_that_side_in_every_mode() {
        let blocks = [block(10..10, 10..30)];
        for mode in [ScrollMode::Shared, ScrollMode::Aligned, ScrollMode::Padded] {
            let mut view = View::new(100, 120, &blocks, 30.0);
            view.start_in(mode);
            view.go_to_line(Side::Right, 60);
            let f = view.frame();
            let anchor_row = f.right.iter().find(|r| r.content == Content::Line(59)).map(|r| r.y);
            assert_eq!(anchor_row, Some(10.0), "{mode:?}");
        }
    }

    #[test]
    fn going_past_the_last_line_shows_the_last_line_at_the_anchor() {
        let mut view = View::new(50, 50, &[], 30.0);
        view.go_to_line(Side::Right, 240);
        let f = view.frame();
        assert_eq!(f.right.iter().find(|r| r.content == Content::Line(49)).map(|r| r.y), Some(10.0));
    }

    fn contents(rows: &[Row]) -> Vec<Content> {
        rows.iter().map(|r| r.content).collect()
    }

    #[test]
    fn unchanged_runs_collapse_to_a_row_keeping_four_context_lines() {
        let mut view = View::new(100, 100, &[block(50..51, 50..51)], 40.0);
        view.set_collapsed(true);
        let rows = contents(&view.frame().left);
        assert_eq!(rows[0], Content::Fold { hidden: 46, id: 0 });
        assert_eq!(&rows[1..6], &[46, 47, 48, 49, 50].map(Content::Line));
        assert_eq!(&rows[6..10], &[51, 52, 53, 54].map(Content::Line));
        assert_eq!(rows[10], Content::Fold { hidden: 45, id: 2 });
        assert_eq!(rows.len(), 11);
    }

    #[test]
    fn expanding_a_collapsed_region_shows_its_lines() {
        let mut view = View::new(100, 100, &[block(50..51, 50..51)], 200.0);
        view.set_collapsed(true);
        view.expand(0);
        let rows = contents(&view.frame().left);
        assert_eq!(&rows[0..3], &[0, 1, 2].map(Content::Line));
        assert_eq!(rows.last(), Some(&Content::Fold { hidden: 45, id: 2 }));
    }

    #[test]
    fn toggling_collapse_keeps_the_anchor_line() {
        let blocks = [block(50..51, 50..51), block(300..301, 300..301)];
        let mut view = View::new(400, 400, &blocks, 30.0);
        view.go_to_line(Side::Left, 301);
        let before = view.anchor_line();
        view.set_collapsed(true);
        assert!((view.anchor_line() - before).abs() < 1e-3);
        view.set_collapsed(false);
        assert!((view.anchor_line() - before).abs() < 1e-3);
    }

    #[test]
    fn going_to_a_line_inside_a_collapsed_region_expands_it() {
        let mut view = View::new(100, 100, &[block(50..51, 50..51)], 30.0);
        view.set_collapsed(true);
        view.go_to_line(Side::Left, 20);
        assert!(contents(&view.frame().left).contains(&Content::Line(19)));
        assert!((view.anchor_line() - 19.0).abs() < 1e-3);
    }

    #[test]
    fn next_and_previous_change_visit_every_change_block_then_stop_at_the_edge() {
        let blocks = [block(3..4, 3..4), block(40..40, 40..45), block(90..92, 95..95)];
        for collapse in [false, true] {
            for mode in [ScrollMode::Shared, ScrollMode::Aligned, ScrollMode::Padded] {
                let mut view = View::new(120, 123, &blocks, 30.0);
                view.set_collapsed(collapse);
                view.start_in(mode);
                let mut seen = Vec::new();
                while view.step_change(true) {
                    seen.push(view.current_change());
                }
                assert_eq!(seen, vec![Some(0), Some(1), Some(2)], "{mode:?} collapse={collapse}");
                assert!(view.step_change(false));
                assert_eq!(view.current_change(), Some(1));
                assert!(view.step_change(false));
                assert!(!view.step_change(false));
                assert_eq!(view.change_count(), 3);
            }
        }
    }

    #[test]
    fn a_change_block_jumped_to_sits_at_the_anchor() {
        let mut view = View::new(200, 200, &[block(120..121, 120..121)], 30.0);
        assert!(view.step_change(true));
        assert!((view.anchor_line() - 120.0).abs() < 1e-3);
    }

    #[test]
    fn last_change_can_be_jumped_to_directly() {
        let blocks = [block(3..4, 3..4), block(90..92, 95..95)];
        let mut view = View::new(120, 123, &blocks, 30.0);
        view.jump_to_last_change();
        assert_eq!(view.current_change(), Some(1));
    }

    /// Deterministic random block lists (no extra test dependencies).
    fn random_blocks(seed: u64) -> (u32, u32, Vec<ChangeBlock>) {
        let mut s = seed;
        let mut next = |m: u32| {
            s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((s >> 33) % m as u64) as u32
        };
        let (mut l, mut r, mut blocks) = (0u32, 0u32, Vec::new());
        for _ in 0..next(12) + 1 {
            let eq = next(8);
            let (ln, rn) = match next(3) {
                0 => (0, next(5) + 1),
                1 => (next(5) + 1, 0),
                _ => (next(5) + 1, next(5) + 1),
            };
            l += eq;
            r += eq;
            blocks.push(block(l..l + ln, r..r + rn));
            l += ln;
            r += rn;
        }
        // a long tail keeps the scroll tests away from the end-of-file clamp
        let tail = next(8) + 20;
        (l + tail, r + tail, blocks)
    }

    #[test]
    fn scrolling_down_never_moves_either_side_up_in_any_mode() {
        for seed in 0..200 {
            let (ll, rl, blocks) = random_blocks(seed);
            for (mode, collapse) in [ScrollMode::Shared, ScrollMode::Aligned, ScrollMode::Padded].into_iter().flat_map(|m| [(m, false), (m, true)]) {
                for lead in [Side::Left, Side::Right] {
                    let mut view = View::new(ll, rl, &blocks, 9.0);
                    view.set_collapsed(collapse);
                    view.set_mode(mode);
                    let mut prev = view.frame();
                    for _ in 0..200 {
                        view.scroll_by(0.37, lead);
                        let f = view.frame();
                        assert!(f.left_top >= prev.left_top - 1e-4 && f.right_top >= prev.right_top - 1e-4, "seed {seed} {mode:?} {lead:?}");
                        prev = f;
                    }
                }
            }
        }
    }

    #[test]
    fn scrolling_down_then_up_returns_to_the_same_place() {
        for seed in 0..200 {
            let (ll, rl, blocks) = random_blocks(seed);
            for (mode, collapse) in [ScrollMode::Shared, ScrollMode::Aligned, ScrollMode::Padded].into_iter().flat_map(|m| [(m, false), (m, true)]) {
                let mut view = View::new(ll, rl, &blocks, 9.0);
                view.set_collapsed(collapse);
                view.set_mode(mode);
                view.scroll_by(2.0, Side::Left);
                let start = view.frame();
                // Collapsing the tail can put this sample near the final display row.
                let available = match mode { ScrollMode::Aligned => View::last_top(view.left_lines) - view.left_top, _ => View::last_top(view.combined) - view.top };
                let delta = 5.5_f32.min(available);
                view.scroll_by(delta, Side::Left);
                view.scroll_by(-delta, Side::Left);
                let back = view.frame();
                assert!((start.left_top - back.left_top).abs() < 1e-3 && (start.right_top - back.right_top).abs() < 1e-3, "seed {seed} {mode:?}");
            }
        }
    }

    fn folded_row(rows: &[Row]) -> Option<(f32, Content)> {
        rows.iter().find(|r| matches!(r.content, Content::Folded { .. })).map(|r| (r.y, r.content))
    }

    #[test]
    fn single_line_changes_cannot_be_folded() {
        for mode in [ScrollMode::Shared, ScrollMode::Aligned, ScrollMode::Padded] {
            for (left, right) in [(2..3, 2..3), (2..2, 2..3), (2..3, 2..2)] {
                let mut view = View::new(10, 10, &[block(left, right)], 40.0);
                view.set_mode(mode);
                view.toggle_fold(0);
                assert!(!view.folded(0), "{mode:?}: a one-line change cannot be folded");
                assert!(!view.frame().connectors[0].foldable, "the gutter must not offer folding");
                assert_eq!(folded_row(&view.frame().left), None);
                assert_eq!(folded_row(&view.frame().right), None);
                view.fold_blocks(&[0]);
                assert!(!view.folded(0), "restoring folds must also skip one-line changes");
            }
        }
    }

    #[test]
    fn multiline_changes_keep_folding_available_when_folded() {
        for mode in [ScrollMode::Shared, ScrollMode::Aligned, ScrollMode::Padded] {
            for (left, right) in [(2..4, 2..3), (2..3, 2..4), (2..2, 2..4), (2..4, 2..2)] {
                let mut view = View::new(10, 10, &[block(left, right)], 40.0);
                view.set_mode(mode);
                assert!(view.frame().connectors[0].foldable);
                view.toggle_fold(0);
                assert!(view.folded(0));
                assert!(view.frame().connectors[0].foldable, "a collapsed multiline block still offers unfolding");
                view.toggle_fold(0);
                assert!(!view.folded(0));
                view.fold_blocks(&[0]);
                assert!(view.folded(0));
            }
        }
    }

    #[test]
    fn refinement_unfolds_a_block_that_shrinks_to_one_line() {
        let mut view = View::new(10, 10, &[block(2..5, 2..5)], 40.0);
        view.toggle_fold(0);
        view.apply_refinements(vec![(0, Refinement::Refined(block(3..4, 3..4)))]);
        assert!(!view.folded(0));
        assert_eq!(folded_row(&view.frame().left), None);
        assert!(!view.frame().connectors[0].foldable);
    }

    #[test]
    fn a_folded_change_block_is_one_row_on_both_sides() {
        let mut view = View::new(10, 12, &[block(2..5, 2..7)], 40.0);
        view.toggle_fold(0);
        let f = view.frame();
        assert_eq!(folded_row(&f.left), Some((2.0, Content::Folded { block: 0, left: 3, right: 5 })));
        assert_eq!(folded_row(&f.right), Some((2.0, Content::Folded { block: 0, left: 3, right: 5 })));
        assert_eq!(f.left[2].tint, Some((Kind::Modified, false)));
        // the lines after it move up; the Connector is a one-row band
        assert_eq!(f.left[3].content, Content::Line(5));
        assert_eq!(f.right[3].content, Content::Line(7));
        assert_eq!(f.connectors, vec![Connector { left: (2.0, 3.0), right: (2.0, 3.0), kind: Kind::Modified, ignored: false, block: 0, folded: true, foldable: true }]);
        view.toggle_fold(0);
        assert_eq!(folded_row(&view.frame().left), None);
        assert_eq!(view.frame().left[3].content, Content::Line(3));
    }

    #[test]
    fn next_and_previous_change_still_stop_at_folded_blocks() {
        let mut view = View::new(200, 200, &[block(20..22, 20..22), block(100..102, 100..102), block(180..182, 180..182)], 30.0);
        view.toggle_fold(1);
        assert!(view.step_change(true));
        assert!(view.step_change(true));
        assert_eq!(view.current_change(), Some(1));
        assert!(view.folded(1), "stepping onto it leaves it folded");
    }

    #[test]
    fn going_to_a_line_inside_a_folded_block_unfolds_it() {
        let mut view = View::new(100, 100, &[block(50..60, 50..60)], 30.0);
        view.toggle_fold(0);
        view.go_to_line(Side::Right, 55);
        assert!(!view.folded(0));
        assert!(view.frame().right.iter().any(|r| r.content == Content::Line(54)));
        view.toggle_fold(0);
        view.reveal(Side::Left, 57);
        assert!(!view.folded(0));
    }

    #[test]
    fn folding_works_with_collapsed_regions_and_padded_mode() {
        let mut view = View::new(100, 103, &[block(50..51, 50..54)], 40.0);
        view.set_collapsed(true);
        view.set_mode(ScrollMode::Padded);
        view.toggle_fold(0);
        let f = view.frame();
        let (yl, _) = folded_row(&f.left).unwrap();
        let (yr, _) = folded_row(&f.right).unwrap();
        assert_eq!(yl, yr, "lined up");
        assert!(!f.left.iter().any(|r| r.content == Content::Filler), "no Filler Rows for a folded block");
        view.unfold_all();
        assert_eq!(folded_row(&view.frame().left), None);
    }

    #[test]
    fn folds_survive_a_dropped_neighbour_during_refinement() {
        let mut view = View::new(20, 20, &[block(2..3, 2..3), block(8..10, 8..10)], 40.0);
        view.toggle_fold(1);
        view.apply_refinements(vec![(0, Refinement::Dropped)]);
        assert!(view.folded(0), "the folded block is now block 0");
        assert_eq!(view.folded_blocks(), vec![(8..10, 8..10)]);
    }

    #[test]
    fn caret_steps_over_collapsed_regions() {
        let mut view = View::new(100, 100, &[block(50..51, 50..51)], 40.0);
        view.set_collapsed(true);
        // rows: fold(0..46), 46..=54, fold(55..100)
        assert_eq!(view.step_line(Side::Left, 46, -1), 46, "nothing above the fold to land on");
        assert_eq!(view.step_line(Side::Left, 48, 1), 49);
        assert_eq!(view.step_line(Side::Left, 54, 1), 54);
        assert_eq!(view.step_line(Side::Right, 47, -20), 46);
        view.set_collapsed(false);
        assert_eq!(view.step_line(Side::Left, 46, -1), 45);
        assert_eq!(view.step_line(Side::Left, 98, 5), 99);
    }

    #[test]
    fn revealing_a_line_scrolls_just_enough_and_expands_its_region() {
        let mut view = View::new(200, 200, &[], 20.0);
        view.reveal(Side::Left, 25);
        let f = view.frame();
        assert_eq!(f.left.iter().find(|r| r.content == Content::Line(25)).map(|r| r.y), Some(19.0), "bottom row");
        view.reveal(Side::Left, 3);
        assert!((view.frame().left_top - 3.0).abs() < 1e-3, "top row");
        view.reveal(Side::Left, 10);
        assert!((view.frame().left_top - 3.0).abs() < 1e-3, "already visible: no scroll");

        let mut view = View::new(100, 100, &[block(50..51, 50..51)], 40.0);
        view.set_collapsed(true);
        view.reveal(Side::Right, 80);
        assert!(view.frame().right.iter().any(|r| r.content == Content::Line(80)));
    }

    #[test]
    fn the_change_block_under_a_line_gives_both_sides_ranges() {
        let view = View::new(10, 11, &[block(2..4, 2..5), block(7..7, 8..9)], 20.0);
        assert_eq!(view.block_at(Side::Left, 3), Some((2..4, 2..5)));
        assert_eq!(view.block_at(Side::Right, 8), Some((7..7, 8..9)));
        assert_eq!(view.block_at(Side::Left, 5), None);
    }

    #[test]
    fn the_unrefined_blocks_are_listed_anywhere_in_batches() {
        let blocks: Vec<_> = (0..300).map(|i| block(i * 10..i * 10 + 1, i * 10..i * 10 + 1)).collect();
        let mut view = View::new(3000, 3000, &blocks, 40.0);
        let first: Vec<usize> = view.unrefined_blocks(256).iter().map(|(i, _)| *i).collect();
        assert_eq!(first, (0..256).collect::<Vec<_>>(), "far from the viewport too, at most a batch");
        view.apply_refinements(first.iter().map(|&i| (i, Refinement::TooBig)).collect());
        assert_eq!(view.unrefined_blocks(256).len(), 44);
    }

    #[test]
    fn only_blocks_near_the_viewport_wait_for_refinement() {
        let blocks = [block(5..6, 5..6), block(500..501, 500..501)];
        for mode in [ScrollMode::Shared,ScrollMode::Aligned,ScrollMode::Padded] {
            let mut view = View::new(1000, 1000, &blocks, 40.0);
            view.start_in(mode);
            let want: Vec<usize> = view.blocks_to_refine().iter().map(|(i, _)| *i).collect();
            assert_eq!(want, vec![0],"{mode:?}");
            view.apply_refinements(vec![(0, Refinement::TooBig)]);
            assert!(view.blocks_to_refine().is_empty(), "a block is refined once");
            view.go_to_line(Side::Left, 501);
            let want: Vec<usize> = view.blocks_to_refine().iter().map(|(i, _)| *i).collect();
            assert_eq!(want, vec![1],"{mode:?}");
        }
    }

    #[test]
    #[ignore="release refinement polling with many offscreen change blocks"]
    fn refinement_polling_perf_5000_blocks() {
        let blocks:Vec<_>=(0..5000).map(|i|block(i*20..i*20+1,i*20..i*20+1)).collect();
        for mode in [ScrollMode::Shared,ScrollMode::Aligned,ScrollMode::Padded] {
            let mut view=View::new(100000,100000,&blocks,30.0);view.start_in(mode);view.go_to_line(Side::Left,50001);
            let candidates=view.blocks_to_refine();assert!(candidates.iter().any(|(i,_)|*i==2500));assert!(candidates.len()<=4);
            let mut samples=Vec::new();
            for _ in 0..30 {let start=std::time::Instant::now();for _ in 0..100 {std::hint::black_box(view.blocks_to_refine());}samples.push(start.elapsed().as_secs_f64()*1000.0/100.0);}
            crate::perf::record(&format!("view/refine-poll-5000-blocks/{mode:?}"),"ms",&samples);
        }
    }

    #[test]
    fn refined_blocks_bring_inline_changes_and_dropped_blocks_disappear() {
        let blocks = [block(1..2, 1..2), block(3..4, 3..4)];
        let mut view = View::new(6, 6, &blocks, 40.0);
        view.attach_line_lengths(&[5; 6], &[5; 6]);
        let refined = ChangeBlock { left: 1..2, right: 1..2, ignored: false, inline: Some(vec![lenz_engine::InlineChange { left: 1..3, right: 1..2 }]) };
        view.apply_refinements(vec![(0, Refinement::Refined(refined)), (1, Refinement::Dropped)]);
        let f = view.frame();
        assert_eq!(f.left[1].inline, vec![1..3]);
        assert_eq!(f.right[1].inline, vec![1..2]);
        assert_eq!(tint_of(&f.left, 3), None);
        assert_eq!(f.connectors.len(), 1);
        assert_eq!(view.change_count(), 1);
    }

    /// View-model half of the performance guard (the engine's is `cargo bench -p lenz-engine`): building the
    /// view for a 100k-line pair with Collapsed Regions on, then one scroll step plus a frame, as a paint does.
    /// Fails above twice its budget; `scripts/perf-check.sh` runs it.
    #[test]
    #[ignore = "timing: cargo test --release -p lenz view_model_perf -- --ignored --nocapture"]
    fn view_model_perf_on_a_100k_line_pair() {
        const BUILD: f64 = 1.2;
        const FRAME: f64 = 0.008;
        let blocks: Vec<ChangeBlock> = (0..2000u32).map(|k| block(k * 50 + 10..k * 50 + 12, k * 50 + 10..k * 50 + 13)).collect();
        let median = |mut v: Vec<f64>| {
            v.sort_by(f64::total_cmp);
            v[v.len() / 2]
        };
        let (mut builds, mut frames, mut unified) = (Vec::new(), Vec::new(), Vec::new());
        for _ in 0..7 {
            let t = std::time::Instant::now();
            let mut view = View::new(100_000, 102_000, &blocks, 50.0);
            view.attach_line_lengths(&vec![40; 100_000], &vec![40; 102_000]);
            view.set_collapsed(true);
            builds.push(t.elapsed().as_secs_f64() * 1e3);
            for mode in [ScrollMode::Shared, ScrollMode::Aligned, ScrollMode::Padded] {
                view.set_mode(mode);
                let t = std::time::Instant::now();
                for _ in 0..1000 {
                    view.scroll_by(3.0, Side::Left);
                    std::hint::black_box(view.frame());
                }
                frames.push(t.elapsed().as_secs_f64() * 1e3 / 1000.0);
            }
            view.set_mode(ScrollMode::Shared);
            view.scroll_by(f32::NEG_INFINITY, Side::Left);
            view.set_unified(true);
            let t = std::time::Instant::now();
            for _ in 0..1000 {
                view.scroll_by(3.0, Side::Left);
                std::hint::black_box(view.frame());
            }
            unified.push(t.elapsed().as_secs_f64() * 1e3 / 1000.0);
            view.set_unified(false);
        }
        crate::perf::record("view/build", "ms", &builds);
        crate::perf::record("view/scroll-layout", "ms", &frames);
        crate::perf::record("view/unified-scroll-layout", "ms", &unified);
        let unified = median(unified);
        eprintln!("view model, 100k lines: Unified scroll + frame {unified:.4} ms (budget {FRAME})");
        crate::perf::check(unified <= 2.0 * FRAME, "Unified frame exceeds budget");
        let (build, frame) = (median(builds), median(frames));
        eprintln!("view model, 100k lines: build {build:.2} ms (budget {BUILD}), scroll + frame {frame:.4} ms (budget {FRAME})");
        crate::perf::check(build <= 2.0 * BUILD, "view build exceeds budget");
        crate::perf::check(frame <= 2.0 * FRAME, "scroll layout exceeds budget");
    }

    const MODES: [ScrollMode; 4] = [ScrollMode::Shared, ScrollMode::Aligned, ScrollMode::Padded, ScrollMode::Independent];

    #[test]
    fn display_rows_match_display_row_in_one_pass() {
        let mut v = View::new(200, 230, &[block(10..10, 10..30), block(60..70, 80..85), block(150..151, 165..166)], 10.0);
        let check = |v: &View, what: &str| {
            for side in [Side::Left, Side::Right] {
                let lines: Vec<u32> = (0..=240).chain([5, 199, 0, 120]).collect();
                let one = lines.iter().map(|&l| v.display_row(side, l)).collect::<Vec<_>>();
                assert_eq!(v.display_rows(side, lines), one, "{what} {side:?}");
            }
        };
        check(&v, "plain");
        v.set_collapsed(true);
        check(&v, "collapsed");
        v.toggle_fold(1);
        check(&v, "folded");
        let stamp = v.layout_stamp();
        for mode in MODES {
            v.set_mode(mode);
            check(&v, &format!("{mode:?}"));
        }
        v.set_unified(true);
        check(&v, "unified");
        assert_ne!(v.layout_stamp(), stamp, "a new layout gets a new stamp");
    }

    #[test]
    fn scrollbar_rows_follow_collapsed_regions_and_filler_rows() {
        // the right Side inserts 20 lines at line 10
        let mut v = View::new(100, 120, &[block(10..10, 10..30)], 10.0);
        let totals = |v: &View| (v.scrollbar(Side::Left).total, v.scrollbar(Side::Right).total);
        assert_eq!(totals(&v), (100.0, 120.0));
        assert_eq!(v.scrollbar(Side::Left).visible, 10.0);
        v.set_mode(ScrollMode::Padded);
        assert_eq!(totals(&v), (120.0, 120.0), "Filler Rows count on the shorter Side");
        assert_eq!(v.display_row(Side::Left, 10), 30.0, "the line after the insertion sits below its Filler Rows");
        v.set_mode(ScrollMode::Shared);
        v.set_collapsed(true);
        // left: region, 6..10, 10..14, region; right: region, 6..10, the 20 inserted lines, 30..34, region
        assert_eq!(totals(&v), (10.0, 30.0));
        assert_eq!((v.display_row(Side::Left, 7), v.display_row(Side::Left, 50)), (2.0, 9.0), "a hidden line is its region's row");
        v.set_unified(true);
        assert_eq!(totals(&v), (30.0, 30.0), "Unified: one column of rows, whichever Side");
        assert_eq!((v.display_row(Side::Left, 12), v.display_row(Side::Right, 32)), (27.0, 27.0), "an unchanged line is one row");
        // a b c d against a B c x d: ' a', '-b', '+B', ' c', '+x', ' d'
        let mut v = View::new(4, 5, &[block(1..2, 1..2), block(3..3, 3..4)], 4.0);
        v.set_unified(true);
        assert_eq!(v.scrollbar(Side::Left).total, 6.0);
        let rows = [(Side::Left, 1), (Side::Right, 1), (Side::Right, 3), (Side::Left, 3)].map(|(side, line)| v.display_row(side, line));
        assert_eq!(rows, [1.0, 2.0, 4.0, 5.0]);
    }

    #[test]
    fn scroll_to_follows_each_scroll_mode() {
        // the right Side inserts 20 lines at line 10, so its row 50 faces the left Side's row 30
        let other_top = |mode, lead: Side, top: f64| {
            let mut v = View::new(100, 120, &[block(10..10, 10..30)], 10.0);
            v.start_in(mode);
            v.scroll_to(lead, top);
            assert!((v.scrollbar(lead).top - top).abs() < 1e-3, "{mode:?} {lead:?}: {:?}", v.scrollbar(lead));
            v.scrollbar(View::other(lead)).top
        };
        for (mode, left, right) in [(ScrollMode::Shared, 30.0, 50.0), (ScrollMode::Aligned, 30.0, 50.0), (ScrollMode::Padded, 50.0, 30.0), (ScrollMode::Independent, 0.0, 0.0)] {
            assert!((other_top(mode, Side::Right, 50.0) - left).abs() < 1e-3, "{mode:?}: left follows the right Side");
            assert!((other_top(mode, Side::Left, 30.0) - right).abs() < 1e-3, "{mode:?}: right follows the left Side");
        }
    }

    #[test]
    fn scroll_to_lands_in_one_call_over_dense_insertions_and_in_unified() {
        // the right Side inserts 9 lines after every 10th left line: 90 of its 190 rows are insertions
        let blocks: Vec<_> = (0..10).map(|k| block(10 * k + 5..10 * k + 5, 19 * k + 5..19 * k + 14)).collect();
        for mode in MODES {
            for (side, targets) in [(Side::Left, [3.0, 17.5, 42.0, 77.0]), (Side::Right, [3.0, 50.0, 121.5, 170.0])] {
                for top in targets {
                    let mut v = View::new(100, 190, &blocks, 10.0);
                    v.start_in(mode);
                    v.scroll_to(side, top);
                    let landed = v.scrollbar(side).top;
                    assert!((landed - top).abs() <= 1.0, "{mode:?} {side:?}: {top} landed at {landed}");
                }
            }
        }
        // Unified: one 100-line block is 200 rows, its deleted lines then its inserted ones
        for mode in [ScrollMode::Shared, ScrollMode::Padded] {
            let mut v = View::new(100, 100, &[block(0..100, 0..100)], 10.0);
            v.start_in(mode);
            v.set_unified(true);
            for top in [100.0, 37.0, 150.0] {
                v.scroll_to(Side::Right, top);
                assert!((v.scrollbar(Side::Right).top - top).abs() <= 1.0, "{mode:?}: {top} landed at {:?}", v.scrollbar(Side::Right));
            }
        }
    }

    /// Page `bar`'s pane down to its end, then up to 0, through `scroll_to`: every page must move the top.
    pub(crate) fn page_through(name: &str, mut bar: impl FnMut() -> Scrollbar, mut scroll_to: impl FnMut(f64)) {
        for down in [true, false] {
            for page in 0.. {
                let before = bar();
                let target = before.page(down);
                if target == before.top { break; }
                assert!(page <= (before.total / before.visible) as usize * 2 + 10, "{name}: paging {} never ends: {before:?}", if down { "down" } else { "up" });
                scroll_to(target);
                let after = bar().top;
                assert!(if down { after > before.top } else { after < before.top }, "{name}: {before:?} → {target} landed at {after}");
            }
            let end = bar();
            assert!(if down { end.top + end.visible >= end.total } else { end.top == 0.0 }, "{name}: stopped at {end:?}");
        }
    }

    #[test]
    fn unified_pages_past_a_long_insertion_in_aligned_and_independent() {
        // the insertion after line 50, then after line 50,000, where f32 steps are coarse
        for (mode, at) in [(ScrollMode::Aligned, 50), (ScrollMode::Independent, 50), (ScrollMode::Aligned, 50_000), (ScrollMode::Independent, 50_000)] {
            let v = std::cell::RefCell::new(View::new(at + 50, at + 150, &[block(at..at, at..at + 100)], 10.0));
            v.borrow_mut().start_in(mode);
            v.borrow_mut().set_unified(true);
            page_through(&format!("{mode:?} at {at}"), || v.borrow().scrollbar(Side::Right), |top| v.borrow_mut().scroll_to(Side::Right, top));
        }
    }

    #[test]
    fn a_top_past_the_end_keeps_the_thumb_at_the_track_end_and_pages_down_in_place() {
        for unified in [false, true] {
            for mode in MODES {
                // the last change sits at the anchor, so the top is past `total - visible`
                let mut v = View::new(100, 100, &[block(97..98, 97..98)], 10.0);
                v.start_in(mode);
                v.set_unified(unified);
                v.go_to_change(0);
                let bar = v.scrollbar(Side::Right);
                assert!(bar.page(true) >= bar.top, "{mode:?} unified={unified}: {bar:?}");
                let (start, size) = bar.thumb(100.0, 4.0).unwrap();
                assert!((start + size - 100.0).abs() < 1e-3, "{mode:?} unified={unified}: {bar:?}");
            }
        }
    }

    #[test]
    fn block_marks_carry_kind_and_current() {
        use crate::scrollbar::{Mark, MarkKind};
        let blocks = [block(20..20, 20..23), block(30..32, 33..33), block(40..41, 41..42)];
        // a tall viewport keeps every block below the anchor: none is current yet
        let mut v = View::new(60, 61, &blocks, 30.0);
        let mark = |row: f64, rows: f64, kind, target| Mark { row, rows, kind, target };
        assert_eq!(v.block_marks(Side::Left), [mark(20.0, 0.0, MarkKind::Added, 0), mark(30.0, 2.0, MarkKind::Deleted, 1), mark(40.0, 1.0, MarkKind::Modified, 2)]);
        assert_eq!(v.block_marks(Side::Right), [mark(20.0, 3.0, MarkKind::Added, 0), mark(33.0, 0.0, MarkKind::Deleted, 1), mark(41.0, 1.0, MarkKind::Modified, 2)]);
        v.go_to_change(1);
        let kinds = |v: &View, side| v.block_marks(side).iter().map(|m| m.kind).collect::<Vec<_>>();
        assert_eq!(kinds(&v, Side::Left), [MarkKind::Added, MarkKind::Current, MarkKind::Modified]);
        v.set_mode(ScrollMode::Padded);
        assert_eq!(v.block_marks(Side::Left), v.block_marks(Side::Right), "Padded: both Sides span each block's rows");
        assert_eq!(v.block_marks(Side::Left)[1], mark(33.0, 2.0, MarkKind::Current, 1));
    }

    #[test]
    fn go_to_change_makes_it_current_and_visible() {
        let blocks = [block(20..20, 20..23), block(30..32, 33..33), block(40..41, 41..42)];
        for mode in MODES {
            let mut v = View::new(60, 61, &blocks, 10.0);
            v.start_in(mode);
            v.go_to_change(2);
            assert_eq!(v.current_change(), Some(2), "{mode:?}");
            assert!(v.line_visible(Side::Left, 40) && v.line_visible(Side::Right, 41), "{mode:?}");
            v.go_to_change(3);
            assert_eq!(v.current_change(), Some(2), "{mode:?}: past the last block nothing moves");
        }
    }
}
