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
    debug_assert!(text.len() <= WINDOW_WORK + 257);
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

#[derive(Clone)]
struct Hidden { source: Range<usize>, display: usize }

/// The same bounded text and pixel origin are used for painting and pointer conversion.
#[derive(Clone, Default)]
pub(crate) struct Row {
    pub text: String,
    pub origin: f32,
    pub fragments: Vec<(Range<usize>, f32)>,
    display: Range<usize>,
    windows: Vec<Arc<Projected>>,
    hidden: Vec<Hidden>,
    end: Option<usize>,
}

impl Row {
    pub fn complete(&self) -> bool { self.end.is_some() }

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
        let display = self.windows.iter().find_map(|window| Some(window.display.start + window.caret(source)?))
            .or_else(|| self.hidden.iter().find(|h| h.source.start <= source && source <= h.source.end).map(|h| h.display))?;
        (self.display.start <= display && display <= self.display.end).then(|| display - self.display.start)
    }

    pub fn hints(&self) -> Vec<Range<usize>> {
        self.windows.iter().flat_map(|window| window.shown.hints.iter().filter_map(|(_, range, _)| {
            let start = (window.display.start + range.start).max(self.display.start);
            let end = (window.display.start + range.end).min(self.display.end);
            (start < end).then(|| start - self.display.start..end - self.display.start)
        })).collect()
    }

    pub fn source_at(&self, display: usize) -> Option<usize> {
        let at = self.display.start + display.min(self.text.len());
        if at == self.display.end && let Some(end) = self.end { return Some(end); }
        let window = self.windows.iter().rev().find(|window| window.display.start <= at && at <= window.display.end)?;
        Some(window.source_at(at - window.display.start))
    }

    pub fn hit(&self, window: &Window, metrics: Metrics<'_>, x: f32) -> Option<usize> {
        if self.text.is_empty() { return self.end; }
        let (range, origin) = self.fragments.iter().rev().find(|(_, origin)| *origin <= x)
            .or_else(|| self.fragments.first())?;
        let at = shape(window, metrics, self.text.get(range.clone())?).closest_index_for_x(px((x - origin).max(0.0)));
        self.source_at(range.start + at)
    }

    pub fn x_for(&self, window: &Window, metrics: Metrics<'_>, at: usize) -> Option<f32> {
        if self.text.is_empty() && at == 0 { return Some(self.origin); }
        let (range, origin) = self.fragments.iter().rev().find(|(range, _)| range.start <= at && at <= range.end)?;
        Some(*origin + f32::from(shape(window, metrics, self.text.get(range.clone())?).x_for_index(at - range.start)))
    }

}

#[derive(Clone, PartialEq)]
struct Target { skip: f32, width: f32, display: Option<Range<usize>>, source_end: Option<usize> }

struct Line {
    tab: TabId,
    target: Target,
    start: Cursor,
    started: bool,
    position: Position,
    checkpoints: VecDeque<Position>,
    pieces: Vec<Piece>,
    hidden: Vec<Hidden>,
    row: Row,
    extent: Option<f32>,
    end: Option<Cursor>,
    ready: bool,
    failed: bool,
    touched: bool,
}

impl Line {
    fn new(view: &View<'_>, line: usize, target: Target, start: Option<Cursor>) -> Option<Self> {
        let start = start.or_else(|| view.display_start(line))?;
        if start.line() != line || !start.matches(view.snapshot(), view.inlay_hints())
            || target.display.as_ref().is_some_and(|range| start.display_byte() > range.start) { return None; }
        Some(Self { tab: view.tab().id(), target, start: start.clone(), started: false,
            position: Position { cursor: start, x: 0.0, carry: String::new(), carry_display: 0, carry_windows: Vec::new() },
            checkpoints: VecDeque::new(), pieces: Vec::new(), hidden: Vec::new(), row: Row::default(), extent: None, end: None, ready: false, failed: false, touched: true })
    }

    fn retarget(&mut self, target: Target) {
        if self.target == target { return; }
        if self.target.display == target.display && self.target.skip == target.skip && self.target.width == target.width {
            self.target = target;
            if self.ready { self.finish_row(); }
            return;
        }
        if let Some((old, _)) = self.target.display.as_ref().zip(target.display.as_ref())
            .filter(|(old, new)| old.start == new.start && old.end < new.end && self.target.width == target.width) {
            if self.position.x >= target.width {
                self.target = target;
                self.extent = None; self.end = None;
                self.finish_row();
                return;
            }
            if let Some(position) = self.checkpoints.iter().rev().find(|p| p.cursor.display_byte() <= old.end).cloned() {
                let committed = if position.carry.is_empty() { position.cursor.display_byte() } else { position.carry_display };
                self.pieces.retain(|piece| piece.display.end <= committed);
                self.position = position;
                self.target = target;
                self.started = false; self.ready = false; self.failed = false; self.extent = None; self.end = None;
                self.row = Row::default();
                return;
            }
        }
        let best = self.checkpoints.iter().chain(std::iter::once(&self.position))
            .filter(|p| target.skip > 0.0 && p.x <= target.skip).max_by(|a,b| a.x.total_cmp(&b.x)).cloned();
        self.position = best.unwrap_or_else(|| Position { cursor: self.start.clone(), x: 0.0, carry: String::new(), carry_display: 0, carry_windows: Vec::new() });
        self.target = target;
        self.pieces.clear(); self.hidden.clear(); self.row = Row::default();
        self.ready = false; self.failed = false; self.started = false;
        self.extent = None; self.end = None;
    }

    fn consume(&mut self, projected: &Projected, window: &Window, metrics: Metrics<'_>) {
        self.started = true;
        self.checkpoints.push_back(self.position.clone());
        if self.checkpoints.len() > CHECKPOINTS { self.checkpoints.pop_front(); }
        let from = self.target.display.as_ref().map_or(0, |range| range.start.saturating_sub(projected.display.start).min(projected.shown.text.len()));
        let to = self.target.display.as_ref().map_or(projected.shown.text.len(), |range| range.end.saturating_sub(projected.display.start).min(projected.shown.text.len()));
        let Some(body) = projected.shown.text.get(from..to) else { self.failed = true; return };
        let complete = projected.complete || self.target.display.as_ref().is_some_and(|range| range.end <= projected.display.end);
        let start = if self.position.carry.is_empty() { projected.display.start + from } else { self.position.carry_display };
        let mut text = std::mem::take(&mut self.position.carry);
        text.push_str(body);
        let mut windows = std::mem::take(&mut self.position.carry_windows);
        if !body.is_empty() { windows.push(Arc::new(projected.clone())); }
        if projected.shown.text.is_empty() && !projected.source.is_empty() {
            if let Some(last) = self.hidden.last_mut()
                && last.display == projected.display.start && last.source.end == projected.source.start {
                last.source.end = projected.source.end;
            } else { self.hidden.push(Hidden { source: projected.source.clone(), display: projected.display.start }); }
        }
        let shaped = shape(window, metrics, &text);
        let commit = if complete { text.len() } else { text.grapheme_indices(true).next_back().map_or(0, |(at,_)| at) };
        let right = self.position.x + f32::from(shaped.x_for_index(commit));
        if commit > 0 && right >= self.target.skip {
            self.pieces.push(Piece { display: start..start + commit, origin: self.position.x, text: Arc::new(text.clone()), windows: windows.clone() });
        }
        self.position.x = right;
        self.position.carry = text.get(commit..).unwrap_or_default().to_owned();
        self.position.carry_display = start + commit;
        self.position.carry_windows = windows.into_iter().filter(|w| w.display.end > start + commit).collect();
        self.position.cursor = projected.next().clone();
        if complete {
            self.extent = Some(right);
            self.end = Some(self.position.cursor.clone());
        }
        self.ready = complete || right >= self.target.skip + self.target.width;
        if self.ready { self.finish_row(); }
    }

    fn source_end(&self) -> Option<usize> {
        self.end.as_ref()?;
        if self.target.display.is_some() { self.target.source_end } else { self.end.as_ref().map(Cursor::source_byte) }
    }

    fn finish_row(&mut self) {
        let Some(first) = self.pieces.first() else {
            self.row = Row { end: self.source_end(), ..Default::default() };
            return;
        };
        let (start, origin) = (first.display.start, first.origin);
        let mut text = String::new();
        let mut end = start;
        let mut windows = Vec::new();
        let mut fragments = Vec::new();
        for piece in &self.pieces {
            let from = end.max(piece.display.start);
            if let Some(part) = piece.text.get(from - piece.display.start..piece.display.end - piece.display.start) {
                let local = text.len();
                text.push_str(part);
                fragments.push((local..text.len(), piece.origin));
                end = piece.display.end;
            }
            for window in &piece.windows {
                if !windows.iter().any(|old: &Arc<Projected>| old.display == window.display && old.source == window.source) { windows.push(window.clone()); }
            }
        }
        self.row = Row { text, origin, fragments, display: start..end, windows, hidden: self.hidden.iter().filter(|h| start <= h.display && h.display <= end).cloned().collect(), end: self.source_end() };
    }
}

#[derive(Default)]
struct Pane {
    lines: VecDeque<Line>,
    pending: Option<Request>,
    pending_display: Option<usize>,
    serial: u64,
    work: usize,
    seen: bool,
    active: bool,
    metric: Option<(gpui::Font, u32)>,
}

#[derive(Default)]
pub(crate) struct Cache { panes: [Pane;2], commands: Vec<Command> }

impl Cache {
    #[cfg(all(test, feature = "test-support"))]
    pub fn retained_windows(&self) -> usize {
        self.panes.iter().flat_map(|pane| &pane.lines).map(|line| {
            line.position.carry_windows.len() + line.checkpoints.iter().map(|p| p.carry_windows.len()).sum::<usize>()
                + line.pieces.iter().map(|p| p.windows.len()).sum::<usize>() + line.row.windows.len()
        }).sum()
    }

    pub fn begin(&mut self, metrics: Metrics<'_>) {
        for pane in &mut self.panes {
            let metric = (metrics.font.clone(), metrics.font_size.to_bits());
            if pane.metric.as_ref() != Some(&metric) { pane.lines.clear(); pane.metric = Some(metric); }
            pane.work = FRAME_WORK;
            pane.seen = false;
            pane.active = false;
            for line in &mut pane.lines { line.touched = false; }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn row(&mut self, pane: u8, view: &View<'_>, line: usize, skip: f32, width: f32, batch: Option<&Batch>, window: &Window, metrics: Metrics<'_>) -> Row {
        self.project(pane, view, line, Target { skip, width, display: None, source_end: None }, None, batch, window, metrics)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn wrapped(&mut self, pane: u8, view: &View<'_>, cursor: Cursor, display: Range<usize>, source_end: Option<usize>, width: f32, batch: Option<&Batch>, window: &Window, metrics: Metrics<'_>) -> Row {
        self.project(pane, view, cursor.line(), Target { skip: 0.0, width, display: Some(display), source_end }, Some(cursor), batch, window, metrics)
    }

    #[allow(clippy::too_many_arguments)]
    fn project(&mut self, pane: u8, view: &View<'_>, line: usize, target: Target, start: Option<Cursor>, batch: Option<&Batch>, window: &Window, metrics: Metrics<'_>) -> Row {
        let display_start = target.display.as_ref().map(|range| range.start);
        let Some(slot) = self.panes.get_mut(pane as usize) else { return Row::default() };
        slot.active = true;
        slot.lines.retain(|line| line.tab == view.tab().id() && line.position.cursor.matches(view.snapshot(), view.inlay_hints()));
        if slot.pending.as_ref().is_some_and(|request| request.tab != view.tab().id() || !request.cursor.matches(view.snapshot(), view.inlay_hints()))
            && let Some(old) = slot.pending.take() { self.commands.push(Command::CancelProjection { pane, request: old.request }); }
        let at = slot.lines.iter().position(|l| l.tab == view.tab().id() && l.position.cursor.line() == line && l.target.display.as_ref().map(|range| range.start) == display_start && l.position.cursor.matches(view.snapshot(), view.inlay_hints()));
        let Some(mut entry) = at.and_then(|at| slot.lines.remove(at)).or_else(|| Line::new(view,line,target.clone(),start)) else { return Row::default() };
        entry.retarget(target);
        entry.touched = true;
        if slot.pending.as_ref().is_some_and(|r| r.tab == entry.tab && r.cursor.line() == line && slot.pending_display == display_start) {
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
            if !entry.started && slot.work >= WINDOW_WORK
                && let Some(part) = view.display_window(&mut entry.position.cursor.clone(),WINDOW_WORK) {
                slot.work = slot.work.saturating_sub(part.work);
                entry.consume(&part,window,metrics);
            }
            if !entry.ready && slot.pending.is_none() {
                slot.serial = slot.serial.wrapping_add(1).max(1);
                let request = Request { pane, tab: entry.tab, cursor: entry.position.cursor.clone(), request: slot.serial, context: slot.serial };
                self.commands.push(Command::PrepareProjection(request.clone()));
                slot.pending = Some(request);
                slot.pending_display = display_start;
                slot.seen = true;
            }
        }
        let row = entry.row.clone();
        slot.lines.push_front(entry);
        row
    }

    pub fn get(&self, pane: u8, view: &View<'_>, line: usize, skip: f32, width: f32) -> Option<&Row> {
        Some(&self.panes.get(pane as usize)?.lines.iter().find(|l| l.tab == view.tab().id() && l.position.cursor.line() == line && l.position.cursor.matches(view.snapshot(), view.inlay_hints()) && l.target.display.is_none() && l.target.skip == skip && l.target.width == width && l.ready)?.row)
    }

    pub fn get_wrapped(&self, pane: u8, view: &View<'_>, line: usize, display_start: usize, width: f32) -> Option<&Row> {
        Some(&self.panes.get(pane as usize)?.lines.iter().find(|entry| entry.tab == view.tab().id() && entry.position.cursor.line() == line
            && entry.target.display.as_ref().is_some_and(|range| range.start == display_start) && entry.target.width == width
            && entry.position.cursor.matches(view.snapshot(), view.inlay_hints()) && entry.ready)?.row)
    }

    pub fn cursor(&self, pane: u8, view: &View<'_>, line: usize, display_start: usize) -> Option<Cursor> {
        self.panes.get(pane as usize)?.lines.iter().find(|entry| entry.tab == view.tab().id() && entry.start.line() == line
            && entry.target.display.as_ref().is_some_and(|range| range.start == display_start) && entry.start.matches(view.snapshot(), view.inlay_hints())).map(|entry| entry.start.clone())
    }

    pub fn extent(&self, pane: u8, view: &View<'_>, line: usize) -> Option<f32> {
        self.panes.get(pane as usize)?.lines.iter().find(|l| l.tab == view.tab().id() && l.position.cursor.line() == line && l.target.display.is_none() && l.position.cursor.matches(view.snapshot(), view.inlay_hints()))?.extent
    }

    pub fn finish(&mut self) -> Vec<Command> {
        for (pane,slot) in self.panes.iter_mut().enumerate() {
            if !slot.active { slot.lines.clear(); }
            let mut offscreen = 0;
            slot.lines.retain(|line| { if line.touched { true } else { offscreen += 1; offscreen <= OFFSCREEN_LINES } });
            if !slot.seen && let Some(old) = slot.pending.take() { self.commands.push(Command::CancelProjection { pane: pane as u8, request: old.request }); }
        }
        std::mem::take(&mut self.commands)
    }
}
