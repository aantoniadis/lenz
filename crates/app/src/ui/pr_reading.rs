//! Native Markdown body layout inside the shared PR timeline.
use super::*;
use crate::pr_conversation::Page;
use crate::rich::{self, Fill, Row};
use lenz_content::markdown::BlockKind;
use lenz_gpui::markdown::{ReadingStyle, indent, shape};
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

pub(in crate::ui) struct NativeRow { pub line: ShapedLine, pub x: f32, pub height: f32, pub task: Option<(f32, bool)> }
pub(in crate::ui) struct Layout {
    pub page: Arc<Page>,
    pub rows: Vec<Option<NativeRow>>,
    pub tops: Vec<f32>,
    pub style: ReadingStyle,
}
impl Layout {
    pub fn new(source: &Page, style: ReadingStyle, p: &theme::Palette, char_w: f32, text: &gpui::WindowTextSystem, current: &dyn Fn() -> bool) -> Option<Self> {
        let mut page = Page { side: source.side.clone(), side_at: source.side_at, ..Page::default() };
        let mut rows = Vec::new();
        let mut tops = Vec::new();
        let mut y = 0.;
        let mut mapping = vec![0; source.rows.len() + 1];
        let mut regions = source.markdown.iter().peekable();
        let mut old = 0;
        while old < source.rows.len() {
            if !current() { return None; }
            mapping[old] = page.rows.len();
            if regions.peek().is_some_and(|r| r.rows.start == old) {
                let region = regions.next()?;
                let doc = &region.document;
                let left = region.columns.start as f32 * char_w;
                let right = (region.columns.end as f32 * char_w).min(left + style.column * 1.3);
                let mut previous = None;
                for block in &doc.blocks {
                    if !current() { return None; }
                    y += style.space_before(&block.kind, previous);
                    previous = Some(&block.kind);
                    let x = indent(&style, left, block);
                    let fs = match block.kind { BlockKind::Heading(n) => style.heading[n.saturating_sub(1).min(5) as usize], _ => style.size };
                    let height = (fs * 1.6).max(style.line);
                    let fill = match block.kind { BlockKind::Rule => Fill::Rule, BlockKind::Code { .. } | BlockKind::Literal | BlockKind::Properties => Fill::Code, _ if block.quote_depth > 0 => Fill::Quote, _ => Fill::None };
                    // Keep cell separators and link columns aligned in the timeline's compact table rows.
                    if matches!(block.kind, BlockKind::TableRow { .. }) {
                        let before = page.rows.len();
                        let mut spans = rich::markdown_spans(doc, block, block.text.clone());
                        for span in &mut spans { span.text = span.text.replace('\t', " │ "); }
                        rich::wrap(&[rich::Block { spans, indent: (x / char_w.max(1.)).floor() as usize, ..Default::default() }], region.columns.end, 0, &mut page.rows);
                        for _ in before..page.rows.len() { tops.push(y); rows.push(None); y += line_h(); }
                        continue;
                    }
                    let mut at = block.text.start;
                    for line in doc.text.get(block.text.clone())?.trim_end_matches('\n').split('\n') {
                        let end = at + line.len();
                        let shaped = shape(doc, block, at..end, fs, &style, p, text, &[], None);
                        let available = (right - x).max(1.);
                        let marker = if let BlockKind::ListItem { marker, .. } = &block.kind { if at == block.text.start { marker.len() + 1 } else { 0 } } else { 0 };
                        let hang = if marker > 0 { f32::from(shaped.x_for_index(marker)) } else { 0. };
                        let ranges = wrapped(line, &shaped, available, hang);
                        for range in ranges {
                            let range = at + range.start..at + range.end;
                            let shaped = if range.start == at && range.end == end { shaped.clone() } else { shape(doc, block, range.clone(), fs, &style, p, text, &[], None) };
                            let task = if let BlockKind::ListItem { checked: Some(done), .. } = block.kind {
                                (range.start == block.text.start).then(|| (f32::from(shaped.x_for_index(marker)), done))
                            } else { None };
                            let row_x = x + if range.start > at { hang } else { 0. };
                            let spans = rich::markdown_spans(doc, block, range);
                            tops.push(y);
                            page.rows.push(Row { spans, indent: (row_x / char_w.max(1.)).floor() as usize, fill });
                            rows.push(Some(NativeRow { line: shaped, x: row_x, height, task }));
                            y += height;
                        }
                        at = end + 1;
                    }
                }
                old = region.rows.end;
                mapping[old] = page.rows.len();
            } else {
                tops.push(y);
                page.rows.push(source.rows[old].clone());
                rows.push(None);
                y += line_h();
                old += 1;
            }
        }
        mapping[old] = page.rows.len();
        while page.rows.len() < page.side.len() {
            tops.push(y);
            page.rows.push(Row::default());
            rows.push(None);
            y += line_h();
        }
        tops.push(y);
        page.boxes = source.boxes.iter().map(|b| crate::pr_conversation::CardBox { top: mapping[b.top], bottom: mapping[b.bottom], ..b.clone() }).collect();
        Some(Self { page: Arc::new(page), rows, tops, style })
    }
    pub fn top(&self, row: usize) -> f32 { self.tops.get(row).copied().unwrap_or_else(|| *self.tops.last().unwrap_or(&0.)) }
    pub fn height(&self, row: usize) -> f32 { self.rows.get(row).and_then(Option::as_ref).map_or(line_h(), |r| r.height) }
    pub fn row_at(&self, y: f32) -> usize { self.tops.partition_point(|top| *top <= y).saturating_sub(1).min(self.rows.len().saturating_sub(1)) }
    pub fn max_scroll(&self, height: f32) -> usize {
        let end = self.top(self.rows.len()).max(self.page.side.len() as f32 * line_h());
        if end <= height { 0 } else { self.tops.partition_point(|top| *top < end - height).min(self.rows.len().saturating_sub(1)) }
    }
    pub fn column_at(&self, row: usize, x: f32, nearest: bool) -> Option<usize> {
        let native = self.rows.get(row)?.as_ref()?;
        let r = self.page.rows.get(row)?;
        if !nearest && x < native.x { return None; }
        let x = px((x - native.x).max(0.));
        let byte = if nearest { native.line.closest_index_for_x(x) } else { native.line.index_for_x(x).unwrap_or(native.line.text.len()) };
        Some(r.indent + rich::width(native.line.text.get(..byte)?))
    }
    pub fn x_for_column(&self, row: usize, column: usize) -> Option<f32> {
        let native = self.rows.get(row)?.as_ref()?;
        let r = self.page.rows.get(row)?;
        let mut col = r.indent;
        let byte = native.line.text.char_indices().find_map(|(byte, ch)| {
            if col >= column { Some(byte) } else { col += rich::width(ch.encode_utf8(&mut [0; 4])); None }
        }).unwrap_or(native.line.text.len());
        Some(native.x + f32::from(native.line.x_for_index(byte)))
    }
}
fn wrapped(line: &str, shaped: &ShapedLine, available: f32, hang: f32) -> Vec<Range<usize>> {
    if f32::from(shaped.width) <= available || line.is_empty() { return std::iter::once(0..line.len()).collect(); }
    let lookup = super::super::markdown::WrapLookup::new(shaped);
    let mut out = Vec::new();
    let mut start = 0;
    while start < line.len() {
        let available = (available - if start > 0 { hang } else { 0. }).max(1.);
        let x = f32::from(lookup.x_for_index(start));
        let mut stop = lookup.closest_index_for_x(px(x + available)).max(start);
        while stop > start && f32::from(lookup.x_for_index(stop)) - x > available {
            stop = line.get(..stop).unwrap_or_default().grapheme_indices(true).next_back().map_or(start, |(i, _)| i);
        }
        if stop <= start { stop = start + line.get(start..).unwrap_or_default().graphemes(true).next().map_or(0, str::len); }
        if stop < line.len() && let Some(space) = line.get(start..stop).unwrap_or_default().rfind(' ') && space > 0 { stop = start + space + 1; }
        out.push(start..stop);
        start = stop;
    }
    out
}
