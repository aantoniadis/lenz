//! A disposable read-only input-method projection. Never a buffer, save baseline or analysis source.
use super::*;
use crate::text::{TextEdit, TextSnapshot};

/// Weak exact source identity for a captured input-method transaction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PresentationSource(SemanticSource);
impl PresentationSource {
    pub fn new(source: &TextSnapshot) -> Self { Self(SemanticSource::new(source)) }
    pub fn matches(&self, source: &TextSnapshot) -> bool { self.0.matches(source) }
}

pub(super) struct Presentation {
    base: SemanticSource,
    selection: Option<Selection>,
    tab: Tab,
    snapshot: Arc<TextSnapshot>,
}
impl Presentation {
    pub(super) fn current(&self, state: &State) -> bool {
        state.active == Some(self.tab.id) && state.shown(self.tab.pane).is_some_and(|tab| tab.id == self.tab.id)
            && state.view(self.tab.id).is_some_and(|view| self.base.matches(view.snapshot())
                && view.tab.selection == self.selection && view.edit_disabled().is_none())
    }
}
impl State {
    /// The one transient display tab, or the ordinary shown tab when no composition is active.
    pub fn presentation_shown(&self, pane: u8) -> Option<&Tab> { self.presentation_view(self.shown(pane)?.id()).map(|view| view.tab()) }
    /// Read-only display geometry; editing, providers and persistence must continue to use `view`.
    pub fn presentation_view(&self, id: TabId) -> Option<View<'_>> {
        let view = self.view(id)?;
        if let Some(p) = self.presentation.as_ref().filter(|p| p.tab.id == id && p.current(self)) {
            Some(View { tab: &p.tab, snapshot: &p.snapshot, read_only: true, ..view })
        } else { Some(view) }
    }
    pub(super) fn display_tab_mut(&mut self, id: TabId) -> Option<&mut Tab> {
        if self.presentation.as_ref().is_some_and(|p| p.tab.id == id && p.current(self)) {
            self.presentation.as_mut().map(|p| &mut p.tab)
        } else { self.tabs.iter_mut().find(|tab| tab.id == id) }
    }
}
impl Session {
    pub(super) fn present_text(&mut self, tab: TabId, base: PresentationSource, base_selection: Option<Selection>, edit: TextEdit, selected: Selection) -> Effect {
        if self.text_action_disabled(CommandId::Paste).is_some() { return Effect::default(); }
        let Some(view) = self.state.view(tab).filter(|view| self.state.active == Some(tab)
            && base.matches(view.snapshot()) && view.tab.selection == base_selection) else { return Effect::default() };
        let version = self.versions.fetch_add(1, Ordering::Relaxed) + 1;
        let Ok(snapshot) = view.snapshot().apply_edits(version, &[edit]) else { return Effect::default() };
        if [selected.anchor, selected.head].iter().any(|byte| *byte > snapshot.len_bytes() || snapshot.floor_char_boundary(*byte) != *byte) {
            return Effect::default();
        }
        let mut shown = view.tab.duplicate(tab, view.tab.pane);
        shown.new_content(); shown.set_top(view.tab.top_line(), view.tab.top_row()); shown.collapsed.clear(); shown.comparison_side = view.tab.comparison_side; shown.markdown_source = true;
        shown.selection = Some(selected);
        shown.top_line = shown.top_line.min(snapshot.line_count() - 1);
        self.state.presentation = Some(Box::new(Presentation { base: base.0, selection: base_selection, tab: shown, snapshot: Arc::new(snapshot) }));
        self.keep_selection_visible();
        Effect { repaint: true, ..Effect::default() }
    }
}
