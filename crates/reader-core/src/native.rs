//! Native horizontal measurement goals. Text and semantic navigation remain owned by the session.
use crate::{
    folds::Visible,
    projection::Cursor,
    session::{Selection, State},
    tabs::TabId,
};
use std::{ops::Range, sync::Arc};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeReveal {
    pub request: u64,
    pub pane: u8,
    pub tab: TabId,
    pub source: usize,
    pub identity: Cursor,
    pub context: u64,
}

/// Opaque identity of the viewport whose complete native extents were measured.
#[derive(Clone, Debug)]
pub struct NativeViewport(Arc<Viewport>);

#[derive(Debug)]
struct Viewport {
    pub(crate) pane: u8,
    pub(crate) tab: TabId,
    identity: Cursor,
    context: u64,
    epoch: u64,
    size: (usize, usize),
    top: (usize, usize, usize),
    visible: Arc<Visible>,
    declarations: Option<Arc<lenz_content::highlight::Declarations>>,
    wrapped: bool,
}
impl PartialEq for Viewport {
    fn eq(&self, other: &Self) -> bool {
        self.pane == other.pane && self.tab == other.tab && self.identity == other.identity
            && self.context == other.context && self.epoch == other.epoch && self.size == other.size && self.top == other.top
            && self.wrapped == other.wrapped && Arc::ptr_eq(&self.visible, &other.visible)
            && match (&self.declarations, &other.declarations) {
                (None, None) => true, (Some(a), Some(b)) => Arc::ptr_eq(a, b), _ => false,
            }
    }
}
impl Eq for Viewport {}
impl PartialEq for NativeViewport { fn eq(&self, other: &Self) -> bool { self.0 == other.0 } }
impl Eq for NativeViewport {}
impl NativeViewport {
    pub(crate) fn tab(&self) -> TabId { self.0.tab }
    pub(crate) fn capture(state: &State, reveals: &Reveals, pane: u8) -> Option<Self> {
        let tab = state.shown(pane)?;
        let view = state.presentation_view(tab.id())?;
        let tab = view.tab();
        Some(Self(Arc::new(Viewport { pane, tab: tab.id(), identity: view.display_start(tab.top_line())?,
            context: reveals.context(pane)?, epoch: reveals.serial, size: state.text_size(pane)?,
            top: (tab.top_line(), tab.top_row(), tab.column()), visible: view.visible(),
            declarations: view.declarations_handle().cloned(), wrapped: state.wrap_cols(tab).is_some() })))
    }
    pub(crate) fn current(&self, state: &State, reveals: &Reveals) -> bool {
        let Some(now) = Self::capture(state, reveals, self.0.pane) else { return false };
        self == &now
    }
}

struct Pending {
    goal: NativeReveal,
    selection: Option<Selection>,
    visible: Arc<Visible>,
    top: (usize, usize),
}

impl Pending {
    fn current(&self, state: &State) -> bool {
        state
            .shown(self.goal.pane)
            .is_some_and(|tab| tab.id() == self.goal.tab)
            && state.presentation_view(self.goal.tab).is_some_and(|view| {
                self.goal
                    .identity
                    .matches(view.snapshot(), view.inlay_hints())
                    && view.tab().selection() == self.selection
                    && self.top == (view.tab().top_line(), view.tab().top_row())
                    && Arc::ptr_eq(&self.visible, &view.visible())
            })
    }
}

#[derive(Default)]
pub(crate) struct Reveals {
    serial: u64,
    contexts: [u64; 2],
    pending: [Option<Pending>; 2],
}

impl Reveals {
    pub fn context(&self, pane: u8) -> Option<u64> {
        self.contexts.get(pane as usize).copied()
    }
    pub fn goal(&self, pane: u8, state: &State) -> Option<&NativeReveal> {
        self.pending
            .get(pane as usize)?
            .as_ref()
            .filter(|pending| pending.current(state))
            .map(|pending| &pending.goal)
    }
    pub fn cancel(&mut self) {
        self.serial = self.serial.wrapping_add(1);
        self.pending = Default::default();
    }
    pub fn invalidate(&mut self, state: &State) {
        for slot in &mut self.pending {
            if slot.as_ref().is_some_and(|pending| !pending.current(state)) {
                *slot = None;
            }
        }
    }
    pub fn issue(&mut self, state: &State, tab: TabId, source: usize) {
        let Some(view) = state.presentation_view(tab) else { return };
        let pane = view.tab().pane();
        let Some(context) = self.context(pane) else {
            return;
        };
        if state.shown(pane).is_none_or(|shown| shown.id() != tab)
            || source > view.snapshot().len_bytes()
            || view.snapshot().floor_char_boundary(source) != source
        {
            return;
        }
        let Some(identity) = view.display_start(view.snapshot().line_of(source)) else {
            return;
        };
        self.serial = self.serial.wrapping_add(1).max(1);
        self.pending[pane as usize] = Some(Pending {
            goal: NativeReveal {
                request: self.serial,
                pane,
                tab,
                source,
                identity,
                context,
            },
            selection: view.tab().selection(),
            visible: view.visible(),
            top: (view.tab().top_line(), view.tab().top_row()),
        });
    }
    pub fn regenerate(&mut self, state: &State, pane: u8) {
        self.serial = self.serial.wrapping_add(1);
        let old = self.pending.get_mut(pane as usize).and_then(Option::take);
        if let Some(old) = old.filter(|old| {
            state.presentation_view(old.goal.tab).is_some_and(|view| {
                old.goal
                    .identity
                    .matches(view.snapshot(), view.inlay_hints())
                    && view.tab().selection() == old.selection
            })
        }) {
            self.issue(state, old.goal.tab, old.goal.source);
        }
    }
    pub fn set_context(&mut self, state: &State, pane: u8, context: u64) -> bool {
        let Some(current) = self.contexts.get_mut(pane as usize) else {
            return false;
        };
        if *current == context {
            return false;
        }
        *current = context;
        self.regenerate(state, pane);
        true
    }
    pub fn measured(
        &mut self,
        state: &State,
        goal: &NativeReveal,
        cols: usize,
        bounds: Range<usize>,
    ) -> Option<usize> {
        if bounds.start >= bounds.end
            || cols == 0
            || self.context(goal.pane) != Some(goal.context)
            || state.text_size(goal.pane).is_none_or(|size| size.0 != cols)
            || self.goal(goal.pane, state) != Some(goal)
        {
            return None;
        }
        let view = state.presentation_view(goal.tab)?;
        let old = view.tab().column();
        let column = reveal_column(old, bounds, cols, state.wrap_cols(view.tab()).is_some());
        self.pending[goal.pane as usize] = None;
        Some(column)
    }
}

fn reveal_column(old: usize, bounds: Range<usize>, cols: usize, wrapped: bool) -> usize {
    let width = bounds.end.saturating_sub(bounds.start);
    if width >= cols {
        return bounds.start;
    }
    let margin = 3.min(cols.saturating_sub(width) / 2);
    let first = bounds.end.saturating_add(margin).saturating_sub(cols);
    let last = bounds.start.saturating_sub(margin).max(first);
    if wrapped {
        first
    } else {
        old.clamp(first, last)
    }
}
