//! Native, lazy Markdown geometry. Rows borrow the shared rendered text; only
//! viewport cells are painted. Shared source anchors remain the navigation API.
use crate::{
    app::App,
    chrome::{color, style_with},
};
use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind};
use lenz::{
    markdown::{
        BlockKind, CalloutKind, MarkdownCommand, ParsedPair, PresentationMode, Projection, RenderSelection,
        SourceAnchor, TableAlignment,
    },
    view::{Kind, ScrollMode, Side},
};
use ratatui::{buffer::Buffer, layout::Rect, style::Modifier};
use std::{
    ops::Range,
    sync::{Arc, Weak, mpsc::Receiver},
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

struct Piece {
    /// The document `text` is in (Inline mixes both).
    side: Side,
    /// Inline: an inserted or removed span.
    change: Option<Kind>,
    text: Range<usize>,
    column: usize,
}
enum Pieces {
    Empty,
    One(Piece),
    Table(Vec<Piece>),
    /// Inline spans of one or both documents on a row.
    Flow(Vec<Piece>),
}
impl Row {
    fn block(&self) -> Option<usize> {
        self.block.map(|b| b as usize)
    }
    fn marker(&self) -> Option<usize> {
        self.marker.map(|m| m as usize)
    }
}
impl Pieces {
    fn as_mut_slice(&mut self) -> &mut [Piece] {
        match self {
            Self::Empty => &mut [],
            Self::One(p) => std::slice::from_mut(p),
            Self::Table(p) | Self::Flow(p) => p,
        }
    }
}
impl std::ops::Deref for Pieces {
    type Target = [Piece];
    fn deref(&self) -> &[Piece] {
        match self {
            Self::Empty => &[],
            Self::One(p) => std::slice::from_ref(p),
            Self::Table(p) | Self::Flow(p) => p,
        }
    }
}
struct Row {
    /// The document `block` is in.
    side: Side,
    /// The change mark: added, removed or modified (Inline).
    change: Option<Kind>,
    /// `u32` indices keep rows small: a document has many.
    block: Option<u32>,
    pieces: Pieces,
    width: usize,
    marker: Option<u32>,
    /// Text drawn before the pieces that is not in the document: a callout's kind, "Properties", a footnote number.
    /// Boxed: rare, and rows are many.
    label: Option<Box<(Box<str>, LabelKind)>>,
}
#[derive(Clone, Copy, PartialEq)]
enum LabelKind {
    Callout(CalloutKind),
    /// A titled foldable callout's ▸ / ▾, after its title.
    Fold,
    /// Row `row` of an image drawn `cols` × `rows` cells, `x` columns in.
    Image { row: u16, rows: u16, cols: u16, x: u16 },
    Properties,
    Footnote,
}
struct Group {
    rows: [Range<usize>; 2],
}
pub(crate) struct Layout {
    projection: Arc<Projection>,
    /// One column with changes in place, in pane 1 (Right).
    inline: bool,
    widths: [u16; 2],
    rows: [Vec<Row>; 2],
    groups: Vec<Group>,
    mode: ScrollMode,
    widest: [usize; 2],
    block_rows: [Vec<Range<usize>>; 2],
    text_rows: [Vec<usize>; 2],
    block_decorations: [Vec<Vec<usize>>; 2],
    /// Callouts the user toggled, per document, and the callouts whose bodies are hidden now.
    toggled: std::collections::HashSet<(usize, u32)>,
    shut: [std::collections::HashSet<u32>; 2],
    /// Pane rows that toggle a foldable callout: (document, callout id).
    folds: [std::collections::HashMap<usize, (usize, u32)>; 2],
    /// Decoded images by document and image block, and the image revision they are from.
    pictures: [std::collections::HashMap<usize, crate::md_images::Picture>; 2],
    image_revision: u64,
}
type Syntax = [Vec<(usize, lenz::highlight::Parsed)>; 2];
struct CodeSyntax {
    pair: Weak<ParsedPair>,
    _token: Arc<()>,
    job: Option<Receiver<Option<Syntax>>>,
    parsed: Syntax,
}
#[derive(Default)]
pub(crate) struct Adapter {
    layout: Option<Box<Layout>>,
    syntax: Option<CodeSyntax>,
    top: [usize; 2],
    anchors: [Option<SourceAnchor>; 2],
    panes: [Rect; 2],
    hidden: Option<(Rect, SourceAnchor)>,
    click: Option<(std::time::Instant, Side, usize, u8)>,
    // A soft wrap has two visual insertion positions at the same shared byte.
    caret_affinity: Option<(RenderSelection, usize)>,
    /// Callouts whose fold the user toggled: (document, callout id).
    toggled: std::collections::HashSet<(usize, u32)>,
    pub(crate) images: crate::md_images::Images,
}
fn cell_width(s: &str) -> usize {
    if s == "\t" {
        4
    } else if s.chars().any(lenz::document::hides_text) {
        1
    } else {
        s.width().max(1)
    }
}
fn width(s: &str) -> usize {
    s.graphemes(true).map(cell_width).sum()
}
fn side(i: usize) -> Side {
    if i == 0 { Side::Left } else { Side::Right }
}
impl Layout {
    fn new(projection: Arc<Projection>, widths: [u16; 2], mode: ScrollMode, inline: bool, toggled: &std::collections::HashSet<(usize, u32)>, pictures: ([std::collections::HashMap<usize, crate::md_images::Picture>; 2], u64)) -> Self {
        let mut result = Self {
            pictures: pictures.0,
            image_revision: pictures.1,
            toggled: toggled.clone(),
            shut: Default::default(),
            folds: Default::default(),
            projection,
            inline,
            widths,
            mode,
            rows: Default::default(),
            groups: Vec::new(),
            widest: [0; 2],
            block_rows: Default::default(),
            text_rows: Default::default(),
            block_decorations: Default::default(),
        };
        // Table widths are shared by contiguous rows, never wrapped with prose.
        let mut tables: [Vec<Option<Vec<usize>>>; 2] = Default::default();
        for (i, table) in tables.iter_mut().enumerate() {
            let doc = &result.projection.pair.sides[i];
            table.resize_with(doc.blocks.len(), || None);
            let mut at = 0;
            while at < doc.blocks.len() {
                if !matches!(doc.blocks[at].kind, BlockKind::TableRow { .. }) {
                    at += 1;
                    continue;
                }
                let start = at;
                let mut columns = Vec::new();
                while let Some(block) = doc.blocks.get(at)
                    && let BlockKind::TableRow { cells, .. } = &block.kind
                {
                    columns.resize(columns.len().max(cells.len()), 0usize);
                    for (j, cell) in cells.iter().enumerate() {
                        columns[j] = columns[j].max(width(&doc.text[cell.clone()]));
                    }
                    at += 1;
                }
                for item in &mut table[start..at] {
                    *item = Some(columns.clone());
                }
            }
        }
        let mut markers: [std::collections::HashMap<usize, usize>; 2] = Default::default();
        for i in 0..2 {
            let doc = &result.projection.pair.sides[i];
            result.block_rows[i].resize(doc.blocks.len(), 0..0);
            result.block_decorations[i].resize_with(doc.blocks.len(), Vec::new);
        }
        for (index, d) in result.projection.decorations.iter().enumerate() {
            let i = d.side as usize;
            if d.markup {
                markers[i].insert(d.hunk, index);
                continue;
            }
            let doc = &result.projection.pair.sides[i];
            let at = doc.blocks.partition_point(|b| b.text.end <= d.text.start);
            for b in at..doc.blocks.len() {
                if doc.blocks[b].text.start >= d.text.end {
                    break;
                }
                result.block_decorations[i][b].push(index);
            }
        }
        // a callout with a change inside starts open; the shared rule decides each foldable callout
        for i in 0..2 {
            let doc = &result.projection.pair.sides[i];
            let mut changed = std::collections::HashSet::new();
            for (b, block) in doc.blocks.iter().enumerate() {
                if let Some(c) = block.callout && !result.block_decorations[i][b].is_empty() { changed.insert(c.id); }
            }
            for ib in result.projection.inline.iter().filter(|ib| ib.side as usize == i && ib.change.is_some()) {
                if let Some(c) = doc.blocks[ib.block].callout { changed.insert(c.id); }
            }
            for c in doc.blocks.iter().filter_map(|b| b.callout) {
                if c.shut(changed.contains(&c.id), toggled.contains(&(i, c.id))) { result.shut[i].insert(c.id); }
            }
        }
        // callouts whose kind label was laid out, per side
        let mut callouts: [std::collections::HashSet<u32>; 2] = Default::default();
        let projection = result.projection.clone();
        if inline {
            // one column, two cells left for the change mark
            let inner = [widths[1].saturating_sub(2).max(1); 2];
            let mut at = 0;
            for (g, group) in projection.groups.iter().enumerate() {
                let start = result.rows[1].len();
                while let Some(ib) = projection.inline.get(at).filter(|ib| ib.group == g) {
                    at += 1;
                    let from = result.rows[1].len();
                    let (first, shut) = result.fold_state(ib.side as usize, ib.block);
                    if shut && !first {
                        result.block_rows[ib.side as usize][ib.block] = from..from;
                        continue;
                    }
                    if ib.spans.len() > 1 || ib.change == Some(Kind::Deleted) {
                        // an untitled callout's kind label comes before its first block, flowed or not
                        let block = &projection.pair.sides[ib.side as usize].blocks[ib.block];
                        if let Some(c) = block.callout
                            && callouts[ib.side as usize].insert(c.id)
                            && !block.runs.first().is_some_and(|r| r.style.callout_title)
                        {
                            result.rows[1].push(Row {
                                side: ib.side,
                                change: None,
                                block: Some(ib.block as u32),
                                pieces: Pieces::Empty,
                                width: usize::from(block.quote_depth) * 2 + 2 + c.kind.label().len(),
                                marker: None,
                                label: Some(Box::new((c.kind.label().into(), LabelKind::Callout(c.kind)))),
                            });
                        }
                        result.flow(ib, inner[1]);
                        if first { result.close_callout(1, ib.side as usize, ib.block, from, shut); }
                    } else {
                        result.lay_block(1, ib.side as usize, ib.block, start, &tables, &mut callouts, inner);
                    }
                    for row in &mut result.rows[1][from..] {
                        if row.block().is_some() {
                            row.change = ib.change;
                        }
                    }
                }
                for &hunk in &group.hunks {
                    // each markup marker once: the new side's, else the old side's
                    if let Some(&index) = markers[1].get(&hunk).or_else(|| markers[0].get(&hunk)) {
                        result.rows[1].push(Row {
                            side: projection.decorations[index].side,
                            change: None,
                            block: None,
                            pieces: Pieces::Empty,
                            width: 29,
                            marker: Some(index as u32),
                            label: None,
                        });
                    }
                }
                result.groups.push(Group { rows: [0..0, start..result.rows[1].len()] });
            }
            for row in &mut result.rows[1] {
                for piece in row.pieces.as_mut_slice() {
                    piece.column += 2;
                }
                row.width += 2;
            }
            result.widest[1] = result.rows[1].iter().map(|r| r.width).max().unwrap_or(0);
        } else {
            // two cells left of each pane for the change mark, as in Inline
            let inner = widths.map(|w| w.saturating_sub(2).max(1));
            for group in &projection.groups {
                let starts = [result.rows[0].len(), result.rows[1].len()];
                for i in 0..2 {
                    for block_index in group.blocks[i].clone() {
                        let from = result.rows[i].len();
                        result.lay_block(i, i, block_index, starts[i], &tables, &mut callouts, inner);
                        // a block added or removed whole is marked by its bar alone, in that color
                        let change = projection.whole_change(side(i), block_index).or_else(|| {
                            result.block_decorations[i][block_index].first().map(|&d| projection.decorations[d].kind)
                        });
                        for row in &mut result.rows[i][from..] {
                            row.change = change;
                        }
                    }
                    for &hunk in &group.hunks {
                        if let Some(&index) = markers[i].get(&hunk) {
                            result.rows[i].push(Row {
                                side: side(i),
                                change: None,
                                block: None,
                                pieces: Pieces::Empty,
                                width: 29,
                                marker: Some(index as u32),
                                label: None,
                            });
                        }
                    }
                    result.widest[i] = result.widest[i].max(
                        result.rows[i][starts[i]..]
                            .iter()
                            .map(|r| r.width)
                            .max()
                            .unwrap_or(0),
                    );
                }
                if mode == ScrollMode::Padded {
                    let height =
                        (result.rows[0].len() - starts[0]).max(result.rows[1].len() - starts[1]);
                    for (i, start) in starts.iter().enumerate() {
                        while result.rows[i].len() - start < height {
                            result.rows[i].push(Row {
                                side: side(i),
                                change: None,
                                block: None,
                                pieces: Pieces::Empty,
                                width: 0,
                                marker: None,
                                label: None,
                            });
                        }
                    }
                }
                result.groups.push(Group {
                    rows: [
                        starts[0]..result.rows[0].len(),
                        starts[1]..result.rows[1].len(),
                    ],
                });
            }
            for i in 0..2 {
                for row in &mut result.rows[i] {
                    for piece in row.pieces.as_mut_slice() {
                        piece.column += 2;
                    }
                    row.width += 2;
                }
                result.widest[i] = result.rows[i].iter().map(|r| r.width).max().unwrap_or(0);
            }
        }
        // per document: the rows of its pane that show some of its text
        for i in 0..2 {
            let pane = result.pane_of(side(i));
            result.text_rows[i] = result.rows[pane]
                .iter()
                .enumerate()
                .filter_map(|(r, row)| row.pieces.iter().any(|p| p.side == side(i)).then_some(r))
                .collect();
        }
        result
    }
    /// Whether `block` starts a foldable callout of document `s`, and whether that callout's body is hidden.
    fn fold_state(&self, s: usize, block: usize) -> (bool, bool) {
        let doc = &self.projection.pair.sides[s];
        let Some(c) = doc.blocks[block].callout.filter(|c| c.folded.is_some()) else { return (false, false) };
        let first = block.checked_sub(1).is_none_or(|p| doc.blocks[p].callout.map(|p| p.id) != Some(c.id));
        (first, self.shut[s].contains(&c.id))
    }
    /// A foldable callout's first row (its kind label, or its title line) toggles it and shows ▸ or ▾; a shut
    /// callout keeps only that row.
    fn close_callout(&mut self, i: usize, s: usize, block: usize, start: usize, shut: bool) {
        let Some(c) = self.projection.pair.sides[s].blocks[block].callout else { return };
        if self.rows[i].len() <= start { return; }
        if shut {
            self.rows[i].truncate(start + 1);
            let end = self.block_rows[s][block].end.min(start + 1);
            self.block_rows[s][block].end = end;
        }
        self.folds[i].insert(start, (s, c.id));
        let mark = if shut { "▸" } else { "▾" };
        let row = &mut self.rows[i][start];
        match &mut row.label {
            Some(label) if matches!(label.1, LabelKind::Callout(_)) => label.0 = format!("{} {mark}", label.0).into(),
            _ => row.label = Some(Box::new((mark.into(), LabelKind::Fold))),
        }
    }
    /// The pane showing document `side`: Inline shows both in pane 1.
    fn pane_of(&self, side: Side) -> usize {
        if self.inline { 1 } else { side as usize }
    }
    /// Lays out block `block_index` of document `s` into pane `i`, then a spacing row.
    #[allow(clippy::too_many_arguments)]
    fn lay_block(
        &mut self,
        i: usize,
        s: usize,
        block_index: usize,
        group_start: usize,
        tables: &[Vec<Option<Vec<usize>>>; 2],
        callouts: &mut [std::collections::HashSet<u32>; 2],
        widths: [u16; 2],
    ) {
        let projection = self.projection.clone();
        let doc = &projection.pair.sides[s];
        let from = self.rows[i].len();
        let (first, shut) = self.fold_state(s, block_index);
        if shut && !first {
            self.block_rows[s][block_index] = from..from;
            return;
        }
                let block = &doc.blocks[block_index];
                let block_start = self.rows[i].len();
                // an untitled callout shows its kind on a line of its own; a titled one shows its title text
                if let Some(c) = block.callout
                    && callouts[s].insert(c.id)
                    && !block.runs.first().is_some_and(|r| r.style.callout_title)
                {
                    let prefix = usize::from(block.quote_depth) * 2;
                    self.rows[i].push(Row {
                        side: Side::Left,
                        change: None,
                        block: Some(block_index as u32),
                        pieces: Pieces::Empty,
                        width: prefix + 2 + c.kind.label().len(),
                        marker: None,
                        label: Some(Box::new((c.kind.label().into(), LabelKind::Callout(c.kind)))),
                    });
                }
                let rows = matches!(block.kind, BlockKind::Properties)
                    .then(|| lenz::markdown::properties(&doc.text[block.text.clone()]))
                    .flatten();
                if let Some(picture) = self.pictures[s].get(&block_index).cloned() {
                    let prefix = usize::from(block.quote_depth) * 2 + usize::from(block.list_depth.saturating_sub(1)) * 2;
                    let prefix = prefix.min(usize::from(widths[i].saturating_sub(1)));
                    let (cols, n) = crate::md_images::cell_box([picture.width(), picture.height()], widths[i].saturating_sub(prefix as u16));
                    for row in 0..n {
                        self.rows[i].push(Row {
                            side: Side::Left,
                            change: None,
                            block: Some(block_index as u32),
                            pieces: Pieces::Empty,
                            width: prefix + usize::from(cols),
                            marker: None,
                            label: Some(Box::new(("".into(), LabelKind::Image { row, rows: n, cols, x: prefix as u16 }))),
                        });
                    }
                } else if rows.is_some() {
                    self.rows[i].push(Row {
                        side: Side::Left,
                        change: None,
                        block: Some(block_index as u32),
                        pieces: Pieces::Empty,
                        width: 10,
                        marker: None,
                        label: Some(Box::new(("Properties".into(), LabelKind::Properties))),
                    });
                    // key at 0, values from column 14; `- item` lines join the row above
                    // Slicing: offsets come from splitting on `\n`, `find(':')` and trims, so char boundaries.
                    let mut at = block.text.start;
                    for line in doc.text[block.text.clone()].split('\n') {
                        let start = at;
                        at += line.len() + 1;
                        let trimmed = line.trim_start();
                        if trimmed.is_empty() || trimmed.starts_with('#') {
                            continue;
                        }
                        let lead = start + line.len() - trimmed.len();
                        if let Some(item) = trimmed.strip_prefix("- ")
                            && let Some(Row { pieces: Pieces::Table(pieces), width: row_width, .. }) = self.rows[i].last_mut()
                            && !pieces.is_empty()
                        {
                            let column = if pieces.len() == 1 { 14 } else { *row_width + 2 };
                            let text = lead + 2..lead + 2 + item.trim_end().len();
                            *row_width = column + width(&doc.text[text.clone()]);
                            pieces.push(Piece { side: Side::Left, change: None, text, column });
                            continue;
                        }
                        let Some(colon) = line.find(':') else { continue };
                        let mut pieces = vec![Piece { side: Side::Left, change: None, text: start..start + colon, column: 0 }];
                        let value = line[colon + 1..].trim();
                        let mut row_width = colon;
                        if !value.is_empty() {
                            let from = start + colon + 1 + (line[colon + 1..].len() - line[colon + 1..].trim_start().len());
                            pieces.push(Piece { side: Side::Left, change: None, text: from..from + value.len(), column: 14 });
                            row_width = 14 + width(value);
                        }
                        self.rows[i].push(Row {
                            side: Side::Left,
                            change: None,
                            block: Some(block_index as u32),
                            pieces: Pieces::Table(pieces),
                            width: row_width,
                            marker: None,
                            label: None,
                        });
                    }
                } else if let BlockKind::TableRow {
                    cells, alignment, ..
                } = &block.kind
                {
                    let widths = tables[s][block_index].as_ref().unwrap();
                    let mut column = 0;
                    let mut pieces = Vec::new();
                    for (j, cell) in cells.iter().enumerate() {
                        let padding = widths[j].saturating_sub(width(&doc.text[cell.clone()]));
                        let before = match alignment.get(j) {
                            Some(TableAlignment::Right) => padding,
                            Some(TableAlignment::Center) => padding / 2,
                            _ => 0,
                        };
                        pieces.push(Piece {
                            side: Side::Left,
                            change: None,
                            text: cell.clone(),
                            column: column + before,
                        });
                        column += widths[j] + 3;
                    }
                    self.rows[i].push(Row {
                        side: Side::Left,
                        change: None,
                        block: Some(block_index as u32),
                        pieces: Pieces::Table(pieces),
                        width: column.saturating_sub(3),
                        marker: None,
                        label: None,
                    });
                } else {
                    let scrolling =
                        matches!(block.kind, BlockKind::Code { .. } | BlockKind::Literal | BlockKind::Properties);
                    // a footnote's number hangs in front of its text
                    let note = if matches!(block.kind, BlockKind::Footnote { .. }) { 4 } else { 0 };
                    let prefix = (usize::from(block.quote_depth) * 2
                        + usize::from(block.list_depth.saturating_sub(1)) * 2
                        + note)
                        .min(usize::from(widths[i].saturating_sub(1)));
                    let limit = usize::from(widths[i].max(1)).saturating_sub(prefix).max(1);
                    let mut start = block.text.start;
                    let mut column = 0;
                    // the byte after the row's last space, and the row's width there: prose wraps between words
                    let mut space: Option<(usize, usize)> = None;
                    let row = |text: Range<usize>, width: usize| Row {
                        side: Side::Left,
                        change: None,
                        block: Some(block_index as u32),
                        pieces: Pieces::One(Piece { side: Side::Left, change: None, text, column: prefix }),
                        width: prefix + width,
                        marker: None,
                        label: None,
                    };
                    for (offset, g) in doc.text[block.text.clone()].grapheme_indices(true) {
                        let byte = block.text.start + offset;
                        if g == "\n" {
                            self.rows[i].push(row(start..byte, column));
                            (start, column, space) = (byte + 1, 0, None);
                            continue;
                        }
                        if !scrolling && column > 0 && column + cell_width(g) > limit {
                            if let Some((at, before)) = space.filter(|&(at, _)| at > start && at < byte) {
                                self.rows[i].push(row(start..at, before));
                                (start, column) = (at, column - before);
                            }
                            // a word wider than the pane breaks where it must
                            if column > 0 && column + cell_width(g) > limit {
                                self.rows[i].push(row(start..byte, column));
                                (start, column) = (byte, 0);
                            }
                            space = None;
                        }
                        column += cell_width(g);
                        if g.chars().all(char::is_whitespace) {
                            space = Some((byte + g.len(), column));
                        }
                    }
                    if start < block.text.end || self.rows[i].len() == group_start {
                        self.rows[i].push(Row {
                            side: Side::Left,
                            change: None,
                            block: Some(block_index as u32),
                            pieces: Pieces::One(Piece {
                                side: Side::Left,
                                change: None,
                                text: start..block.text.end,
                                column: prefix,
                            }),
                            width: prefix + column,
                            marker: None,
                        label: None,
                        });
                    }
                }
                if let BlockKind::Footnote { label } = &block.kind
                    && let Some(row) = self.rows[i].get_mut(block_start)
                {
                    row.label = Some(Box::new((format!("{label}.").into(), LabelKind::Footnote)));
                }
                self.block_rows[s][block_index] = block_start..self.rows[i].len();
                if first { self.close_callout(i, s, block_index, block_start, shut); }
                // Semantic block spacing belongs only to this adapter.
                self.rows[i].push(Row {
                    side: Side::Left,
                    change: None,
                    block: None,
                    pieces: Pieces::Empty,
                    width: 0,
                    marker: None,
                        label: None,
                });
        // every row and piece of the block belongs to document `s`
        for row in &mut self.rows[i][from..] {
            row.side = side(s);
            for piece in row.pieces.as_mut_slice() {
                piece.side = side(s);
            }
        }
    }
    /// Inline spans of one block (words changed in place, or a whole removed block), wrapped by cells into pane 1.
    /// Slicing: span ranges come from the core's inline arrangement; graphemes keep boundaries.
    fn flow(&mut self, ib: &lenz::markdown::InlineBlock, width: u16) {
        let projection = self.projection.clone();
        let doc = &projection.pair.sides[ib.side as usize];
        let block = &doc.blocks[ib.block];
        let prefix = (usize::from(block.quote_depth) * 2 + usize::from(block.list_depth.saturating_sub(1)) * 2)
            .min(usize::from(width.saturating_sub(1)));
        let limit = usize::from(width.max(1)).saturating_sub(prefix).max(1);
        let block_start = self.rows[1].len();
        let mut pieces: Vec<Piece> = Vec::new();
        let mut column = 0;
        let push = |rows: &mut Vec<Row>, pieces: &mut Vec<Piece>, column: usize| {
            rows.push(Row {
                side: ib.side,
                change: None,
                block: Some(ib.block as u32),
                pieces: Pieces::Flow(std::mem::take(pieces)),
                width: prefix + column,
                marker: None,
                label: None,
            });
        };
        // the piece holding the row's last space, the byte after it, and the row's width there
        let mut space: Option<(usize, usize, usize)> = None;
        for span in &ib.spans {
            let text = &projection.pair.sides[span.side as usize].text;
            for (offset, g) in text[span.text.clone()].grapheme_indices(true) {
                let byte = span.text.start + offset;
                if g == "\n" {
                    push(&mut self.rows[1], &mut pieces, column);
                    (column, space) = (0, None);
                    continue;
                }
                if column > 0 && column + cell_width(g) > limit {
                    // wrap between words: what follows the last space moves to the next row
                    if let Some((k, at, before)) = space.filter(|&(_, _, before)| before < column) {
                        let mut rest = pieces.split_off(k + 1);
                        if at < pieces[k].text.end {
                            let tail = Piece { text: at..pieces[k].text.end, column: prefix + before, ..pieces[k] };
                            pieces[k].text.end = at;
                            rest.insert(0, tail);
                        }
                        push(&mut self.rows[1], &mut pieces, before);
                        for p in &mut rest { p.column -= before; }
                        pieces = rest;
                        column -= before;
                    }
                    if column > 0 && column + cell_width(g) > limit {
                        push(&mut self.rows[1], &mut pieces, column);
                        column = 0;
                    }
                    space = None;
                }
                match pieces.last_mut() {
                    Some(p) if p.side == span.side && p.change == span.change && p.text.end == byte => {
                        p.text.end = byte + g.len();
                    }
                    _ => pieces.push(Piece {
                        side: span.side,
                        change: span.change,
                        text: byte..byte + g.len(),
                        column: prefix + column,
                    }),
                }
                column += cell_width(g);
                if g.chars().all(char::is_whitespace) {
                    space = Some((pieces.len() - 1, byte + g.len(), column));
                }
            }
        }
        if !pieces.is_empty() || self.rows[1].len() == block_start {
            push(&mut self.rows[1], &mut pieces, column);
        }
        self.block_rows[ib.side as usize][ib.block] = block_start..self.rows[1].len();
        self.rows[1].push(Row {
            side: ib.side,
            change: None,
            block: None,
            pieces: Pieces::Empty,
            width: 0,
            marker: None,
            label: None,
        });
    }
    fn marker(&self, row: &Row) -> Option<SourceAnchor> {
        let d = &self.projection.decorations[row.marker()?];
        Some(SourceAnchor {
            side: d.side,
            byte: d.source.start,
        })
    }
    fn row_at(&self, anchor: SourceAnchor) -> usize {
        let i = anchor.side as usize;
        let pane = self.pane_of(anchor.side);
        let doc = &self.projection.pair.sides[i];
        if let Some(row) = self.rows[pane]
            .iter()
            .position(|r| self.marker(r) == Some(anchor))
        {
            return row;
        }
        let block = doc
            .blocks
            .partition_point(|b| b.source.end <= anchor.byte)
            .min(doc.blocks.len().saturating_sub(1));
        let rows = self.block_rows[i].get(block).cloned().unwrap_or(0..0);
        if let Some(mapped) = self.projection.source_point(anchor)
            && let Some(mut row) = self.text_row(anchor.side, mapped) {
            // a block's label rows (Properties, a callout's kind) come before its first text
            while row > 0 && self.rows[pane][row - 1].label.is_some() && self.rows[pane][row - 1].pieces.is_empty() && self.rows[pane][row - 1].block() == self.rows[pane][row].block() {
                row -= 1;
            }
            return row;
        }
        rows.start
    }
    /// The row of [`Self::pane_of`] `side` showing `byte` of that document.
    fn text_row(&self, side: Side, byte: usize) -> Option<usize> {
        let pane = self.pane_of(side);
        let indices = &self.text_rows[side as usize];
        let at = indices
            .partition_point(|&r| {
                self.rows[pane][r]
                    .pieces
                    .iter()
                    .find(|p| p.side == side)
                    .is_some_and(|p| p.text.start <= byte)
            })
            .saturating_sub(1);
        indices.get(at).copied()
    }
    fn caret_row(
        &self,
        selection: RenderSelection,
        affinity: Option<(RenderSelection, usize)>,
    ) -> Option<usize> {
        affinity
            .filter(|(old, row)| {
                *old == selection
                    && self.rows[self.pane_of(selection.side)]
                        .get(*row)
                        .is_some_and(|r| {
                            r.pieces.iter().any(|p| {
                                p.side == selection.side && p.text.start <= selection.head && selection.head <= p.text.end
                            })
                        })
            })
            .map(|(_, row)| row)
            .or_else(|| self.text_row(selection.side, selection.head))
    }
    fn motion(
        &self,
        pane: Side,
        byte: usize,
        key: KeyCode,
        height: usize,
        row: usize,
    ) -> Option<(usize, usize)> {
        let i = pane as usize;
        // rows of the pane that shows this document, and only its pieces (Inline mixes both)
        let shown = self.pane_of(pane);
        if key == KeyCode::Home {
            return self.rows[shown].get(row)?.pieces.iter().find(|p| p.side == pane).map(|p| (p.text.start, row));
        }
        if key == KeyCode::End {
            return self.rows[shown].get(row)?.pieces.iter().rfind(|p| p.side == pane).map(|p| (p.text.end, row));
        }
        let doc = &self.projection.pair.sides[i];
        // `byte` is the current selection's; this layout may be of the previous projection
        if byte > doc.text.len() || !doc.text.is_char_boundary(byte) {
            return None;
        }
        let piece = self.rows[shown]
            .get(row)?
            .pieces
            .iter()
            .find(|p| p.side == pane && p.text.start <= byte && byte <= p.text.end)?;
        let column = piece.column + width(&doc.text[piece.text.start..byte]);
        let delta = if matches!(key, KeyCode::Up | KeyCode::PageUp) {
            -1
        } else {
            1
        };
        let distance = if matches!(key, KeyCode::PageUp | KeyCode::PageDown) {
            height.max(1)
        } else {
            1
        };
        let indices = &self.text_rows[i];
        let at = indices.partition_point(|&r| r < row);
        let next = at
            .saturating_add_signed(delta * distance as isize)
            .min(indices.len().saturating_sub(1));
        let target_row = *indices.get(next)?;
        let row = &self.rows[shown][target_row];
        for piece in row.pieces.iter().filter(|p| p.side == pane) {
            let mut x = piece.column;
            for (offset, g) in doc.text[piece.text.clone()].grapheme_indices(true) {
                if column < x + cell_width(g) {
                    return Some((piece.text.start + offset, target_row));
                }
                x += cell_width(g);
            }
            if column <= x {
                return Some((piece.text.end, target_row));
            }
        }
        row.pieces.iter().rfind(|p| p.side == pane).map(|p| (p.text.end, target_row))
    }
    fn corresponding(&self, row: usize, i: usize) -> usize {
        let group = self.groups.partition_point(|g| g.rows[i].end <= row);
        self.groups.get(group).map_or(self.rows[1 - i].len(), |g| {
            g.rows[1 - i].start
                + row
                    .saturating_sub(g.rows[i].start)
                    .min(g.rows[1 - i].len().saturating_sub(1))
        })
    }
}
impl Adapter {
    pub(crate) fn suspend(&mut self) {
        self.panes = [Rect::default(); 2];
        self.hidden = None;
        self.clamp();
    }
    pub(crate) fn clear(&mut self) {
        let mut images = std::mem::take(&mut self.images);
        images.clear();
        *self = Self { images, ..Self::default() };
    }
    pub(crate) fn ready(&self,projection:&Arc<Projection>)->bool {
        !self.busy()&&self.layout.as_ref().is_some_and(|layout|Arc::ptr_eq(&layout.projection,projection))
    }
    pub(crate) fn busy(&self) -> bool {
        self.syntax.as_ref().is_some_and(|s| s.job.is_some()) || self.images.busy()
    }
    pub(crate) fn poll(&mut self) -> bool {
        if self.images.poll() { return true; }
        let Some(syntax) = &mut self.syntax else {
            return false;
        };
        if let Some(job) = &syntax.job {
            match job.try_recv() {
                Ok(result) => {
                    syntax.parsed = result.unwrap_or_default();
                    syntax.job = None;
                    return true;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    syntax.job = None;
                    return true;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
        false
    }
    fn prepare(&mut self, projection: Arc<Projection>, widths: [u16; 2], mode: ScrollMode, inline: bool) {
        if self.syntax.as_ref().is_none_or(|s| {
            s.pair
                .upgrade()
                .is_none_or(|p| !Arc::ptr_eq(&p, &projection.pair))
        }) {
            let token = Arc::new(());
            let weak = Arc::downgrade(&token);
            let pair = projection.pair.clone();
            let has_code = pair.sides.iter().any(|d| {
                d.blocks
                    .iter()
                    .any(|b| matches!(b.kind, BlockKind::Code { language: Some(_) }))
            });
            let job = has_code.then(|| {
                lenz::work::submit(weak.clone(), move || {
                    std::array::from_fn(|i| {
                        let doc = &pair.sides[i];
                        doc.blocks
                            .iter()
                            .enumerate()
                            .filter_map(|(index, block)| {
                                if let BlockKind::Code {
                                    language: Some(language),
                                } = &block.kind
                                {
                                    let path = match language.as_str() {
                                        "rust" => "code.rs".into(),
                                        "python" => "code.py".into(),
                                        "javascript" | "js" => "code.js".into(),
                                        "typescript" | "ts" => "code.ts".into(),
                                        "bash" | "sh" => "code.sh".into(),
                                        other => format!("code.{other}"),
                                    };
                                    let language =
                                        lenz::highlight::detect(std::path::Path::new(&path), "")?;
                                    let cancel = weak.clone();
                                    lenz::highlight::parse_while(
                                        language,
                                        Arc::from(&doc.text[block.text.clone()]),
                                        move || cancel.strong_count() > 0,
                                    )
                                    .map(|p| (index, p))
                                } else {
                                    None
                                }
                            })
                            .collect()
                    })
                })
            });
            // fold toggles name callouts of the documents they were made in
            self.toggled.clear();
            self.syntax = Some(CodeSyntax {
                pair: Arc::downgrade(&projection.pair),
                _token: token,
                job,
                parsed: Default::default(),
            });
        }
        if self.layout.as_ref().is_none_or(|l| {
            !Arc::ptr_eq(&l.projection, &projection) || l.widths != widths || l.mode != mode || l.inline != inline || l.toggled != self.toggled || l.image_revision != self.images.revision
        }) {
            let layout = Layout::new(projection, widths, mode, inline, &self.toggled, (self.images.pictures(), self.images.revision));
            self.images.keep(layout.rows.iter().flatten().filter_map(|r| match r.label.as_deref() {
                Some((_, LabelKind::Image { row: 0, rows, cols, .. })) => {
                    let block = r.block()?;
                    Some((r.side as usize, block, layout.pictures[r.side as usize].get(&block)?, *cols, *rows))
                }
                _ => None,
            }));
            self.layout = Some(Box::new(layout));
            self.caret_affinity = None;
            self.anchors = [None; 2];
        }
    }
    fn clamp(&mut self) {
        if let Some(layout) = &self.layout {
            for i in 0..2 {
                self.top[i] = self.top[i].min(
                    layout.rows[i]
                        .len()
                        .saturating_sub(usize::from(self.panes[i].height.max(1))),
                );
            }
        }
    }
    fn scroll(&mut self, delta: isize, pane: Side) {
        let i = pane as usize;
        self.clamp();
        self.top[i] = self.top[i].saturating_add_signed(delta);
        self.clamp();
        if let Some(layout) = &self.layout
            && !layout.mode.separate_tops()
            && !layout.inline
        {
            self.top[1 - i] = layout.corresponding(self.top[i], i);
        }
        self.clamp();
    }
    fn hit(
        &self,
        x: u16,
        y: u16,
        pane: Side,
        horizontal: usize,
    ) -> Option<(Side, usize, Option<SourceAnchor>)> {
        let i = pane as usize;
        let area = self.panes[i];
        if !area.contains((x, y).into()) {
            return None;
        }
        let layout = self.layout.as_ref()?;
        let row = layout.rows[i].get(self.top[i] + usize::from(y - area.y))?;
        if let Some(anchor) = layout.marker(row) {
            return Some((anchor.side, 0, Some(anchor)));
        }
        let doc = &layout.projection.pair.sides[row.side as usize];
        let scrolling = row.block().is_some_and(|b| {
            matches!(
                doc.blocks[b].kind,
                BlockKind::Code { .. } | BlockKind::Literal | BlockKind::Properties | BlockKind::TableRow { .. }
            )
        });
        let target = usize::from(x - area.x) + if scrolling { horizontal } else { 0 };
        for (n, piece) in row.pieces.iter().enumerate() {
            let text = &layout.projection.pair.sides[piece.side as usize].text;
            let mut column = piece.column;
            for (offset, g) in text[piece.text.clone()].grapheme_indices(true) {
                if target < column + cell_width(g) {
                    return Some((piece.side, piece.text.start + offset, None));
                }
                column += cell_width(g);
            }
            // just past a piece's end: its end, unless the next piece starts there (Inline rows join pieces)
            if target <= column && row.pieces.get(n + 1).is_none_or(|next| next.column > target) {
                return Some((piece.side, piece.text.end, None));
            }
        }
        row.pieces.last().map(|p| (p.side, p.text.end, None))
    }
    /// The foldable callout whose title row is under (x, y): (document, callout id).
    fn fold_at(&self, x: u16, y: u16, pane: Side) -> Option<(usize, u32)> {
        let i = pane as usize;
        if !self.panes[i].contains((x, y).into()) { return None; }
        self.layout.as_ref()?.folds[i].get(&(self.top[i] + usize::from(y - self.panes[i].y))).copied()
    }
    pub(crate) fn horizontal_max(&self, pane: Side) -> usize {
        self.layout.as_ref().map_or(0, |l| {
            l.widest[pane as usize].saturating_sub(usize::from(l.widths[pane as usize]))
        })
    }
    /// Where document `pane` is scrolled to: the first of its text at or below the top of the pane showing it.
    fn anchor(&self, pane: Side) -> Option<SourceAnchor> {
        let layout = self.layout.as_ref()?;
        let i = pane as usize;
        let shown = layout.pane_of(pane);
        // Inline shows unchanged text once, from the new document: the old document is at the same block of the
        // row's alignment group
        if layout.inline
            && pane == Side::Left
            && let Some(row) = layout.rows[shown].iter().skip(self.top[shown]).find(|r| r.block().is_some())
            && row.side == Side::Right
            && let Some(b) = row.block()
            && let Some(group) = layout.projection.groups.iter().find(|g| g.blocks[1].contains(&b))
        {
            let old = group.blocks[0].start + (b - group.blocks[1].start);
            return layout.projection.pair.sides[0]
                .blocks
                .get(old.min(group.blocks[0].end.saturating_sub(1)))
                .filter(|_| !group.blocks[0].is_empty())
                .map(|block| SourceAnchor { side: Side::Left, byte: block.source.start });
        }
        layout.rows[shown].iter().skip(self.top[shown]).find_map(|r| {
            layout.marker(r).or_else(|| {
                let doc = &layout.projection.pair.sides[i];
                let mapped = r.pieces.iter().find(|p| p.side == pane).and_then(|piece| {
                    let at = piece.text.start;
                    let head = at + doc.text[at..].chars().next()?.len_utf8();
                    layout
                        .projection
                        .source_selection(RenderSelection {
                            side: pane,
                            anchor: at,
                            head,
                        })
                        .and_then(|selected| {
                            doc.line_starts
                                .get(selected.anchor.line as usize)
                                .map(|start| SourceAnchor {
                                    side: pane,
                                    byte: start + selected.anchor.col,
                                })
                        })
                });
                mapped.or_else(|| {
                    r.block().filter(|_| r.side == pane).map(|b| SourceAnchor {
                        side: pane,
                        byte: doc.blocks[b].source.start,
                    })
                })
            })
        })
    }
    fn draw(
        &mut self,
        buf: &mut Buffer,
        palette: &lenz::theme::Palette,
        selection: Option<RenderSelection>,
        horizontal: [usize; 2],
        hit: Option<(SourceAnchor, Range<usize>, Vec<Range<usize>>)>,
    ) {
        self.hidden = None;
        let Some(layout) = &self.layout else {
            return;
        };
        let caret_row = selection.and_then(|s| layout.caret_row(s, self.caret_affinity));
        for (i, &horizontal_offset) in horizontal.iter().enumerate() {
            let pane = self.panes[i];
            if pane.width == 0 || pane.height == 0 {
                continue;
            }
            for (y, row) in layout.rows[i]
                .iter()
                .skip(self.top[i])
                .take(usize::from(pane.height))
                .enumerate()
            {
                let y = pane.y + y as u16;
                if let Some(index) = row.marker() {
                    buf.set_stringn(
                        pane.x,
                        y,
                        "Markup change · Show Source",
                        usize::from(pane.width),
                        style_with(palette)
                            .fg(color(if layout.projection.decorations[index].ignored {
                                palette.dim
                            } else {
                                palette.accent
                            }))
                            .add_modifier(Modifier::UNDERLINED),
                    );
                    continue;
                }
                let Some(block_index) = row.block() else {
                    continue;
                };
                let doc = &layout.projection.pair.sides[row.side as usize];
                let block = &doc.blocks[block_index];
                // a change mark in the margin, in both layouts
                if let Some(kind) = row.change {
                    let tint = palette.block[match kind { Kind::Inserted => 0, Kind::Deleted => 1, Kind::Modified => 2, Kind::Conflict => 3 }];
                    buf.set_string(pane.x, y, "▌", style_with(palette).fg(color(tint)));
                }
                let margin = 2;
                if let Some((_, LabelKind::Image { row: k, rows, cols, x })) = row.label.as_deref() {
                    if let Some(picture) = layout.pictures[row.side as usize].get(&block_index) {
                        let id = self.images.show(row.side as usize, block_index, picture, *cols, *rows);
                        crate::md_images::draw_row(buf, pane, pane.x + margin + x, y, *k, *rows, *cols, picture, palette.bg, id);
                    }
                    continue;
                }
                let callout = block.callout.map(|c| lenz::theme::callout_color(palette, c.kind));
                if let Some((label, kind)) = row.label.as_deref() {
                    let (text, style, x) = match kind {
                        LabelKind::Callout(k) => (
                            format!("{} {label}", lenz::theme::callout_icon(*k)),
                            style_with(palette).fg(color(lenz::theme::callout_color(palette, *k))).add_modifier(Modifier::BOLD),
                            usize::from(block.quote_depth) * 2,
                        ),
                        LabelKind::Properties => (label.to_string(), style_with(palette).fg(color(palette.dim)), 0),
                        LabelKind::Image { .. } => (String::new(), style_with(palette), 0),
                        // after the title: row widths already hold the margin
                        LabelKind::Fold => (label.to_string(), style_with(palette).fg(color(callout.unwrap_or(palette.dim))), (row.width + 1).saturating_sub(usize::from(margin))),
                        LabelKind::Footnote => (
                            label.to_string(),
                            style_with(palette).fg(color(palette.dim)),
                            usize::from(block.quote_depth) * 2 + usize::from(block.list_depth.saturating_sub(1)) * 2,
                        ),
                    };
                    let x = x + usize::from(margin);
                    if x < usize::from(pane.width) {
                        buf.set_stringn(pane.x + x as u16, y, text, usize::from(pane.width) - x, style);
                    }
                }
                let code = matches!(
                    block.kind,
                    BlockKind::Code { .. } | BlockKind::Literal | BlockKind::Properties | BlockKind::TableRow { .. }
                );
                let offset = if code { horizontal_offset } else { 0 };
                for depth in 0..block.quote_depth.min(pane.width.saturating_sub(1) / 2) {
                    let x = pane.x + margin + depth.saturating_mul(2);
                    if x < pane.right() {
                        buf.set_string(x, y, "│", style_with(palette).fg(color(callout.unwrap_or(palette.dim))));
                    }
                }
                if matches!(block.kind, BlockKind::Rule) {
                    buf.set_stringn(
                        pane.x,
                        y,
                        "─".repeat(usize::from(pane.width)),
                        usize::from(pane.width),
                        style_with(palette).fg(color(palette.dim)),
                    );
                    continue;
                }
                if matches!(block.kind, BlockKind::Code { .. }) {
                    buf.set_style(
                        Rect::new(pane.x, y, pane.width, 1),
                        style_with(palette).bg(color(palette.gutter_bg)),
                    );
                }
                let captures = self
                    .syntax
                    .as_ref()
                    .and_then(|s| s.parsed[row.side as usize].iter().find(|(index, _)| *index == block_index))
                    .map(|(_, p)| {
                        let at = row
                            .pieces
                            .first()
                            .map_or(block.text.start, |p| p.text.start);
                        let line = self.top[i] + usize::from(y - pane.y)
                            - layout.block_rows[row.side as usize][block_index].start;
                        let line_start = at;
                        (
                            line_start,
                            p.spans(line..line + 1).pop().unwrap_or_default(),
                        )
                    });
                for (cell, piece) in row.pieces.iter().enumerate() {
                    if matches!(row.pieces, Pieces::Table(_)) && matches!(block.kind, BlockKind::TableRow { .. }) && cell > 0 {
                        let x = piece.column.saturating_sub(2);
                        if x >= offset && x < offset + usize::from(pane.width) {
                            buf.set_string(
                                pane.x + (x - offset) as u16,
                                y,
                                "│",
                                style_with(palette).fg(color(palette.dim)),
                            );
                        }
                    }
                    let mut column = piece.column;
                    // Inline mixes documents on a row: each piece is styled from its own document and block
                    let (doc, block) = if piece.side == row.side {
                        (doc, block)
                    } else {
                        let doc = &layout.projection.pair.sides[piece.side as usize];
                        match doc.blocks.get(doc.blocks.partition_point(|b| b.text.end <= piece.text.start)) {
                            Some(block) => (doc, block),
                            None => continue,
                        }
                    };
                    let whole = layout.projection.whole_change(row.side, block_index).is_some();
                    let decorations: &[usize] = if layout.inline || whole { &[] } else { &layout.block_decorations[i][block_index] };
                    for (byte, g) in doc.text[piece.text.clone()].grapheme_indices(true) {
                        let byte = piece.text.start + byte;
                        let cells = cell_width(g);
                        if column >= offset + usize::from(pane.width) {
                            break;
                        }
                        if column >= offset && column + cells <= offset + usize::from(pane.width) {
                            let mut style = style_with(palette);
                            if matches!(block.kind, BlockKind::Code { .. }) {
                                style = style.bg(color(palette.gutter_bg));
                            }
                            // changed words as in Inline: new side green and underlined, old side red; a block that
                            // changed as a whole is left to its bar
                            let whole = |d: &lenz::markdown::Decoration| d.text.start <= block.text.start && d.text.end >= block.text.end;
                            if let Some(d) = decorations
                                .iter()
                                .map(|&index| &layout.projection.decorations[index])
                                .find(|d| d.text.start < byte + g.len() && byte < d.text.end && !whole(d))
                            {
                                style = if d.ignored {
                                    style.fg(color(palette.dim))
                                } else if row.side == Side::Left {
                                    style.fg(color(lenz::theme::removed_color(palette)))
                                } else {
                                    style.fg(color(palette.status[0])).add_modifier(Modifier::UNDERLINED)
                                };
                            }
                            if matches!(
                                block.kind,
                                BlockKind::Heading(_) | BlockKind::TableRow { header: true, .. }
                            ) {
                                style =
                                    style.add_modifier(Modifier::BOLD).fg(color(palette.accent));
                            }
                            // runs are sorted and disjoint: a binary search, not a scan of the block per cell
                            let at = block.runs.partition_point(|r| r.text.end <= byte);
                            if let Some(run) = block.runs.get(at).filter(|r| r.text.start <= byte) {
                                if run.style.strong {
                                    style = style.add_modifier(Modifier::BOLD);
                                }
                                if run.style.emphasis {
                                    style = style.add_modifier(Modifier::ITALIC);
                                }
                                if run.style.strike {
                                    style = style.add_modifier(Modifier::CROSSED_OUT);
                                }
                                if run.style.link && run.style.external {
                                    style = style
                                        .add_modifier(Modifier::UNDERLINED)
                                        .fg(color(palette.syntax.function));
                                } else if run.style.link {
                                    style = style.fg(color(palette.accent));
                                }
                                if run.style.code {
                                    style = style.fg(color(palette.accent));
                                }
                                if run.style.highlight {
                                    style = style.bg(color(lenz::theme::highlight_bg(palette)));
                                }
                                if run.style.tag {
                                    style = style.fg(color(palette.syntax.keyword));
                                }
                                if run.style.footnote {
                                    style = style.add_modifier(Modifier::BOLD).fg(color(palette.accent));
                                }
                                if run.style.callout_title
                                    && let Some(c) = callout
                                {
                                    style = style.add_modifier(Modifier::BOLD).fg(color(c));
                                }
                            }
                            if let BlockKind::ListItem { marker, checked: Some(true) } = &block.kind
                                && byte >= block.text.start + marker.len() + 5
                            {
                                style = style.add_modifier(Modifier::CROSSED_OUT).fg(color(palette.dim));
                            }
                            if matches!(block.kind, BlockKind::Footnote { .. })
                                || (matches!(block.kind, BlockKind::Properties) && cell == 0 && row.label.is_none())
                            {
                                style = style.fg(color(palette.dim));
                            }
                            // coloured and underlined or struck, never a fill that could pass for a highlight
                            match piece.change {
                                Some(Kind::Deleted) => {
                                    style = style
                                        .add_modifier(Modifier::CROSSED_OUT)
                                        .fg(color(lenz::theme::removed_color(palette)));
                                }
                                Some(Kind::Inserted) if row.change == Some(Kind::Modified) => {
                                    style = style.add_modifier(Modifier::UNDERLINED).fg(color(palette.status[0]));
                                }
                                _ => {}
                            }
                            if let Some((start, spans)) = &captures
                                && let Some((_, capture)) = spans
                                    .iter()
                                    .find(|(r, _)| r.start + start <= byte && byte < r.end + start)
                            {
                                style = style.fg(color(palette.syntax_color(*capture)));
                            }
                            if hit.as_ref().is_some_and(|(a, _, ranges)| {
                                a.side == piece.side
                                    && ranges
                                        .iter()
                                        .any(|r| r.start < byte + g.len() && byte < r.end)
                            }) {
                                style = style.bg(color(palette.found_bg)).fg(color(palette.fg));
                            }
                            if selection.is_some_and(|s| {
                                s.side == piece.side
                                    && s.anchor.min(s.head) < byte + g.len()
                                    && byte < s.anchor.max(s.head)
                            }) {
                                style = style.bg(color(palette.sel_bg)).fg(color(palette.fg));
                            }
                            if selection.is_some_and(|s| s.side == piece.side)
                                && caret_row == Some(self.top[i] + usize::from(y - pane.y))
                                && selection.is_some_and(|s| {
                                    s.head == byte
                                        || (s.head == piece.text.end
                                            && byte + g.len() == piece.text.end
                                            && column + cells == offset + usize::from(pane.width))
                                })
                            {
                                style = style.add_modifier(Modifier::REVERSED);
                            }
                            let capped = lenz::document::cap_marks(g);
                            let printable = if g == "\t" {
                                "    "
                            } else if g.chars().any(lenz::document::hides_text) {
                                "·"
                            } else {
                                capped.as_ref()
                            };
                            buf.set_string(pane.x + (column - offset) as u16, y, printable, style);
                        }
                        column += cells;
                    }
                    if selection.is_some_and(|s| {
                        s.side == piece.side
                            && s.head == piece.text.end
                            && caret_row == Some(self.top[i] + usize::from(y - pane.y))
                    }) && column >= offset
                        && column < offset + usize::from(pane.width)
                    {
                        buf.set_string(
                            pane.x + (column - offset) as u16,
                            y,
                            "▏",
                            style_with(palette).fg(color(palette.caret)),
                        );
                    }
                }
            }
        }
        if let Some((anchor, _, ranges)) = hit
            && (ranges.is_empty() || ranges.iter().all(Range::is_empty))
        {
            let pane = self.panes[layout.pane_of(anchor.side)];
            if pane.width > 0 && pane.height > 0 {
                let area = Rect::new(pane.x, pane.bottom() - 1, pane.width, 1);
                buf.set_stringn(
                    area.x,
                    area.y,
                    "Hidden source match · Show Source",
                    usize::from(area.width),
                    style_with(palette).fg(color(palette.accent)),
                );
                self.hidden = Some((area, anchor));
            }
        }
    }
}
impl App {
    pub(crate) fn comparison_command(&mut self, command: lenz::commands::Command) {
        let anchor = if matches!(
            command,
            lenz::commands::Command::SetPresentation(PresentationMode::Source)
        ) {
            self.session.markdown_anchor(self.focus)
        } else {
            None
        };
        self.session.execute(command);
        if let Some(anchor) = anchor {
            let doc = self.session.document_arc();
            self.session
                .markdown_command(MarkdownCommand::ShowSource(anchor), doc);
        }
    }
    pub(crate) fn markdown_select_all(&mut self) {
        if let Some(p) = self.session.markdown_projection() {
            let end = p.pair.sides[self.focus as usize].text.len();
            self.markdown_select(RenderSelection {
                side: self.focus,
                anchor: 0,
                head: end,
            });
        }
    }
    fn markdown_select(&mut self, selection: RenderSelection) {
        self.markdown.caret_affinity = None;
        let source = self.session.markdown_projection().and_then(|p| {
            let text=&p.pair.sides[selection.side as usize].text;
            let head=selection.head+text.get(selection.head..)?.chars().next().map_or(0,char::len_utf8);
            p.source_selection(RenderSelection{side:selection.side,anchor:selection.head,head}).and_then(|s| {
                p.pair.sides[s.side as usize]
                    .line_starts
                    .get(s.anchor.line as usize)
                    .map(|start| SourceAnchor {
                        side: s.side,
                        byte: start + s.anchor.col,
                    })
            })
        });
        let doc = self.session.document_arc();
        self.session
            .markdown_command(MarkdownCommand::Select(selection), doc.clone());
        if let Some(anchor) = source {
            self.session
                .markdown_command(MarkdownCommand::SetAnchor(anchor), doc);
            self.markdown.anchors[anchor.side as usize] = Some(anchor);
        }
    }
    /// Rendered Markdown shown Inline: one full-width pane.
    pub(crate) fn markdown_inline(&self) -> bool {
        self.session.markdown_mode() == PresentationMode::Rendered
            && self.session.markdown_layout() == lenz::markdown::MarkdownLayout::Inline
    }
    pub(crate) fn markdown_draw(&mut self, buf: &mut Buffer, panes: &[(Side, Rect)]) {
        let Some(projection) = self.session.markdown_projection().cloned() else {
            return;
        };
        let inline = self.markdown_inline();
        if inline {
            // keyboard scrolling, drags and selections act on the one pane
            self.focus = Side::Right;
        }
        let widths = if self.diff_area.width < 70 || inline {
            [self.diff_area.width; 2]
        } else {
            [
                self.left_width(),
                self.diff_area.width - self.left_width() - 1,
            ]
        };
        if let Some(set) = self.session.change_set() {
            self.markdown.images.load(&projection.pair, &set, self.session.current(), self.session.spec(), self.session.cwd());
        }
        self.markdown
            .prepare(projection, widths, self.session.view().mode(), inline);
        self.markdown.panes = [Rect::default(); 2];
        for &(side, pane) in panes {
            self.markdown.panes[side as usize] = pane;
        }
        if inline {
            // one pane: the new side's position, or the old side's when it is just above (a removed block)
            let anchors = [Side::Left, Side::Right].map(|s| self.session.markdown_anchor(s));
            if anchors != self.markdown.anchors
                && let Some(layout) = self.markdown.layout.as_ref()
            {
                let new = anchors[1].map(|a| layout.row_at(a));
                let old = anchors[0].filter(|_| anchors[0] != self.markdown.anchors[0]).map(|a| layout.row_at(a));
                let height = usize::from(self.markdown.panes[1].height.max(1));
                let top = match (old, new) {
                    (Some(o), Some(n)) if o < n && n - o < height => Some(o),
                    (_, Some(n)) => Some(n),
                    (o, None) => o,
                };
                if let Some(top) = top {
                    self.markdown.top[1] = top;
                }
                self.markdown.anchors = anchors;
            }
        } else {
            for i in 0..2 {
                let anchor = self.session.markdown_anchor(side(i));
                if self.markdown.anchors[i] != anchor {
                    if let Some(anchor) = anchor {
                        self.markdown.top[i] = self.markdown.layout.as_ref().unwrap().row_at(anchor);
                    }
                    self.markdown.anchors[i] = anchor;
                }
            }
        }
        self.markdown.clamp();
        for i in 0..2 {
            self.horizontal[i] = self.horizontal[i].min(self.markdown.horizontal_max(side(i)));
        }
        self.markdown.draw(
            buf,
            &self.palette,
            self.session.markdown_selection(),
            self.horizontal,
            self.session.markdown_find_hit(),
        );
    }
    fn markdown_scroll(&mut self, delta: isize, pane: Side) {
        self.markdown.scroll(delta, pane);
        for i in 0..2 {
            let inline = self.markdown.layout.as_ref().is_some_and(|l| l.inline);
            // Inline shows both documents in one pane: both keep their place, for switching to Side by side
            if !inline && self.session.view().mode().separate_tops() && i != pane as usize {
                continue;
            }
            if let Some(anchor) = self.markdown.anchor(side(i)) {
                let doc = self.session.document_arc();
                self.session
                    .markdown_command(MarkdownCommand::SetAnchor(anchor), doc);
                self.markdown.anchors[i] = Some(anchor);
            }
        }
    }
    pub(crate) fn markdown_context_pointer(&mut self, x: u16, y: u16) {
        let pane = self.pointer_side(x);
        if self.session.markdown_selection().is_some_and(|s| s.anchor != s.head) { self.focus = pane; return; }
        if let Some((side, byte, marker)) = self
            .markdown
            .hit(x, y, pane, self.horizontal[pane as usize])
        {
            self.focus = pane;
            if let Some(anchor) = marker {
                let doc = self.session.document_arc();
                self.session.markdown_command(MarkdownCommand::ClearSelection, doc.clone());
                self.session
                    .markdown_command(MarkdownCommand::SetAnchor(anchor), doc);
                self.markdown.anchors[anchor.side as usize] = Some(anchor);
            } else if self
                .session
                .markdown_selection()
                .is_none_or(|s| s.side != side || s.anchor == s.head)
            {
                self.markdown_select(RenderSelection {
                    side,
                    anchor: byte,
                    head: byte,
                });
            }
        }
    }
    pub(crate) fn markdown_pointer(&mut self, x: u16, y: u16, extend: bool) -> bool {
        let pane = self.pointer_side(x);
        if let Some((area, anchor)) = self.markdown.hidden
            && area.contains((x, y).into())
        {
            let doc = self.session.document_arc();
            self.session
                .markdown_command(MarkdownCommand::ShowSource(anchor), doc);
            return true;
        }
        // a foldable callout's title opens or closes it
        if !extend && let Some(key) = self.markdown.fold_at(x, y, pane) {
            if !self.markdown.toggled.remove(&key) { self.markdown.toggled.insert(key); }
            return true;
        }
        if let Some((side, byte, marker)) = self
            .markdown
            .hit(x, y, pane, self.horizontal[pane as usize])
        {
            self.focus = pane;
            if let Some(anchor) = marker {
                let doc = self.session.document_arc();
                self.session
                    .markdown_command(MarkdownCommand::ShowSource(anchor), doc);
                return true;
            }
            let current = self.session.markdown_selection();
            // an extended selection stays in its document (Inline rows mix both)
            if extend && current.is_some_and(|s| s.side != side) {
                return true;
            }
            let anchor = if extend {
                current.filter(|s| s.side == side).map_or(byte, |s| s.anchor)
            } else {
                byte
            };
            self.markdown_select(RenderSelection {
                side,
                anchor,
                head: byte,
            });
            return true;
        }
        false
    }
    pub(crate) fn markdown_input(&mut self, event: &Event) -> bool {
        if self.session.markdown_mode() != PresentationMode::Rendered {
            if self.session.markdown_loading()
                && matches!(event,Event::Mouse(m) if self.diff_area.contains((m.column,m.row).into())&&matches!(m.kind,MouseEventKind::Down(MouseButton::Left)|MouseEventKind::Drag(MouseButton::Left)))
            {
                return true;
            }
            return false;
        }
        match event {
            Event::Mouse(m) if self.diff_area.contains((m.column, m.row).into()) => {
                let pane = self.pointer_side(m.column);
                match m.kind {
                    MouseEventKind::ScrollDown | MouseEventKind::ScrollUp
                        if m.modifiers.contains(KeyModifiers::SHIFT) =>
                    {
                        let old = self.focus;
                        self.focus = pane;
                        self.interaction(if m.kind == MouseEventKind::ScrollDown {
                            lenz::commands::Interaction::ScrollRight
                        } else {
                            lenz::commands::Interaction::ScrollLeft
                        });
                        self.focus = old;
                    }
                    MouseEventKind::ScrollRight | MouseEventKind::ScrollLeft => {
                        let old = self.focus;
                        self.focus = pane;
                        self.interaction(if m.kind == MouseEventKind::ScrollRight {
                            lenz::commands::Interaction::ScrollRight
                        } else {
                            lenz::commands::Interaction::ScrollLeft
                        });
                        self.focus = old;
                    }
                    MouseEventKind::ScrollDown => self.markdown_scroll(3, pane),
                    MouseEventKind::ScrollUp => self.markdown_scroll(-3, pane),
                    MouseEventKind::Down(MouseButton::Left) => {
                        let shift = m.modifiers.contains(KeyModifiers::SHIFT);
                        if !self.markdown_pointer(m.column, m.row, shift) {
                            return true;
                        }
                        if !shift && let Some(mut s) = self.session.markdown_selection() {
                            let now = std::time::Instant::now();
                            let count = self
                                .markdown
                                .click
                                .filter(|(time, side, byte, _)| {
                                    now.duration_since(*time)
                                        < std::time::Duration::from_millis(400)
                                        && *side == s.side
                                        && *byte == s.head
                                })
                                .map_or(1, |(_, _, _, count)| count % 3 + 1);
                            self.markdown.click = Some((now, s.side, s.head, count));
                            if count > 1
                                && let Some(p) = self.session.markdown_projection()
                            {
                                let text = &p.pair.sides[s.side as usize].text;
                                if count == 2 {
                                    // segment the caret's line, not the whole rendered text
                                    let from = text[..s.head].rfind('\n').map_or(0, |at| at + 1);
                                    let to = text[s.head..].find('\n').map_or(text.len(), |at| s.head + at);
                                    if let Some((at, word)) = text[from..to]
                                        .unicode_word_indices()
                                        .map(|(at, w)| (from + at, w))
                                        .find(|(at, w)| *at <= s.head && s.head < *at + w.len())
                                    {
                                        s.anchor = at;
                                        s.head = at + word.len();
                                    }
                                } else {
                                    s.anchor = text[..s.head].rfind('\n').map_or(0, |at| at + 1);
                                    s.head = text[s.head..]
                                        .find('\n')
                                        .map_or(text.len(), |at| s.head + at + 1);
                                }
                                self.markdown_select(s);
                            }
                        }
                    }
                    MouseEventKind::Drag(MouseButton::Left) => {
                        let area = self.markdown.panes[self.focus as usize];
                        if area.height == 0 {
                            return true;
                        }
                        if m.row < area.y {
                            self.markdown_scroll(-3, self.focus);
                        } else if m.row >= area.bottom() {
                            self.markdown_scroll(3, self.focus);
                        }
                        self.markdown_pointer(
                            m.column,
                            m.row.clamp(area.y, area.bottom() - 1),
                            true,
                        );
                    }
                    _ => return false,
                }
                true
            }
            Event::Key(k)
                if k.kind != KeyEventKind::Release && !k.modifiers.contains(KeyModifiers::ALT) =>
            {
                let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
                let shift = k.modifiers.contains(KeyModifiers::SHIFT);
                // Enter on a foldable callout's title row opens or closes it
                if k.code == KeyCode::Enter && !ctrl && !shift
                    && let Some(selection) = self.session.markdown_selection()
                    && let Some(layout) = self.markdown.layout.as_deref()
                    && let Some(row) = layout.caret_row(selection, self.markdown.caret_affinity)
                    && let Some(&key) = layout.folds[layout.pane_of(selection.side)].get(&row)
                {
                    if !self.markdown.toggled.remove(&key) { self.markdown.toggled.insert(key); }
                    return true;
                }
                if k.code == KeyCode::Esc {
                    if self.session.find().is_some() {
                        self.session.clear_find();
                    } else if self.session.markdown_selection().is_some() {
                        let doc = self.session.document_arc();
                        self.session
                            .markdown_command(MarkdownCommand::ClearSelection, doc);
                    } else {
                        return false;
                    }
                    return true;
                }
                if matches!(
                    k.code,
                    KeyCode::Left
                        | KeyCode::Right
                        | KeyCode::Up
                        | KeyCode::Down
                        | KeyCode::Home
                        | KeyCode::End
                        | KeyCode::PageUp
                        | KeyCode::PageDown
                ) && (shift || self.session.markdown_selection().is_some())
                {
                    let Some(p) = self.session.markdown_projection() else {
                        return true;
                    };
                    let text = &p.pair.sides[self.focus as usize].text;
                    let mut s = self
                        .session
                        .markdown_selection()
                        .filter(|s| s.side == self.focus)
                        .unwrap_or(RenderSelection {
                            side: self.focus,
                            anchor: 0,
                            head: 0,
                        });
                    let previous = || {
                        text[..s.head]
                            .grapheme_indices(true)
                            .next_back()
                            .map_or(0, |(at, _)| at)
                    };
                    let next = || {
                        text[s.head..]
                            .graphemes(true)
                            .next()
                            .map_or(s.head, |g| s.head + g.len())
                    };
                    let visual_row = self
                        .markdown
                        .layout
                        .as_ref()
                        .and_then(|l| l.caret_row(s, self.markdown.caret_affinity));
                    let mut affinity = None;
                    s.head = match k.code {
                        KeyCode::Left => previous(),
                        KeyCode::Right => next(),
                        KeyCode::Home if ctrl => 0,
                        KeyCode::End if ctrl => text.len(),
                        KeyCode::Home
                        | KeyCode::End
                        | KeyCode::Up
                        | KeyCode::Down
                        | KeyCode::PageUp
                        | KeyCode::PageDown => self
                            .markdown
                            .layout
                            .as_ref()
                            .and_then(|l| {
                                l.motion(
                                    self.focus,
                                    s.head,
                                    k.code,
                                    usize::from(self.markdown.panes[self.focus as usize].height),
                                    visual_row?,
                                )
                            })
                            .map(|(byte, row)| {
                                affinity = Some(row);
                                byte
                            })
                            .unwrap_or(s.head),
                        _ => unreachable!(),
                    };
                    if !shift {
                        s.anchor = s.head;
                    }
                    self.markdown_select(s);
                    self.markdown.caret_affinity = affinity.map(|row| (s, row));
                    if let Some(layout) = &self.markdown.layout
                        && let Some(row) = layout.caret_row(s, self.markdown.caret_affinity)
                    {
                        let i = self.focus as usize;
                        let h = usize::from(self.markdown.panes[i].height.max(1));
                        if row < self.markdown.top[i] {
                            self.markdown_scroll(
                                row as isize - self.markdown.top[i] as isize,
                                self.focus,
                            );
                        } else if row >= self.markdown.top[i] + h {
                            self.markdown_scroll(
                                (row + 1 - h) as isize - self.markdown.top[i] as isize,
                                self.focus,
                            );
                        }
                    }
                    return true;
                }
                let page = usize::from(
                    self.markdown.panes[self.focus as usize]
                        .height
                        .saturating_sub(2)
                        .max(1),
                ) as isize;
                let delta = match k.code {
                    KeyCode::Down | KeyCode::Char('j') => 1,
                    KeyCode::Up | KeyCode::Char('k') => -1,
                    KeyCode::PageDown | KeyCode::Char(' ') => page,
                    KeyCode::PageUp => -page,
                    KeyCode::Home => isize::MIN,
                    KeyCode::End => isize::MAX,
                    _ => return false,
                };
                self.markdown_scroll(delta, self.focus);
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ready(app: &mut App) {
        let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            app.poll();
            if !app.busy() {
                return;
            }
            assert!(std::time::Instant::now() < until);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }
    use ratatui::{Terminal, backend::TestBackend};
    fn draw(app: &mut App, width: u16, height: u16) {
        let mut t = Terminal::new(TestBackend::new(width, height)).unwrap();
        t.draw(|f| app.render(f)).unwrap();
    }
    fn pair() -> (tempfile::TempDir, App) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("a.md"),
            "# Old\n\n```rust\nfn old() {}\n```\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("b.md"),
            "# New\n\n```rust\nfn new() {}\n```\n",
        )
        .unwrap();
        let request = lenz::cli::parse(
            &["--no-index".into(), "a.md".into(), "b.md".into()],
            dir.path(),
        )
        .unwrap();
        let mut app = App::open(request, dir.path().into());
        ready(&mut app);
        app.session.execute(lenz::commands::Command::SetMarkdownLayout(lenz::markdown::MarkdownLayout::SideBySide));
        (dir, app)
    }
    #[test]
    fn pane_drag_rendered_markdown_reflows_unequal_widths_and_keeps_selection() {
        let (dir,mut app)=pair();draw(&mut app,160,36);
        app.comparison_command(lenz::commands::Command::SetPresentation(PresentationMode::Rendered));ready(&mut app);draw(&mut app,160,36);ready(&mut app);draw(&mut app,160,36);
        app.markdown_select_all();let selected=app.session.markdown_selection();
        let y=app.diff_area.y+2;let divider=app.diff_area.x+app.left_width();
        let mouse=|kind,x|Event::Mouse(crossterm::event::MouseEvent{kind,column:x,row:y,modifiers:KeyModifiers::NONE});
        app.handle(mouse(MouseEventKind::Down(MouseButton::Left),divider));app.handle(mouse(MouseEventKind::Drag(MouseButton::Left),110));app.handle(mouse(MouseEventKind::Up(MouseButton::Left),110));draw(&mut app,160,36);
        assert_eq!(app.session.markdown_selection(),selected);
        let layout=app.markdown.layout.as_ref().unwrap();assert_eq!(layout.widths,[110,49]);assert_eq!(app.markdown.panes[1].x,111);assert_eq!(app.pointer_side(100),Side::Left);
        app.interaction(lenz::commands::Interaction::ResetPaneSizes);draw(&mut app,160,36);assert_eq!(app.markdown.layout.as_ref().unwrap().widths,[80,79]);
        assert_eq!(std::fs::read_to_string(dir.path().join("b.md")).unwrap(),"# New\n\n```rust\nfn new() {}\n```\n");
    }
    #[test]
    #[ignore = "bounded source and rendered Markdown pane drag event/render benchmark"]
    fn pane_drag_render_workload() {
        let (dir,mut app)=pair();let before=std::fs::read(dir.path().join("b.md")).unwrap();
        let mut terminal=Terminal::new(TestBackend::new(160,36)).unwrap();
        let mut peak=crate::perf::rss_mib().unwrap_or(0.0);
        for mode in [PresentationMode::Source,PresentationMode::Rendered] {
            app.comparison_command(lenz::commands::Command::SetPresentation(mode));ready(&mut app);terminal.draw(|f|app.render(f)).unwrap();ready(&mut app);terminal.draw(|f|app.render(f)).unwrap();
            if mode==PresentationMode::Rendered {app.markdown_select_all();}
            let selection=app.session.markdown_selection();let source_selection=app.selection;let y=app.diff_area.y+2;
            let mouse=|kind,x|Event::Mouse(crossterm::event::MouseEvent{kind,column:x,row:y,modifiers:KeyModifiers::NONE});
            app.handle(mouse(MouseEventKind::Down(MouseButton::Left),app.diff_area.x+app.left_width()));
            let mut times=Vec::new();for i in 0..240 {let started=std::time::Instant::now();app.handle(mouse(MouseEventKind::Drag(MouseButton::Left),40+(i%80)as u16));terminal.draw(|f|app.render(f)).unwrap();times.push(started.elapsed().as_secs_f64()*1000.0);if i%20==0 {peak=peak.max(crate::perf::rss_mib().unwrap_or(0.0));}}
            app.handle(mouse(MouseEventKind::Up(MouseButton::Left),80));assert_eq!(app.session.markdown_selection(),selection);assert_eq!(app.selection,source_selection);
            crate::perf::record(if mode==PresentationMode::Source{"tui/pane-drag-source"}else{"tui/pane-drag-markdown"},"ms",&times);
        }
        crate::perf::record("tui/pane-drag-peak","MiB",&[peak]);if let Some(rss)=crate::perf::rss_mib(){crate::perf::record("tui/pane-drag-settled","MiB",&[rss]);}
        assert_eq!(std::fs::read(dir.path().join("b.md")).unwrap(),before);
    }

    #[test]
    fn markdown_tui_layout_is_lazy_reused_bounded_and_released_with_cancelled_syntax() {
        let (_dir, mut app) = pair();
        draw(&mut app, 160, 36);
        assert!(app.markdown.layout.is_none());
        assert!(app.markdown.syntax.is_none());
        assert!(app.session.markdown_projection().is_none());
        app.comparison_command(lenz::commands::Command::SetPresentation(
            PresentationMode::Rendered,
        ));
        ready(&mut app);
        draw(&mut app, 160, 36);
        ready(&mut app);
        let ptr = std::ptr::from_ref(&**app.markdown.layout.as_ref().unwrap());
        let syntax = Arc::downgrade(&app.markdown.syntax.as_ref().unwrap()._token);
        let pair = app.session.markdown_projection().unwrap().pair.clone();
        assert!(!app.markdown.syntax.as_ref().unwrap().parsed[1].is_empty());
        for _ in 0..20 {
            app.markdown_scroll(1, Side::Right);
            draw(&mut app, 160, 36);
            assert_eq!(
                ptr,
                std::ptr::from_ref(&**app.markdown.layout.as_ref().unwrap())
            );
        }
        app.session.execute(lenz::commands::Command::ToggleCase);
        ready(&mut app);
        draw(&mut app, 160, 36);
        assert!(Arc::ptr_eq(
            &pair,
            &app.session.markdown_projection().unwrap().pair
        ));
        assert!(syntax.upgrade().is_some());
        for (w, h) in [(1, 1), (32, 12), (160, 36), (8, 3), (160, 36)] {
            draw(&mut app, w, h);
            if app.diff_area.height >= 4 {
                assert_eq!(
                    app.markdown.layout.as_ref().unwrap().widths,
                    if w < 70 {
                        [w; 2]
                    } else {
                        [w / 2, w - w / 2 - 1]
                    }
                );
            } else {
                assert_eq!(app.markdown.panes, [Rect::default(); 2]);
            }
        }
        app.comparison_command(lenz::commands::Command::SetPresentation(
            PresentationMode::Source,
        ));
        draw(&mut app, 160, 36);
        assert!(app.markdown.layout.is_none());
        assert!(app.markdown.syntax.is_none());
        assert!(syntax.upgrade().is_none());
    }
    #[test]
    fn markdown_tui_padded_groups_have_equal_native_row_heights_and_keep_hunk_order() {
        let (_dir, mut app) = pair();
        app.session
            .execute(lenz::commands::Command::SetScroll(ScrollMode::Padded));
        app.comparison_command(lenz::commands::Command::SetPresentation(
            PresentationMode::Rendered,
        ));
        ready(&mut app);
        draw(&mut app, 32, 12);
        let layout = app.markdown.layout.as_ref().unwrap();
        let mut order = Vec::new();
        for (group, shared) in layout.groups.iter().zip(&layout.projection.groups) {
            assert_eq!(group.rows[0].len(), group.rows[1].len());
            order.extend(shared.hunks.iter().copied());
        }
        let source =
            lenz::markdown::source_diff(app.session.document().unwrap(), app.session.view(), 0);
        let expected = source.hunks.iter().map(|h| h.id).collect::<Vec<_>>();
        assert_eq!(order, expected);
    }
    fn source_layout(source: &str, width: u16) -> Layout {
        let doc = lenz::document::TextDocument::new(
            &source.replace("one", "old").replace("aaa", "bbb"),
            source,
            std::path::Path::new("a.md"),
            std::path::Path::new("b.md"),
        );
        let parsed = lenz::markdown::parse_pair(
            [doc.text_arc(Side::Left), doc.text_arc(Side::Right)],
            lenz::markdown::Limits::DEFAULT,
            &|| true,
        )
        .unwrap();
        let options = lenz_engine::Options::default();
        let view = doc.view(&doc.line_diff(&options), 20.0);
        let projection = lenz::markdown::project(
            Arc::new(parsed),
            lenz::markdown::source_diff(&doc, &view, 0),
            &|| true,
        )
        .unwrap();
        Layout::new(Arc::new(projection), [width; 2], ScrollMode::Shared, false, &Default::default(), Default::default())
    }
    fn row_texts(layout: &Layout, pane: usize) -> Vec<String> {
        layout.rows[pane].iter().filter(|r| r.block().is_some() && !r.pieces.is_empty())
            .map(|r| r.pieces.iter().map(|p| &layout.projection.pair.sides[p.side as usize].text[p.text.clone()]).collect::<String>())
            .collect()
    }
    fn texts(layout: &Layout) -> String { row_texts(layout, 1).join("|") }
    #[test]
    fn markdown_tui_callouts_fold_and_unfold() {
        let layout = source_layout("> [!note]- Hidden title\n> secret body\n\nafter\n", 40);
        let shown = texts(&layout);
        assert!(shown.contains("Hidden title") && !shown.contains("secret") && shown.contains("after"), "{shown}");
        let id = layout.projection.pair.sides[1].blocks.iter().find_map(|b| b.callout).unwrap().id;
        let open = Layout::new(layout.projection.clone(), [40; 2], ScrollMode::Shared, false, &[(1, id)].into_iter().collect(), Default::default());
        assert!(texts(&open).contains("secret body"), "a toggle opens it");
        // a callout with a change inside starts open, so no change is hidden
        let changed = source_layout("> [!note]- Title\n> one body\n", 40);
        assert!(texts(&changed).contains("one body"));
        // `+` starts open; an untitled folded callout keeps its kind label
        assert!(texts(&source_layout("> [!tip]+ T\n> visible\n", 40)).contains("visible"));
        let untitled = source_layout("> [!tip]-\n> hidden\n", 40);
        assert!(!texts(&untitled).contains("hidden"));
        assert!(untitled.rows[1].iter().any(|r| r.label.as_deref().is_some_and(|(l, _)| l.contains("Tip"))));
    }
    #[test]
    fn markdown_tui_prose_wraps_between_words() {
        let layout = source_layout("alpha beta gamma delta epsilon\n", 14); // 12 cells after the change-mark margin
        assert_eq!(row_texts(&layout, 1), ["alpha beta ", "gamma delta ", "epsilon"]);
        // a word wider than the pane still breaks
        let layout = source_layout("abcdefghijklmnopqrstuvwxyz\n", 12);
        assert_eq!(row_texts(&layout, 1), ["abcdefghij", "klmnopqrst", "uvwxyz"]);
        // Inline: changed words flow, and rows still end between words
        let layout = source_layout("one two three four five six seven eight\n", 14);
        let inline = Layout::new(layout.projection.clone(), [14; 2], ScrollMode::Shared, true, &Default::default(), Default::default());
        let rows = row_texts(&inline, 1);
        assert!(rows.len() > 1, "{rows:?}");
        for row in &rows[..rows.len() - 1] { assert!(row.ends_with(' '), "a row ends between words: {rows:?}"); }
        for row in &rows { assert!(cell_width(row) <= 12, "inside the pane after the change mark: {rows:?}"); }
    }
    #[test]
    fn markdown_tui_layout_affinity_keeps_exact_byte_and_rejects_stale_geometry() {
        let layout = source_layout(&"one two three words ".repeat(20), 16);
        let row = layout.text_rows[1][0];
        let end = layout.rows[1][row].pieces.last().unwrap().text.end;
        let selection = RenderSelection {
            side: Side::Right,
            anchor: 0,
            head: end,
        };
        assert_eq!(
            layout.motion(Side::Right, 0, KeyCode::End, 5, row),
            Some((end, row))
        );
        assert_ne!(layout.text_row(Side::Right, end), Some(row));
        assert_eq!(
            layout.caret_row(selection, Some((selection, row))),
            Some(row)
        );
        assert_eq!(
            layout.motion(Side::Right, end, KeyCode::End, 5, row),
            Some((end, row))
        );
        assert_eq!(
            layout.motion(Side::Right, end, KeyCode::Home, 5, row),
            Some((0, row))
        );
        assert_eq!(
            layout.projection.copy_visible(selection).unwrap(),
            layout.projection.pair.sides[1].text[..end]
        );
        assert_eq!(
            layout.caret_row(selection, Some((selection, usize::MAX))),
            layout.text_row(Side::Right, end)
        );
        let moved = RenderSelection {
            anchor: 0,
            head: end + 1,
            ..selection
        };
        assert_eq!(
            layout.caret_row(moved, Some((selection, row))),
            layout.text_row(Side::Right, end + 1)
        );
    }
    #[test]
    fn markdown_tui_layout_quoted_code_measures_prefix_and_maps_interior_anchor() {
        let src = format!(
            "> ```text\n> {}UNIQUE_END\n> next_line\n> ```\n",
            "a".repeat(100)
        );
        let layout = source_layout(&src, 79);
        let row = layout.text_rows[1][0];
        let piece = layout.rows[1][row].pieces.first().unwrap();
        assert_eq!(piece.column, 4, "the quote prefix after the change-mark margin");
        assert_eq!(
            layout.rows[1][row].width,
            piece.column + width(&layout.projection.pair.sides[1].text[piece.text.clone()])
        );
        assert_eq!(layout.widest[1], 114);
        let mut adapter = Adapter {
            layout: Some(Box::new(layout)),
            ..Adapter::default()
        };
        adapter.top[1] = adapter.layout.as_ref().unwrap().text_rows[1][1];
        let anchor = adapter.anchor(Side::Right).unwrap();
        assert_eq!(anchor.byte, src.find("next_line").unwrap());
        assert_eq!(adapter.horizontal_max(Side::Right), 35);
    }
}
