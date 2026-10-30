//! The Markdown preview: one document's blocks in a reading column, styled and shaped by Lenz's shared reading
//! style, wrapped to the view and scrolled in pixels. Each row keeps its source line: scrolling reports the reading
//! location to the core, and switching to the source keeps the place. Layouts are shaped off the UI thread.

use std::{ops::Range, sync::Arc};
use unicode_segmentation::UnicodeSegmentation;

use gpui::{App, Bounds, Pixels, ShapedLine, TextAlign, Window, point, px};
use lenz_content::{
    markdown::{Block, BlockKind},
    theme::Palette,
};
use lenz_gpui::markdown::{ReadingStyle, indent, quotes, shape};
use code_core::markdown::Preview;

/// Space around the reading column.
pub const PAD: f32 = 24.0;

pub struct Piece {
    pub x: f32,
    pub range: Range<usize>,
    pub shaped: ShapedLine,
}

#[derive(Clone, Debug, PartialEq)]
pub enum RowKind {
    Text,
    /// A code block's or literal block's line, on its background.
    Code,
    Rule,
    /// A local image drawn at this size.
    Image { block: usize, w: f32, h: f32 },
    /// A table row's cell edges.
    Table { grid: Vec<f32>, header: bool },
}

pub struct Row {
    pub y: f32,
    pub h: f32,
    pub block: usize,
    /// The source line this row shows (approximate inside wrapped paragraphs).
    pub line: usize,
    pub kind: RowKind,
    pub pieces: Vec<Piece>,
}

pub struct Layout {
    pub preview: Arc<Preview>,
    pub width: f32,
    pub style: Arc<ReadingStyle>,
    pub palette: Arc<Palette>,
    /// The reading column: left edge and width.
    pub left: f32,
    pub column: f32,
    pub rows: Vec<Row>,
    pub height: f32,
}

impl Layout {
    /// Lay `preview` out for a view `width` wide; `line_of` maps a source byte to its line.
    pub fn new(preview: Arc<Preview>, width: f32, style: Arc<ReadingStyle>, palette: Arc<Palette>, line_of: &dyn Fn(usize) -> usize, ts: &gpui::WindowTextSystem) -> Self {
        Self::build(preview, width, style, palette, line_of, ts, false, &|| true)
    }

    /// Documentation wraps code as well as prose so narrow popups retain every character.
    pub fn popup(preview: Arc<Preview>, width: f32, style: Arc<ReadingStyle>, palette: Arc<Palette>, ts: &gpui::WindowTextSystem, current: &dyn Fn() -> bool) -> Self {
        Self::build(preview, width, style, palette, &|_| 0, ts, true, current)
    }

    #[allow(clippy::too_many_arguments)]
    fn build(preview: Arc<Preview>, width: f32, style: Arc<ReadingStyle>, palette: Arc<Palette>, line_of: &dyn Fn(usize) -> usize, ts: &gpui::WindowTextSystem, wrap_code: bool, current: &dyn Fn() -> bool) -> Self {
        let column = (width - 2.0 * PAD).max(if wrap_code {1.0} else {40.0});
        let left = ((width - column) / 2.0).max(PAD.min(width / 4.0));
        let mut l = Layout { preview: preview.clone(), width, style: style.clone(), palette: palette.clone(), left, column, rows: Vec::new(), height: PAD };
        let doc = &preview.doc;
        let s = &*style;
        let mut previous: Option<&BlockKind> = None;
        for (b, block) in doc.blocks.iter().enumerate() {
            if !current() { break; }
            l.height += s.space_before(&block.kind, previous);
            previous = Some(&block.kind);
            let x = indent(s, left, block);
            let right = left + column - if block.callout.is_some() { s.size } else { 0.0 };
            let first = line_of(block.source.start);
            let syntax = preview.syntax.get(&b).map_or(&[][..], Vec::as_slice);
            match &block.kind {
                BlockKind::Rule => l.push(b, first, s.size, RowKind::Rule, Vec::new()),
                BlockKind::Image { alt, .. } => match preview.images.as_ref().and_then(|i| i.get(&b)) {
                    Some(Ok(pixels)) => {
                        let (w, h) = (pixels.width() as f32, pixels.height() as f32);
                        let scale = ((right - x) / w).min(1.0);
                        l.push(b, first, h * scale + 4.0, RowKind::Image { block: b, w: w * scale, h: h * scale }, Vec::new());
                    }
                    found => {
                        let why = match found {
                            Some(Err(why)) => why,
                            _ if preview.images.is_some() => "not shown",
                            _ => "loading…",
                        };
                        let label = format!("[image: {alt}] — {why}");
                        let len = label.len();
                        let shaped = ts.shape_line(label.into(), px(s.size), &[run(s.body.clone(), s.muted, len)], None);
                        l.push(b, first, s.line, RowKind::Text, vec![Piece { x, range: block.text.clone(), shaped }]);
                    }
                },
                BlockKind::Code { .. } | BlockKind::Literal | BlockKind::Properties => {
                    // One row per line, never wrapped (the column clips them); a fenced block starts a line later.
                    let fence = usize::from(matches!(block.kind, BlockKind::Code { .. }) && doc.source[block.source.clone()].trim_start().starts_with(['`', '~']));
                    let mut at = block.text.start;
                    for (i, text) in doc.text[block.text.clone()].split('\n').enumerate() {
                        if !current() { break; }
                        let range = at..at + text.len();
                        at = range.end + 1;
                        if range.is_empty() && range.end >= block.text.end && i > 0 {
                            break; // the final line ending
                        }
                        if wrap_code {
                            for (range, shaped) in popup_flow(doc, block, range, s.size - 2.0, s, &palette, ts, syntax, (right-x-8.0).max(1.0), current) {
                                l.push(b, first + fence + i, s.line * 0.85, RowKind::Code, vec![Piece { x: x + 8.0, range, shaped }]);
                            }
                        } else {
                            let shaped = shape(doc, block, range.clone(), s.size - 2.0, s, &palette, ts, syntax, None);
                            l.push(b, first + fence + i, s.line * 0.85, RowKind::Code, vec![Piece { x: x + 8.0, range, shaped }]);
                        }
                    }
                }
                BlockKind::TableRow { cells, header, .. } if wrap_code => {
                    for cell in cells {
                        for (range, shaped) in popup_flow(doc, block, cell.clone(), s.size, s, &palette, ts, &[], right-x, current) {
                            l.push(b, first, s.line, RowKind::Text, vec![Piece{x,range,shaped}]);
                        }
                    }
                    if *header { l.push(b, first, 2.0, RowKind::Rule, Vec::new()); }
                }
                BlockKind::TableRow { cells, header, .. } => {
                    let n = cells.len().max(1) as f32;
                    let w = (right - x) / n;
                    let grid: Vec<f32> = (0..=cells.len()).map(|i| x + i as f32 * w).collect();
                    let pieces = cells
                        .iter()
                        .enumerate()
                        .map(|(i, c)| Piece { x: grid[i] + 6.0, range: c.clone(), shaped: shape(doc, block, c.clone(), s.size, s, &palette, ts, &[], None) })
                        .collect();
                    l.push(b, first, s.line, RowKind::Table { grid, header: *header }, pieces);
                }
                kind => {
                    let size = match kind {
                        BlockKind::Heading(n) => s.heading[usize::from(n.clamp(&1, &6) - 1)],
                        _ => s.size,
                    };
                    let height = (size * 1.6).max(s.line);
                    let rows = if wrap_code {popup_flow(doc, block, block.text.clone(), size, s, &palette, ts, &[], right-x, current)}
                        else {flow(doc, block, block.text.clone(), size, s, &palette, ts, &[], right-x)};
                    for (i, (range, shaped)) in rows.into_iter().enumerate() {
                        // Lines of a wrapped paragraph: the source line its text starts on.
                        let line = if i == 0 { first } else { source_byte(doc, range.start).map_or(first, line_of) };
                        l.push(b, line, height, RowKind::Text, vec![Piece { x, range, shaped }]);
                    }
                }
            }
        }
        l.height += PAD;
        l
    }

    fn push(&mut self, block: usize, line: usize, h: f32, kind: RowKind, pieces: Vec<Piece>) {
        self.rows.push(Row { y: self.height, h, block, line, kind, pieces });
        self.height += h;
    }

    /// The row at `y` (clamped to the rows).
    pub fn row_at(&self, y: f32) -> Option<&Row> {
        let i = self.rows.partition_point(|r| r.y + r.h <= y);
        self.rows.get(i.min(self.rows.len().saturating_sub(1)))
    }

    /// Where source line `line` is: the top of the first row showing it or a later line.
    pub fn y_of_line(&self, line: usize) -> f32 {
        let i = self.rows.partition_point(|r| r.line < line);
        match (self.rows.get(i), i.checked_sub(1).and_then(|p| self.rows.get(p))) {
            // Inside a block whose rows start before it: that block's row.
            (Some(r), Some(p)) if r.line > line && p.line <= line => p.y,
            (Some(r), _) => r.y,
            (None, _) => self.rows.last().map_or(0.0, |r| r.y),
        }
    }

    /// The rows meeting `top..top + h`.
    pub fn visible(&self, top: f32, h: f32) -> &[Row] {
        let start = self.rows.partition_point(|r| r.y + r.h <= top);
        let end = self.rows.partition_point(|r| r.y < top + h);
        &self.rows[start..end.max(start)]
    }

    /// The rendered-text byte nearest a point (`x` from the area's left, `y` in layout coordinates: from the top of
    /// the layout), on the row there; `None` where no row holds text.
    pub fn byte_at(&self, x: f32, y: f32) -> Option<usize> {
        let row = self.row_at(y)?;
        let piece = row.pieces.iter().rev().find(|p| p.x <= x).or(row.pieces.first())?;
        let i = piece.shaped.closest_index_for_x(px(x - piece.x)).min(piece.range.len());
        Some(piece.range.start + i)
    }

    /// The rendered text of `r`: each block's part, blocks joined by line breaks (list markers are part of the text).
    pub fn text_of(&self, r: Range<usize>) -> String {
        let doc = &self.preview.doc;
        let parts: Vec<&str> = doc.blocks.iter().filter(|b| b.text.start < r.end && b.text.end > r.start).filter_map(|b| doc.text.get(b.text.start.max(r.start)..b.text.end.min(r.end))).collect();
        parts.join("\n")
    }

    /// Paint the rows meeting the view, scrolled by `top`, inside `area`, with `selection` (rendered-text bytes) over
    /// their text. `image` paints block `b`'s texture.
    pub fn paint(&self, window: &mut Window, cx: &mut App, area: Bounds<Pixels>, top: f32, selection: Option<Range<usize>>, mut image: impl FnMut(&mut Window, usize, Bounds<Pixels>)) {
        let (x0, y0, h) = (f32::from(area.origin.x), f32::from(area.origin.y), f32::from(area.size.height));
        let (s, p) = (&*self.style, &*self.palette);
        let doc = &self.preview.doc;
        for row in self.visible(top, h) {
            let y = y0 + row.y - top;
            let block = &doc.blocks[row.block];
            decorate(window, s, p, block, x0 + self.left, self.column, y, row.h);
            match &row.kind {
                RowKind::Code => {
                    let x = x0 + indent(s, self.left, block);
                    lenz_gpui::fill(window, x, y, self.left + self.column - (x - x0), row.h, s.code_bg);
                }
                RowKind::Rule => lenz_gpui::fill(window, x0 + self.left, y + row.h / 2.0, self.column, 1.0, s.rule),
                RowKind::Table { grid, header } => {
                    if *header {
                        lenz_gpui::fill(window, x0 + grid[0], y, grid[grid.len() - 1] - grid[0], row.h, s.code_bg);
                    }
                    for gx in grid {
                        lenz_gpui::fill(window, x0 + gx, y, 1.0, row.h, s.rule);
                    }
                    lenz_gpui::fill(window, x0 + grid[0], y + row.h - 1.0, grid[grid.len() - 1] - grid[0], 1.0, s.rule);
                }
                RowKind::Image { block, w, h } => {
                    let x = x0 + indent(s, self.left, &doc.blocks[*block]);
                    let at = Bounds { origin: point(px(x), px(y + 2.0)), size: gpui::size(px(*w), px(*h)) };
                    lenz_gpui::image::checker(window, area, (x, y + 2.0, *w, *h));
                    image(window, *block, at);
                }
                RowKind::Text => {}
            }
            for (i, piece) in row.pieces.iter().enumerate() {
                let cell = match &row.kind {
                    RowKind::Table { grid, .. } => Some((grid[i], grid[i + 1])),
                    _ => None,
                };
                let at = point(px(x0 + piece.x), px(y));
                if let Some((a, b)) = cell {
                    let mask = Bounds { origin: point(px(x0 + a), px(y)), size: gpui::size(px(b - a), px(row.h)) };
                    window.with_content_mask(Some(gpui::ContentMask { bounds: mask }), |window| {
                        let _ = piece.shaped.paint_background(at, px(row.h), TextAlign::Left, None, window, cx);
                    });
                } else {
                    let _ = piece.shaped.paint_background(at, px(row.h), TextAlign::Left, None, window, cx);
                }
                if let Some(sel) = selection.as_ref().filter(|s| s.start < piece.range.end && s.end > piece.range.start) {
                    let (a, b) = (sel.start.max(piece.range.start) - piece.range.start, sel.end.min(piece.range.end) - piece.range.start);
                    let (xa, xb) = (f32::from(piece.shaped.x_for_index(a)), f32::from(piece.shaped.x_for_index(b)));
                    lenz_gpui::fill(window, x0 + piece.x + xa, y, (xb - xa).max(2.0), row.h, p.sel_bg);
                }
                match cell {
                    // A cell's text stays in its cell.
                    Some((a, b)) => {
                        let mask = Bounds { origin: point(px(x0 + a), px(y)), size: gpui::size(px(b - a), px(row.h)) };
                        window.with_content_mask(Some(gpui::ContentMask { bounds: mask }), |window| {
                            let _ = piece.shaped.paint(at, px(row.h), TextAlign::Left, None, window, cx);
                        });
                    }
                    None => {
                        let _ = piece.shaped.paint(at, px(row.h), TextAlign::Left, None, window, cx);
                    }
                }
            }
        }
    }
}

/// Quote bars and a callout's tint and edge beside a block's row.
#[allow(clippy::too_many_arguments)]
fn decorate(window: &mut Window, s: &ReadingStyle, p: &Palette, block: &Block, left: f32, column: f32, y: f32, h: f32) {
    let pad = if let Some(c) = block.callout {
        let (bg, color, _) = s.callout(p, c.kind);
        lenz_gpui::fill(window, left, y, column, h, bg);
        lenz_gpui::fill(window, left, y, 3.0, h, color);
        s.size
    } else {
        0.0
    };
    for q in 0..quotes(block) {
        lenz_gpui::fill(window, left + pad + q as f32 * s.size + 2.0, y, 3.0, h, s.quote_bar);
    }
}

/// A block's text wrapped at word boundaries into rows `width` wide: each row's range and shaped text.
#[allow(clippy::too_many_arguments)]
fn flow(doc: &lenz_content::markdown::MarkdownDocument, block: &Block, range: Range<usize>, size: f32, s: &ReadingStyle, p: &Palette, ts: &gpui::WindowTextSystem, syntax: &[(Range<usize>, lenz_content::highlight::Capture)], width: f32) -> Vec<(Range<usize>, ShapedLine)> {
    let mut rows = Vec::new();
    let mut at = range.start;
    // ponytail: reshapes the rest of a line per wrapped row (quadratic in a line's rows); index glyphs as Lenz's
    // WrapLookup does if very long paragraphs show up in profiles.
    while at <= range.end {
        let end = doc.text[at..range.end].find('\n').map_or(range.end, |n| at + n);
        let mut a = at;
        loop {
            let shaped = shape(doc, block, a..end, size, s, p, ts, syntax, None);
            if f32::from(shaped.width) <= width || a >= end {
                rows.push((a..end, shaped));
                break;
            }
            let text = &doc.text[a..end];
            let mut stop = shaped.closest_index_for_x(px(width.max(0.0))).min(text.len());
            while stop > 0 && (!text.is_char_boundary(stop) || f32::from(shaped.x_for_index(stop)) > width) {
                stop -= 1;
            }
            if let Some(space) = text[..stop].rfind(' ').filter(|&sp| sp > 0) {
                stop = space + 1;
            }
            if stop == 0 {
                stop = text.chars().next().map_or(text.len(), char::len_utf8);
            }
            rows.push((a..a + stop, shape(doc, block, a..a + stop, size, s, p, ts, syntax, None)));
            a += stop;
        }
        at = end + 1;
    }
    rows
}

/// Shape each source line once to choose wrap boundaries, then only its visible-sized pieces.
#[allow(clippy::too_many_arguments)]
fn popup_flow(doc: &lenz_content::markdown::MarkdownDocument, block: &Block, range: Range<usize>, size: f32, s: &ReadingStyle, p: &Palette, ts: &gpui::WindowTextSystem, syntax: &[(Range<usize>, lenz_content::highlight::Capture)], width: f32, current: &dyn Fn() -> bool) -> Vec<(Range<usize>, ShapedLine)> {
    let mut rows=Vec::new(); let mut at=range.start;
    while at<=range.end && current() {
        // Bound shaping to 4 KiB except one oversized grapheme, which must remain whole.
        let mut boundaries=vec![0]; let mut newline=0;
        let mut graphemes=doc.text.get(at..range.end).unwrap_or_default().grapheme_indices(true);
        while current() {
            let Some((offset,grapheme))=graphemes.next() else {break};
            if !current() {return rows;}
            if grapheme.contains('\n') {newline=grapheme.len();break;}
            let end=offset+grapheme.len();
            if end>4096&&offset>0 {break;}
            boundaries.push(end);
            if end>=4096 {break;}
        }
        if !current() {return rows;}
        let end=at+boundaries.last().copied().unwrap_or(0);
        let continued=newline==0&&end<range.end;
        let mut consumed=end;
        let text=doc.text.get(at..end).unwrap_or_default();
        let whole=shape(doc,block,at..end,size,s,p,ts,syntax,None);
        if f32::from(whole.width)<=width || text.is_empty() {rows.push((at..end,whole));}
        else {
            // GPUI's coordinate helpers scan glyphs; prefix maxima make repeated wraps logarithmic.
            let (mut index,mut x)=(0, f32::NEG_INFINITY);
            let glyphs:Vec<_>=whole.runs.iter().flat_map(|run|&run.glyphs).map(|glyph|{
                let gx=f32::from(glyph.position.x); index=index.max(glyph.index); x=x.max(gx); (index,x,glyph.index,gx)
            }).collect();
            let x_at=|byte|glyphs.get(glyphs.partition_point(|g|g.0<byte)).map_or(f32::from(whole.width),|g|g.3);
            let byte_at=|x:f32|{
                let i=glyphs.partition_point(|g|g.1<x);
                let Some(g)=glyphs.get(i) else{return text.len()};
                let (previous,px)=i.checked_sub(1).and_then(|i|glyphs.get(i)).map_or((0,0.0),|g|(g.2,g.3));
                if g.3-x<x-px {g.2}else{previous}
            };
            let mut start=0;
            while start<text.len() && current() {
                let limit=x_at(start)+width.max(0.0);
                let target=byte_at(limit).min(text.len()).max(start);
                let mut boundary=boundaries.partition_point(|b|*b<=target).saturating_sub(1);
                let mut stop=boundaries.get(boundary).copied().unwrap_or(start);
                while stop>start&&x_at(stop)>limit {boundary=boundary.saturating_sub(1);stop=boundaries.get(boundary).copied().unwrap_or(start);}
                if let Some(space)=text.get(start..stop).unwrap_or_default().rfind(' ').filter(|&n|n>0) {
                    stop=boundaries.get(boundaries.partition_point(|b|*b<=start+space+1).saturating_sub(1)).copied().unwrap_or(start);
                }
                if stop<=start {stop=boundaries.get(boundaries.partition_point(|b|*b<=start)).copied().unwrap_or(text.len());}
                // Shape the unfinished last wrap with its continuation in the next bounded chunk.
                if continued && stop==text.len() && start>0 {consumed=at+start;break;}
                let mut part=at+start..at+stop;
                let mut shaped=shape(doc,block,part.clone(),size,s,p,ts,syntax,None);
                if f32::from(shaped.width)>width {
                    // Standalone shaping can change kerning or bidi advances at a wrap boundary.
                    let mut low=boundaries.partition_point(|b|*b<=start);
                    let mut high=boundaries.partition_point(|b|*b<=stop).saturating_sub(1);
                    while low<high&&current() {
                        let mid=low+(high-low).div_ceil(2);
                        let end=boundaries.get(mid).copied().unwrap_or(stop);
                        let trial=shape(doc,block,at+start..at+end,size,s,p,ts,syntax,None);
                        if f32::from(trial.width)<=width {low=mid;} else {high=mid-1;}
                    }
                    stop=boundaries.get(low).copied().unwrap_or(stop);
                    part=at+start..at+stop;
                    shaped=shape(doc,block,part.clone(),size,s,p,ts,syntax,None);
                }
                rows.push((part,shaped));
                start=stop;
            }
        }
        if newline>0 {at=end+newline;} else if consumed<range.end {at=consumed;} else {break;}
    }
    rows
}

/// The source byte rendered text byte `at` comes from, when it maps directly.
fn source_byte(doc: &lenz_content::markdown::MarkdownDocument, at: usize) -> Option<usize> {
    let seg = doc.segments.get(doc.segments.partition_point(|s| s.text.end <= at))?;
    match &seg.origin {
        lenz_content::markdown::Origin::Direct(src) | lenz_content::markdown::Origin::Normalized(src) if seg.text.start <= at => Some(src.start + (at - seg.text.start).min(src.len())),
        _ => None,
    }
}

fn run(font: gpui::Font, color: u32, len: usize) -> gpui::TextRun {
    gpui::TextRun { len, font, color: gpui::rgb(color).into(), background_color: None, underline: None, strikethrough: None }
}
