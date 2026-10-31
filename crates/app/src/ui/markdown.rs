//! Native Markdown geometry. Semantic selection and source mapping stay in the core.
use super::*;
use crate::markdown::SourcePos as _;
use crate::markdown::{
    BlockKind, MarkdownCommand, PresentationMode, Projection, RenderSelection, SourceAnchor,
    TableAlignment,
};
use gpui::RenderImage;
use lenz_gpui::markdown::{LIST_STEP, ReadingStyle, indent, quotes, sans_family, shape, shape_with};
use unicode_segmentation::UnicodeSegmentation;

// Wrapping queries the same native line many times. GPUI's helpers scan from the
// first glyph; index the runs once instead. Ordinary monotone runs borrow glyphs
// without per-glyph storage. Prefix maxima preserve first-match semantics for
// reordered glyphs, including the immediately preceding glyph used by nearest-x.
struct WrapRun<'a> {
    glyphs: &'a [gpui::ShapedGlyph],
    max_index: usize,
    max_x: Pixels,
    index_prefix: Vec<usize>,
    x_prefix: Vec<Pixels>,
}
pub(super) struct WrapLookup<'a> {
    line: &'a gpui::LineLayout,
    runs: Vec<WrapRun<'a>>,
}
impl<'a> WrapLookup<'a> {
    pub(super) fn new(line: &'a gpui::LineLayout) -> Self {
        let mut max_index = 0;
        let mut max_x = px(f32::NEG_INFINITY);
        let runs = line.runs.iter().filter(|r| !r.glyphs.is_empty()).map(|run| {
            let glyphs = run.glyphs.as_slice();
            let mut index_prefix = Vec::new();
            if glyphs.windows(2).any(|g| g[0].index > g[1].index) {
                let mut maximum = 0;
                index_prefix = glyphs.iter().map(|g| { maximum = maximum.max(g.index); maximum }).collect();
            }
            let mut x_prefix = Vec::new();
            if glyphs.windows(2).any(|g| g[0].position.x > g[1].position.x) {
                let mut maximum = px(f32::NEG_INFINITY);
                x_prefix = glyphs.iter().map(|g| { maximum = maximum.max(g.position.x); maximum }).collect();
            }
            max_index = max_index.max(index_prefix.last().copied().unwrap_or(glyphs.last().unwrap().index));
            max_x = max_x.max(x_prefix.last().copied().unwrap_or(glyphs.last().unwrap().position.x));
            WrapRun { glyphs, max_index, max_x, index_prefix, x_prefix }
        }).collect();
        Self { line, runs }
    }
    pub(super) fn x_for_index(&self, index: usize) -> Pixels {
        let run = self.runs.partition_point(|r| r.max_index < index);
        let Some(run) = self.runs.get(run) else { return self.line.width; };
        let at = if run.index_prefix.is_empty() { run.glyphs.partition_point(|g| g.index < index) }
                 else { run.index_prefix.partition_point(|i| *i < index) };
        run.glyphs[at].position.x
    }
    pub(super) fn closest_index_for_x(&self, x: Pixels) -> usize {
        let run_index = self.runs.partition_point(|r| r.max_x < x);
        let Some(run) = self.runs.get(run_index) else {
            return if self.line.len == 1 && x <= self.line.width / 2. { 0 } else { self.line.len };
        };
        let at = if run.x_prefix.is_empty() { run.glyphs.partition_point(|g| g.position.x < x) }
                 else { run.x_prefix.partition_point(|position| *position < x) };
        let glyph = &run.glyphs[at];
        let previous = if at > 0 { Some(&run.glyphs[at - 1]) }
                       else { run_index.checked_sub(1).and_then(|i| self.runs[i].glyphs.last()) };
        let (previous_index, previous_x) = previous.map_or((0, px(0.)), |g| (g.index, g.position.x));
        if glyph.position.x - x < x - previous_x { glyph.index } else { previous_index }
    }
}

struct Piece {
    /// The document the range is in (Inline mixes both).
    side: Side,
    /// Inline: an inserted or removed span.
    change: Option<Kind>,
    range: Range<usize>,
    x: f32,
    shaped: ShapedLine,
}
/// What a row shows besides its text pieces.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum RowKind {
    Text,
    /// A callout's title line (its pieces are the title text, if any); clicking it folds or unfolds the callout.
    CalloutTitle(crate::markdown::Callout),
    /// The "Properties" label above front matter rows.
    PropertiesLabel,
    /// A front matter row: key piece, then value pieces.
    Property,
    /// A local image, by block.
    Image(usize),
}
struct Row {
    kind: RowKind,
    /// The document `block` is in.
    side: Side,
    /// The change bar: added, removed or modified.
    change: Option<Kind>,
    y: f32,
    height: f32,
    block: Option<usize>,
    pieces: Vec<Piece>,
    marker: Option<usize>,
    decorations: Arc<[usize]>,
    first: bool,
    /// A table row's cell edges.
    grid: Vec<f32>,
}
pub(super) struct Layout {
    projection: Arc<Projection>,
    /// [`Adapter::syntax_rev`] when built: code colours arrive after the first layout.
    syntax_rev: u64,
    widths: [f32;2],
    mode: ScrollMode,
    palette: Arc<theme::Palette>,
    font: Font,
    style: Arc<ReadingStyle>,
    unfolded: Arc<HashSet<(usize, u32)>>,
    images: Arc<ImageMap>,
    /// One column with changes in place (pane 1 only).
    pub(super) inline: bool,
    /// Each side's reading column: left edge and width.
    column: [(f32, f32); 2],
    rows: [Vec<Row>; 2],
    heights: [f32; 2],
    widest: [f32; 2],
    text_rows: [Vec<(usize, usize)>; 2],
}
#[derive(Default)]
pub(super) struct Adapter {
    syntax: Option<CodeSyntax>,
    pub(super) layout: Option<Arc<Layout>>,
    top: [f32; 2],
    horizontal: [f32; 2],
    anchors: [Option<SourceAnchor>; 2],
    affinity: Option<(RenderSelection, usize)>,
    pub(super) dragging: bool,
    /// Pane widths a paint saw differ from the layout's, and since when: rebuilt once they hold for [`SETTLE`].
    resized: Option<([f32; 2], std::time::Instant)>,
    /// The one pending re-render after [`SETTLE`], while a drag holds the layout.
    settle: Option<gpui::Task<()>>,
    /// Bumped whenever code-block syntax colours arrive.
    syntax_rev: u64,
    /// The layout being shaped in the background, and what it is for: replaced when the wanted layout changes.
    building: Option<(Wanted, gpui::Task<()>)>,
    /// The reading style and the palette and code font size it was made for.
    reading: Option<(Arc<theme::Palette>, f32, Arc<ReadingStyle>)>,
    /// Folded callouts the user opened, by side and callout id.
    unfolded: Arc<HashSet<(usize, u32)>>,
}

/// What a layout is built from. A layout of another projection of the same pair (an option change) can stand in
/// on screen while its replacement is shaped; one of another pair cannot.
#[derive(Clone)]
struct Wanted {
    projection: Arc<Projection>,
    widths: [f32; 2],
    mode: ScrollMode,
    palette: Arc<theme::Palette>,
    font: Font,
    style: Arc<ReadingStyle>,
    unfolded: Arc<HashSet<(usize, u32)>>,
    images: Arc<ImageMap>,
    inline: bool,
    syntax: Arc<Syntax>,
    syntax_rev: u64,
}
impl Wanted {
    fn built(&self, l: &Layout) -> bool {
        Arc::ptr_eq(&l.projection, &self.projection) && l.widths == self.widths && l.mode == self.mode
            && Arc::ptr_eq(&l.palette, &self.palette) && l.font == self.font && l.syntax_rev == self.syntax_rev
            && Arc::ptr_eq(&l.style, &self.style) && Arc::ptr_eq(&l.unfolded, &self.unfolded)
            && Arc::ptr_eq(&l.images, &self.images) && l.inline == self.inline
    }
    fn same(&self, other: &Wanted) -> bool {
        Arc::ptr_eq(&self.projection, &other.projection) && self.widths == other.widths && self.mode == other.mode
            && Arc::ptr_eq(&self.palette, &other.palette) && self.font == other.font && self.syntax_rev == other.syntax_rev
            && Arc::ptr_eq(&self.style, &other.style) && Arc::ptr_eq(&self.unfolded, &other.unfolded)
            && Arc::ptr_eq(&self.images, &other.images) && self.inline == other.inline
    }
    fn build(&self, text_system: &gpui::WindowTextSystem) -> Layout {
        Layout::new(self, text_system)
    }
}
/// How long pane widths must hold before a paint rebuilds the layout: reshaping a large document takes
/// 50–150 ms, too long to repeat every frame of a window-resize drag.
const SETTLE: std::time::Duration = std::time::Duration::from_millis(100);
type Syntax = [HashMap<usize, Vec<(Range<usize>, Capture)>>; 2];
struct CodeSyntax {
    pair: std::sync::Weak<crate::markdown::ParsedPair>,
    _token: Arc<()>,
    job: Option<std::sync::mpsc::Receiver<Option<Syntax>>>,
    parsed: Arc<Syntax>,
    task: Option<gpui::Task<()>>,
}
impl CodeSyntax {
    fn new(pair: Arc<crate::markdown::ParsedPair>) -> Self {
        let token = Arc::new(());
        let weak = Arc::downgrade(&token);
        let identity = Arc::downgrade(&pair);
        let has_code = pair.sides.iter().any(|d| {
            d.blocks
                .iter()
                .any(|b| matches!(b.kind, BlockKind::Code { language: Some(_) }))
        });
        let job = has_code.then(|| {
            crate::work::submit(weak.clone(), move || {
                std::array::from_fn(|i| {
                    let cancel = weak.clone();
                    crate::markdown::code_syntax(&pair.sides[i], move || cancel.strong_count() > 0)
                })
            })
        });
        Self {
            pair: identity,
            _token: token,
            job,
            parsed: Arc::new(Default::default()),
            task: None,
        }
    }
    fn poll(&mut self) -> bool {
        let Some(job) = &self.job else {
            return false;
        };
        match job.try_recv() {
            Ok(result) => {
                self.parsed = Arc::new(result.unwrap_or_default());
                self.job = None;
                true
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.job = None;
                false
            }
            Err(_) => false,
        }
    }
}
pub(super) struct Frame {
    layout: Arc<Layout>,
    top: [f32; 2],
    horizontal: [f32; 2],
    selection: Option<RenderSelection>,
    affinity: Option<(RenderSelection, usize)>,
    find: Option<(SourceAnchor, Range<usize>, Vec<Range<usize>>)>,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum FillKind {
    ChangeBar,
    /// Code block and callout backgrounds.
    Block,
    /// Tags and inline code.
    Pill,
    /// Inline: behind an inserted or removed span.
    Change,
    /// Quote bars, list guides, table borders and rules.
    Line,
}
#[derive(Clone, Copy, Debug)]
pub(super) struct Fill {
    /// What the fill is for; only tests ask.
    #[cfg_attr(not(all(test, feature = "gui-test")), allow(dead_code))]
    pub kind: FillKind,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub color: u32,
    pub radius: f32,
    /// An outline in `color` instead of a fill: `Some(true)` dashed.
    pub border: Option<bool>,
}
fn rounded(window: &mut Window, x: f32, y: f32, w: f32, h: f32, radius: f32, color: u32) {
    window.paint_quad(quad(
        Bounds { origin: point(px(x), px(y)), size: size(px(w.max(0.)), px(h.max(0.))) },
        px(radius),
        Hsla::from(rgb(color)),
        px(0.),
        gpui::transparent_black(),
        Default::default(),
    ));
}
#[allow(clippy::too_many_arguments)]
fn outline(window: &mut Window, x: f32, y: f32, w: f32, h: f32, radius: f32, color: u32, dashed: bool) {
    window.paint_quad(quad(
        Bounds { origin: point(px(x), px(y)), size: size(px(w.max(0.)), px(h.max(0.))) },
        px(radius),
        gpui::transparent_black(),
        px(1.5),
        Hsla::from(rgb(color)),
        if dashed { gpui::BorderStyle::Dashed } else { gpui::BorderStyle::Solid },
    ));
}
fn shape_label(window: &Window, font: &Font, size: f32, label: &str, color: u32) -> ShapedLine {
    let run = TextRun { len: label.len(), font: font.clone(), color: rgb(color).into(), background_color: None, underline: None, strikethrough: None };
    window.text_system().shape_line(label.to_string().into(), px(size), &[run], None)
}
/// Decoded local images by side and block.
/// A decoded image and its size in pixels.
pub(super) type Decoded = (Arc<RenderImage>, [u32; 2]);
pub(super) type ImageMap = [HashMap<usize, Decoded>; 2];
use crate::markdown::IMAGE_BYTES;
#[derive(Default)]
pub(super) struct ImageCache {
    /// The document the images are for.
    pair: Option<std::sync::Weak<crate::markdown::ParsedPair>>,
    /// Each side's image blocks and the files they show.
    blocks: [Vec<(usize, PathBuf)>; 2],
    /// Decoded images; `None` while decoding or when an image could not be shown.
    loaded: HashMap<PathBuf, Option<Decoded>>,
    bytes: usize,
    pending: usize,
    /// Bumped whenever the cache is reset, so a decode started for an earlier document is ignored.
    generation: u64,
    shown: Arc<ImageMap>,
    /// Images to remove from the GPU atlas at the next frame.
    stale: Vec<Arc<RenderImage>>,
}
/// A local image file as a texture and its size; `None` when it is not a readable image.
fn decode_image(path: &Path) -> Option<Decoded> {
    let pixels = crate::markdown::decode_image(path)?;
    let size = [pixels.width(), pixels.height()];
    Some((lenz_gpui::image::texture(pixels), size))
}
fn pane(i: usize) -> Side {
    if i == 0 { Side::Left } else { Side::Right }
}
// Native shaping needs both semantic ranges and the active typography/palette.
/// Inline spans of one block, placed left to right and wrapped at word boundaries into rows of pane `pane`.
/// Spans of the block's own side start at `from` (after a callout title).
/// Slicing: offsets come from span ranges, `find('\n')`, char-boundary checks and grapheme indices.
#[allow(clippy::too_many_arguments)]
fn flow(
    l: &mut Layout,
    pane: usize,
    side: usize,
    b: usize,
    spans: &[crate::markdown::InlineSpan],
    from: usize,
    indent: f32,
    right: f32,
    fs: f32,
    height: f32,
    text_system: &gpui::WindowTextSystem,
) {
    let projection = l.projection.clone();
    let style = l.style.clone();
    let palette = l.palette.clone();
    let mut pieces: Vec<Piece> = Vec::new();
    let mut x = indent;
    let flush = |l: &mut Layout, pieces: &mut Vec<Piece>| {
        if pieces.is_empty() {
            return;
        }
        let end = pieces.last().map_or(indent, |p| p.x + f32::from(p.shaped.width));
        l.widest[pane] = l.widest[pane].max(end + 16.);
        l.rows[pane].push(Row {
            side: if side == 0 { Side::Left } else { Side::Right },
            change: None,
            kind: RowKind::Text,
            y: l.heights[pane],
            height,
            block: Some(b),
            pieces: std::mem::take(pieces),
            marker: None,
            decorations: Arc::from([]),
            first: false,
            grid: Vec::new(),
        });
        l.heights[pane] += height;
    };
    for span in spans {
        let doc = &projection.pair.sides[span.side as usize];
        let own = span.side as usize == side;
        let index = if own { b } else { doc.blocks.partition_point(|x| x.text.end <= span.text.start) };
        let Some(block) = doc.blocks.get(index) else { continue };
        let mut range = span.text.clone();
        if own {
            range.start = range.start.max(from);
        }
        let struck = span.change == Some(Kind::Deleted);
        let mut at = range.start;
        while at < range.end {
            let line_end = doc.text[at..range.end].find('\n').map_or(range.end, |n| at + n);
            let mut a = at;
            while a < line_end {
                // changed words take their change's colour, and two different changes don't touch
                let color = span.change.map(|k| theme::mix(style.text, if k == Kind::Deleted { style.removed } else { style.added }, 0.5));
                // room for the outlines: a gap wherever changed and unchanged text meet
                if a == range.start && pieces.last().is_some_and(|p| p.change != span.change) {
                    x += 5.;
                }
                let shaped = shape_with(doc, block, a..line_end, fs, &style, &palette, text_system, &[], color, struck);
                let available = right - x;
                if f32::from(shaped.width) <= available {
                    let width = f32::from(shaped.width);
                    pieces.push(Piece { side: span.side, change: span.change, range: a..line_end, x, shaped });
                    x += width;
                    a = line_end;
                    continue;
                }
                // cut at the last word boundary that fits
                let text = &doc.text[a..line_end];
                let lookup = WrapLookup::new(&shaped);
                let mut stop = lookup.closest_index_for_x(px(available.max(0.))).min(text.len());
                while stop > 0 && !text.is_char_boundary(stop) {
                    stop -= 1;
                }
                while stop > 0 && f32::from(lookup.x_for_index(stop)) > available {
                    stop = text[..stop].grapheme_indices(true).next_back().map_or(0, |(i, _)| i);
                }
                if stop < text.len() {
                    match text[..stop].rfind(' ') {
                        Some(space) if space > 0 => stop = space + 1,
                        // no word boundary fits after other pieces: start the word on the next row
                        _ if x > indent => stop = 0,
                        _ => {}
                    }
                }
                if stop == 0 {
                    if !pieces.is_empty() {
                        flush(l, &mut pieces);
                        x = indent;
                        continue;
                    }
                    stop = text.graphemes(true).next().map_or(text.len(), str::len);
                }
                let shaped = shape_with(doc, block, a..a + stop, fs, &style, &palette, text_system, &[], color, struck);
                pieces.push(Piece { side: span.side, change: span.change, range: a..a + stop, x, shaped });
                flush(l, &mut pieces);
                x = indent;
                a += stop;
            }
            if line_end < range.end {
                flush(l, &mut pieces);
                x = indent;
                at = line_end + 1;
            } else {
                at = line_end;
            }
        }
    }
    flush(l, &mut pieces);
}
impl Layout {
    fn new(wanted: &Wanted, text_system: &gpui::WindowTextSystem) -> Self {
        let Wanted { projection, widths, mode, palette, font, style, unfolded, images, inline, syntax, syntax_rev } = wanted.clone();
        let syntax = &*syntax;
        let mut result = Self {
            projection,
            widths,
            syntax_rev,
            mode,
            palette,
            font,
            style,
            unfolded,
            images,
            inline,
            column: [(0., 0.); 2],
            rows: Default::default(),
            heights: [0.; 2],
            widest: [0.; 2],
            text_rows: Default::default(),
        };
        let mut tables: [Vec<Option<Vec<f32>>>; 2] = Default::default();
        let mut decorations: [Vec<Vec<usize>>; 2] = Default::default();
        for i in 0..2 {
            let doc = &result.projection.pair.sides[i];
            tables[i].resize_with(doc.blocks.len(), || None);
            decorations[i].resize_with(doc.blocks.len(), Vec::new);
            let mut b = 0;
            while b < doc.blocks.len() {
                if !matches!(doc.blocks[b].kind, BlockKind::TableRow { .. }) {
                    b += 1;
                    continue;
                }
                let start = b;
                let mut widths = Vec::<f32>::new();
                while let Some(block) = doc.blocks.get(b)
                    && let BlockKind::TableRow { cells, .. } = &block.kind
                {
                    widths.resize(widths.len().max(cells.len()), 0.);
                    for (j, cell) in cells.iter().enumerate() {
                        widths[j] = widths[j].max(f32::from(
                            shape(
                                doc,
                                block,
                                cell.clone(),
                                result.style.size * 0.93,
                                &result.style,
                                &result.palette,
                                text_system,
                                &[],
                                None,
                            )
                            .width,
                        ));
                    }
                    b += 1;
                }
                for table in &mut tables[i][start..b] {
                    *table = Some(widths.clone());
                }
            }
        }
        for (d, decoration) in result
            .projection
            .decorations
            .iter()
            .enumerate()
            .filter(|(_, d)| !d.markup)
        {
            let i = decoration.side as usize;
            let blocks = &result.projection.pair.sides[i].blocks;
            let start = blocks.partition_point(|b| b.text.end <= decoration.text.start);
            for (b, block) in blocks
                .iter()
                .enumerate()
                .skip(start)
                .take_while(|(_, b)| b.text.start < decoration.text.end)
            {
                if block.text.end > decoration.text.start {
                    decorations[i][b].push(d);
                }
            }
        }
        let decorations = decorations.map(|side| {
            side.into_iter()
                .map(Arc::<[usize]>::from)
                .collect::<Vec<_>>()
        });
        let markers: HashMap<_, _> = result
            .projection
            .decorations
            .iter()
            .enumerate()
            .filter(|(_, d)| d.markup)
            .map(|(index, d)| ((d.hunk, d.side as usize), index))
            .collect();
        let style = result.style.clone();
        result.column = widths.map(|width| {
            let column = style.column.min((width - 48.).max(1.));
            (((width - column) / 2.).max(24.), column)
        });
        // the previous block on each side, for spacing; callouts whose title is laid out, and the folded ones
        // ponytail: a block knows only its innermost callout, so folding an outer callout leaves nested ones open
        let mut previous: [Option<(usize, usize)>; 2] = [None; 2];
        let mut titled: [HashSet<u32>; 2] = Default::default();
        let mut folded: [HashSet<u32>; 2] = Default::default();
        // a callout with changes inside starts open, so no change is hidden
        let changed: [HashSet<u32>; 2] = std::array::from_fn(|i| {
            result.projection.pair.sides[i]
                .blocks
                .iter()
                .zip(&decorations[i])
                .filter(|(_, d)| !d.is_empty())
                .filter_map(|(b, _)| b.callout.map(|c| c.id))
                .collect()
        });
        // Each item lays out one block (whole, or as Inline spans) into a pane, or a group's change markers.
        enum Item<'a> {
            Block { pane: usize, side: usize, block: usize, spans: Option<&'a [crate::markdown::InlineSpan]>, change: Option<Kind> },
            Markers { pane: usize, group: usize },
        }
        let projection = result.projection.clone();
        let mut items = Vec::new();
        if inline {
            let mut at = 0;
            for (g, _) in projection.groups.iter().enumerate() {
                while let Some(ib) = projection.inline.get(at).filter(|ib| ib.group == g) {
                    // removed blocks and interleaved ones flow as spans; others keep their own layout
                    let flows = ib.spans.len() > 1 || ib.change == Some(Kind::Deleted);
                    items.push(Item::Block { pane: 1, side: ib.side as usize, block: ib.block, spans: flows.then_some(&ib.spans[..]), change: ib.change });
                    at += 1;
                }
                items.push(Item::Markers { pane: 1, group: g });
            }
        } else {
            for (g, group) in projection.groups.iter().enumerate() {
                for (i, side) in decorations.iter().enumerate() {
                    items.extend(group.blocks[i].clone().map(|b| Item::Block {
                        pane: i,
                        side: i,
                        block: b,
                        spans: None,
                        // a block added or removed whole has its bar alone, in that color
                        change: projection.whole_change(if i == 0 { Side::Left } else { Side::Right }, b)
                            .or_else(|| side[b].first().map(|&d| projection.decorations[d].kind)),
                    }));
                    items.push(Item::Markers { pane: i, group: g });
                }
            }
        }
        for item in items {
            let (i, s, b, spans, change) = match item {
                Item::Markers { pane, group } => {
                    let group = &projection.groups[group];
                    for &hunk in &group.hunks {
                        // Inline shows each markup marker once: the new side's, else the old side's
                        let marker = if inline {
                            markers.get(&(hunk, 1)).or_else(|| markers.get(&(hunk, 0))).copied()
                        } else {
                            markers.get(&(hunk, pane)).copied()
                        };
                        if let Some(d) = marker {
                            result.rows[pane].push(Row {
                                side: projection.decorations[d].side,
                                change: None,
                                kind: RowKind::Text,
                                y: result.heights[pane],
                                height: line_h() + 6.,
                                block: None,
                                pieces: Vec::new(),
                                marker: Some(d),
                                decorations: Arc::from([]),
                                first: true,
                                grid: Vec::new(),
                            });
                            result.heights[pane] += line_h() + 6.;
                        }
                    }
                    // side by side: both panes end each group at the same height unless scrolled apart
                    if !inline && pane == 1 && !mode.separate_tops() {
                        let h = result.heights[0].max(result.heights[1]);
                        result.heights = [h; 2];
                    }
                    continue;
                }
                Item::Block { pane, side, block, spans, change } => (pane, side, block, spans, change),
            };
            let sd = if s == 0 { Side::Left } else { Side::Right };
            let (left, column) = result.column[i];
            let doc = &projection.pair.sides[s];
            let rows_before = result.rows[i].len();
            'body: {
                    let block = &doc.blocks[b];
                    let captures = syntax[s].get(&b).map_or(&[][..], Vec::as_slice);
                    if block.callout.is_some_and(|c| folded[s].contains(&c.id)) {
                        break 'body;
                    }
                    let pad = if block.callout.is_some() { style.size } else { 0. };
                    let code_pad = if matches!(block.kind, BlockKind::Code { .. }) { 12. } else { 0. };
                    let indent = indent(&style, left, block) + code_pad;
                    let right = left + column - pad - code_pad;
                    let fs = match block.kind {
                        BlockKind::Heading(n) => style.heading[usize::from(n.clamp(1, 6)) - 1],
                        BlockKind::Code { .. } | BlockKind::Literal => style.size * 0.87,
                        BlockKind::Footnote { .. } => style.size * 0.88,
                        BlockKind::Properties => style.size * 0.9,
                        BlockKind::TableRow { .. } => style.size * 0.93,
                        _ => style.size,
                    };
                    let height = match block.kind {
                        BlockKind::Heading(_) => fs * 1.3,
                        BlockKind::Code { .. } | BlockKind::Literal => fs * 1.55,
                        BlockKind::TableRow { .. } => fs * 2.1,
                        _ => fs * 1.6,
                    };
                    result.heights[i] +=
                        style.space_before(&block.kind, previous[i].map(|(ps, p)| &projection.pair.sides[ps].blocks[p].kind)) + code_pad;
                    previous[i] = Some((s, b));
                    let mut text = block.text.clone();
                    if matches!(block.kind, BlockKind::Code { .. }) && doc.text[text.clone()].ends_with('\n') {
                        text.end -= 1;
                    }
                    if let Some(c) = block.callout
                        && titled[s].insert(c.id)
                    {
                        // the title is the block's first line when its runs say so; else the kind's label is drawn
                        let has_title = block.runs.first().is_some_and(|r| r.style.callout_title);
                        let end = doc.text[text.clone()].find('\n').map_or(text.end, |n| text.start + n);
                        let pieces = if has_title {
                            let shaped = shape(doc, block, text.start..end, style.size, &style, &result.palette, text_system, &[], None);
                            vec![Piece { side: sd, change: None, range: text.start..end, x: indent + style.size * 1.5, shaped }]
                        } else {
                            Vec::new()
                        };
                        result.heights[i] += style.size * 0.5;
                        result.rows[i].push(Row {
                            side: sd,
                            change: None,
                            kind: RowKind::CalloutTitle(c),
                            y: result.heights[i],
                            height: style.line,
                            block: Some(b),
                            pieces,
                            marker: None,
                            decorations: decorations[s][b].clone(),
                            first: true,
                            grid: Vec::new(),
                        });
                        result.heights[i] += style.line;
                        // a click inverts the starting state: folded for `-` unless it holds changes, else open
                        if c.shut(changed[s].contains(&c.id), result.unfolded.contains(&(s, c.id))) {
                            folded[s].insert(c.id);
                            break 'body;
                        }
                        if has_title {
                            text.start = (end + 1).min(text.end);
                        }
                    }
                    if let Some((_, [w, h])) = result.images[s].get(&b).filter(|_| spans.is_none()) {
                        let width = (*w as f32).min(right - indent).max(1.);
                        let height = width * *h as f32 / (*w).max(1) as f32;
                        result.rows[i].push(Row {
                            side: sd,
                            change: None,
                            kind: RowKind::Image(b),
                            y: result.heights[i],
                            height,
                            block: Some(b),
                            pieces: Vec::new(),
                            marker: None,
                            decorations: decorations[s][b].clone(),
                            first: true,
                            grid: Vec::new(),
                        });
                        result.heights[i] += height;
                        break 'body;
                    }
                    if let BlockKind::TableRow {
                        cells, alignment, ..
                    } = &block.kind
                        && spans.is_none()
                    {
                        let widths = tables[s][b].as_ref().unwrap();
                        let cell_pad = fs * 0.8;
                        let mut x = indent;
                        let mut grid = vec![x];
                        let pieces = cells
                            .iter()
                            .enumerate()
                            .map(|(j, r)| {
                                let shaped = shape(doc, block, r.clone(), fs, &style, &result.palette, text_system, captures, None);
                                let extra = widths[j] - f32::from(shaped.width);
                                let before = match alignment.get(j) {
                                    Some(TableAlignment::Right) => extra,
                                    Some(TableAlignment::Center) => extra / 2.,
                                    _ => 0.,
                                };
                                let piece = Piece { side: sd, change: None, range: r.clone(), x: x + cell_pad + before, shaped };
                                x += widths[j] + 2. * cell_pad;
                                grid.push(x);
                                piece
                            })
                            .collect();
                        result.widest[i] = result.widest[i].max(x + 16.);
                        result.rows[i].push(Row {
                            side: sd,
                            change: None,
                            kind: RowKind::Text,
                            y: result.heights[i],
                            height,
                            block: Some(b),
                            pieces,
                            marker: None,
                            decorations: decorations[s][b].clone(),
                            first: true,
                            grid,
                        });
                        result.heights[i] += height;
                        break 'body;
                    }
                    // Slicing: line offsets come from splitting on `\n` and `find(':')`, so char boundaries.
                    if matches!(block.kind, BlockKind::Properties)
                        && spans.is_none()
                        && crate::markdown::properties(&doc.text[text.clone()]).is_some()
                    {
                        result.rows[i].push(Row {
                            side: sd,
                            change: None,
                            kind: RowKind::PropertiesLabel,
                            y: result.heights[i],
                            height: fs * 1.6,
                            block: Some(b),
                            pieces: Vec::new(),
                            marker: None,
                            decorations: decorations[s][b].clone(),
                            first: true,
                            grid: Vec::new(),
                        });
                        result.heights[i] += fs * 1.6;
                        let mut at = text.start;
                        for line in doc.text[text.clone()].split('\n') {
                            let start = at;
                            at += line.len() + 1;
                            let trimmed = line.trim_start();
                            if trimmed.is_empty() || trimmed.starts_with('#') {
                                continue;
                            }
                            let lead = start + line.len() - trimmed.len();
                            if let Some(item) = trimmed.strip_prefix("- ")
                                && lead > start
                                && let Some(row) = result.rows[i].last_mut().filter(|r| r.kind == RowKind::Property)
                            {
                                let range = lead + 2..lead + 2 + item.trim_end().len();
                                let x = row.pieces.last().map_or(indent + 130., |p| p.x + f32::from(p.shaped.width) + 14.);
                                let tags = row.pieces.first().is_some_and(|k| doc.text[k.range.clone()].trim() == "tags");
                                let shaped = shape(doc, block, range.clone(), fs, &style, &result.palette, text_system, &[], tags.then_some(style.tag_fg));
                                row.pieces.push(Piece { side: sd, change: None, range, x, shaped });
                                continue;
                            }
                            let Some(colon) = line.find(':') else { continue };
                            let key = start..start + colon;
                            let value = line[colon + 1..].trim();
                            let mut pieces = vec![Piece {
                                side: sd,
                                change: None,
                                range: key.clone(),
                                x: indent,
                                shaped: shape(doc, block, key, fs, &style, &result.palette, text_system, &[], Some(style.muted)),
                            }];
                            if !value.is_empty() {
                                let from = start + colon + 1 + (line[colon + 1..].len() - line[colon + 1..].trim_start().len());
                                let range = from..from + value.len();
                                let tags = line[..colon].trim() == "tags";
                                let shaped = shape(doc, block, range.clone(), fs, &style, &result.palette, text_system, &[], tags.then_some(style.tag_fg));
                                pieces.push(Piece { side: sd, change: None, range, x: indent + 130., shaped });
                            }
                            result.rows[i].push(Row {
                                side: sd,
                            change: None,
                            kind: RowKind::Property,
                                y: result.heights[i],
                                height: fs * 1.6,
                                block: Some(b),
                                pieces,
                                marker: None,
                                decorations: decorations[s][b].clone(),
                                first: false,
                                grid: Vec::new(),
                            });
                            result.heights[i] += fs * 1.6;
                        }
                        break 'body;
                    }
                    if let Some(spans) = spans {
                        let rows_before = result.rows[i].len();
                        flow(&mut result, i, s, b, spans, text.start, indent, right, fs, height, text_system);
                        let first = rows_before;
                        for row in &mut result.rows[i][first..] {
                            row.first = false;
                        }
                        if let Some(row) = result.rows[i].get_mut(first) {
                            row.first = true;
                        }
                        break 'body;
                    }
                    let mut ranges = Vec::new();
                    let mut at = text.start;
                    let available = (right - indent).max(1.);
                    for line in doc.text[text.clone()].split('\n') {
                        let end = at + line.len();
                        let shaped = shape(doc, block, at..end, fs, &style, &result.palette, text_system, captures, None);
                        let nowrap =
                            matches!(block.kind, BlockKind::Code { .. } | BlockKind::Literal | BlockKind::Properties);
                        if nowrap || f32::from(shaped.width) <= available {
                            // one row: keep the shaped line rather than shaping it again
                            ranges.push((at..end, Some(shaped)));
                        } else {
                            let lookup = WrapLookup::new(&shaped);
                            let mut start = 0;
                            while start < line.len() {
                                let x = f32::from(lookup.x_for_index(start));
                                let mut stop =
                                    lookup.closest_index_for_x(px(x + available)).max(start);
                                while stop > start
                                    && f32::from(lookup.x_for_index(stop)) - x > available
                                {
                                    stop = line[..stop]
                                        .grapheme_indices(true)
                                        .next_back()
                                        .map_or(start, |(i, _)| i);
                                }
                                if stop <= start {
                                    stop =
                                        start + line[start..].graphemes(true).next().unwrap().len();
                                }
                                if stop < line.len()
                                    && let Some(space) = line[start..stop].rfind(' ')
                                    && space > 0
                                {
                                    stop = start + space + 1;
                                }
                                ranges.push((at + start..at + stop, None));
                                start = stop;
                            }
                        }
                        at = end + 1;
                    }
                    for (row, (r, shaped)) in ranges.into_iter().enumerate() {
                        let shaped = shaped.unwrap_or_else(|| {
                            shape(doc, block, r.clone(), fs, &style, &result.palette, text_system, captures, None)
                        });
                        result.widest[i] =
                            result.widest[i].max(indent + f32::from(shaped.width) + 16.);
                        result.rows[i].push(Row {
                            side: sd,
                            change: None,
                            kind: RowKind::Text,
                            y: result.heights[i],
                            height,
                            block: Some(b),
                            pieces: vec![Piece {
                                side: sd,
                                change: None,
                                range: r,
                                x: indent,
                                shaped,
                            }],
                            marker: None,
                            decorations: decorations[s][b].clone(),
                            first: row == 0,
                            grid: Vec::new(),
                        });
                        result.heights[i] += height;
                    }
                    result.heights[i] += code_pad;
            }
            // every row of the item carries its side and change bar; Inline marks spans, not whole rows
            let whole = !inline && projection.whole_change(sd, b).is_some();
            for row in &mut result.rows[i][rows_before..] {
                row.side = sd;
                row.change = change;
                if inline || whole {
                    row.decorations = Arc::from([]);
                }
            }
        }
        // per document: where its text is, in the pane that shows it
        for pane in 0..2 {
            for (row, r) in result.rows[pane].iter().enumerate() {
                for p in &r.pieces {
                    result.text_rows[p.side as usize].push((p.range.end, row));
                }
            }
        }
        for rows in &mut result.text_rows {
            rows.sort_by_key(|(end, _)| *end);
        }
        result
    }
    /// The pane showing document `side`: Inline shows both in pane 1.
    fn pane_of(&self, side: usize) -> usize {
        if self.inline { 1 } else { side }
    }
    fn row_at(&self, i: usize, y: f32) -> usize {
        self.rows[i]
            .partition_point(|r| r.y + r.height <= y)
            .min(self.rows[i].len().saturating_sub(1))
    }
    /// The row showing `byte` of document `side`, in [`Self::pane_of`] that side.
    fn text_row(&self, side: usize, byte: usize) -> usize {
        let index = self.text_rows[side].partition_point(|(end, _)| *end < byte);
        self.text_rows[side]
            .get(index)
            .map_or(self.rows[self.pane_of(side)].len().saturating_sub(1), |(_, row)| *row)
    }
    fn anchor_y(&self, anchor: SourceAnchor) -> f32 {
        let side = anchor.side as usize;
        let i = self.pane_of(side);
        if let Some(row) = self.rows[i].iter().find(|r| r.marker.is_some_and(|d| self.projection.decorations[d].side == anchor.side && self.projection.decorations[d].source.start == anchor.byte)) {
            return row.y;
        }
        if let Some(byte) = self.projection.source_point(anchor) {
            let at = self.text_rows[side].partition_point(|(end, _)| *end <= byte);
            let mut row = self.text_rows[side].get(at).map_or(self.rows[i].len().saturating_sub(1), |(_, row)| *row);
            // a block's label rows (Properties, a callout's kind) come before its first text
            while row > 0
                && let (Some(a), Some(b)) = (self.rows[i].get(row - 1), self.rows[i].get(row))
                && a.block == b.block
                && a.pieces.is_empty()
            {
                row -= 1;
            }
            return self.rows[i].get(row).map_or(0., |r| r.y);
        }
        self.rows[i]
            .iter()
            .find(|r| {
                if let Some(d) = r.marker {
                    return self.projection.decorations[d].source.end >= anchor.byte;
                }
                r.side == anchor.side
                    && r.block.is_some_and(|b| self.projection.pair.sides[side].blocks[b].source.end >= anchor.byte)
            })
            .map_or(self.heights[i], |r| r.y)
    }
    fn source_at(&self, side: Side, byte: usize) -> Option<SourceAnchor> {
        let doc = &self.projection.pair.sides[side as usize];
        let end = byte
            + doc
                .text
                .get(byte..)?
                .chars()
                .next()
                .map_or(0, char::len_utf8);
        self.projection
            .source_selection(RenderSelection {
                side,
                anchor: byte,
                head: end,
            })
            .and_then(|s| {
                doc.line_starts
                    .get(s.anchor.line as usize)
                    .map(|start| SourceAnchor {
                        side,
                        byte: start + s.anchor.col,
                    })
            })
    }
    fn row_anchor(&self, i: usize, index: usize) -> Option<SourceAnchor> {
        let row = self.rows[i].get(index)?;
        if let Some(d) = row.marker {
            let d = &self.projection.decorations[d];
            return Some(SourceAnchor { side: d.side, byte: d.source.start });
        }
        row.pieces
            .first()
            .and_then(|p| self.source_at(p.side, p.range.start))
            .or_else(|| {
                row.block.map(|b| SourceAnchor {
                    side: row.side,
                    byte: self.projection.pair.sides[row.side as usize].blocks[b].source.start,
                })
            })
    }
    /// Panes and scrolling move apart: Inline has one pane, Independent and Aligned scroll each pane.
    fn separate(&self) -> bool {
        self.inline || self.mode.separate_tops()
    }
    /// The document position under `(x, y)` in pane `i`: its side, byte and row. With `only`, pieces of the other
    /// side are passed over (an Inline selection stays in one document).
    fn hit(&self, i: usize, y: f32, x: f32, only: Option<Side>) -> Option<(Side, usize, usize)> {
        let row_index = self.row_at(i, y);
        let row = self.rows[i].get(row_index)?;
        let mut pieces = row.pieces.iter().filter(|p| only.is_none_or(|side| p.side == side));
        let piece = pieces
            .clone()
            .rev()
            .find(|p| x >= p.x)
            .or_else(|| pieces.next())?;
        Some((
            piece.side,
            piece.range.start
                + piece
                    .shaped
                    .closest_index_for_x(px((x - piece.x).max(0.)))
                    .min(piece.range.len()),
            row_index,
        ))
    }
}
impl Frame {
    pub(super) fn inline(&self) -> bool {
        self.layout.inline
    }
    /// The shapes under and between the text of side `i`'s visible rows, in pane coordinates: change bars, code
    /// and callout boxes, pills behind tags and inline code, quote bars, list guides, table borders and rules.
    /// Slicing: pieces' and runs' ranges come from the layout of this document, so char boundaries.
    pub(super) fn fills(&self, i: usize, p: &theme::Palette, h: f32) -> Vec<Fill> {
        let l = &self.layout;
        let s = &l.style;
        let doc = &l.projection.pair.sides[i];
        let rows = &l.rows[i];
        let (left, column) = l.column[i];
        let (top, hx) = (self.top[i], self.horizontal[i]);
        let start = rows.partition_point(|r| r.y + r.height < top);
        let end = start + rows[start..].iter().take_while(|r| r.y < top + h).count();
        let block_of = |r: &Row| r.block.map(|b| &doc.blocks[b]);
        let mut out = Vec::new();
        let mut boxed = HashSet::new();
        let line = |out: &mut Vec<Fill>, x: f32, y: f32, w: f32, h: f32, color: u32| {
            out.push(Fill { border: None, kind: FillKind::Line, x: x - hx, y: y - top, w, h, color, radius: 0. });
        };
        for (n, row) in rows.iter().enumerate().take(end).skip(start) {
            let y = row.y - top;
            let Some(block) = block_of(row) else { continue };
            let b = row.block.unwrap_or_default();
            // boxes span every row of a code block or callout, also those scrolled out of view
            let keys = [
                block.callout.map(|c| (true, c.id as usize)),
                matches!(block.kind, BlockKind::Code { .. }).then_some((false, b)),
            ];
            for key in keys.into_iter().flatten().filter(|k| boxed.insert(*k)) {
                let same = |r: &Row| match key {
                    (true, id) => block_of(r).and_then(|x| x.callout).is_some_and(|c| c.id as usize == id),
                    (false, b) => r.block == Some(b),
                };
                let first = rows[..n].iter().rposition(|r| !same(r)).map_or(0, |k| k + 1);
                let last = n + rows[n..].iter().take_while(|r| same(r)).count() - 1;
                let (pad, radius, color) = match (key.0, block.callout) {
                    (true, Some(c)) => (s.size * 0.5, 4., s.callout(p, c.kind).0),
                    _ => (12., 6., s.code_bg),
                };
                out.push(Fill {
                    border: None,
                    kind: FillKind::Block,
                    x: left - hx,
                    y: rows[first].y - top - pad,
                    w: column,
                    h: rows[last].y + rows[last].height - rows[first].y + 2. * pad,
                    color,
                    radius,
                });
            }
            if let Some(kind) = row.change {
                let ignored = row.decorations.first().is_some_and(|&d| l.projection.decorations[d].ignored);
                let color = match kind {
                    Kind::Inserted => s.added,
                    Kind::Deleted => s.removed,
                    _ => s.modified,
                };
                out.push(Fill {
                    border: None,
                    kind: FillKind::ChangeBar,
                    x: left - 14. - hx,
                    y,
                    w: 3.,
                    h: row.height,
                    color: if ignored { faded(p, color) } else { color },
                    radius: 1.5,
                });
            }
            // bars and guides reach down to the next row when it continues the same quote or list
            let next = rows.get(n + 1).and_then(|r| block_of(r).map(|x| (r.y, x)));
            let reach = |deeper: &dyn Fn(&crate::markdown::Block) -> bool| {
                next.filter(|(_, x)| deeper(x)).map_or(row.height, |(y, _)| y - row.y)
            };
            let pad = if block.callout.is_some() { s.size } else { 0. };
            for k in 0..quotes(block) {
                let h = reach(&|x| quotes(x) > k && x.callout == block.callout);
                line(&mut out, left + pad + k as f32 * s.size, row.y, 2., h, s.quote_bar);
            }
            for k in 1..block.list_depth {
                let h = reach(&|x| x.list_depth > k);
                let x = left + pad + quotes(block) as f32 * s.size + (k - 1) as f32 * LIST_STEP + 4.;
                line(&mut out, x, row.y, 1., h, s.rule);
            }
            if let (Some(&a), Some(&z)) = (row.grid.first(), row.grid.last()) {
                line(&mut out, a, row.y, z - a, 1., s.rule);
                for &x in &row.grid {
                    line(&mut out, x, row.y, 1., row.height, s.rule);
                }
                if !rows.get(n + 1).is_some_and(|r| !r.grid.is_empty()) {
                    line(&mut out, a, row.y + row.height, z - a + 1., 1., s.rule);
                }
            }
            match &block.kind {
                BlockKind::Rule => line(&mut out, left, row.y + row.height / 2., column, 1., s.rule),
                BlockKind::Footnote { .. }
                    if row.first
                        && !rows[..n].last().and_then(block_of).is_some_and(|x| matches!(x.kind, BlockKind::Footnote { .. })) =>
                {
                    line(&mut out, left, row.y - s.size, column, 1., s.rule)
                }
                _ => {}
            }
            for piece in &row.pieces {
                // words changed in place get a tint; whole added or removed blocks only their bar (and strike)
                // outlined, so a change never looks like a highlight, a code pill or a tag
                if let Some(kind) = piece.change.filter(|_| row.change == Some(Kind::Modified)) {
                    out.push(Fill {
                        border: Some(kind == Kind::Deleted),
                        kind: FillKind::Change,
                        x: piece.x - 3. - hx,
                        y: y + 2.,
                        w: f32::from(piece.shaped.width) + 6.,
                        h: row.height - 4.,
                        color: if kind == Kind::Deleted { s.removed } else { s.added },
                        radius: 4.,
                    });
                }
                let tags = row.kind == RowKind::Property
                    && row.pieces.first().is_some_and(|k| doc.text[k.range.clone()].trim() == "tags");
                let first = block.runs.partition_point(|r| r.text.end <= piece.range.start);
                let runs = block.runs[first..].iter().take_while(|r| r.text.start < piece.range.end);
                let mut spans: Vec<(Range<usize>, bool)> = runs
                    .filter(|r| {
                        r.style.tag
                            || (r.style.code
                                && !matches!(block.kind, BlockKind::Code { .. } | BlockKind::Literal | BlockKind::Properties))
                    })
                    .map(|r| (r.text.start.max(piece.range.start)..r.text.end.min(piece.range.end), r.style.tag))
                    .collect();
                if tags && !std::ptr::eq(piece, &row.pieces[0]) {
                    spans.push((piece.range.clone(), true));
                }
                for (r, tag) in spans {
                    let ax = f32::from(piece.shaped.x_for_index(r.start - piece.range.start));
                    let bx = f32::from(piece.shaped.x_for_index(r.end - piece.range.start));
                    let (padx, bh) = if tag { (6., row.height * 0.72) } else { (3., row.height * 0.8) };
                    out.push(Fill {
                        border: None,
                        kind: FillKind::Pill,
                        x: piece.x + ax - padx - hx,
                        y: y + (row.height - bh) / 2.,
                        w: bx - ax + 2. * padx,
                        h: bh,
                        color: if tag { s.tag_bg } else { s.code_bg },
                        radius: if tag { bh / 2. } else { 4. },
                    });
                }
                // drawn after the pills, so an outline stays visible over inline code
                // Side by side: changed words outlined as in Inline, old side dashed red, new side green; a block that
                // changed as a whole has its bar only
                for d in row.decorations.iter().map(|&d| &l.projection.decorations[d]) {
                    let whole = d.text.start <= block.text.start && d.text.end >= block.text.end;
                    let (a, z) = (d.text.start.max(piece.range.start), d.text.end.min(piece.range.end));
                    if whole || a >= z {
                        continue;
                    }
                    let ax = f32::from(piece.shaped.x_for_index(a - piece.range.start));
                    let zx = f32::from(piece.shaped.x_for_index(z - piece.range.start));
                    let old = row.side == Side::Left;
                    let color = if old { s.removed } else { s.added };
                    out.push(Fill {
                        border: Some(old),
                        kind: FillKind::Change,
                        x: piece.x + ax - 1.5 - hx,
                        y: y + 2.,
                        w: zx - ax + 3.,
                        h: row.height - 4.,
                        color: if d.ignored { faded(p, color) } else { color },
                        radius: 4.,
                    });
                }
            }
        }
        out
    }
    pub(super) fn paint(
        &self,
        p: &theme::Palette,
        bounds: Bounds<Pixels>,
        widths: [f32;2],
        font: &Font,
        window: &mut Window,
        cx: &mut App,
    ) {
        let y0 = f32::from(bounds.origin.y);
        let h = f32::from(bounds.size.height).max(0.);
        let l = &self.layout;
        let s = &l.style;
        let mut bold = s.body.clone();
        bold.weight = gpui::FontWeight::BOLD;
        for i in 0..2 {
            let pane_w=widths[i];
            let x = f32::from(bounds.origin.x) + if i==0{0.0}else{widths[0]+GUTTER_W};
            let (left, _) = l.column[i];
            let hx = self.horizontal[i];
            let clip_bounds = Bounds {
                origin: point(px(x), px(y0)),
                size: size(px((pane_w - super::scrollbars::strip_w(pane_w)).max(0.)), px(h)),
            };
            window.with_content_mask(
                Some(ContentMask {
                    bounds: clip_bounds,
                }),
                |window| {
                    for f in self.fills(i, p, h) {
                        match f.border {
                            Some(dashed) => outline(window, x + f.x, y0 + f.y, f.w, f.h, f.radius, f.color, dashed),
                            None => rounded(window, x + f.x, y0 + f.y, f.w, f.h, f.radius, f.color),
                        }
                    }
                    let doc = &l.projection.pair.sides[i];
                    let rows = &l.rows[i];
                    let start = rows.partition_point(|r| r.y + r.height < self.top[i]);
                    for (row_index, row) in rows
                        .iter()
                        .enumerate()
                        .skip(start)
                        .take_while(|(_, r)| r.y < self.top[i] + h)
                    {
                        let y = y0 + row.y - self.top[i];
                        if let Some(d) = row.marker {
                            let d = &l.projection.decorations[d];
                            let line = doc.source_pos(d.source.start).map_or(1, |p| p.line + 1);
                            let label = format!("Markup change · source line {line}  → Source");
                            let shaped = shape_label(window, font, font_size(), &label, p.accent);
                            let bh = row.height - 8.;
                            rounded(window, x + left - hx, y + 4., f32::from(shaped.width) + 20., bh, bh / 2., shade(p, d.kind, d.ignored, tint));
                            let _ = shaped.paint(point(px(x + left - hx + 10.), px(y)), px(row.height), TextAlign::Left, None, window, cx);
                            continue;
                        }
                        let block = row.block.map(|b| &doc.blocks[b]);
                        for piece in &row.pieces {
                            let tx = x + piece.x - hx;
                            let _ = piece.shaped.paint_background(
                                point(px(tx), px(y)),
                                px(row.height),
                                TextAlign::Left,
                                None,
                                window,
                                cx,
                            );
                            let mut highlight = |r: Range<usize>, color: u32| {
                                let a = r.start.max(piece.range.start);
                                let b = r.end.min(piece.range.end);
                                if a < b {
                                    let ax =
                                        f32::from(piece.shaped.x_for_index(a - piece.range.start));
                                    let bx =
                                        f32::from(piece.shaped.x_for_index(b - piece.range.start));
                                    rounded(window, tx + ax, y + 1., (bx - ax).max(1.), row.height - 2., 3., color);
                                }
                            };
                            if let Some((anchor, _, spans)) = &self.find
                                && anchor.side == piece.side
                            {
                                for span in spans {
                                    highlight(span.clone(), p.sel_bg);
                                }
                            }
                            if let Some(s) = self.selection.filter(|s| s.side == piece.side) {
                                highlight(s.anchor.min(s.head)..s.anchor.max(s.head), p.sel_bg);
                            }
                            let _ = piece.shaped.paint(
                                point(px(tx), px(y)),
                                px(row.height),
                                TextAlign::Left,
                                None,
                                window,
                                cx,
                            );
                            if let Some(s) = self.selection.filter(|s| s.side == piece.side) {
                                let caret_row = self
                                    .affinity
                                    .filter(|(old, _)| *old == s)
                                    .map_or_else(|| l.text_row(s.side as usize, s.head), |(_, r)| r);
                                if caret_row == row_index
                                    && s.head >= piece.range.start
                                    && s.head <= piece.range.end
                                {
                                    let at = f32::from(
                                        piece.shaped.x_for_index(s.head - piece.range.start),
                                    );
                                    fill(window, tx + at, y + 1., 1., row.height - 2., p.caret);
                                }
                            }
                        }
                        let Some(b) = block else { continue };
                        match row.kind {
                            RowKind::CalloutTitle(c) => {
                                let (_, color, icon) = s.callout(p, c.kind);
                                let ix = row.pieces.first().map_or(x + indent(s, left, b) - hx, |pc| x + pc.x - s.size * 1.5 - hx);
                                let icon = shape_label(window, &bold, s.size, icon, color);
                                let _ = icon.paint(point(px(ix), px(y)), px(row.height), TextAlign::Left, None, window, cx);
                                if row.pieces.is_empty() {
                                    let label = shape_label(window, &bold, s.size, c.kind.label(), color);
                                    let _ = label.paint(point(px(ix + s.size * 1.5), px(y)), px(row.height), TextAlign::Left, None, window, cx);
                                }
                            }
                            RowKind::PropertiesLabel => {
                                let label = shape_label(window, &s.body, s.size * 0.8, "Properties", s.muted);
                                let _ = label.paint(point(px(x + left - hx), px(y)), px(row.height), TextAlign::Left, None, window, cx);
                            }
                            RowKind::Image(b) => {
                                if let Some((image, [w, h])) = l.images[i].get(&b) {
                                    let height = row.height;
                                    let width = height * *w as f32 / (*h).max(1) as f32;
                                    let bounds = Bounds { origin: point(px(x + indent(s, left, &doc.blocks[b]) - hx), px(y)), size: size(px(width), px(height)) };
                                    let _ = window.paint_image(bounds, bounds, gpui::Corners::all(px(4.)), image.clone(), 0, false);
                                }
                            }
                            RowKind::Text | RowKind::Property => {}
                        }
                        if let BlockKind::Footnote { label } = &b.kind
                            && row.first
                        {
                            let label = shape_label(window, &s.body, s.size * 0.88, &format!("{label}."), s.muted);
                            let _ = label.paint(point(px(x + indent(s, left, b) - s.size * 1.6 - hx), px(y)), px(row.height), TextAlign::Left, None, window, cx);
                        }
                        if let BlockKind::Code { language: Some(language) } = &b.kind
                            && row.first
                        {
                            let label = shape_label(window, &s.body, s.size * 0.75, language, s.muted);
                            let (_, column) = l.column[i];
                            let _ = label.paint(point(px(x + left + column - 10. - f32::from(label.width) - hx), px(y - 11.)), px(s.size), TextAlign::Left, None, window, cx);
                        }
                        if let BlockKind::ListItem { marker, checked: Some(done) } = &b.kind
                            && row.first
                            && let at = b.text.start + marker.len() + 1
                            && let Some(pc) = row.pieces.first().filter(|pc| pc.range.contains(&at))
                        {
                            let side = (s.size * 0.95).round();
                            let bx = x + pc.x + f32::from(pc.shaped.x_for_index(at - pc.range.start)) - hx;
                            let by = y + (row.height - side) / 2.;
                            if *done {
                                rounded(window, bx, by, side, side, 4., s.check);
                                let tick = shape_label(window, &bold, side * 0.8, "✓", p.bg);
                                let _ = tick.paint(point(px(bx + (side - f32::from(tick.width)) / 2.), px(by)), px(side), TextAlign::Left, None, window, cx);
                            } else {
                                outline(window, bx, by, side, side, 4., s.muted, false);
                            }
                        }
                    }
                    if let Some((anchor, _, spans)) = &self.find
                        && l.pane_of(anchor.side as usize) == i
                        && spans.iter().all(Range::is_empty)
                    {
                        fill(window, x, y0 + 36., pane_w, line_h() + 6., p.status_bg);
                        text(
                            window,
                            cx,
                            font,
                            x + 8.,
                            y0 + 39.,
                            "Match in Markdown source → Source",
                            p.accent,
                        );
                    }
                },
            );
        }
    }
}
impl Viewer {
    /// The Preview is shown Inline: one full-width pane. Source and "Preparing" keep two panes.
    pub(super) fn markdown_inline(&self) -> bool {
        self.comparison.markdown_mode() == PresentationMode::Rendered
            && self.merge.is_none()
            && self.comparison.markdown_layout() == crate::markdown::MarkdownLayout::Inline
    }
    pub(super) fn markdown_active(&self) -> bool {
        self.comparison.markdown_edit_disabled().is_some()
    }
    pub(super) fn markdown_command(&mut self, command: MarkdownCommand) {
        let doc = self.doc().cloned();
        self.comparison
            .markdown_command_document(command, doc.as_deref().map(|d| &d.text));
    }
    /// The layout for input. Hit-testing works against the layout on screen (at the current pane widths); only
    /// when there is none for this projection is one built here, synchronously, so a key or click is never lost.
    pub(super) fn markdown_prepare(&mut self, window: &Window) {
        if let Some(wanted) = self.markdown_wanted(window)
            && self.markdown.layout.as_ref().is_none_or(|l| !Arc::ptr_eq(&l.projection, &wanted.projection))
        {
            self.markdown.building = None;
            self.markdown_install(wanted.build(window.text_system()));
        }
        self.markdown_clamp(window);
    }

    /// For paint: what layout is wanted and not on screen, unless a resize drag is still under way (the previous
    /// layout is kept, clipped to the new panes, until the widths hold for [`SETTLE`]). `None` when nothing is to be
    /// built now; `Some(true)` means a drag is holding it.
    fn markdown_needed(&mut self, window: &Window) -> Option<Result<Wanted, ()>> {
        let wanted = self.markdown_wanted(window)?;
        let shown = self.markdown.layout.as_ref();
        if shown.is_some_and(|l| wanted.built(l)) {
            self.markdown.building = None;
            self.markdown.resized = None;
            return None;
        }
        let resized_only = shown.is_some_and(|l| {
            l.widths != wanted.widths && Wanted { widths: l.widths, ..wanted.clone() }.built(l)
        });
        if resized_only {
            let holding = match self.markdown.resized {
                Some((seen, since)) if seen == wanted.widths => since.elapsed() < SETTLE,
                _ => {
                    self.markdown.resized = Some((wanted.widths, std::time::Instant::now()));
                    true
                }
            };
            if holding {
                return Some(Err(()));
            }
        }
        if self.markdown.building.as_ref().is_some_and(|(w, _)| w.same(&wanted)) {
            return None;
        }
        // Another pair's layout must not stand in for this one (the TUI shows "Preparing" too).
        if shown.is_some_and(|l| !Arc::ptr_eq(&l.projection.pair, &wanted.projection.pair)) {
            self.markdown.layout = None;
        }
        Some(Ok(wanted))
    }

    /// The layout the current projection, panes, theme and syntax call for; `None` outside Rendered.
    fn markdown_wanted(&mut self, window: &Window) -> Option<Wanted> {
        if self.comparison.markdown_mode() != PresentationMode::Rendered || self.merge.is_some() {
            self.markdown = Adapter::default();
            return None;
        }
        let projection = self.comparison.markdown_projection().cloned()?;
        for side in [Side::Left, Side::Right] {
            if self.comparison.markdown_anchor(side).is_none() {
                let line = self.comparison.view.anchor_line_on(side) as u32 + 1;
                self.comparison.go_to_line(side, line);
            }
        }
        if self
            .markdown
            .syntax
            .as_ref()
            .is_none_or(|s| !std::sync::Weak::ptr_eq(&s.pair, &Arc::downgrade(&projection.pair)))
        {
            self.markdown.syntax = Some(CodeSyntax::new(projection.pair.clone()));
        }
        if self.markdown.syntax.as_mut().is_some_and(CodeSyntax::poll) {
            self.markdown.syntax_rev += 1;
        }
        let widths = self.markdown_text_widths(window);
        if widths.iter().all(|w| *w <= 0.) {
            self.markdown.layout = None;
            self.markdown.building = None;
            self.markdown.top = [0.; 2];
            self.markdown.horizontal = [0.; 2];
            return None;
        }
        let inline = self.markdown_inline();
        if inline {
            self.diff_focus = Side::Right;
        }
        let base = font_size();
        let reading = self.markdown.reading.as_ref().filter(|(palette, size, style)| {
            Arc::ptr_eq(palette, &self.palette) && *size == base && style.mono == self.font
        });
        let style = match reading {
            Some((_, _, style)) => style.clone(),
            None => {
                let installed = window.text_system().all_font_names();
                let body = font(sans_family(crate::settings::get().gui.reading_font.as_deref(), &installed).to_string());
                let style = Arc::new(ReadingStyle::new(&self.palette, self.font.clone(), body, base));
                self.markdown.reading = Some((self.palette.clone(), base, style.clone()));
                style
            }
        };
        Some(Wanted {
            projection,
            widths,
            mode: self.comparison.view.mode(),
            palette: self.palette.clone(),
            font: self.font.clone(),
            style,
            unfolded: self.markdown.unfolded.clone(),
            images: self.markdown_images.shown.clone(),
            inline,
            syntax: self.markdown.syntax.as_ref().map(|s| s.parsed.clone())?,
            syntax_rev: self.markdown.syntax_rev,
        })
    }

    /// Puts `layout` on screen; each pane's position is taken again from its source anchor.
    fn markdown_install(&mut self, layout: Layout) {
        self.markdown.layout = Some(Arc::new(layout));
        self.markdown.resized = None;
        self.markdown.affinity = None;
        self.markdown.anchors = [None; 2];
    }

    /// Scroll positions from the anchors, clamped to the layout on screen and the current panes.
    fn markdown_clamp(&mut self, window: &Window) {
        let Some(l) = self.markdown.layout.clone() else { return };
        let widths = self.markdown_text_widths(window);
        let h = self.markdown_height(window);
        if l.inline {
            // one pane: the new side's anchor, or the old side's when it is just above (a removed block)
            let anchors = [Side::Left, Side::Right].map(|side| self.comparison.markdown_anchor(side));
            if anchors != self.markdown.anchors {
                let y = anchors[1].map(|a| l.anchor_y(a));
                let old = anchors[0].filter(|_| anchors[0] != self.markdown.anchors[0]).map(|a| l.anchor_y(a));
                let top = match (old, y) {
                    (Some(o), Some(n)) if o < n && n - o < h => Some(o),
                    (_, Some(n)) => Some(n),
                    (o, None) => o,
                };
                if let Some(top) = top {
                    self.markdown.top[1] = top;
                }
                self.markdown.anchors = anchors;
            }
            self.markdown.top = [0., self.markdown.top[1].clamp(0., (l.heights[1] - h).max(0.))];
            self.markdown.horizontal = [0., self.markdown.horizontal[1].clamp(0., (l.widest[1] - widths[1]).max(0.))];
            return;
        }
        for (i, width) in widths.iter().enumerate() {
            let anchor = self.comparison.markdown_anchor(pane(i));
            if anchor != self.markdown.anchors[i] {
                if let Some(a) = anchor {
                    self.markdown.top[i] = l.anchor_y(a);
                }
                self.markdown.anchors[i] = anchor;
            }
            self.markdown.top[i] = self.markdown.top[i].clamp(0., (l.heights[i] - h).max(0.));
            self.markdown.horizontal[i] = self.markdown.horizontal[i].clamp(0., (l.widest[i] - *width).max(0.));
        }
    }

    /// The preview panes' text widths: each pane short of its scrollbar strip, what the text wraps to, is clipped to
    /// and scrolls across.
    fn markdown_text_widths(&self, window: &Window) -> [f32; 2] {
        self.comparison_widths(f32::from(window.viewport_size().width) - self.diff_x()).map(|w| w - super::scrollbars::strip_w(w))
    }

    /// Where pane `i`'s text starts, at the current pane widths (what paint uses).
    fn markdown_pane_x(&self, window: &Window, i: usize) -> f32 {
        let widths = self.comparison_widths(f32::from(window.viewport_size().width) - self.diff_x());
        self.diff_x() + if i == 0 { 0.0 } else { widths[0] + GUTTER_W }
    }

    /// `[view] markdown = "rendered"`: the first comparison starts Rendered when it is Markdown; other files stay in
    /// Source without a notice.
    pub fn start_rendered(&mut self, cx: &mut Context<Self>) {
        if self.comparison.presentation_disabled(PresentationMode::Rendered, self.doc().map(|d| &d.text)).is_none() {
            self.comparison_command(Command::SetPresentation(PresentationMode::Rendered), cx);
        } else {
            self.comparison.prefer_presentation(PresentationMode::Rendered);
        }
    }

    /// Whether a rendered layout is being shaped in the background, for the "Preparing" status.
    pub(super) fn markdown_building(&self) -> bool {
        self.markdown.building.is_some() || self.markdown_images.pending > 0
    }

    /// Finds the local images of the shown document and decodes them in the background, once per document.
    fn markdown_load_images(&mut self, cx: &mut Context<Self>) {
        let Some(projection) = self.comparison.markdown_projection().cloned() else { return };
        let pair = Arc::downgrade(&projection.pair);
        if self.markdown_images.pair.as_ref().is_some_and(|p| std::sync::Weak::ptr_eq(p, &pair)) {
            return;
        }
        let old = std::mem::take(&mut self.markdown_images);
        self.markdown_images.generation = old.generation + 1;
        self.markdown_images.stale = old.stale;
        self.markdown_images.stale.extend(old.loaded.into_values().flatten().map(|(image, _)| image));
        self.markdown_images.pair = Some(pair);
        let cwd = std::env::current_dir().unwrap_or_default();
        self.markdown_images.blocks = crate::markdown::image_sources(&projection.pair, &self.set, self.comparison.current, &self.spec, &cwd);
        let paths: HashSet<PathBuf> = self.markdown_images.blocks.iter().flatten().map(|(_, p)| p.clone()).collect();
        for path in paths {
            self.markdown_images.loaded.insert(path.clone(), None);
            self.markdown_images.pending += 1;
            let generation = self.markdown_images.generation;
            cx.spawn(async move |this, cx| {
                let file = path.clone();
                let image = cx.background_executor().spawn(async move { decode_image(&file) }).await;
                let _ = this.update(cx, |v, cx| v.markdown_image_loaded(generation, path, image, cx));
            })
            .detach();
        }
    }

    fn markdown_image_loaded(&mut self, generation: u64, path: PathBuf, image: Option<Decoded>, cx: &mut Context<Self>) {
        let cache = &mut self.markdown_images;
        if generation != cache.generation || !matches!(cache.loaded.get(&path), Some(None)) {
            // the document changed while it was decoding
            cache.stale.extend(image.map(|(image, _)| image));
            return;
        }
        cache.pending = cache.pending.saturating_sub(1);
        // a failed image keeps no `None` marker, so a second result for the same path is not counted again
        let Some((image, [w, h])) = image else { cache.loaded.remove(&path); return };
        let bytes = w as usize * h as usize * 4;
        if cache.bytes + bytes > IMAGE_BYTES {
            cache.stale.push(image);
            return;
        }
        cache.bytes += bytes;
        cache.loaded.insert(path, Some((image, [w, h])));
        let shown: ImageMap = std::array::from_fn(|i| {
            cache.blocks[i]
                .iter()
                .filter_map(|(b, p)| cache.loaded.get(p).cloned().flatten().map(|image| (*b, image)))
                .collect()
        });
        cache.shown = Arc::new(shown);
        cx.notify();
    }

    /// Images leave the GPU atlas once no document shows them: dropping the last `Arc` frees only the pixels.
    pub(super) fn markdown_release_images(&mut self, window: &mut Window) {
        let shown = self
            .comparison
            .markdown_projection()
            .filter(|_| self.comparison.markdown_mode() == PresentationMode::Rendered && self.merge.is_none())
            .map(|p| Arc::downgrade(&p.pair));
        if self.markdown_images.pair.as_ref().is_some_and(|p| shown.as_ref().is_none_or(|s| !std::sync::Weak::ptr_eq(p, s))) {
            let old = std::mem::take(&mut self.markdown_images);
            self.markdown_images.generation = old.generation + 1;
            self.markdown_images.stale = old.stale;
            self.markdown_images.stale.extend(old.loaded.into_values().flatten().map(|(image, _)| image));
        }
        for image in self.markdown_images.stale.drain(..) {
            let _ = window.drop_image(image);
        }
    }

    pub(super) fn markdown_height(&self, window: &Window) -> f32 {
        let _ = window;
        (self.content_bottom()
            - self.top()
            - self.panel_h()
            - self.picker_v_height())
        .max(0.)
    }
    pub(super) fn markdown_frame(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<Frame> {
        self.markdown_load_images(cx);
        match self.markdown_needed(window) {
            Some(Err(())) if self.markdown.settle.is_none() => {
                self.markdown.settle = Some(cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(SETTLE).await;
                    let _ = this.update(cx, |v, cx| {
                        v.markdown.settle = None;
                        cx.notify();
                    });
                }));
            }
            Some(Ok(wanted)) => {
                let key = wanted.clone();
                // Shaped off the UI thread with a text system of its own: the window's per-frame layout cache is
                // only for the frame being drawn.
                let text_system = gpui::WindowTextSystem::new(cx.text_system().clone());
                let job = wanted.clone();
                let task = cx.spawn(async move |this, cx| {
                    let layout = cx.background_executor().spawn(async move { job.build(&text_system) }).await;
                    let _ = this.update(cx, |v, cx| {
                        if v.markdown.building.as_ref().is_some_and(|(w, _)| w.same(&wanted)) {
                            v.markdown.building = None;
                            v.markdown_install(layout);
                            cx.notify();
                        }
                    });
                });
                self.markdown.building = Some((key, task));
            }
            _ => {}
        }
        self.markdown_clamp(window);
        if let Some(syntax) = self
            .markdown
            .syntax
            .as_mut()
            .filter(|s| s.job.is_some() && s.task.is_none())
        {
            let pair = syntax.pair.clone();
            syntax.task = Some(cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor()
                        .timer(std::time::Duration::from_millis(20))
                        .await;
                    let pending = this
                        .update(cx, |viewer, cx| {
                            let Some(s) = viewer
                                .markdown
                                .syntax
                                .as_mut()
                                .filter(|s| std::sync::Weak::ptr_eq(&s.pair, &pair))
                            else {
                                return false;
                            };
                            let changed = s.poll();
                            let pending = s.job.is_some();
                            if changed {
                                // recoloured by a background rebuild; the current layout stays meanwhile
                                viewer.markdown.syntax_rev += 1;
                                cx.notify();
                            }
                            pending
                        })
                        .unwrap_or(false);
                    if !pending {
                        break;
                    }
                }
            }));
        }
        Some(Frame {
            layout: self.markdown.layout.clone()?,
            top: self.markdown.top,
            horizontal: self.markdown.horizontal,
            selection: self.comparison.markdown_selection(),
            affinity: self.markdown.affinity,
            find: self.comparison.markdown_find_hit(),
        })
    }
    pub(super) fn markdown_horizontal(&mut self, side: Side, dx: f32) {
        if let Some(l) = &self.markdown.layout {
            let i = side as usize;
            self.markdown.horizontal[i] =
                (self.markdown.horizontal[i] + dx).clamp(0., (l.widest[i] - l.widths[i]).max(0.));
        }
    }
    pub(super) fn markdown_scroll(&mut self, side: Side, dy: f32, dx: f32, window: &Window) {
        // a wheel turn never waits for shaping: while the layout it would scroll is still being built in the
        // background, the turn is dropped instead of building it again on the UI thread
        if self.markdown.building.is_some()
            && let Some(wanted) = self.markdown_wanted(window)
            && self.markdown.layout.as_ref().is_none_or(|l| !Arc::ptr_eq(&l.projection, &wanted.projection))
        {
            return;
        }
        self.markdown_prepare(window);
        let Some(l) = &self.markdown.layout else {
            return;
        };
        let i = if l.inline { 1 } else { side as usize };
        self.markdown.horizontal[i] =
            (self.markdown.horizontal[i] + dx).clamp(0., (l.widest[i] - l.widths[i]).max(0.));
        if dy == 0. {
            return;
        }
        let top = (self.markdown.top[i] + dy)
            .clamp(0., (l.heights[i] - self.markdown_height(window)).max(0.));
        if l.separate() {
            self.markdown.top[i] = top;
        } else {
            self.markdown.top = [top; 2];
        }
        let separate = l.separate();
        let anchors: [Option<SourceAnchor>; 2] =
            std::array::from_fn(|j| l.row_anchor(j, l.row_at(j, self.markdown.top[j])));
        for (j, anchor) in anchors.into_iter().enumerate() {
            if (j == i || !separate)
                && let Some(a) = anchor
            {
                self.markdown_command(MarkdownCommand::SetAnchor(a));
                self.markdown.anchors[a.side as usize] = Some(a);
            }
        }
    }
    /// Pane `side`'s preview scrollbar in laid-out pixels (Inline has one pane), once a layout is on screen.
    pub(super) fn markdown_scrollbar(&self, side: Side, window: &Window) -> Option<crate::scrollbar::Scrollbar> {
        let l = self.markdown.layout.as_ref()?;
        let i = if l.inline { 1 } else { side as usize };
        let (top, visible) = (self.markdown.top[i], self.markdown_height(window));
        Some(crate::scrollbar::Scrollbar { total: f64::from(l.heights[i].max(top + visible)), top: f64::from(top), visible: f64::from(visible) })
    }
    /// Scroll pane `side`'s preview so its top is `top` pixels, as the wheel does.
    pub(super) fn markdown_scroll_to(&mut self, side: Side, top: f32, window: &Window) {
        let Some(l) = &self.markdown.layout else { return };
        let i = if l.inline { 1 } else { side as usize };
        self.markdown_scroll(side, top - self.markdown.top[i], 0.0, window);
    }
    pub(super) fn markdown_select(&mut self, s: RenderSelection) {
        let anchor = self
            .markdown
            .layout
            .as_ref()
            .and_then(|l| l.source_at(s.side, s.head));
        self.markdown_command(MarkdownCommand::Select(s));
        self.markdown.affinity = None;
        if let Some(a) = anchor {
            self.markdown_command(MarkdownCommand::SetAnchor(a));
            self.markdown.anchors[s.side as usize] = Some(a);
        }
    }
    pub(super) fn markdown_select_all(&mut self) {
        if let Some(p) = self.comparison.markdown_projection() {
            let end = p.pair.sides[self.diff_focus as usize].text.len();
            self.markdown_select(RenderSelection {
                side: self.diff_focus,
                anchor: 0,
                head: end,
            });
        }
    }
    pub(super) fn markdown_show_source(&mut self, a: SourceAnchor, cx: &mut Context<Self>) {
        self.comparison_command(Command::SetPresentation(PresentationMode::Source), cx);
        self.markdown_command(MarkdownCommand::ShowSource(a));
        self.diff_focus = a.side;
        self.markdown = Adapter::default();
    }
    pub(super) fn markdown_context_pointer(&mut self, side: Side, x: f32, y: f32, window: &Window) {
        self.markdown_prepare(window);
        if self.comparison.markdown_selection().is_some_and(|s| s.anchor != s.head) { return; }
        let i = side as usize;
        let Some(l) = &self.markdown.layout else { return; };
        let tx = x - self.markdown_pane_x(window, i) + self.markdown.horizontal[i];
        let ry = y - self.top() + self.markdown.top[i];
        let row = l.row_at(i, ry);
        if l.rows[i].get(row).is_some_and(|r| r.marker.is_some()) {
            if let Some(anchor) = l.row_anchor(i, row) {
                self.markdown_command(MarkdownCommand::ClearSelection);
                self.markdown_command(MarkdownCommand::SetAnchor(anchor));
                self.markdown.anchors[i] = Some(anchor);
            }
        } else if let Some((side, byte, _)) = l.hit(i, ry, tx, None) {
            self.markdown_select(RenderSelection { side, anchor: byte, head: byte });
        }
    }
    pub(super) fn markdown_click(
        &mut self,
        ev: &MouseDownEvent,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.markdown_active() {
            return false;
        }
        self.markdown_prepare(window);
        let x = f32::from(ev.position.x);
        let y = f32::from(ev.position.y);
        let Some((side, _)) = self.pane_at(window, x) else {
            return true;
        };
        self.diff_focus = side;
        let i = side as usize;
        let Some(l) = self.markdown.layout.as_ref() else {
            return true;
        };
        let tx = x - self.markdown_pane_x(window, i) + self.markdown.horizontal[i];
        let ry = y - self.top() + self.markdown.top[i];
        if let Some((a, _, spans)) = self.comparison.markdown_find_hit()
            && a.side == side
            && spans.iter().all(Range::is_empty)
            && (self.top() + 36. ..self.top() + 62.).contains(&y)
        {
            self.markdown_show_source(a, cx);
            return true;
        }
        let row = l.row_at(i, ry);
        if let Some(RowKind::CalloutTitle(c)) = l.rows[i].get(row).filter(|r| r.y <= ry).map(|r| r.kind)
            && c.folded.is_some()
        {
            let unfolded = Arc::make_mut(&mut self.markdown.unfolded);
            if !unfolded.remove(&(i, c.id)) {
                unfolded.insert((i, c.id));
            }
            cx.notify();
            return true;
        }
        if l.rows[i].get(row).is_some_and(|r| r.marker.is_some()) {
            if let Some(a) = l.row_anchor(i, row) {
                self.markdown_show_source(a, cx);
            }
            return true;
        }
        if let Some((side, at, row)) = l.hit(i, ry, tx, None) {
            let old = self
                .comparison
                .markdown_selection()
                .filter(|s| s.side == side);
            let mut s = RenderSelection {
                side,
                anchor: old.filter(|_| ev.modifiers.shift).map_or(at, |s| s.anchor),
                head: at,
            };
            let doc = &l.projection.pair.sides[side as usize];
            if ev.click_count >= 3 {
                // the clicked document's block: an Inline row can mix both sides' text
                let block = l.rows[i].get(row).filter(|r| r.side == side).and_then(|r| r.block)
                    .unwrap_or_else(|| doc.blocks.partition_point(|b| b.text.end < at));
                if let Some(b) = doc.blocks.get(block) {
                    s.anchor = b.text.start;
                    s.head = b.text.end;
                }
            } else if ev.click_count == 2
                // segment the clicked line, not the whole rendered text
                && let from = doc.text[..at].rfind('\n').map_or(0, |i| i + 1)
                && let to = doc.text[at..].find('\n').map_or(doc.text.len(), |i| at + i)
                && let Some((start, word)) = doc.text[from..to]
                    .unicode_word_indices()
                    .map(|(start, word)| (from + start, word))
                    .find(|(start, word)| *start <= at && at <= start + word.len())
            {
                s.anchor = start;
                s.head = start + word.len();
            }
            self.markdown_select(s);
            self.markdown.affinity = Some((s, row));
            self.markdown.dragging = true;
        }
        cx.notify();
        true
    }
    pub(super) fn markdown_drag(
        &mut self,
        ev: &MouseMoveEvent,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.markdown_active() {
            return false;
        }
        if self.markdown.dragging
            && ev.pressed_button == Some(MouseButton::Left)
            && let Some(s) = self.comparison.markdown_selection()
        {
            self.markdown_prepare(window);
            if let Some(l) = &self.markdown.layout {
                let i = l.pane_of(s.side as usize);
                let x = f32::from(ev.position.x) - self.markdown_pane_x(window, i)
                    + self.markdown.horizontal[i];
                let y = f32::from(ev.position.y) - self.top() + self.markdown.top[i];
                if let Some((_, head, row)) = l.hit(i, y, x, Some(s.side)) {
                    let s = RenderSelection { head, ..s };
                    self.markdown_select(s);
                    self.markdown.affinity = Some((s, row));
                    cx.notify();
                }
            }
        }
        true
    }
    pub(super) fn markdown_key(
        &mut self,
        key: &str,
        cmd: bool,
        shift: bool,
        alt: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.markdown_active() {
            return false;
        }
        if key == "a" && cmd {
            self.markdown_select_all();
            cx.notify();
            return true;
        }
        if key == "escape" {
            self.markdown_command(MarkdownCommand::ClearSelection);
            cx.notify();
            return true;
        }
        if alt {
            if cmd && !shift && matches!(key, "left" | "right") {
                self.markdown_scroll(
                    self.diff_focus,
                    0.,
                    if key == "left" { -52. } else { 52. },
                    window,
                );
                cx.notify();
                return true;
            }
            return false;
        }
        if !matches!(
            key,
            "left" | "right" | "up" | "down" | "home" | "end" | "pageup" | "pagedown" | "space"
        ) {
            return false;
        }
        self.markdown_prepare(window);
        if let Some(s) = self
            .comparison
            .markdown_selection()
            .filter(|s| s.side == self.diff_focus)
        {
            if let Some(l) = &self.markdown.layout {
                let side = s.side as usize;
                let i = l.pane_of(side);
                let doc = &l.projection.pair.sides[side];
                let current = self
                    .markdown
                    .affinity
                    .filter(|(old, _)| *old == s)
                    .map_or_else(|| l.text_row(side, s.head), |(_, row)| row);
                let mut row = current;
                let mut head = s.head;
                if key == "left" {
                    head = doc.text[..head]
                        .grapheme_indices(true)
                        .next_back()
                        .map_or(0, |(i, _)| i);
                    row = l.text_row(side, head);
                } else if key == "right" {
                    head += doc.text[head..].graphemes(true).next().map_or(0, str::len);
                    row = l.text_row(side, head);
                } else if cmd && key == "home" {
                    head = 0;
                    row = 0;
                } else if cmd && key == "end" {
                    head = doc.text.len();
                    row = l.rows[i].len().saturating_sub(1);
                } else if let Some(r) = l.rows[i].get(current) {
                    if key == "home" {
                        head = r.pieces.iter().find(|p| p.side == s.side).map_or(head, |p| p.range.start);
                    } else if key == "end" {
                        head = r.pieces.iter().rfind(|p| p.side == s.side).map_or(head, |p| p.range.end);
                    } else {
                        let x = r
                            .pieces
                            .iter()
                            .find(|p| p.side == s.side && (p.range.contains(&head) || p.range.end == head))
                            .map_or(16., |p| {
                                p.x + f32::from(p.shaped.x_for_index(head - p.range.start))
                            });
                        let step = if matches!(key, "pageup" | "pagedown" | "space") {
                            self.markdown_height(window).max(line_h())
                        } else {
                            r.height
                        };
                        let y = if matches!(key, "up" | "pageup") {
                            r.y - step
                        } else {
                            r.y + step
                        };
                        // Pointer hits intentionally ignore markup-only rows. Keyboard
                        // movement must cross them to reach the next selectable row.
                        let backwards = matches!(key, "up" | "pageup");
                        let target = match key {
                            "up" => current.saturating_sub(1),
                            "down" => (current + 1).min(l.rows[i].len().saturating_sub(1)),
                            _ => l.row_at(i, y.max(0.)),
                        };
                        let has = |r: usize| l.rows[i][r].pieces.iter().any(|p| p.side == s.side);
                        let selectable = if backwards {
                            (0..=target).rev().find(|&r| has(r))
                        } else {
                            (target..l.rows[i].len()).find(|&r| has(r))
                        };
                        if let Some(target) = selectable
                            && let Some((_, at, index)) = l.hit(i, l.rows[i][target].y, x, Some(s.side))
                        {
                            head = at;
                            row = index;
                        }
                    }
                }
                let y = l.rows[i].get(row).map_or(0., |r| r.y);
                let h = l.rows[i].get(row).map_or(line_h(), |r| r.height);
                let viewport = self.markdown_height(window);
                if y < self.markdown.top[i] {
                    self.markdown.top[i] = y;
                } else if y + h > self.markdown.top[i] + viewport {
                    self.markdown.top[i] = (y + h - viewport).max(0.);
                }
                let other_anchor = if !l.separate() {
                    self.markdown.top[1 - i] = self.markdown.top[i];
                    l.row_anchor(1 - i, l.row_at(1 - i, self.markdown.top[1 - i]))
                } else {
                    None
                };
                let selected = RenderSelection {
                    head,
                    anchor: if shift { s.anchor } else { head },
                    ..s
                };
                self.markdown_select(selected);
                if let Some(anchor) = other_anchor {
                    self.markdown_command(MarkdownCommand::SetAnchor(anchor));
                    self.markdown.anchors[anchor.side as usize] = Some(anchor);
                }
                self.markdown.affinity = Some((selected, row));
            }
        } else {
            let page = self.markdown_height(window).max(line_h());
            let dy = match key {
                "up" => -line_h(),
                "down" => line_h(),
                "home" => f32::NEG_INFINITY,
                "end" => f32::INFINITY,
                "pageup" => -page,
                "pagedown" | "space" => page,
                _ => 0.,
            };
            let dx = match key {
                "left" => -52.,
                "right" => 52.,
                _ => 0.,
            };
            self.markdown_scroll(self.diff_focus, dy, dx, window);
        }
        cx.notify();
        true
    }
}

#[cfg(all(test, feature = "gui-test"))]
mod tests {
    use super::super::parity_tests::{click, fixture_named, key, markdown_ready, native_text_system, run};
    use super::*;
    use gpui::TestAppContext;
    #[gpui::test]
    fn gui_markdown_preview_lays_out_a_centred_reading_column(cx: &mut TestAppContext) {
        let long: String = (0..60).map(|i| format!("Word{i} ")).collect();
        let callout = "> [!note]- Folded\n> hidden body\n";
        let f = fixture_named(cx, &format!("# Old\n\n{long}\n\n{callout}"), &format!("# Ledger\n\n{long}\n\n{callout}"), "md");
        run(cx, &f, crate::catalog::Action::Comparison(Command::Preview(crate::markdown::MarkdownLayout::Inline)));
        markdown_ready(cx, &f);
        let shows_body = |v: &Viewer| {
            let l = v.markdown.layout.as_ref().unwrap();
            let doc = &l.projection.pair.sides[1];
            l.rows[1].iter().any(|r| r.pieces.iter().any(|p| doc.text[p.range.clone()].contains("hidden body")))
        };
        let (x, y) = f.window.update(cx, |v, w, _| {
            v.markdown_prepare(w);
            let l = v.markdown.layout.as_ref().unwrap();
            let doc = &l.projection.pair.sides[1];
            let (left, width) = l.column[1];
            assert!(width <= l.style.column + 0.5 && left > 0., "at most the reading column wide");
            assert!((left * 2. + width - l.widths[1]).abs() < 1. || left == 24., "centred");
            let kind = |r: &Row| r.block.map(|b| &doc.blocks[b].kind);
            let heading = l.rows[1].iter().find(|r| matches!(kind(r), Some(BlockKind::Heading(1)))).unwrap();
            assert!(heading.height > 1.7 * font_size(), "a big heading");
            assert!(heading.pieces[0].x >= left, "text starts inside the column");
            let body = l.rows[1].iter().filter(|r| matches!(kind(r), Some(BlockKind::Paragraph))).count();
            assert!(body >= 2, "the paragraph wraps inside the column");
            let title = l.rows[1].iter().find(|r| matches!(r.kind, RowKind::CalloutTitle(c) if c.folded == Some(true))).unwrap();
            assert_eq!(doc.text[title.pieces[0].range.clone()].trim(), "Folded");
            assert!(!shows_body(v), "a folded callout shows its title only");
            (v.markdown_pane_x(w, 1) + title.pieces[0].x + 4., v.top() + title.y - v.markdown.top[1] + 4.)
        }).unwrap();
        click(cx, &f, x, y);
        markdown_ready(cx, &f);
        assert!(f.viewer.read_with(cx, |v, _| shows_body(v)), "clicking the title unfolds it");
        click(cx, &f, x, y);
        markdown_ready(cx, &f);
        assert!(!f.viewer.read_with(cx, |v, _| shows_body(v)), "and folds it again");
    }

    #[gpui::test]
    fn gui_markdown_changes_draw_bars_not_full_width_bands(cx: &mut TestAppContext) {
        let f = fixture_named(cx, "# A\n\nold words here\n\n```sh\nrun\n```\n", "# A\n\nnew words here\n\n```sh\nrun\n```\n", "md");
        run(cx, &f, crate::catalog::Action::Comparison(Command::Preview(crate::markdown::MarkdownLayout::Inline)));
        markdown_ready(cx, &f);
        run(cx, &f, crate::catalog::Action::Comparison(Command::Preview(crate::markdown::MarkdownLayout::SideBySide)));
        // word ranges arrive with the refined diff, which render asks for
        super::super::parity_tests::workflow_wait(cx, &f, |v| v.comparison.diffed == v.comparison.generation);
        f.viewer.update(cx, |v, cx| v.refine_more(cx));
        super::super::parity_tests::workflow_wait(cx, &f, |v| !v.comparison.refining && !v.comparison.markdown_loading());
        markdown_ready(cx, &f);
        f.window.update(cx, |v, w, cx| {
            let frame = v.markdown_frame(w, cx).unwrap();
            let fills = frame.fills(1, v.palette.as_ref(), 400.);
            let (left, column) = frame.layout.column[1];
            assert!(fills.iter().any(|f| f.kind == FillKind::ChangeBar && f.w <= 4. && f.x < left), "a bar left of the changed block");
            assert!(fills.iter().all(|f| f.w <= column + 1.), "no full-width bands");
            // changed words outlined as in Inline: the new side solid green, the old side dashed red
            let s = &frame.layout.style;
            assert!(fills.iter().any(|f| f.kind == FillKind::Change && f.border == Some(false) && f.color == s.added), "new word outlined");
            assert!(frame.fills(0, v.palette.as_ref(), 400.).iter().any(|f| f.kind == FillKind::Change && f.border == Some(true) && f.color == s.removed), "old word outlined");
            assert!(fills.iter().filter(|f| f.kind == FillKind::Change).all(|f| f.w < 200.), "only the changed word, not the line");
            let code = fills.iter().find(|f| f.kind == FillKind::Block).expect("the code block has a box");
            assert!(code.radius > 0. && code.w == column, "a rounded box across the column");
            assert!(frame.layout.rows[1].iter().all(|r| r.pieces.iter().all(|p| p.shaped.text.as_ref() != "3")), "no row numbers in the text");
        }).unwrap();
    }

    #[gpui::test]
    fn gui_markdown_side_by_side_marks_an_added_section_by_its_bar_alone(cx: &mut TestAppContext) {
        let old = "# Title\n\nThe service keeps accounts in balance.\n\n## End\n\nBye.\n";
        let new = "# Title\n\nThe service keeps every account in balance.\n\n## Importing\n\nOpening balances come from a CSV file with one row per account.\n\n## End\n\nBye.\n";
        let f = fixture_named(cx, old, new, "md");
        run(cx, &f, crate::catalog::Action::Comparison(Command::Preview(crate::markdown::MarkdownLayout::SideBySide)));
        super::super::parity_tests::workflow_wait(cx, &f, |v| v.comparison.diffed == v.comparison.generation);
        f.viewer.update(cx, |v, cx| v.refine_more(cx));
        super::super::parity_tests::workflow_wait(cx, &f, |v| !v.comparison.refining && !v.comparison.markdown_loading());
        markdown_ready(cx, &f);
        f.window.update(cx, |v, w, cx| {
            let frame = v.markdown_frame(w, cx).unwrap();
            let l = &frame.layout;
            let doc = &l.projection.pair.sides[1];
            let rows = |text: &str| l.rows[1].iter().filter(|r| r.pieces.iter().any(|p| doc.text[p.range.clone()].contains(text))).collect::<Vec<_>>();
            for row in rows("Opening balances").into_iter().chain(rows("Importing")) {
                assert_eq!(row.change, Some(Kind::Inserted), "an added block's bar is the added color");
                assert!(row.decorations.is_empty(), "no word outlines on a block added whole");
            }
            let edited = rows("every account");
            assert!(edited.iter().all(|r| r.change == Some(Kind::Modified) && !r.decorations.is_empty()), "the edited paragraph keeps its outlines");
            let fills = frame.fills(1, v.palette.as_ref(), 400.);
            let added = rows("Opening balances")[0].y;
            assert!(fills.iter().any(|f| f.kind == FillKind::ChangeBar && f.color == l.style.added && (f.y - added).abs() < l.style.size * 2.), "a bar beside it");
        }).unwrap();
    }

    #[gpui::test]
    fn gui_markdown_preview_shows_local_images_and_refuses_outside_ones(cx: &mut TestAppContext) {
        let outside = tempfile::tempdir().unwrap();
        let f = fixture_named(cx, "# A\n", "# A\n\n![logo](logo.png)\n\n![out](out.png)\n", "md");
        image::RgbaImage::from_pixel(40, 20, image::Rgba([200, 0, 0, 255])).save(f.dir.path().join("logo.png")).unwrap();
        image::RgbaImage::from_pixel(4, 4, image::Rgba([0, 0, 0, 255])).save(outside.path().join("out.png")).unwrap();
        std::os::unix::fs::symlink(outside.path().join("out.png"), f.dir.path().join("out.png")).unwrap();
        run(cx, &f, crate::catalog::Action::Comparison(Command::Preview(crate::markdown::MarkdownLayout::Inline)));
        markdown_ready(cx, &f);
        let images = |v: &Viewer| {
            v.markdown.layout.as_ref().map_or(0, |l| l.rows[1].iter().filter(|r| matches!(r.kind, RowKind::Image(_))).count())
        };
        markdown_ready(cx, &f);
        f.viewer.read_with(cx, |v, _| {
            assert_eq!(images(v), 1, "only the image inside the compared folder");
            let l = v.markdown.layout.as_ref().unwrap();
            let row = l.rows[1].iter().find(|r| matches!(r.kind, RowKind::Image(_))).unwrap();
            assert!((row.height - 20.).abs() < 0.5, "small images keep their size: {}", row.height);
            let doc = &l.projection.pair.sides[1];
            assert!(l.rows[1].iter().any(|r| r.pieces.iter().any(|p| doc.text[p.range.clone()].contains("out"))), "the refused one keeps its alt text");
        });
        run(cx, &f, crate::catalog::Action::Comparison(Command::SetPresentation(PresentationMode::Source)));
        f.window.update(cx, |v, w, _| {
            v.markdown_release_images(w);
            assert!(v.markdown_images.loaded.is_empty() && v.markdown_images.stale.is_empty(), "closing the preview releases its images");
        }).unwrap();
    }

    #[gpui::test]
    fn gui_markdown_open_callouts_fold_and_changed_ones_start_open(cx: &mut TestAppContext) {
        let shows = |v: &Viewer, side: usize, text: &str| {
            let l = v.markdown.layout.as_ref().unwrap();
            let doc = &l.projection.pair.sides[side];
            l.rows[side].iter().any(|r| r.pieces.iter().any(|p| doc.text[p.range.clone()].contains(text)))
        };
        let f = rendered(cx, "> [!note]+ Open\n> body text\n");
        assert!(f.viewer.read_with(cx, |v, _| shows(v, 1, "body text")), "`+` starts open");
        let (x, y) = f.window.update(cx, |v, w, _| {
            let l = v.markdown.layout.as_ref().unwrap();
            let title = l.rows[1].iter().find(|r| matches!(r.kind, RowKind::CalloutTitle(_))).unwrap();
            (v.markdown_pane_x(w, 1) + title.pieces[0].x + 4., v.top() + title.y - v.markdown.top[1] + 4.)
        }).unwrap();
        click(cx, &f, x, y);
        markdown_ready(cx, &f);
        assert!(!f.viewer.read_with(cx, |v, _| shows(v, 1, "body text")), "clicking its title folds it");

        let f = fixture_named(cx, "> [!note]- Details\n> old body\n", "> [!note]- Details\n> new body\n", "md");
        run(cx, &f, crate::catalog::Action::Comparison(Command::Preview(crate::markdown::MarkdownLayout::Inline)));
        markdown_ready(cx, &f);
        run(cx, &f, crate::catalog::Action::Comparison(Command::Preview(crate::markdown::MarkdownLayout::SideBySide)));
        markdown_ready(cx, &f);
        f.window.update(cx, |v, w, _| v.markdown_prepare(w)).unwrap();
        assert!(f.viewer.read_with(cx, |v, _| shows(v, 0, "old body") && shows(v, 1, "new body")), "a folded callout with changes starts open");
    }

    #[gpui::test]
    fn gui_markdown_inline_uses_one_full_width_pane_and_the_switch_changes_it(cx: &mut TestAppContext) {
        let f = rendered_inline(cx, "# A\n\nnew words\n");
        f.window.update(cx, |v, w, cx| {
            let _ = v.render(w, cx);
            let total = f32::from(w.viewport_size().width) - v.diff_x();
            assert_eq!(v.comparison_widths(total), [0., total - GUTTER_W]);
            assert!(v.markdown.layout.as_ref().unwrap().inline);
            assert_eq!(v.header_switch_labels(), ["Source", "Inline", "Side by side"], "one row of modes, like images");
        }).unwrap();
        let (x, y) = f.window.update(cx, |v, _, _| { let c = v.header_switch().into_iter().find(|c| c.2 == "Side by side").unwrap(); (c.0 + c.1 / 2., v.top() - hdr_h() / 2.) }).unwrap();
        click(cx, &f, x, y);
        markdown_ready(cx, &f);
        f.window.update(cx, |v, w, _| {
            let total = f32::from(w.viewport_size().width) - v.diff_x();
            assert!(v.comparison_widths(total)[0] > 0.);
            assert!(!v.markdown.layout.as_ref().unwrap().inline);
        }).unwrap();
        let cell = |cx: &mut TestAppContext, label: &str| f.window.update(cx, |v, w, cx| { let _ = v.render(w, cx); let c = v.header_switch().into_iter().find(|c| c.2 == label).unwrap(); (c.0 + c.1 / 2., v.top() - hdr_h() / 2.) }).unwrap();
        let (x, y) = cell(cx, "Source");
        click(cx, &f, x, y);
        assert_eq!(f.viewer.read_with(cx, |v, _| v.comparison.markdown_mode()), PresentationMode::Source);
        let (x, y) = cell(cx, "Inline");
        click(cx, &f, x, y);
        markdown_ready(cx, &f);
        f.viewer.read_with(cx, |v, _| {
            assert_eq!(v.comparison.markdown_mode(), PresentationMode::Rendered, "one click from Source to the Inline Preview");
            assert!(v.markdown.layout.as_ref().unwrap().inline);
        });
    }

    #[gpui::test]
    fn gui_markdown_inline_lays_out_changes_in_place_once(cx: &mut TestAppContext) {
        let f = fixture_named(cx, "# A\n\nA small service.\n\nGone.\n\nSome *text*.\n", "# A\n\nA small, fast service.\n\nSome _text_.\n\nAdded.\n", "md");
        run(cx, &f, crate::catalog::Action::Comparison(Command::Preview(crate::markdown::MarkdownLayout::Inline)));
        markdown_ready(cx, &f);
        f.viewer.read_with(cx, |v, _| {
            let l = v.markdown.layout.as_ref().unwrap();
            assert!(l.inline && l.rows[0].is_empty());
            let text = |p: &Piece| l.projection.pair.sides[p.side as usize].text[p.range.clone()].to_string();
            let pieces: Vec<_> = l.rows[1].iter().flat_map(|r| &r.pieces).collect();
            assert!(pieces.iter().any(|p| p.side == Side::Right && p.change == Some(Kind::Inserted) && text(p).contains("fast")));
            assert!(pieces.iter().any(|p| p.side == Side::Left && p.change == Some(Kind::Deleted) && text(p).contains("Gone")));
            assert_eq!(pieces.iter().filter(|p| text(p).contains("small")).count(), 1, "unchanged words once");
            assert!(l.rows[1].iter().any(|r| r.change == Some(Kind::Deleted) && r.side == Side::Left));
            assert!(l.rows[1].iter().filter(|r| r.marker.is_some()).count() <= 1, "a markup change pill once, not per side");
        });
    }

    #[gpui::test]
    fn gui_markdown_inline_selection_copy_and_source_follow_the_clicked_side(cx: &mut TestAppContext) {
        let f = fixture_named(cx, "Alpha beta gamma.\n", "Alpha delta gamma.\n", "md");
        run(cx, &f, crate::catalog::Action::Comparison(Command::Preview(crate::markdown::MarkdownLayout::Inline)));
        markdown_ready(cx, &f);
        let at = |v: &Viewer, w: &Window, word: &str| {
            let l = v.markdown.layout.as_ref().unwrap();
            let (row, p) = l.rows[1].iter().flat_map(|r| r.pieces.iter().map(move |p| (r, p)))
                .find(|(_, p)| l.projection.pair.sides[p.side as usize].text[p.range.clone()].contains(word)).unwrap();
            let k = l.projection.pair.sides[p.side as usize].text[p.range.clone()].find(word).unwrap();
            (v.markdown_pane_x(w, 1) + p.x + f32::from(p.shaped.x_for_index(k)) + 2., v.top() + row.y - v.markdown.top[1] + 4.)
        };
        let (x, y) = f.window.update(cx, |v, w, _| at(v, w, "beta")).unwrap();
        click(cx, &f, x, y);
        assert_eq!(f.viewer.read_with(cx, |v, _| v.comparison.markdown_selection().map(|s| s.side)), Some(Side::Left), "a removed word selects the old side");
        // drag from "Alpha" across the removed word to "gamma": one side, the new text
        let ((x0, y0), (x1, y1)) = f.window.update(cx, |v, w, _| (at(v, w, "Alpha"), at(v, w, "gamma"))).unwrap();
        click(cx, &f, x0, y0);
        f.window.update(cx, |v, w, cx| {
            v.on_mouse_move(&MouseMoveEvent { position: point(px(x1 + 20.), px(y1)), pressed_button: Some(MouseButton::Left), ..Default::default() }, w, cx)
        }).unwrap();
        let s = f.viewer.read_with(cx, |v, _| v.comparison.markdown_selection()).unwrap();
        assert_eq!(s.side, Side::Right);
        key(cx, &f, "c", true, false, false);
        let copied = cx.update(|cx| cx.read_from_clipboard().unwrap().text().unwrap());
        assert!(copied.contains("delta") && !copied.contains("beta"), "{copied:?}");
    }

    #[gpui::test]
    fn gui_markdown_inline_next_change_shows_a_removed_block(cx: &mut TestAppContext) {
        let body: String = (0..80).map(|i| format!("Paragraph {i}.\n\n")).collect();
        let f = fixture_named(cx, &format!("{body}REMOVED BLOCK.\n\nTail.\n\n{body}"), &format!("{body}Tail.\n\n{body}"), "md");
        run(cx, &f, crate::catalog::Action::Comparison(Command::Preview(crate::markdown::MarkdownLayout::Inline)));
        markdown_ready(cx, &f);
        run(cx, &f, crate::catalog::Action::Comparison(Command::NextChange));
        f.window.update(cx, |v, w, _| {
            v.markdown_prepare(w);
            let l = v.markdown.layout.as_ref().unwrap();
            let row = l.rows[1].iter().find(|r| r.side == Side::Left && r.change == Some(Kind::Deleted)).unwrap();
            let top = v.markdown.top[1];
            assert!(row.y >= top && row.y < top + v.markdown_height(w), "the struck block is in view");
        }).unwrap();
    }

    #[gpui::test]
    fn gui_markdown_inline_tints_inserted_and_strikes_removed_words(cx: &mut TestAppContext) {
        let f = fixture_named(cx, "Alpha beta gamma.\n", "Alpha delta gamma.\n", "md");
        run(cx, &f, crate::catalog::Action::Comparison(Command::Preview(crate::markdown::MarkdownLayout::Inline)));
        markdown_ready(cx, &f);
        f.window.update(cx, |v, w, cx| {
            let frame = v.markdown_frame(w, cx).unwrap();
            let fills = frame.fills(1, v.palette.as_ref(), 400.);
            let s = &frame.layout.style;
            assert!(fills.iter().any(|f| f.kind == FillKind::ChangeBar && f.color == s.modified));
            // outlined, so changes don't look like highlights, code pills or tags
            assert!(fills.iter().any(|f| f.kind == FillKind::Change && f.border == Some(false) && f.color == s.added), "a solid green outline around the inserted word");
            assert!(fills.iter().any(|f| f.kind == FillKind::Change && f.border == Some(true) && f.color == s.removed), "a dashed red outline around the removed word");
        }).unwrap();
    }

    #[gpui::test]
    fn gui_markdown_inline_end_shows_the_tail(cx: &mut TestAppContext) {
        let left: String = (0..150).map(|i| format!("Paragraph {i} with some words.\n\n")).collect();
        let right = left.replace("Paragraph 10 with", "Paragraph 10 now with").replace("Paragraph 140 with", "Changed 140 with") + "TAIL_END here.\n";
        let f = fixture_named(cx, &left, &right, "md");
        run(cx, &f, crate::catalog::Action::Comparison(Command::Preview(crate::markdown::MarkdownLayout::Inline)));
        markdown_ready(cx, &f);
        key(cx, &f, "end", false, false, false);
        f.window.update(cx, |v, w, cx| {
            assert!(v.markdown_frame(w, cx).is_some());
            let l = v.markdown.layout.as_ref().unwrap();
            let doc = &l.projection.pair.sides[1];
            let tail = doc.text.find("TAIL_END").unwrap();
            let row = l.rows[1].iter().find(|r| r.pieces.iter().any(|p| p.side == Side::Right && p.range.contains(&tail))).unwrap();
            let y = row.y - v.markdown.top[1];
            assert!(y >= 0. && y < v.markdown_height(w), "End shows the tail: row {} top {} height {}", row.y, v.markdown.top[1], l.heights[1]);
        }).unwrap();
    }

    #[gpui::test]
    fn gui_markdown_inline_triple_click_on_a_removed_word_selects_its_old_block(cx: &mut TestAppContext) {
        let f = fixture_named(cx, "# T\n\nKeep.\n\nAlpha beta gamma.\n", "# T\n\nNew one.\n\nNew two.\n\nKeep.\n\nAlpha delta gamma.\n", "md");
        run(cx, &f, crate::catalog::Action::Comparison(Command::Preview(crate::markdown::MarkdownLayout::Inline)));
        markdown_ready(cx, &f);
        let (x, y) = f.window.update(cx, |v, w, _| {
            let l = v.markdown.layout.as_ref().unwrap();
            let (row, p) = l.rows[1].iter().flat_map(|r| r.pieces.iter().map(move |p| (r, p)))
                .find(|(_, p)| p.side == Side::Left && l.projection.pair.sides[0].text[p.range.clone()].contains("beta")).unwrap();
            (v.markdown_pane_x(w, 1) + p.x + 2., v.top() + row.y - v.markdown.top[1] + 4.)
        }).unwrap();
        f.window.update(cx, |v, w, cx| {
            v.on_mouse_down(&MouseDownEvent { button: MouseButton::Left, position: point(px(x), px(y)), click_count: 3, ..Default::default() }, w, cx)
        }).unwrap();
        let s = f.viewer.read_with(cx, |v, _| v.comparison.markdown_selection()).unwrap();
        let text = f.viewer.read_with(cx, |v, _| v.comparison.markdown_projection().unwrap().pair.sides[0].text[s.anchor.min(s.head)..s.anchor.max(s.head)].to_string());
        assert_eq!((s.side, text.as_str()), (Side::Left, "Alpha beta gamma."));
    }

    #[gpui::test]
    fn gui_markdown_inline_wraps_only_between_words(cx: &mut TestAppContext) {
        let left: String = (0..160).map(|i| format!("word{i} ")).collect();
        let right: String = (0..160).map(|i| if i % 3 == 0 { format!("changed{i} ") } else { format!("word{i} ") }).collect();
        let f = fixture_named(cx, &format!("{left}\n"), &format!("{right}\n"), "md");
        cx.simulate_window_resize(f.window.into(), gpui::size(px(700.), px(600.)));
        run(cx, &f, crate::catalog::Action::Comparison(Command::Preview(crate::markdown::MarkdownLayout::Inline)));
        markdown_ready(cx, &f);
        f.viewer.read_with(cx, |v, _| {
            let l = v.markdown.layout.as_ref().unwrap();
            let rows: Vec<_> = l.rows[1].iter().filter(|r| r.pieces.len() > 1 || r.pieces.iter().any(|p| p.change.is_some())).collect();
            assert!(rows.len() > 3, "the paragraph wraps");
            for row in &rows[..rows.len() - 1] {
                let p = row.pieces.last().unwrap();
                let doc = &l.projection.pair.sides[p.side as usize].text;
                let before = doc[..p.range.end].chars().next_back();
                let after = doc[p.range.end..].chars().next();
                assert!(before.is_some_and(char::is_whitespace) || after.is_none_or(char::is_whitespace), "a row ends inside a word: {:?}", &doc[p.range.clone()]);
            }
        });
    }

    #[gpui::test]
    fn gui_markdown_inline_strikes_a_removed_table(cx: &mut TestAppContext) {
        let f = fixture_named(cx, "# T\n\n| a | b |\n| - | - |\n| one | two |\n\nEnd.\n", "# T\n\nEnd.\n", "md");
        run(cx, &f, crate::catalog::Action::Comparison(Command::Preview(crate::markdown::MarkdownLayout::Inline)));
        markdown_ready(cx, &f);
        f.viewer.read_with(cx, |v, _| {
            let l = v.markdown.layout.as_ref().unwrap();
            let doc = &l.projection.pair.sides[0].text;
            assert!(l.rows[1].iter().flat_map(|r| &r.pieces).any(|p| p.side == Side::Left && p.change == Some(Kind::Deleted) && doc[p.range.clone()].contains("one")));
        });
    }

    #[gpui::test]
    fn gui_markdown_preview_outlines_words_of_changes_never_shown_in_source(cx: &mut TestAppContext) {
        let body: String = (0..400).map(|i| format!("Paragraph {i} has some words.\n\n")).collect();
        let right = body.replace("Paragraph 350 has some", "Paragraph 350 has other");
        let f = fixture_named(cx, &body, &right, "md");
        run(cx, &f, crate::catalog::Action::Comparison(Command::Preview(crate::markdown::MarkdownLayout::SideBySide)));
        markdown_ready(cx, &f);
        // render asks for refinement batch by batch until every change is in
        let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            f.window.update(cx, |v, w, cx| { let _ = v.render(w, cx); }).unwrap();
            cx.executor().advance_clock(std::time::Duration::from_millis(20));
            cx.executor().run_until_parked();
            let done = f.viewer.read_with(cx, |v, _| !v.comparison.refining && v.comparison.refine_targets().is_empty() && !v.comparison.markdown_loading());
            if done { break; }
            assert!(std::time::Instant::now() < until);
        }
        f.viewer.read_with(cx, |v, _| {
            let p = v.comparison.markdown_projection().unwrap();
            let doc = &p.pair.sides[1];
            let d = p.decorations.iter().find(|d| d.side == Side::Right && !d.markup).unwrap();
            assert_eq!(&doc.text[d.text.clone()], "other", "the word, not the whole paragraph");
        });
    }

    #[gpui::test]
    fn gui_markdown_preview_independent_scrolling_moves_one_pane(cx: &mut TestAppContext) {
        let text: String = (0..200).map(|i| format!("Paragraph {i}\n\n")).collect();
        let f = rendered(cx, &text);
        for mode in [ScrollMode::Aligned, ScrollMode::Independent] {
            f.viewer.update(cx, |v, cx| v.comparison_command(Command::SetScroll(mode), cx));
            markdown_ready(cx, &f);
            f.window.update(cx, |v, w, _| {
                assert_eq!(v.markdown.layout.as_ref().unwrap().mode, mode);
                v.markdown.top = [0.; 2];
                v.markdown_scroll(Side::Right, 400., 0., w);
                assert!(v.markdown.top[1] > 0., "{mode:?}: the scrolled pane moves");
                assert_eq!(v.markdown.top[0], 0., "{mode:?}: the other pane stays");
            }).unwrap();
        }
    }

    /// Side by side, as most Preview tests expect two panes.
    fn rendered(cx: &mut TestAppContext, text: &str) -> super::super::parity_tests::Fixture {
        rendered_as(cx, text, crate::markdown::MarkdownLayout::SideBySide)
    }
    fn rendered_inline(cx: &mut TestAppContext, text: &str) -> super::super::parity_tests::Fixture {
        rendered_as(cx, text, crate::markdown::MarkdownLayout::Inline)
    }
    fn rendered_as(cx: &mut TestAppContext, text: &str, layout: crate::markdown::MarkdownLayout) -> super::super::parity_tests::Fixture {
        let f = fixture_named(cx, "# Before\n", text, "md");
        f.window
            .update(cx, |v, w, cx| {
                let _ = v.render(w, cx);
                v.run_action(crate::catalog::Action::Comparison(Command::SetPresentation(PresentationMode::Rendered)),w,cx);
                v.run_action(crate::catalog::Action::Comparison(Command::SetMarkdownLayout(layout)),w,cx)
            })
            .unwrap();
        markdown_ready(cx, &f);
        f.window
            .update(cx, |v, w, _| {
                v.markdown_prepare(w);
            })
            .unwrap();
        f
    }
    #[test]
    fn gui_markdown_final_navigation_reveals_hunks_and_markup() {
        let mut native=TestAppContext::build_with_text_system(gpui::TestDispatcher::new(0),None,native_text_system());
        let cx=&mut native;
        let left=(0..120).map(|i|format!("LINE_{i:03} α界 original words extend across the row here\n")).collect::<String>();
        let right=left.replace("LINE_003 α界 original", "RIGHT_CHANGE_003 α界 changed").replace("LINE_070 α界 original", "RIGHT_CHANGE_070 α界 changed");
        let f=fixture_named(cx,&left,&right,"md");
        cx.simulate_window_resize(f.window.into(),gpui::size(px(800.),px(480.)));
        run(cx,&f,crate::catalog::Action::Comparison(Command::Preview(crate::markdown::MarkdownLayout::Inline)));markdown_ready(cx,&f);
        run(cx,&f,crate::catalog::Action::Comparison(Command::Preview(crate::markdown::MarkdownLayout::SideBySide)));markdown_ready(cx,&f);
        for (id,label,input) in [(0,"RIGHT_CHANGE_003","n"),(1,"RIGHT_CHANGE_070","n"),(0,"RIGHT_CHANGE_003","p")] {
            key(cx,&f,input,false,false,false);
            f.window.update(cx,|v,w,cx|{let frame=v.markdown_frame(w,cx).unwrap();let l=&frame.layout;assert_eq!(v.comparison.view.current_change(),Some(id));
                assert!(l.heights[1]>v.markdown_height(w)*3.,"fixture must extend well beyond the viewport");
                assert!(l.rows[1].iter().filter(|r|r.y+r.height>v.markdown.top[1]&&r.y<v.markdown.top[1]+v.markdown_height(w)).any(|r|r.decorations.iter().any(|&d|l.projection.decorations[d].hunk==id)),"visible hunk decoration");
                let visible=l.rows[1].iter().filter(|r|r.y+r.height>v.markdown.top[1]&&r.y<v.markdown.top[1]+v.markdown_height(w)).flat_map(|r|&r.pieces).map(|p|&l.projection.pair.sides[1].text[p.range.clone()]).collect::<String>();assert!(visible.contains(label),"{label}: {visible}");
            }).unwrap();
        }
    }
    #[test]
    fn gui_markdown_final_markup_marker_is_independently_visible() {
        let mut native=TestAppContext::build_with_text_system(gpui::TestDispatcher::new(0),None,native_text_system());
        let cx=&mut native;
        let left=format!("{}[identical](old)\n", "long paragraph words α界 ".repeat(1000));
        let right=left.replace("](old)","](new)");
        let f=fixture_named(cx,&left,&right,"md");run(cx,&f,crate::catalog::Action::Comparison(Command::Preview(crate::markdown::MarkdownLayout::Inline)));markdown_ready(cx,&f);
        key(cx,&f,"n",false,false,false);
        let (x,y,a)=f.window.update(cx,|v,w,cx|{let frame=v.markdown_frame(w,cx).unwrap();let l=&frame.layout;let row=l.rows[1].iter().find(|r|r.marker.is_some()).unwrap();assert!(row.y>=v.markdown.top[1]&&row.y<v.markdown.top[1]+v.markdown_height(w),"selected markup marker must be visible");(v.markdown_pane_x(w,1)+20.,v.top()+row.y-v.markdown.top[1]+3.,l.row_anchor(1,l.rows[1].iter().position(|r|r.marker.is_some()).unwrap()).unwrap())}).unwrap();
        f.window.update(cx,|v,w,cx|v.on_right_down(&MouseDownEvent{position:point(px(x),px(y)),button:MouseButton::Right,..Default::default()},w,cx)).unwrap();
        assert_eq!(f.viewer.read_with(cx,|v,_|v.comparison.markdown_mode()),PresentationMode::Rendered);
        assert_eq!(f.viewer.read_with(cx,|v,_|v.comparison.markdown_anchor(Side::Right)),Some(a));
        key(cx,&f,"escape",false,false,false);
        click(cx,&f,x,y);assert_eq!(f.viewer.read_with(cx,|v,_|v.comparison.markdown_mode()),PresentationMode::Source);assert_eq!(f.viewer.read_with(cx,|v,_|v.comparison.markdown_anchor(Side::Right)),Some(a));
    }
    #[test]
    fn gui_markdown_final_context_uses_pointer_without_left_click() {
        let mut native=TestAppContext::build_with_text_system(gpui::TestDispatcher::new(0),None,native_text_system());
        let cx=&mut native;
        let f=rendered(cx,"# First\n\nsecond direct target\n\nentity &amp; end\n\n- list target\n");
        let context_at=|cx:&mut TestAppContext, needle:&str| {
            let (x,y)=f.window.update(cx,|v,w,_|{v.markdown_prepare(w);let l=v.markdown.layout.as_ref().unwrap();let at=l.projection.pair.sides[1].text.find(needle).unwrap();let row=l.rows[1].iter().find(|r|r.pieces.iter().any(|p|p.range.contains(&at))).unwrap();let piece=row.pieces.iter().find(|p|p.range.contains(&at)).unwrap();(v.markdown_pane_x(w,1)+piece.x+f32::from(piece.shaped.x_for_index(at-piece.range.start))+0.1,v.top()+row.y-v.markdown.top[1]+2.)}).unwrap();
            f.window.update(cx,|v,w,cx|v.on_right_down(&MouseDownEvent{position:point(px(x),px(y)),button:MouseButton::Right,..Default::default()},w,cx)).unwrap();
        };
        // No prior left click, then a stale empty caret on a different paragraph.
        for (needle,line) in [("second",2),("First",0)] {
            context_at(cx,needle);
            f.viewer.read_with(cx,|v,_|{let s=v.comparison.markdown_selection().unwrap();assert_eq!(s.anchor,s.head);assert_eq!(v.comparison.markdown_inspection_position(Side::Right).unwrap().unwrap().line,line);});
            let index=f.viewer.read_with(cx,|v,_|v.menu.as_ref().unwrap().items.iter().position(|(_,a)|matches!(a,MenuAction::InteractionCommand(commands::Interaction::Editor))).unwrap());
            key(cx,&f,"home",false,false,false);for _ in 0..index {key(cx,&f,"down",false,false,false);}key(cx,&f,"enter",false,false,false);
            assert_eq!(f.viewer.read_with(cx,|v,_|v.inspected_position.unwrap().2),Some(line as usize+1));
        }
        context_at(cx,"&");assert!(f.viewer.read_with(cx,|v,_|v.comparison.markdown_inspection_position(Side::Right).is_err()));key(cx,&f,"escape",false,false,false);
        let prefix=f.viewer.read_with(cx,|v,_|{let d=&v.comparison.markdown_projection().unwrap().pair.sides[1];let b=d.blocks.iter().find(|b|matches!(b.kind,BlockKind::ListItem{..})).unwrap();d.text[b.text.start..].chars().next().unwrap().to_string()});
        context_at(cx,&prefix);assert!(f.viewer.read_with(cx,|v,_|v.comparison.markdown_inspection_position(Side::Right).is_err()));key(cx,&f,"escape",false,false,false);
        key(cx,&f,"a",true,false,false);let selection=f.viewer.read_with(cx,|v,_|v.comparison.markdown_selection());context_at(cx,"second");assert_eq!(f.viewer.read_with(cx,|v,_|v.comparison.markdown_selection()),selection);
        key(cx,&f,"escape",false,false,false);
        f.window.update(cx,|v,w,cx|{v.on_right_down(&MouseDownEvent{position:point(px(v.diff_x()+20.),px(v.top()+20.)),button:MouseButton::Right,..Default::default()},w,cx);assert_eq!(v.comparison.markdown_selection(),selection);assert_eq!(v.diff_focus,Side::Left);assert!(v.interaction_disabled(commands::Interaction::Copy).is_none());assert!(v.comparison.markdown_inspection_position(Side::Left).is_ok());}).unwrap();
    }
    #[test]
    fn gui_markdown_final_mode_switch_preserves_source_viewport_and_selection() {
        let mut native=TestAppContext::build_with_text_system(gpui::TestDispatcher::new(0),None,native_text_system());
        let cx=&mut native;
        let right=(0..300).map(|i|format!("Paragraph_{i:03} α界 words words words words words\n\n")).collect::<String>();
        let f=fixture_named(cx,&right.replace("Paragraph_000","Old_000"),&right,"md");
        cx.simulate_window_resize(f.window.into(),gpui::size(px(800.),px(480.)));
        key(cx,&f,"end",false,false,false);
        let source=f.viewer.read_with(cx,|v,_|(v.comparison.view.current_change(),v.comparison.view.top_line_on(Side::Right),v.sel));
        f.window.update(cx,|v,w,cx|v.run_action(crate::catalog::Action::Comparison(Command::SetPresentation(PresentationMode::Rendered)),w,cx)).unwrap();markdown_ready(cx,&f);
        f.window.update(cx,|v,w,cx|{let frame=v.markdown_frame(w,cx).unwrap();assert_eq!((v.comparison.view.current_change(),v.comparison.view.top_line_on(Side::Right),v.sel),source);let l=frame.layout;let visible=l.rows[1].iter().filter(|r|r.y+r.height>v.markdown.top[1]&&r.y<v.markdown.top[1]+v.markdown_height(w)).flat_map(|r|&r.pieces).map(|p|&l.projection.pair.sides[1].text[p.range.clone()]).collect::<String>();assert!(visible.contains("Paragraph_299"),"{visible}");}).unwrap();
        run(cx,&f,crate::catalog::Action::Comparison(Command::SetPresentation(PresentationMode::Source)));key(cx,&f,"home",false,false,false);key(cx,&f,"a",true,false,false);
        let source=f.viewer.read_with(cx,|v,_|(v.comparison.view.current_change(),v.comparison.view.top_line_on(Side::Right),v.sel));
        run(cx,&f,crate::catalog::Action::Comparison(Command::Preview(crate::markdown::MarkdownLayout::Inline)));markdown_ready(cx,&f);
        f.window.update(cx,|v,w,cx|{let frame=v.markdown_frame(w,cx).unwrap();assert_eq!((v.comparison.view.current_change(),v.comparison.view.top_line_on(Side::Right),v.sel),source);let l=frame.layout;assert!(l.rows[1].iter().filter(|r|r.y+r.height>v.markdown.top[1]&&r.y<v.markdown.top[1]+v.markdown_height(w)).flat_map(|r|&r.pieces).any(|p|l.projection.pair.sides[1].text[p.range.clone()].contains("Paragraph_000")));}).unwrap();
    }
    #[gpui::test]
    fn gui_markdown_same_document_options_hold_native_frame_until_acceptance(cx:&mut TestAppContext) {
        let f=rendered(cx,"# Heading\n\n**Changed** words\n");
        let old=f.window.update(cx,|v,w,cx|v.markdown_frame(w,cx).unwrap().layout).unwrap();
        f.window.update(cx,|v,w,cx|{
            v.run_action(crate::catalog::Action::Comparison(Command::SetHighlight(lenz_engine::HighlightLevel::Line)),w,cx);
            assert_ne!(v.comparison.generation,v.comparison.diffed);
            let held=v.markdown_frame(w,cx).unwrap();
            assert!(Arc::ptr_eq(&held.layout,&old));
            assert!(Arc::ptr_eq(&held.layout.projection,v.comparison.markdown_projection().unwrap()));
        }).unwrap();
        super::super::parity_tests::workflow_wait(cx,&f,|v|v.comparison.generation==v.comparison.diffed&&!v.comparison.markdown_loading());
        // the accepted projection's layout is shaped in the background; the held one stays until it lands
        super::super::parity_tests::markdown_ready(cx,&f);
        f.window.update(cx,|v,w,cx|{let new=v.markdown_frame(w,cx).unwrap();assert!(!Arc::ptr_eq(&new.layout,&old));assert!(!Arc::ptr_eq(&new.layout.projection,&old.projection));assert!(Arc::ptr_eq(&new.layout.projection.pair,&old.projection.pair));}).unwrap();
    }
    #[gpui::test]
    fn gui_markdown_literal_math_and_mapped_editor_use_actual_input(cx:&mut TestAppContext) {
        let f=rendered(cx,"# 漢字\n\n$**bold**$ and $$*math*$$\n\nentity &amp;\n");
        f.window.update(cx,|v,w,cx|{let frame=v.markdown_frame(w,cx).unwrap();let doc=&frame.layout.projection.pair.sides[1];assert!(doc.text.contains("$**bold**$ and $$*math*$$"));}).unwrap();
        let (x,y)=f.window.update(cx,|v,w,_|{v.markdown_prepare(w);assert!(v.markdown.layout.is_some());(v.markdown_pane_x(w,1)+16.,v.top()+2.)}).unwrap();
        click(cx,&f,x,y);key(cx,&f,"o",false,false,false);
        assert_eq!(f.viewer.read_with(cx,|v,_|v.inspected_position),Some((0,Side::Right,Some(1),Some(3))));
        let (x,y)=f.window.update(cx,|v,w,_|{v.markdown_prepare(w);let l=v.markdown.layout.as_ref().unwrap();let row=l.rows[1].iter().find(|r|r.pieces.iter().any(|p|l.projection.pair.sides[1].text[p.range.clone()].contains('&'))).unwrap();let piece=row.pieces.iter().find(|p|l.projection.pair.sides[1].text[p.range.clone()].contains('&')).unwrap();let at=l.projection.pair.sides[1].text[piece.range.clone()].find('&').unwrap();(v.markdown_pane_x(w,1)+piece.x+f32::from(piece.shaped.x_for_index(at))+1.,v.top()+row.y+2.)}).unwrap();
        click(cx,&f,x,y);key(cx,&f,"o",false,false,false);
        assert_eq!(f.viewer.read_with(cx,|v,_|v.toast.as_ref().unwrap().0.clone()),"Show in source to inspect this position");
    }
    #[gpui::test]
    fn gui_markdown_native_layout_formats_and_reaches_tail(cx: &mut TestAppContext) {
        let text = format!(
            "# Heading\n\nA **strong** and *italic* and `code` sentence.\n\n> quote\n\n| first | second |\n| --- | ---: |\n| α | long value |\n\n```rust\nfn main() {{}}\n```\n\n{}\n\nTAIL終\n",
            "Long paragraph words α界 e\u{301} ".repeat(1000)
        );
        let f = rendered(cx, &text);
        f.viewer.read_with(cx, |v, _| {
            let l = v.markdown.layout.as_ref().unwrap();
            let rows = &l.rows[1];
            assert_eq!(rows[0].pieces[0].shaped.text.as_ref(), "Heading");
            assert!(rows[0].height > line_h());
            assert!(rows.iter().any(|r| r.pieces.len() == 2));
            assert!(
                rows.iter()
                    .flat_map(|r| &r.pieces)
                    .any(|p| p.shaped.text.as_ref() == "fn main() {}")
            );
            assert!(l.heights[1] > 5000.);
            assert_eq!(
                rows.last()
                    .unwrap()
                    .pieces
                    .last()
                    .unwrap()
                    .shaped
                    .text
                    .as_ref(),
                "TAIL終"
            );
        });
        key(cx, &f, "end", false, false, false);
        let top = f.viewer.read_with(cx, |v, _| v.markdown.top[1]);
        assert!(top > 4000.);
        key(cx, &f, "up", false, false, false);
        assert!(f.viewer.read_with(cx, |v, _| v.markdown.top[1]) < top);
        let anchor = f.viewer.read_with(cx, |v, _| {
            v.comparison.markdown_anchor(Side::Right).unwrap()
        });
        assert!(
            anchor.byte > text.len() / 2,
            "scroll inside a long block must not map to its start"
        );
        run(cx,&f,crate::catalog::Action::Comparison(Command::SetPresentation(PresentationMode::Source)));
        assert!(f.viewer.read_with(cx, |v, _| v.markdown.layout.is_none()));
    }
    #[gpui::test]
    fn gui_markdown_click_selection_copy_marker_and_source_restore(cx: &mut TestAppContext) {
        let f = rendered(cx, "# Hello **world**\n\n<!-- hidden target -->\n");
        let (x, y) = f.window.update(cx, |v, w, _| {
            assert!(v.markdown.layout.is_some());
            (v.markdown_pane_x(w, 1) + 20., v.top() + 15.)
        }).unwrap();
        click(cx, &f, x, y);
        assert!(
            f.viewer
                .read_with(cx, |v, _| v.comparison.markdown_selection().is_some())
        );
        assert_eq!(f.viewer.read_with(cx, |v, _| v.sel), None);
        key(cx, &f, "a", true, false, false);
        key(cx, &f, "c", true, false, false);
        assert_eq!(
            cx.update(|cx| cx.read_from_clipboard().unwrap().text().unwrap()),
            "Hello world"
        );
        run(cx,&f,crate::catalog::Action::Interaction(commands::Interaction::CopyMarkdownSource));
        assert_eq!(
            cx.update(|cx| cx.read_from_clipboard().unwrap().text().unwrap()),
            "# Hello **world**\n"
        );
        let marker = fixture_named(
            cx,
            "# Same\n\n<!-- old -->\n",
            "# Same\n\n<!-- new -->\n",
            "md",
        );
        marker.viewer.update(cx, |v, cx| {
            v.comparison_command(Command::SetPresentation(PresentationMode::Rendered), cx)
        });
        markdown_ready(cx, &marker);
        marker
            .window
            .update(cx, |v, w, _| v.markdown_prepare(w))
            .unwrap();
        let f = marker;
        let (mx, my) = f.window.update(cx, |v, w, _| {
            let l = v.markdown.layout.as_ref().unwrap();
            let r = l.rows[1].iter().find(|r| r.marker.is_some()).unwrap();
            (v.markdown_pane_x(w, 1) + 20., v.top() + r.y + 3.)
        }).unwrap();
        click(cx, &f, mx, my);
        assert_eq!(
            f.viewer.read_with(cx, |v, _| v.comparison.markdown_mode()),
            PresentationMode::Source
        );
        assert!(f.viewer.read_with(cx, |v, _| v.markdown.layout.is_none()));
    }
    #[gpui::test]
    fn gui_markdown_hidden_find_goto_pending_and_modal_preserve_source_selection(
        cx: &mut TestAppContext,
    ) {
        let f = fixture_named(cx, "# Before\n", "# Heading\n\n<!-- target -->\n", "md");
        let original = Selection::caret(Side::Left, Pos::new(0, 2));
        f.viewer.update(cx, |v, cx| {
            v.sel = Some(original);
            v.comparison_command(Command::SetPresentation(PresentationMode::Rendered), cx);
        });
        click(cx, &f, 700., 80.);
        key(cx, &f, "a", true, false, false);
        assert_eq!(f.viewer.read_with(cx, |v, _| v.sel), Some(original));
        markdown_ready(cx, &f);
        f.viewer.update(cx, |v, _| v.diff_focus = Side::Right);
        key(cx, &f, "f", true, false, false);
        for ch in "target".chars() {
            key(cx, &f, &ch.to_string(), false, false, false);
        }
        assert!(
            f.viewer.read_with(cx, |v, _| v
                .comparison
                .markdown_find_hit()
                .is_some_and(|(_, _, spans)| spans.iter().all(Range::is_empty))),
            "find: {:?}, mode {:?}",
            f.viewer
                .read_with(cx, |v, _| v.comparison.find.as_ref().map(|f| (
                    f.query.clone(),
                    f.side,
                    f.hits.clone(),
                    f.at
                ))),
            f.viewer.read_with(cx, |v, _| v.comparison.markdown_mode())
        );
        assert_eq!(f.viewer.read_with(cx, |v, _| v.sel), Some(original));
        let (x, y) = f
            .window
            .update(cx, |v, w, _| {
                v.markdown_prepare(w);
                assert!(v.markdown.layout.is_some());
                (v.markdown_pane_x(w, 1) + 20., v.top() + 44.)
            })
            .unwrap();
        click(cx, &f, x, y);
        assert_eq!(
            f.viewer.read_with(cx, |v, _| v.comparison.markdown_mode()),
            PresentationMode::Source
        );
        f.window
            .update(cx, |v, w, cx| {
                v.comparison_command(Command::SetPresentation(PresentationMode::Rendered), cx);
                v.about = Some(about::About::open());
                // a click on the header's Source while About is open
                let (x, y) = { let c = v.header_switch().into_iter().find(|c| c.2 == "Source").unwrap(); (c.0 + c.1 / 2., v.top() - hdr_h() / 2.) };
                v.on_mouse_down(&MouseDownEvent { button: MouseButton::Left, position: point(px(x), px(y)), ..Default::default() }, w, cx);
                assert!(v.markdown_active());
                v.close_about(w);
            })
            .unwrap();
    }
    #[gpui::test]
    fn gui_markdown_resize_drag_keeps_the_layout_until_the_widths_settle(cx: &mut TestAppContext) {
        let f = rendered(cx, &"A paragraph long enough to wrap at several widths. ".repeat(40));
        let layout = |cx: &mut TestAppContext, paint: bool| {
            f.window
                .update(cx, |v, w, cx| {
                    if paint { let _ = v.markdown_frame(w, cx); } else { v.markdown_prepare(w); }
                    v.markdown.layout.clone().unwrap()
                })
                .unwrap()
        };
        let first = layout(cx, true);
        // mid-drag: the paint keeps the previous layout, clipped to the new panes, and starts no rebuild
        for w in [700., 650.] {
            cx.simulate_window_resize(f.window.into(), size(px(w), px(600.)));
            assert!(Arc::ptr_eq(&layout(cx, true), &first));
            assert!(!f.viewer.read_with(cx, |v, _| v.markdown_building()));
        }
        // the widths hold: the next paint starts a background rebuild; the layout on screen stays until it lands
        f.viewer.update(cx, |v, _| v.markdown.resized = v.markdown.resized.map(|(w, t)| (w, t - SETTLE)));
        assert!(Arc::ptr_eq(&layout(cx, true), &first));
        assert!(f.viewer.read_with(cx, |v, _| v.markdown_building()));
        cx.run_until_parked();
        let settled = layout(cx, true);
        assert!(!Arc::ptr_eq(&settled, &first) && v_widths(cx, &f) == settled.widths);
        assert!(!f.viewer.read_with(cx, |v, _| v.markdown_building()));
        // input uses the layout on screen and hit-tests at the current pane widths
        cx.simulate_window_resize(f.window.into(), size(px(500.), px(600.)));
        assert!(Arc::ptr_eq(&layout(cx, false), &settled));
        let (x, expected) = f.window.update(cx, |v, w, _| (v.markdown_pane_x(w, 1), v.diff_x() + v_widths_of(v, w)[0] + GUTTER_W)).unwrap();
        assert_eq!(x, expected);
    }
    /// The far right of the widest unwrapped line is reachable, short of the strip, and the next frame keeps it.
    #[gpui::test]
    fn gui_markdown_preview_scrolls_to_the_end_of_its_widest_line(cx: &mut TestAppContext) {
        for inline in [false, true] {
            let text = format!("# Wide\n\n```\n{}\n```\n", "x".repeat(400));
            let f = if inline { rendered_inline(cx, &text) } else { rendered(cx, &text) };
            f.window.update(cx, |v, w, cx| {
                v.markdown_scroll(Side::Right, 0., 1e6, w);
                let _ = v.markdown_frame(w, cx);
                let _ = v.render(w, cx);
                let l = v.markdown.layout.clone().unwrap();
                assert!(l.widest[1] > l.widths[1], "the line is wider than the pane");
                assert_eq!(l.widths, v.markdown_text_widths(w), "settled at the current panes");
                assert_eq!(v.markdown.horizontal[1], l.widest[1] - l.widths[1], "inline {inline}");
            }).unwrap();
        }
    }
    fn v_widths_of(v: &Viewer, w: &Window) -> [f32; 2] {
        v.comparison_widths(f32::from(w.viewport_size().width) - v.diff_x())
    }
    /// The panes' text widths, short of their scrollbar strips: what a layout wraps to.
    fn v_widths(cx: &mut TestAppContext, f: &super::super::parity_tests::Fixture) -> [f32; 2] {
        f.window.update(cx, |v, w, _| v_widths_of(v, w).map(|w| w - super::super::scrollbars::strip_w(w))).unwrap()
    }
    #[gpui::test]
    fn gui_the_settings_markdown_default_starts_markdown_rendered_and_leaves_other_files_alone(cx: &mut TestAppContext) {
        for (extension, rendered) in [("md", true), ("txt", false)] {
            let f = fixture_named(cx, "# Old\n", "# New\n", extension);
            f.window.update(cx, |v, _, cx| v.start_rendered(cx)).unwrap();
            super::super::parity_tests::markdown_projected(cx, &f);
            let mode = f.viewer.read_with(cx, |v, _| v.comparison.markdown_mode());
            assert_eq!(mode == PresentationMode::Rendered, rendered, "{extension}");
        }
    }
    #[gpui::test]
    fn gui_markdown_switch_shows_source_until_the_background_layout_lands(cx: &mut TestAppContext) {
        let f = fixture_named(cx, "# Before\n", &"Some **rendered** text.\n\n".repeat(200), "md");
        f.window
            .update(cx, |v, w, cx| {
                let _ = v.render(w, cx);
                v.run_action(crate::catalog::Action::Comparison(Command::SetPresentation(PresentationMode::Rendered)),w,cx)
            })
            .unwrap();
        super::super::parity_tests::markdown_projected(cx, &f);
        // the first paint after the projection is ready starts the layout and shows the source meanwhile
        let first = f.window.update(cx, |v, w, cx| (v.markdown_frame(w, cx).is_some(), v.markdown_building())).unwrap();
        assert_eq!(first, (false, true));
        cx.run_until_parked();
        let ready = f.window.update(cx, |v, w, cx| (v.markdown_frame(w, cx).is_some(), v.markdown_building())).unwrap();
        assert_eq!(ready, (true, false));
    }
    #[gpui::test]
    fn gui_markdown_all_themes_resize_release_and_source_laziness(cx: &mut TestAppContext) {
        let source = fixture_named(cx, "# old\n", "# new\n", "md");
        source
            .window
            .update(cx, |v, w, cx| {
                assert!(v.markdown_frame(w, cx).is_none());
                assert!(v.comparison.markdown_projection().is_none());
            })
            .unwrap();
        let f = rendered(
            cx,
            "# Heading\n\n**strong** normal\n\n```\nlong code line α界\n```\n",
        );
        let pair = f.viewer.read_with(cx, |v, _| {
            Arc::downgrade(&v.comparison.markdown_projection().unwrap().pair)
        });
        for entry in theme::builtin_entries() {
            f.window
                .update(cx, |v, w, cx| v.tool(Tool::Theme, 0., w, cx))
                .unwrap();
            let at = f.viewer.read_with(cx, |v, _| {
                v.theme_chooser
                    .as_ref()
                    .unwrap()
                    .entries
                    .iter()
                    .position(|e| e.id == entry.id)
                    .unwrap()
            });
            key(cx, &f, "home", false, false, false);
            for _ in 0..at {
                key(cx, &f, "down", false, false, false);
            }
            key(cx, &f, "enter", false, false, false);
            for (w, h) in [(800., 600.), (300., 150.), (80., 80.), (0., 0.)] {
                cx.simulate_window_resize(f.window.into(), size(px(w), px(h)));
                cx.executor().run_until_parked();
                f.window
                    .update(cx, |v, w, _| {
                        v.markdown_prepare(w);
                        let Some(l) = v.markdown.layout.as_ref() else {
                            assert!(f32::from(w.viewport_size().width) <= GUTTER_W);
                            return;
                        };
                        assert!(Arc::ptr_eq(&l.projection.pair, &pair.upgrade().unwrap()));
                        assert!(l.rows[1].iter().all(|r| r.y.is_finite()));
                    })
                    .unwrap();
                key(cx, &f, "end", false, false, false);
                key(cx, &f, "up", false, false, false);
            }
        }
        f.viewer.update(cx, |v, cx| {
            v.comparison_command(Command::SetPresentation(PresentationMode::Source), cx)
        });
        assert!(f.viewer.read_with(cx, |v, _| v.markdown.layout.is_none()));
        f.viewer.update(cx, |v, _| {
            v.comparison.markdown_invalidate();
        });
        cx.executor().run_until_parked();
        assert!(pair.upgrade().is_none());
    }
    #[gpui::test]
    fn gui_markdown_focused_horizontal_wheel_and_wrapped_end_affinity(cx: &mut TestAppContext) {
        let content = format!(
            "# Title\n\n{}\n\n> ```\n> {}\n> ```\n",
            "Words α界 e\u{301} ".repeat(80),
            "long ".repeat(200)
        );
        let f = fixture_named(cx, &content.replace("Title", "Before"), &content, "md");
        f.window
            .update(cx, |v, w, cx| {
                let _ = v.render(w, cx);
            })
            .unwrap();
        f.viewer.update(cx, |v, cx| {
            v.comparison_command(Command::SetPresentation(PresentationMode::Rendered), cx);
            v.comparison_command(Command::SetMarkdownLayout(crate::markdown::MarkdownLayout::SideBySide), cx)
        });
        markdown_ready(cx, &f);
        f.window
            .update(cx, |v, w, cx| {
                v.comparison_command(Command::SetScroll(ScrollMode::Aligned), cx);
                v.markdown_prepare(w);
            })
            .unwrap();
        let (x, y) = f.window.update(cx, |v, w, _| {
            let l = v.markdown.layout.as_ref().unwrap();
            let r = l.rows[1].iter().find(|r| r.block == Some(1)).unwrap();
            (v.markdown_pane_x(w, 1) + 20., v.top() + r.y + 4.)
        }).unwrap();
        click(cx, &f, x, y);
        key(cx, &f, "end", false, false, false);
        let end = f
            .viewer
            .read_with(cx, |v, _| v.comparison.markdown_selection().unwrap());
        let (row, first_end) = f.viewer.read_with(cx, |v, _| {
            let row = v.markdown.affinity.unwrap().1;
            (
                row,
                v.markdown.layout.as_ref().unwrap().rows[1][row]
                    .pieces
                    .last()
                    .unwrap()
                    .range
                    .end,
            )
        });
        assert_eq!(end.head, first_end);
        key(cx, &f, "home", false, false, false);
        assert_eq!(
            f.viewer
                .read_with(cx, |v, _| v.markdown.affinity.unwrap().1),
            row
        );
        f.viewer.update(cx, |v, _| v.diff_focus = Side::Right);
        key(cx, &f, "right", true, false, true);
        assert!(f.viewer.read_with(cx, |v, _| v.markdown.horizontal[1] > 0.
            && v.markdown.horizontal[0] == 0.));
        f.window
            .update(cx, |v, w, cx| {
                let ev = ScrollEvent {
                    position: point(px(v.diff_x() + 20.), px(v.top() + 40.)),
                    delta: ScrollDelta::Lines(point(-5., 0.)),
                    ..Default::default()
                };
                v.on_wheel(&ev, w, cx);
            })
            .unwrap();
        assert!(f.viewer.read_with(cx, |v, _| v.markdown.horizontal[0] > 0.));
        let source = f.viewer.read_with(cx, |v, _| v.sel);
        key(cx, &f, "g", true, false, false);
        key(cx, &f, "5", false, false, false);
        key(cx, &f, "enter", false, false, false);
        assert_eq!(f.viewer.read_with(cx, |v, _| v.sel), source);
    }
    #[gpui::test]
    fn gui_markdown_vertical_caret_crosses_markup_rows(cx: &mut TestAppContext) {
        let f = fixture_named(
            cx,
            "Before text\n\n<!-- old -->\n\nAfter text\n",
            "Before text\n\n<!-- new -->\n\nAfter text\n",
            "md",
        );
        f.viewer.update(cx, |v, cx| {
            v.comparison_command(Command::SetPresentation(PresentationMode::Rendered), cx)
        });
        markdown_ready(cx, &f);
        let (x, before_y, after_y) = f
            .window
            .update(cx, |v, w, _| {
                v.markdown_prepare(w);
                let l = v.markdown.layout.as_ref().unwrap();
                let rows = &l.rows[1];
                let marker = rows.iter().position(|r| r.marker.is_some()).unwrap();
                assert!(marker > 0 && marker + 1 < rows.len());
                (
                    v.markdown_pane_x(w, 1) + 16.,
                    v.top() + rows[marker - 1].y + 2.,
                    v.top() + rows[marker + 1].y + 2.,
                )
            })
            .unwrap();
        for (from, key_name, expected, shift) in [
            (before_y, "down", 12, false),
            (after_y, "up", 0, false),
            (before_y, "down", 12, true),
            (after_y, "up", 0, true),
        ] {
            click(cx, &f, x, from);
            let original = f
                .viewer
                .read_with(cx, |v, _| v.comparison.markdown_selection().unwrap());
            key(cx, &f, key_name, false, shift, false);
            let selected = f
                .viewer
                .read_with(cx, |v, _| v.comparison.markdown_selection().unwrap());
            assert_eq!(selected.head, expected, "{key_name}, shift={shift}");
            assert_eq!(
                selected.anchor,
                if shift { original.anchor } else { expected }
            );
        }
        assert_eq!(f.viewer.read_with(cx, |v, _| v.sel), None);
    }

    fn picker_confirmation(
        cx: &mut TestAppContext,
        question: &str,
    ) -> super::super::parity_tests::Fixture {
        let f = fixture_named(
            cx,
            "Before text\n\n<!-- old -->\n\nAfter text\n",
            "Before text\n\n<!-- new -->\n\nAfter text\n",
            "md",
        );
        let base = f.dir.path().join("base");
        let target = f.dir.path().join("result.md");
        std::fs::write(&base, b"binary\0base").unwrap();
        std::fs::write(&target, "original result\n").unwrap();
        let opened = version_picker::open_files(
            &base,
            &f.dir.path().join("left.md"),
            &f.dir.path().join("right.md"),
            &target,
            f.dir.path(),
        )
        .unwrap()
        .unwrap();
        f.viewer.update(cx, |v, cx| {
            v.start_picker(opened, false, cx);
            v.sel = Some(Selection::caret(Side::Left, Pos::new(0, 2)));
        });
        run(cx,&f,crate::catalog::Action::Comparison(Command::Preview(crate::markdown::MarkdownLayout::Inline)));
        markdown_ready(cx, &f);
        let (x, y) = f
            .window
            .update(cx, |v, w, _| {
                v.markdown_prepare(w);
                assert!(v.markdown.layout.is_some());
                (v.markdown_pane_x(w, 1) + 16., v.top() + 2.)
            })
            .unwrap();
        click(cx, &f, x, y);
        match question {
            "close" => {
                key(cx, &f, "2", false, false, false);
                key(cx, &f, "escape", false, false, false);
            }
            "overwrite" | "overwrite_done" => {
                key(cx, &f, "2", false, false, false);
                std::fs::write(&target, "external result\n").unwrap();
                key(
                    cx,
                    &f,
                    if question == "overwrite" {
                        "s"
                    } else {
                        "enter"
                    },
                    true,
                    false,
                    false,
                );
            }
            "done" => key(cx, &f, "enter", true, false, false),
            _ => unreachable!(),
        }
        use crate::merge_session::Ask;
        let expected = match question {
            "close" => Ask::Close,
            "overwrite" => Ask::Overwrite { done: false },
            "overwrite_done" => Ask::Overwrite { done: true },
            "done" => Ask::DoneWithConflicts,
            _ => unreachable!(),
        };
        assert_eq!(
            f.viewer
                .read_with(cx, |v, _| v.picker_v.as_ref().unwrap().ask),
            Some(expected)
        );
        f
    }

    #[gpui::test]
    fn gui_markdown_version_picker_confirmation_blocks_menu(cx: &mut TestAppContext) {
        for question in ["close", "overwrite", "overwrite_done", "done"] {
            let f = picker_confirmation(cx, question);
            let before = super::super::parity_tests::picker_preview_state(cx, &f);
            for name in [":", "1", "down"] {
                key(cx, &f, name, false, false, false);
                assert!(
                    f.viewer.read_with(cx, |v, _| v.menu.is_none() && v.command_palette.is_none()),
                    "{question}: {name}"
                );
                assert_eq!(
                    super::super::parity_tests::picker_preview_state(cx, &f),
                    before,
                    "{question}: {name}"
                );
            }
            // The question's advertised accept/dismiss actions remain usable.
            if question == "close" {
                key(cx, &f, "s", true, false, false);
                assert_eq!(
                    std::fs::read(f.dir.path().join("result.md")).unwrap(),
                    std::fs::read(f.dir.path().join("right.md")).unwrap()
                );
            } else {
                key(cx, &f, "escape", false, false, false);
            }
            assert!(
                f.viewer
                    .read_with(cx, |v, _| v.picker_v.as_ref().unwrap().ask.is_none())
            );
            run(cx,&f,crate::catalog::Action::Comparison(Command::SetPresentation(PresentationMode::Source)));
            assert_eq!(
                f.viewer.read_with(cx, |v, _| v.comparison.markdown_mode()),
                PresentationMode::Source
            );
            assert_eq!(
                f.viewer.read_with(cx, |v, _| v.sel),
                Some(Selection::caret(Side::Left, Pos::new(0, 2)))
            );
        }
    }

    #[gpui::test]
    fn gui_markdown_version_picker_confirmation_blocks_pointer(cx: &mut TestAppContext) {
        for question in ["close", "overwrite", "overwrite_done", "done"] {
            let f = picker_confirmation(cx, question);
            let before = super::super::parity_tests::picker_preview_state(cx, &f);
            let (x, text_y, marker_y) = f.window.update(cx, |v, w, _| {
                let l = v.markdown.layout.as_ref().unwrap();
                let marker = l.rows[1].iter().find(|r| r.marker.is_some()).unwrap();
                (
                    v.markdown_pane_x(w, 1) + 50.,
                    v.top() + l.rows[1].last().unwrap().y + 2.,
                    v.top() + marker.y + 2.,
                )
            }).unwrap();
            // A drag begun before the question must also stay frozen.
            f.window
                .update(cx, |v, w, cx| {
                    v.on_mouse_move(
                        &MouseMoveEvent {
                            position: point(px(x), px(text_y)),
                            pressed_button: Some(MouseButton::Left),
                            ..Default::default()
                        },
                        w,
                        cx,
                    )
                })
                .unwrap();
            assert_eq!(
                super::super::parity_tests::picker_preview_state(cx, &f),
                before,
                "{question}: drag"
            );
            for (label, y) in [("text", text_y), ("marker", marker_y)] {
                click(cx, &f, x, y);
                assert_eq!(
                    super::super::parity_tests::picker_preview_state(cx, &f),
                    before,
                    "{question}: {label}"
                );
            }
        }
    }

    #[gpui::test]
    fn gui_markdown_version_picker_menu_controls_real_preview(cx: &mut TestAppContext) {
        let f = fixture_named(cx, "[guide](old)\n", "[guide](new)\n", "md");
        let base = f.dir.path().join("base");
        let target = f.dir.path().join("result.md");
        std::fs::write(&base, b"binary\0base").unwrap();
        std::fs::write(&target, "# After\n").unwrap();
        let opened = version_picker::open_files(
            &base,
            &f.dir.path().join("left.md"),
            &f.dir.path().join("right.md"),
            &target,
            f.dir.path(),
        )
        .unwrap()
        .unwrap();
        f.viewer
            .update(cx, |v, cx| v.start_picker(opened, false, cx));
        run(cx,&f,crate::catalog::Action::Comparison(Command::Preview(crate::markdown::MarkdownLayout::Inline)));
        markdown_ready(cx, &f);
        run(cx,&f,crate::catalog::Action::Comparison(Command::Preview(crate::markdown::MarkdownLayout::SideBySide)));markdown_ready(cx,&f);
        f.window
            .update(cx, |v, w, _| {
                v.markdown_prepare(w);
                assert!(v.markdown.layout.is_some());
            })
            .unwrap();
        let source_bytes_before=[std::fs::read(f.dir.path().join("left.md")).unwrap(),std::fs::read(f.dir.path().join("right.md")).unwrap()];
        let source_hash=|bytes:&[Vec<u8>;2]|{use std::hash::{Hash,Hasher};let mut h=std::collections::hash_map::DefaultHasher::new();bytes.hash(&mut h);h.finish()};let source_hash_before=source_hash(&source_bytes_before);
        f.viewer.read_with(cx,|v,_|{for id in 0..v.comparison.view.change_count(){assert!(v.comparison.markdown_projection().unwrap().represents_hunk(id));}});
        key(cx,&f,"n",false,false,false);assert_eq!(f.viewer.read_with(cx,|v,_|v.comparison.view.current_change()),Some(0));
        let (x,y,a)=f.window.update(cx,|v,w,_|v.markdown_test_marker(w,0,Side::Left)).unwrap();click(cx,&f,x,y);assert_eq!(f.viewer.read_with(cx,|v,_|v.comparison.markdown_anchor(a.side)),Some(a));
        run(cx,&f,crate::catalog::Action::Comparison(Command::Preview(crate::markdown::MarkdownLayout::Inline)));markdown_ready(cx,&f);
        key(cx,&f,"2",false,false,false);key(cx,&f,"s",true,false,false);
        assert_eq!(std::fs::read(&target).unwrap(),source_bytes_before[1]);
        f.viewer.read_with(cx,|v,_|{let p=v.picker_v.as_ref().unwrap();assert_eq!(p.chosen,Some(1));assert!(!p.dirty);});
        key(cx,&f,"z",true,false,false);
        f.viewer.read_with(cx,|v,_|{let p=v.picker_v.as_ref().unwrap();assert_eq!(p.chosen,None);assert!(p.dirty);assert!(v.picker_v_title().unwrap().contains('●'));});
        assert_eq!(f.viewer.read_with(cx,|v,_|v.comparison.markdown_mode()),PresentationMode::Rendered);
        let source_bytes_after=[std::fs::read(f.dir.path().join("left.md")).unwrap(),std::fs::read(f.dir.path().join("right.md")).unwrap()];let source_hash_after=source_hash(&source_bytes_after);assert_eq!(source_hash_before,source_hash_after);assert_eq!(source_bytes_before,source_bytes_after);
        run(cx,&f,crate::catalog::Action::Comparison(Command::SetPresentation(PresentationMode::Source)));
        assert!(f.viewer.read_with(cx, |v, _| v.markdown.layout.is_none()));
        assert_eq!(std::fs::read_to_string(target).unwrap(), "[guide](new)\n");
    }
}

#[cfg(all(test, feature="gui-test"))]
impl Viewer {
    pub(super) fn markdown_test_marker(&mut self,window:&Window,hunk:usize,side:Side)->(f32,f32,SourceAnchor) {
        self.markdown_prepare(window);let l=self.markdown.layout.as_ref().unwrap();let i=side as usize;
        let row=l.rows[i].iter().find(|r|r.marker.is_some_and(|d|l.projection.decorations[d].hunk==hunk)).unwrap();
        (self.markdown_pane_x(window,i)+16.,self.top()+row.y-self.markdown.top[i]+2.,l.row_anchor(i,l.row_at(i,row.y)).unwrap())
    }
    pub(super) fn markdown_test_text(&mut self,window:&Window,side:Side,needle:&str)->(f32,f32) {
        self.markdown_prepare(window);let l=self.markdown.layout.as_ref().unwrap();let i=side as usize;let byte=l.projection.pair.sides[i].text.find(needle).unwrap();
        let row=l.rows[i].iter().find(|r|r.pieces.iter().any(|p|p.range.contains(&byte))).unwrap();let piece=row.pieces.iter().find(|p|p.range.contains(&byte)).unwrap();
        (self.markdown_pane_x(window,i)+piece.x+f32::from(piece.shaped.x_for_index(byte-piece.range.start))+1.,self.top()+row.y-self.markdown.top[i]+2.)
    }

}

#[cfg(all(test, feature = "gui-test"))]
mod performance {
    use super::*;
    use super::super::parity_tests::{fixture_named, key, native_text_system, run};
    use gpui::TestAppContext;

    #[test]
    fn gui_markdown_wrap_lookup_matches_native_scan() {
        for groups in [vec![vec![]], vec![vec![(0, 0.)]],
            vec![vec![(0, 0.), (1, 7.), (4, 7.), (8, 14.)], vec![], vec![(9, 22.)]],
            vec![vec![(4, 9.), (0, 0.), (8, 4.)], vec![(2, 15.), (12, 11.)]]]
        {
            let line = gpui::LineLayout {
                width: px(30.), len: 13,
                runs: groups.into_iter().map(|glyphs| gpui::ShapedRun {
                    font_id: gpui::FontId(0),
                    glyphs: glyphs.into_iter().map(|(index, x)| gpui::ShapedGlyph {
                        id: gpui::GlyphId(0), position: gpui::point(px(x), px(0.)), index, is_emoji: false,
                    }).collect(),
                }).collect(), ..Default::default()
            };
            let lookup = WrapLookup::new(&line);
            for index in 0..16 { assert_eq!(lookup.x_for_index(index), line.x_for_index(index)); }
            for x in -4..65 { assert_eq!(lookup.closest_index_for_x(px(x as f32 / 2.)), line.closest_index_for_x(px(x as f32 / 2.))); }
        }
        // A long ordinary run borrows native glyphs; it allocates only run metadata.
        let line = gpui::LineLayout { width:px(100000.),len:100000,runs:vec![gpui::ShapedRun {
            font_id:gpui::FontId(0),glyphs:(0..100000).map(|index|gpui::ShapedGlyph {
                id:gpui::GlyphId(0),position:gpui::point(px(index as f32),px(0.)),index,is_emoji:false,
            }).collect(),
        }],..Default::default() };
        let lookup = WrapLookup::new(&line);
        assert_eq!(lookup.runs.len(), 1);
        assert!(lookup.runs[0].index_prefix.is_empty() && lookup.runs[0].x_prefix.is_empty());
        assert_eq!(lookup.x_for_index(99999), px(99999.));
    }

    #[test]
    fn gui_markdown_wrap_lookup_native_unicode() {
        let mut cx = TestAppContext::build_with_text_system(gpui::TestDispatcher::new(0), None, native_text_system());
        let f = fixture_named(&mut cx, "# left", "# right", "md");
        f.window.update(&mut cx, |_, w, _| {
            for text in ["", "x", "office affine ffi", "漢字 é 👩‍💻 👨‍👩‍👧‍👦", "abc אבג مرحبا xyz"] {
                let line = w.text_system().shape_line(text.to_owned().into(), px(14.),
                    &[gpui::TextRun { len:text.len(),font:gpui::font("sans-serif"),..Default::default() }],None);
                let lookup = WrapLookup::new(&line);
                for index in 0..=text.len() { assert_eq!(lookup.x_for_index(index),line.x_for_index(index)); }
                for x in -4..=(f32::from(line.width) as i32 + 4)*2 {
                    assert_eq!(lookup.closest_index_for_x(px(x as f32 / 2.)),line.closest_index_for_x(px(x as f32 / 2.)));
                }
            }
        }).unwrap();
    }

    /// Release-only measurement entry point; never linked into a production app.
    #[test]
    #[ignore]
    fn gui_markdown_release_completion_perf() {
        let root = std::env::var("LENZ_MARKDOWN_FIXTURES").expect("owned fixture directory required");
        let size = std::env::var("LENZ_MARKDOWN_SIZE").unwrap_or_else(|_| "small".into());
        let left = std::fs::read_to_string(format!("{root}/{size}-left.md")).unwrap();
        let right = std::fs::read_to_string(format!("{root}/{size}-right.md")).unwrap();
        let options = pulldown_cmark::Options::ENABLE_TABLES | pulldown_cmark::Options::ENABLE_STRIKETHROUGH
            | pulldown_cmark::Options::ENABLE_TASKLISTS | pulldown_cmark::Options::ENABLE_MATH;
        let events = [&left, &right].map(|s| pulldown_cmark::Parser::new_ext(s, options).count());
        let limits = crate::markdown::Limits::DEFAULT;
        if size == "large" {
            assert!(left.len() < limits.bytes_per_side && right.len() < limits.bytes_per_side);
            assert!(events.iter().all(|count| *count > limits.events_per_side));
            assert!(matches!(crate::markdown::parse_pair([Arc::from(left.as_str()), Arc::from(right.as_str())], limits, &|| true), Err(crate::markdown::MarkdownError::Limit("parser events"))));
            println!("LENZ_MARKDOWN {}", serde_json::json!({"fixture":size,"parser_events":events,"fallback":"parser events"}));
            return;
        }
        let mut cx = TestAppContext::build_with_text_system(gpui::TestDispatcher::new(0), None, native_text_system());
        let f = fixture_named(&mut cx, &left, &right, "md");
        cx.simulate_window_resize(f.window.into(), gpui::size(px(1200.), px(800.)));
        let layout = std::env::var("LENZ_MARKDOWN_LAYOUT").unwrap_or_else(|_| "inline".into());
        if layout == "side-by-side" {
            f.viewer.update(&mut cx, |v, cx| v.comparison_command(Command::SetMarkdownLayout(crate::markdown::MarkdownLayout::SideBySide), cx));
        }
        let mut samples = Vec::new();
        for iteration in 0..6 {
            f.viewer.read_with(&cx, |v, _| {
                assert!(v.markdown.layout.is_none());
                if iteration == 0 { assert!(v.comparison.markdown_projection().is_none()); }
            });
            let start = Instant::now();
            run(&mut cx,&f,crate::catalog::Action::Comparison(Command::Preview(crate::markdown::MarkdownLayout::Inline)));
            loop {
                cx.executor().advance_clock(std::time::Duration::from_millis(2));
                cx.executor().run_until_parked();
                let ready = f.window.update(&mut cx, |v, w, cx| {
                    let has_frame = v.markdown_frame(w, cx).is_some();
                    has_frame && !v.comparison.markdown_loading()
                        && v.comparison.diffed == v.comparison.generation
                        && v.markdown.syntax.as_ref().is_some_and(|s| s.job.is_none())
                }).unwrap();
                if ready { break; }
                assert!(start.elapsed().as_secs() < 30, "rendered readiness timeout");
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            // the previous round's Copy leaves a caret, and End then means the end of its row
            key(&mut cx, &f, "escape", false, false, false);
            key(&mut cx, &f, "end", false, false, false);
            f.window.update(&mut cx, |v, w, cx| {
                assert!(v.markdown_frame(w, cx).is_some());
                let layout = v.markdown.layout.as_ref().unwrap();
                let i = Side::Right as usize;
                let doc = &layout.projection.pair.sides[i];
                let tail = doc.text.find("MARKDOWN_TAIL_END").expect("fixture tail");
                let row = layout.rows[i].iter().find(|row| row.pieces.iter().any(|p| p.side == Side::Right && p.range.contains(&tail))).expect("native shaped tail row");
                let y = row.y - v.markdown.top[i];
                assert!(y >= 0. && y < f32::from(w.viewport_size().height) - v.top(), "End must expose shaped tail");
                assert!(row.pieces.iter().any(|piece| piece.shaped.width > px(0.)));
                assert!((0..v.comparison.view.change_count()).all(|id| layout.projection.represents_hunk(id)));
            }).unwrap();
            let elapsed = start.elapsed().as_secs_f64() * 1000.;
            if let Ok(limit) = std::env::var("LENZ_MARKDOWN_MAX_MS") {
                assert!(elapsed < limit.parse::<f64>().unwrap(), "native layout completion {elapsed}ms exceeds explicit regression budget");
            }
            samples.push(elapsed);
            key(&mut cx, &f, "a", true, false, false);
            key(&mut cx, &f, "c", true, false, false);
            assert!(cx.update(|cx| cx.read_from_clipboard().unwrap().text().unwrap()).contains("MARKDOWN_TAIL_END"));
            run(&mut cx,&f,crate::catalog::Action::Comparison(Command::SetPresentation(PresentationMode::Source)));
        }
        println!("LENZ_MARKDOWN {}", serde_json::json!({"workload_version":1,"fixture":size,"layout":layout,"request_to_shaped_tail_ms":samples,"native_platform_text_system":true,"source_bytes":[left.len(),right.len()],"physical_lines":[left.lines().count(),right.lines().count()],"parser_events":events,"physical_presentation":false}));
        assert_eq!(std::fs::read_to_string(f.dir.path().join("right.md")).unwrap(), right);
    }
}
