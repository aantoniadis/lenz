//! Bounded terminal measurement over shared resumable projections.
use reader_core::{
    projection::{Batch, Cursor, Request, WINDOW_WORK, Window},
    session::{Command, State, View},
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

#[derive(Clone, Copy, PartialEq, Eq)]
struct Target {
    skip: usize,
    width: usize,
}

struct Line {
    tab: TabId,
    target: Target,
    position: Position,
    checkpoints: VecDeque<Position>,
    cells: Arc<Vec<Cell>>,
    complete: bool,
    extent: Option<usize>,
    end: Option<Cursor>,
    ready: bool,
    failed: bool,
    touched: bool,
}

impl Line {
    fn new(view: &View<'_>, line: usize, target: Target) -> Option<Self> {
        Some(Self {
            tab: view.tab().id(),
            target,
            position: Position {
                cursor: view.display_start(line)?,
                column: 0,
                carry: None,
            },
            checkpoints: VecDeque::new(),
            cells: Arc::default(),
            complete: false,
            extent: None,
            end: None,
            ready: false,
            failed: false,
            touched: true,
        })
    }

    fn retarget(&mut self, view: &View<'_>, target: Target) {
        if self.target == target {
            return;
        }
        let best = self
            .checkpoints
            .iter()
            .chain(std::iter::once(&self.position))
            .filter(|p| p.column <= target.skip)
            .max_by_key(|p| p.column)
            .cloned();
        let position = best.or_else(|| {
            Some(Position {
                cursor: view.display_start(self.position.cursor.line())?,
                column: 0,
                carry: None,
            })
        });
        let Some(position) = position else {
            self.failed = true;
            return;
        };
        self.position = position;
        self.target = target;
        self.cells = Arc::default();
        self.complete =
            self.end.as_ref() == Some(&self.position.cursor) && self.position.carry.is_none();
        self.ready = self.complete;
        self.failed = false;
    }

    fn emit(&mut self, mut cell: Cell) {
        debug_assert!(cell.text().len() <= reader_core::projection::GRAPHEME_BYTES + 1);
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
        if cell.column >= self.target.skip
            && cell.column.saturating_add(cell.width)
                <= self.target.skip.saturating_add(self.target.width)
        {
            Arc::make_mut(&mut self.cells).push(cell.clone());
        }
        self.position.column = self.position.column.saturating_add(cell.width);
    }

    fn consume(&mut self, window: &Window) {
        self.checkpoints.push_back(self.position.clone());
        if self.checkpoints.len() > CHECKPOINTS {
            self.checkpoints.pop_front();
        }
        let carry = self.position.carry.take();
        let prefix = carry.as_ref().map_or(0, |cell| cell.text().len());
        let mut text = carry
            .as_ref()
            .map_or_else(String::new, |cell| cell.text().to_owned());
        text.push_str(&window.shown.text);
        let text = Arc::new(text);
        let mut graphemes = text.grapheme_indices(true).peekable();
        while let Some((at, grapheme)) = graphemes.next() {
            let mut cell = if let Some(carry) = carry.as_ref().filter(|_| at < prefix) {
                let mut cell = carry.clone();
                cell.text = text.clone();
                cell.range = at..at + grapheme.len();
                cell
            } else {
                let local = at - prefix;
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
            if graphemes.peek().is_none() && !window.complete {
                cell.text = Arc::new(grapheme.to_owned());
                cell.range = 0..grapheme.len();
                self.position.carry = Some(cell);
            } else {
                self.emit(cell);
            }
        }
        self.position.cursor = window.next().clone();
        self.complete = window.complete;
        if self.complete {
            self.extent = Some(self.position.column);
            self.end = Some(self.position.cursor.clone());
        }
        self.ready = self.complete
            || self.position.column >= self.target.skip.saturating_add(self.target.width);
    }
}

#[derive(Default)]
struct Pane {
    lines: VecDeque<Line>,
    pending: Option<Request>,
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
            pane.work = FRAME_WORK;
            pane.seen = false;
            for line in &mut pane.lines {
                line.touched = false;
            }
        }
    }

    pub fn row(
        &mut self,
        pane: u8,
        view: &View<'_>,
        line: usize,
        skip: usize,
        width: usize,
        batch: Option<&Batch>,
    ) -> Row {
        let target = Target { skip, width };
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
                && entry
                    .position
                    .cursor
                    .matches(view.snapshot(), view.inlay_hints())
        });
        let Some(mut entry) = at
            .and_then(|at| slot.lines.remove(at))
            .or_else(|| Line::new(view, line, target))
        else {
            return Row {
                cells: Arc::default(),
                end: None,
                extent: None,
                failed: true,
                pending: false,
            };
        };
        entry.retarget(view, target);
        entry.touched = true;
        if slot
            .pending
            .as_ref()
            .is_some_and(|r| r.tab == entry.tab && r.cursor.line() == line)
        {
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
            if entry.position.cursor.display_byte() == 0
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
                slot.seen = true;
            }
        }
        let row = Row {
            cells: entry.cells.clone(),
            end: (entry.complete && entry.position.column <= skip.saturating_add(width))
                .then(|| entry.position.cursor.source_byte()),
            extent: entry.extent,
            failed: entry.failed,
            pending: !entry.ready && !entry.failed,
        };
        slot.lines.push_front(entry);

        row
    }

    pub fn clamp(&mut self, pane: u8, view: &View<'_>, cols: usize, extent: usize) {
        if let Some(identity) = view.display_start(view.tab().top_line()) {
            let max = extent.saturating_sub(cols);
            if view.tab().column() > max {
                self.commands.push(Command::ClampColumn {
                    pane,
                    tab: view.tab().id(),
                    identity,
                    cols,
                    max,
                });
            }
        }
    }

    pub fn finish(&mut self) -> Vec<Command> {
        for (pane, slot) in self.panes.iter_mut().enumerate() {
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
