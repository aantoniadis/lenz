//! Bounded terminal measurement over shared resumable projections.
use code_core::{
    projection::{Batch, Cursor, Request, WINDOW_WORK, Window},
    session::{Command, NativeReveal, NativeViewport, State, View},
    tabs::TabId,
};
use std::{collections::VecDeque, ops::Range, sync::Arc};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

const FRAME_WORK: usize = 128 * 1024;
const OFFSCREEN_LINES: usize = 32;
const CHECKPOINTS: usize = 8;

#[derive(Clone)]
pub(crate) struct Cell {
    pub column: usize,
    pub width: usize,
    pub byte: usize,
    pub display: usize,
    text: Arc<String>,
    range: Range<usize>,
    pub hint: bool,
    pub caret: bool,
}

impl Cell {
    pub fn text(&self) -> &str {
        self.text.get(self.range.clone()).unwrap_or("")
    }
}

#[derive(Clone)]
struct Position {
    cursor: Cursor,
    column: usize,
    // The final grapheme is undecidable until the next chunk (or the end of the line).
    carry: Option<Cell>,
}

#[derive(Clone, PartialEq, Eq)]
struct Target {
    skip: usize,
    width: usize,
    display: Option<Range<usize>>,
    source_end: Option<usize>,
    goal: Option<NativeReveal>,
    cols: usize,
    indent: usize,
}

struct Line {
    tab: TabId,
    target: Target,
    position: Position,
    start: Cursor,
    started: bool,
    checkpoints: VecDeque<Position>,
    cells: Arc<Vec<Cell>>,
    complete: bool,
    extent: Option<usize>,
    end: Option<Cursor>,
    ready: bool,
    failed: bool,
    touched: bool,
    goal_display: Option<usize>,
    bounds: Option<Range<usize>>,
}

impl Line {
    fn new(view: &View<'_>, line: usize, target: Target, start: Option<Cursor>) -> Option<Self> {
        let start = start.or_else(|| view.display_start(line))?;
        Some(Self {
            tab: view.tab().id(),
            target,
            position: Position {
                cursor: start.clone(),
                column: 0,
                carry: None,
            },
            start,
            started: false,
            checkpoints: VecDeque::new(),
            cells: Arc::default(),
            complete: false,
            extent: None,
            end: None,
            ready: false,
            failed: false,
            touched: true,
            goal_display: None,
            bounds: None,
        })
    }

    fn retarget(&mut self, target: Target) {
        if self.target == target {
            return;
        }
        if self.target.goal == target.goal && self.target.display == target.display
            && self.target.skip == target.skip
            && self.target.width == target.width
        {
            self.target = target;
            return;
        }
        if let Some((old, _)) = self
            .target
            .display
            .as_ref()
            .zip(target.display.as_ref())
            .filter(|(old, new)| {
                old.start == new.start && old.end < new.end && self.target.width == target.width
            })
        {
            if self.position.column >= target.width {
                self.target = target;
                return;
            }
            if let Some(position) = self
                .checkpoints
                .iter()
                .rev()
                .find(|p| p.cursor.display_byte() <= old.end)
                .cloned()
            {
                Arc::make_mut(&mut self.cells).retain(|cell| cell.column < position.column);
                self.position = position;
                self.target = target;
                self.complete = false;
                self.ready = false;
                self.started = false;
                self.failed = false;
                return;
            }
        }
        let preserve = self.ready && self.target.display == target.display
            && self.target.skip == target.skip && self.target.width == target.width;
        let new_goal = self.target.goal != target.goal;
        if new_goal { self.goal_display = None; self.bounds = None; }
        self.position = self
            .checkpoints
            .iter()
            .chain(std::iter::once(&self.position))
            .filter(|p| if let Some(goal) = &target.goal {
                p.cursor.source_byte() < goal.source
            } else { target.skip > 0 && p.column <= target.skip })
            .max_by_key(|p| p.column)
            .cloned()
            .unwrap_or_else(|| Position {
                cursor: self.start.clone(),
                column: 0,
                carry: None,
            });
        self.target = target;
        if !preserve { self.cells = Arc::default(); }
        self.complete =
            self.end.as_ref() == Some(&self.position.cursor) && self.position.carry.is_none();
        self.ready = self.complete;
        self.failed = false;
        self.started = false;
    }

    fn emit(&mut self, mut cell: Cell) {
        debug_assert!(cell.text().len() <= code_core::projection::GRAPHEME_BYTES + 1);
        debug_assert!(
            cell.display
                <= self
                    .position
                    .cursor
                    .display_byte()
                    .saturating_add(WINDOW_WORK)
        );
        cell.column = self.position.column;
        cell.width = UnicodeWidthStr::width(cell.text());
        if self.bounds.is_none() && self.goal_display.is_some_and(|display| (cell.display..cell.display + cell.text().len()).contains(&display)) {
            self.bounds = Some(cell.column..cell.column.saturating_add(cell.width.max(1)));
        }
        if cell.column >= self.target.skip
            && (cell.column.saturating_add(cell.width) <= self.target.skip.saturating_add(self.target.width)
                || (cell.column == self.target.skip && cell.width > self.target.width && self.target.width > 0))
            && !self.cells.iter().any(|old| old.display == cell.display)
        {
            Arc::make_mut(&mut self.cells).push(cell.clone());
        }
        self.position.column = self.position.column.saturating_add(cell.width);
    }

    fn consume(&mut self, window: &Window) {
        self.started = true;
        if self.goal_display.is_none() {
            self.goal_display = self.target.goal.as_ref().and_then(|goal| window.caret_display(goal.source));
        }
        self.checkpoints.push_back(self.position.clone());
        if self.checkpoints.len() > CHECKPOINTS {
            self.checkpoints.pop_front();
        }
        let from = self.target.display.as_ref().map_or(0, |range| {
            range
                .start
                .saturating_sub(window.display.start)
                .min(window.shown.text.len())
        });
        let to = self
            .target
            .display
            .as_ref()
            .map_or(window.shown.text.len(), |range| {
                range
                    .end
                    .saturating_sub(window.display.start)
                    .min(window.shown.text.len())
            });
        let Some(body) = window.shown.text.get(from..to) else {
            self.failed = true;
            return;
        };
        let complete = window.complete
            || self
                .target
                .display
                .as_ref()
                .is_some_and(|range| range.end <= window.display.end);
        let carry = self.position.carry.take();
        let prefix = carry.as_ref().map_or(0, |cell| cell.text().len());
        let mut text = carry
            .as_ref()
            .map_or_else(String::new, |cell| cell.text().to_owned());
        text.push_str(body);
        let text = Arc::new(text);
        let mut graphemes = text.grapheme_indices(true).peekable();
        while let Some((at, grapheme)) = graphemes.next() {
            let mut cell = if let Some(carry) = carry.as_ref().filter(|_| at < prefix) {
                let mut cell = carry.clone();
                cell.text = text.clone();
                cell.range = at..at + grapheme.len();
                cell
            } else {
                let local = from + at - prefix;
                let source = window.source_at(local);
                Cell {
                    column: 0,
                    width: 0,
                    byte: source,
                    display: window.display.start + local,
                    text: text.clone(),
                    range: at..at + grapheme.len(),
                    hint: window
                        .shown
                        .hints
                        .iter()
                        .any(|(_, range, _)| range.contains(&local)),
                    caret: window.caret(source) == Some(local),
                }
            };
            if graphemes.peek().is_none() && !complete {
                cell.text = Arc::new(grapheme.to_owned());
                cell.range = 0..grapheme.len();
                self.position.carry = Some(cell);
            } else {
                self.emit(cell);
            }
        }
        self.position.cursor = window.next().clone();
        self.complete = complete;
        if self.complete {
            self.extent = Some(self.position.column);
            self.end = Some(self.position.cursor.clone());
            if self.bounds.is_none() && self.goal_display == Some(self.target.display.as_ref().map_or(window.display.end, |r| r.end)) {
                self.bounds = Some(self.position.column..self.position.column.saturating_add(1));
            }
        }
        self.ready = self.complete || (self.position.column >= self.target.skip.saturating_add(self.target.width)
            && (self.target.goal.is_none() || self.bounds.is_some()));
    }
}

#[derive(Default)]
struct Pane {
    lines: VecDeque<Line>,
    pending: Option<Request>,
    pending_display: Option<usize>,
    layout: Option<(TabId, code_core::layout::Work)>,
    serial: u64,
    work: usize,
    seen: bool,
}

#[derive(Default)]
pub(crate) struct Cache {
    panes: [Pane; 2],
    commands: Vec<Command>,
}

pub(crate) struct Row {
    pub cells: Arc<Vec<Cell>>,
    pub end: Option<usize>,
    pub extent: Option<usize>,
    pub failed: bool,
    pub pending: bool,
}

impl Cache {
    pub fn begin(&mut self, state: &State) {
        for pane in &mut self.panes {
            pane.lines.retain(|line| {
                state.view(line.tab).is_some_and(|view| {
                    line.position
                        .cursor
                        .matches(view.snapshot(), view.inlay_hints())
                })
            });
            pane.layout = None;
            pane.work = FRAME_WORK;
            pane.seen = false;
            for line in &mut pane.lines {
                line.touched = false;
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn row(
        &mut self,
        pane: u8,
        view: &View<'_>,
        line: usize,
        skip: usize,
        width: usize,
        batch: Option<&Batch>,
        goal: Option<&NativeReveal>,
    ) -> Row {
        self.project(
            pane,
            view,
            line,
            Target {
                skip,
                width,
                display: None,
                source_end: None,
                goal: goal.cloned(),
                cols: width,
                indent: 0,
            },
            None,
            batch,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn wrapped(
        &mut self,
        pane: u8,
        view: &View<'_>,
        cursor: Cursor,
        display: Range<usize>,
        source_end: Option<usize>,
        skip: usize,
        indent: usize,
        width: usize,
        batch: Option<&Batch>,
        goal: Option<&NativeReveal>,
    ) -> Row {
        self.project(
            pane,
            view,
            cursor.line(),
            Target {
                skip: skip.saturating_sub(indent),
                width: width.saturating_sub(indent.saturating_sub(skip)),
                display: Some(display),
                source_end,
                goal: goal.cloned(),
                cols: width,
                indent,
            },
            Some(cursor),
            batch,
        )
    }

    fn project(
        &mut self,
        pane: u8,
        view: &View<'_>,
        line: usize,
        target: Target,
        start: Option<Cursor>,
        batch: Option<&Batch>,
    ) -> Row {
        let (skip, width) = (target.skip, target.width);
        let display_start = target.display.as_ref().map(|range| range.start);
        let Some(slot) = self.panes.get_mut(pane as usize) else {
            return Row {
                cells: Arc::default(),
                end: None,
                extent: None,
                failed: true,
                pending: false,
            };
        };
        let at = slot.lines.iter().position(|entry| {
            entry.tab == view.tab().id()
                && entry.position.cursor.line() == line
                && entry.target.display.as_ref().map(|range| range.start) == display_start
                && entry
                    .position
                    .cursor
                    .matches(view.snapshot(), view.inlay_hints())
        });
        let Some(mut entry) = at
            .and_then(|at| slot.lines.remove(at))
            .or_else(|| Line::new(view, line, target.clone(), start))
        else {
            return Row {
                cells: Arc::default(),
                end: None,
                extent: None,
                failed: true,
                pending: false,
            };
        };
        entry.retarget(target);
        entry.touched = true;
        if slot.pending.as_ref().is_some_and(|r| {
            r.tab == entry.tab && r.cursor.line() == line && slot.pending_display == display_start
        }) {
            slot.seen = true;
            if let Some(batch) = batch.filter(|batch| slot.pending.as_ref() == Some(&batch.request))
            {
                if batch.request.cursor == entry.position.cursor {
                    entry.failed = batch.failed;
                    for window in &batch.windows {
                        entry.consume(window);
                        slot.work = slot.work.saturating_sub(window.work);
                        if entry.ready {
                            break;
                        }
                    }
                }
                if let Some(old) = slot.pending.take() {
                    self.commands.push(Command::CancelProjection {
                        pane,
                        request: old.request,
                    });
                }
            } else if slot
                .pending
                .as_ref()
                .is_some_and(|r| r.cursor != entry.position.cursor)
                && let Some(old) = slot.pending.take()
            {
                self.commands.push(Command::CancelProjection {
                    pane,
                    request: old.request,
                });
            }
        }
        if !entry.ready && !entry.failed {
            // Keep the first frame cheap; cold prefixes continue through the existing core worker lane.
            if !entry.started
                && slot.work >= WINDOW_WORK
                && let Some(window) =
                    view.display_window(&mut entry.position.cursor.clone(), WINDOW_WORK)
            {
                slot.work = slot.work.saturating_sub(window.work);
                entry.consume(&window);
            }
            if !entry.ready && slot.pending.is_none() {
                slot.serial = slot.serial.wrapping_add(1).max(1);
                let request = Request {
                    pane,
                    tab: entry.tab,
                    cursor: entry.position.cursor.clone(),
                    request: slot.serial,
                    context: slot.serial,
                };
                self.commands
                    .push(Command::PrepareProjection(request.clone()));
                slot.pending = Some(request);
                slot.pending_display = display_start;
                slot.seen = true;
            }
        }
        if let Some((goal, bounds)) = entry.target.goal.as_ref().zip(entry.bounds.clone()) {
            self.commands.push(Command::MeasuredReveal { goal: goal.clone(), cols: entry.target.cols, bounds: bounds.start.saturating_add(entry.target.indent)..bounds.end.saturating_add(entry.target.indent) });
        }
        let row = Row {
            cells: entry.cells.clone(),
            end: (entry.complete && entry.position.column <= skip.saturating_add(width))
                .then(|| {
                    if entry.target.display.is_some() {
                        entry.target.source_end
                    } else {
                        Some(entry.position.cursor.source_byte())
                    }
                })
                .flatten(),
            extent: entry.extent.map(|extent| extent.saturating_add(entry.target.indent).saturating_add(1)),
            failed: entry.failed,
            pending: !entry.ready && !entry.failed,
        };
        slot.lines.push_front(entry);

        row
    }

    pub fn layout(&mut self, pane: u8, tab: TabId, work: code_core::layout::Work) {
        if let Some(slot) = self
            .panes
            .get_mut(pane as usize)
            .filter(|slot| slot.layout.is_none())
        {
            slot.layout = Some((tab, work));
        }
    }

    pub fn cursor(&self, pane: u8, view: &View<'_>, line: usize, display: usize) -> Option<Cursor> {
        self.panes
            .get(pane as usize)?
            .lines
            .iter()
            .find(|entry| {
                entry.tab == view.tab().id()
                    && entry.position.cursor.line() == line
                    && entry
                        .target
                        .display
                        .as_ref()
                        .is_some_and(|range| range.start == display)
                    && entry.start.matches(view.snapshot(), view.inlay_hints())
            })
            .map(|entry| entry.start.clone())
    }

    pub fn clamp(&mut self, viewport: Option<NativeViewport>, view: &View<'_>, cols: usize, extent: usize) {
        if let Some(viewport) = viewport {
            let max = extent.saturating_sub(cols);
            if view.tab().column() > max {
                self.commands.push(Command::ClampColumn { viewport, max });
            }
        }
    }

    pub fn finish(&mut self) -> Vec<Command> {
        for (pane, slot) in self.panes.iter_mut().enumerate() {
            if let Some((tab, work)) = slot.layout.take() {
                self.commands.push(Command::PrepareLayout {
                    pane: pane as u8,
                    tab,
                    work,
                });
            }
            let mut offscreen = 0;
            slot.lines.retain(|line| {
                if line.touched {
                    return true;
                }
                offscreen += 1;
                offscreen <= OFFSCREEN_LINES
            });
            if !slot.seen
                && let Some(old) = slot.pending.take()
            {
                self.commands.push(Command::CancelProjection {
                    pane: pane as u8,
                    request: old.request,
                });
            }
        }
        std::mem::take(&mut self.commands)
    }
}
