//! Bounded native shaping over the core's projected stream.
use std::{collections::VecDeque, ops::Range, sync::Arc};
use gpui::{Hsla, SharedString, ShapedLine, TextRun, Window, px};
use lenz_gpui::Metrics;
use reader_core::{projection::{Batch, Cursor, Request, Window as Projected, WINDOW_WORK}, session::{Command, View}, tabs::TabId};
use unicode_segmentation::UnicodeSegmentation;

const FRAME_WORK: usize = 64 * 1024;
const CHECKPOINTS: usize = 8;
const OFFSCREEN_LINES: usize = 16;

fn shape(window: &Window, metrics: Metrics<'_>, text: &str) -> ShapedLine {
    let run = TextRun { len: text.len(), font: metrics.font.clone(), color: Hsla::default(), background_color: None, underline: None, strikethrough: None };
    window.text_system().shape_line(SharedString::from(text.to_owned()), px(metrics.font_size), &[run], None)
}

#[derive(Clone)]
struct Position {
    cursor: Cursor,
    x: f32,
    carry: String,
    carry_display: usize,
    carry_windows: Vec<Arc<Projected>>,
}

#[derive(Clone)]
struct Piece {
    display: Range<usize>,
    origin: f32,
    text: Arc<String>,
    windows: Vec<Arc<Projected>>,
}

/// The same bounded text and pixel origin are used for painting and pointer conversion.
#[derive(Clone, Default)]
pub(crate) struct Row {
    pub text: String,
    pub origin: f32,
    pub extent: Option<f32>,
    display: Range<usize>,
    windows: Vec<Arc<Projected>>,
    end: Option<usize>,
}

impl Row {
    pub fn range(&self, source: Range<usize>) -> Range<usize> {
        let mut out: Option<Range<usize>> = None;
        for window in &self.windows {
            let local = window.range(source.clone());
            let global = window.display.start + local.start..window.display.start + local.end;
            let range = global.start.max(self.display.start)..global.end.min(self.display.end);
            if range.start < range.end {
                out = Some(out.map_or(range.clone(), |old| old.start.min(range.start)..old.end.max(range.end)));
            }
        }
        out.map_or(0..0, |r| r.start - self.display.start..r.end - self.display.start)
    }

    pub fn caret(&self, source: usize) -> Option<usize> {
        self.windows.iter().find_map(|window| {
            let at = window.display.start + window.caret(source)?;
            self.display.contains(&at).then_some(at - self.display.start)
                .or_else(|| (at == self.display.end).then_some(at - self.display.start))
        })
    }

    pub fn hints(&self) -> Vec<Range<usize>> {
        self.windows.iter().flat_map(|window| window.shown.hints.iter().filter_map(|(_, range, _)| {
            let start = (window.display.start + range.start).max(self.display.start);
            let end = (window.display.start + range.end).min(self.display.end);
            (start < end).then_some(start - self.display.start..end - self.display.start)
        })).collect()
    }

    pub fn source_at(&self, display: usize) -> Option<usize> {
        let at = self.display.start + display.min(self.text.len());
        if at == self.display.end && let Some(end) = self.end { return Some(end); }
        let window = self.windows.iter().rev().find(|window| window.display.start <= at && at <= window.display.end)?;
        Some(window.source_at(at - window.display.start))
    }

    pub fn hit(&self, window: &Window, metrics: Metrics<'_>, x: f32) -> Option<usize> {
        let at = shape(window, metrics, &self.text).closest_index_for_x(px((x - self.origin).max(0.0)));
        self.source_at(at)
    }
}

struct Line {
    tab: TabId,
    skip: f32,
    width: f32,
    position: Position,
    checkpoints: VecDeque<Position>,
    pieces: Vec<Piece>,
    row: Row,
    extent: Option<f32>,
    end: Option<Cursor>,
    ready: bool,
    failed: bool,
    touched: bool,
}

impl Line {
    fn new(view: &View<'_>, line: usize, skip: f32, width: f32) -> Option<Self> {
        Some(Self { tab: view.tab().id(), skip, width,
            position: Position { cursor: view.display_start(line)?, x: 0.0, carry: String::new(), carry_display: 0, carry_windows: Vec::new() },
            checkpoints: VecDeque::new(), pieces: Vec::new(), row: Row::default(), extent: None, end: None, ready: false, failed: false, touched: true })
    }

    fn retarget(&mut self, view: &View<'_>, skip: f32, width: f32) {
        if self.skip == skip && self.width == width { return; }
        let best = self.checkpoints.iter().chain(std::iter::once(&self.position))
            .filter(|p| p.x <= skip).max_by(|a,b| a.x.total_cmp(&b.x)).cloned();
        let Some(position) = best.or_else(|| Some(Position { cursor: view.display_start(self.position.cursor.line())?, x: 0.0, carry: String::new(), carry_display: 0, carry_windows: Vec::new() })) else { self.failed = true; return };
        self.position = position;
        self.skip = skip;
        self.width = width;
        self.pieces.clear();
        self.row = Row::default();
        self.ready = false;
        self.failed = false;
    }

    fn consume(&mut self, projected: &Projected, window: &Window, metrics: Metrics<'_>) {
        self.checkpoints.push_back(self.position.clone());
        if self.checkpoints.len() > CHECKPOINTS { self.checkpoints.pop_front(); }
        let start = if self.position.carry.is_empty() { projected.display.start } else { self.position.carry_display };
        let mut text = std::mem::take(&mut self.position.carry);
        text.push_str(&projected.shown.text);
        let mut windows = std::mem::take(&mut self.position.carry_windows);
        windows.push(Arc::new(projected.clone()));
        let shaped = shape(window, metrics, &text);
        let commit = if projected.complete { text.len() } else { text.grapheme_indices(true).next_back().map_or(0, |(at,_)| at) };
        let right = self.position.x + f32::from(shaped.x_for_index(commit));
        if !text.is_empty() && right >= self.skip {
            self.pieces.push(Piece { display: start..start + commit, origin: self.position.x, text: Arc::new(text.clone()), windows: windows.clone() });
        }
        self.position.x = right;
        self.position.carry = text.get(commit..).unwrap_or_default().to_owned();
        self.position.carry_display = start + commit;
        self.position.carry_windows = windows.into_iter().filter(|w| w.display.end > start + commit).collect();
        self.position.cursor = projected.next().clone();
        if projected.complete {
            self.extent = Some(right);
            self.end = Some(self.position.cursor.clone());
        }
        self.ready = projected.complete || right >= self.skip + self.width;
        if self.ready { self.finish_row(); }
    }

    fn finish_row(&mut self) {
        let Some(first) = self.pieces.first() else {
            self.row = Row { extent: self.extent, end: self.end.as_ref().map(Cursor::source_byte), ..Default::default() };
            return;
        };
        let (start, origin) = (first.display.start, first.origin);
        let mut text = String::new();
        let mut end = start;
        let mut windows = Vec::new();
        for piece in &self.pieces {
            let from = end.max(piece.display.start);
            if let Some(part) = piece.text.get(from - piece.display.start..piece.display.end - piece.display.start) {
                text.push_str(part);
                end = piece.display.end;
            }
            for window in &piece.windows {
                if !windows.iter().any(|old: &Arc<Projected>| old.display == window.display) { windows.push(window.clone()); }
            }
        }
        self.row = Row { text, origin, extent: self.extent, display: start..end, windows, end: self.end.as_ref().map(Cursor::source_byte) };
    }
}

#[derive(Default)]
struct Pane {
    lines: VecDeque<Line>,
    pending: Option<Request>,
    serial: u64,
    work: usize,
    seen: bool,
    metric: Option<(gpui::Font, u32)>,
}

#[derive(Default)]
pub(crate) struct Cache { panes: [Pane;2], commands: Vec<Command> }

impl Cache {
    pub fn begin(&mut self, metrics: Metrics<'_>) {
        for pane in &mut self.panes {
            let metric = (metrics.font.clone(), metrics.font_size.to_bits());
            if pane.metric.as_ref() != Some(&metric) { pane.lines.clear(); pane.metric = Some(metric); }
            pane.work = FRAME_WORK;
            pane.seen = false;
            for line in &mut pane.lines { line.touched = false; }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn row(&mut self, pane: u8, view: &View<'_>, line: usize, skip: f32, width: f32, batch: Option<&Batch>, window: &Window, metrics: Metrics<'_>) -> Row {
        let Some(slot) = self.panes.get_mut(pane as usize) else { return Row::default() };
        let at = slot.lines.iter().position(|l| l.tab == view.tab().id() && l.position.cursor.line() == line && l.position.cursor.matches(view.snapshot(), view.inlay_hints()));
        let Some(mut entry) = at.and_then(|at| slot.lines.remove(at)).or_else(|| Line::new(view,line,skip,width)) else { return Row::default() };
        entry.retarget(view,skip,width);
        entry.touched = true;
        if slot.pending.as_ref().is_some_and(|r| r.tab == entry.tab && r.cursor.line() == line) {
            slot.seen = true;
            if let Some(batch) = batch.filter(|b| slot.pending.as_ref() == Some(&b.request)) {
                if batch.request.cursor == entry.position.cursor {
                    entry.failed = batch.failed;
                    for part in &batch.windows {
                        entry.consume(part,window,metrics);
                        slot.work = slot.work.saturating_sub(part.work);
                        if entry.ready { break; }
                    }
                }
                if let Some(old) = slot.pending.take() { self.commands.push(Command::CancelProjection { pane, request: old.request }); }
            } else if slot.pending.as_ref().is_some_and(|r| r.cursor != entry.position.cursor)
                && let Some(old) = slot.pending.take() { self.commands.push(Command::CancelProjection { pane, request: old.request }); }
        }
        if !entry.ready && !entry.failed {
            if entry.position.cursor.display_byte() == 0 && slot.work >= WINDOW_WORK
                && let Some(part) = view.display_window(&mut entry.position.cursor.clone(),WINDOW_WORK) {
                slot.work = slot.work.saturating_sub(part.work);
                entry.consume(&part,window,metrics);
            }
            if !entry.ready && slot.pending.is_none() {
                slot.serial = slot.serial.wrapping_add(1).max(1);
                let request = Request { pane, tab: entry.tab, cursor: entry.position.cursor.clone(), request: slot.serial, context: slot.serial };
                self.commands.push(Command::PrepareProjection(request.clone()));
                slot.pending = Some(request);
                slot.seen = true;
            }
        }
        let row = entry.row.clone();
        slot.lines.push_front(entry);
        row
    }

    pub fn get(&self, pane: u8, view: &View<'_>, line: usize, skip: f32, width: f32) -> Option<&Row> {
        Some(&self.panes.get(pane as usize)?.lines.iter().find(|l| l.tab == view.tab().id() && l.position.cursor.line() == line && l.position.cursor.matches(view.snapshot(), view.inlay_hints()) && l.skip == skip && l.width == width && l.ready)?.row)
    }

    pub fn extent(&self, pane: u8, view: &View<'_>, line: usize) -> Option<f32> {
        self.panes.get(pane as usize)?.lines.iter().find(|l| l.tab == view.tab().id() && l.position.cursor.line() == line && l.position.cursor.matches(view.snapshot(), view.inlay_hints()))?.extent
    }

    pub fn finish(&mut self) -> Vec<Command> {
        for (pane,slot) in self.panes.iter_mut().enumerate() {
            let mut offscreen = 0;
            slot.lines.retain(|line| { if line.touched { true } else { offscreen += 1; offscreen <= OFFSCREEN_LINES } });
            if !slot.seen && let Some(old) = slot.pending.take() { self.commands.push(Command::CancelProjection { pane: pane as u8, request: old.request }); }
        }
        std::mem::take(&mut self.commands)
    }
}
