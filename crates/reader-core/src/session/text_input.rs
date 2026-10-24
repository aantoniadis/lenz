//! Ordinary input actions share buffer permissions, source coordinates and transaction history.
use super::*;
use crate::text::{EditKind, TextEdit};

impl Session {
    /// Existing adapter test opt-ins are the only callers before the validated-target checkpoint.
    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn enable_text_input_for_test(&mut self) { self.text_input_enabled = true; }

    pub(super) fn text_action_disabled(&self, id: CommandId) -> Option<&'static str> {
        let Some(view) = self.state.active().and_then(|tab| self.state.view(tab.id)) else { return Some("No loaded file is open") };
        if let Some(why) = view.edit_disabled() { return Some(why); }
        if matches!(view.rendering(), Some(Rendering::Preview(_) | Rendering::Pending)) { return Some("Show source to edit text"); }
        if !self.text_input_enabled { return Some("Text input is gated until the editor safety checkpoint"); }
        if id == CommandId::Cut && view.selected_range().is_none() { return Some("Select text to cut"); }
        None
    }
    fn input_selection(&self) -> Option<(TabId, crate::text::SnapshotKey, std::ops::Range<usize>)> {
        let view = self.state.active().and_then(|tab| self.state.view(tab.id))?;
        let range = view.tab.selection.map(|s| s.range()).unwrap_or_else(|| { let start = view.snapshot.reader_line_range(view.tab.top_line).start; start..start });
        Some((view.tab.id, view.snapshot.key(), range))
    }
    pub(super) fn cut(&mut self) -> Effect {
        if self.text_action_disabled(CommandId::Cut).is_some() { return Effect::default(); }
        let Some((tab, expected, range)) = self.input_selection() else { return Effect::default() };
        let Some(text) = self.state.source(tab).and_then(|s| s.copy_range(range.clone())) else { return Effect::default() };
        let mut effect = self.edit(tab, expected, vec![TextEdit { range, text: "".into() }], EditKind::Replace);
        if effect.repaint { effect.copy = Some(text); }
        effect
    }
    pub(super) fn paste(&mut self, text: String) -> Effect {
        if self.text_action_disabled(CommandId::Paste).is_some() { return Effect::default(); }
        let Some((tab, expected, range)) = self.input_selection() else { return Effect::default() };
        self.edit(tab, expected, vec![TextEdit { range, text: text.into() }], EditKind::Paste)
    }
    pub(super) fn delete(&mut self, backward: bool) -> Effect {
        if self.text_action_disabled(CommandId::Delete).is_some() { return Effect::default(); }
        let Some((tab, expected, mut range)) = self.input_selection() else { return Effect::default() };
        let Some(source) = self.state.source(tab) else { return Effect::default() };
        if range.is_empty() {
            let at = range.start;
            range = if backward {
                if source.range(at.saturating_sub(2)..at).as_deref() == Some("\r\n") { at - 2..at }
                else { source.previous_char_boundary(at)..at }
            } else if source.range(at..at.saturating_add(2).min(source.len_bytes())).as_deref() == Some("\r\n") { at..at + 2 }
                else { at..source.next_char_boundary(at) };
        }
        if range.is_empty() { return Effect::default(); }
        self.edit(tab, expected, vec![TextEdit { range, text: "".into() }], EditKind::Delete)
    }
    pub(super) fn set_read_only(&mut self, on: bool) -> Effect {
        if self.disabled(CommandId::ReadOnlyMode).is_some() { return Effect::default(); }
        let Some(buffer) = self.state.active().and_then(|t| t.buffer) else { return Effect::default() };
        if let Some(entry) = self.state.sources.buffers.get_mut(&buffer) { entry.read_only = on; if let Some(draft) = &mut entry.draft { draft.end_group(); } }
        Effect { repaint: true, copy: None, ..Effect::default() }
    }
}
