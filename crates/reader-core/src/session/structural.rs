//! Structured text edits of the focused view: indent and unindent, comment, move and duplicate lines, and paired
//! delimiters. Every action is one transaction of the shared history; adapters only translate the keys or menu rows.
use super::*;
use crate::{
    edit_ops::IndentUnit,
    text::{EditKind, SnapshotKey},
};

impl Session {
    /// Availability of the editing commands that reshape text rather than replace a selection. The source rules are
    /// the text actions' own; a command that can do nothing here says why.
    pub(super) fn structural_action_disabled(&self, id: CommandId) -> Option<&'static str> {
        let Some(view) = self.state.active().and_then(|tab| self.state.view(tab.id())) else { return Some("No loaded file is open") };
        if let Some(why) = view.edit_disabled() { return Some(why); }
        if matches!(view.rendering(), Some(Rendering::Preview(_) | Rendering::Pending)) { return Some("Show source to edit text"); }
        if !self.text_input_enabled { return Some("Text input is gated on this unvalidated platform/backend"); }
        let doc = view.snapshot();
        let Some((_, _, selections, _)) = self.structural_target() else { return Some("No loaded file is open") };
        match id {
            CommandId::ToggleComment => {
                let comments = crate::edit_ops::comments(doc.language());
                if matches!(comments, crate::edit_ops::Comments::None) {
                    Some("This file type has no comment syntax")
                } else if crate::edit_ops::has_text_to_comment(doc, &selections, comments) {
                    None
                } else {
                    Some("There is no text to comment")
                }
            }
            CommandId::MoveLinesUp | CommandId::MoveLinesDown if !crate::edit_ops::can_move_lines(doc, &selections, id == CommandId::MoveLinesDown) => {
                Some(if id == CommandId::MoveLinesDown { "Already on the last line" } else { "Already on the first line" })
            }
            CommandId::DuplicateLinesUp | CommandId::DuplicateLinesDown if doc.len_bytes() == 0 => Some("Nothing to duplicate"),
            _ => None,
        }
    }

    /// The focused view's transaction target: its version and every selection in source order (with which of them is
    /// the primary one), and a caret at the top line when the view has none yet (as typing does).
    fn structural_target(&self) -> Option<(crate::tabs::TabId, SnapshotKey, Vec<Selection>, usize)> {
        let view = self.state.active().and_then(|tab| self.state.view(tab.id()))?;
        let mut selections: Vec<_> = view.tab().selections().collect();
        let primary = view.tab().selection();
        if selections.is_empty() {
            let start = view.snapshot.reader_line_range(view.tab.top_line).start;
            selections.push(Selection { anchor: start, head: start });
        }
        selections.sort_by_key(|selection| (selection.range().start, selection.range().end));
        let at = primary.and_then(|primary| selections.iter().position(|selection| *selection == primary)).unwrap_or(0);
        Some((view.tab.id, view.snapshot().key(), selections, at))
    }

    /// Apply one structural transaction, then give the view the selections the planner produced (and the same one
    /// primary as before) where an edit's default anchor transform cannot express where a line went.
    fn apply_structural(&mut self, tab: crate::tabs::TabId, expected: SnapshotKey, edits: Vec<crate::text::TextEdit>, after: Option<(Vec<Selection>, usize)>) -> Effect {
        let effect = self.edit(tab, expected, edits, EditKind::Replace);
        if !effect.repaint { return effect; }
        let Some((selections, primary)) = after else { return effect };
        let Some(t) = self.tab_mut(tab) else { return effect };
        let Some(primary) = selections.get(primary.min(selections.len().saturating_sub(1))).copied() else { return effect };
        let others: Vec<_> = selections.into_iter().filter(|selection| *selection != primary).collect();
        t.set_selections(primary, others);
        effect
    }

    /// The active file's indentation step: EditorConfig, then the project's choice, then the language default.
    /// Resolved for the action itself, so a changed `.editorconfig` or project choice applies at once; this runs on
    /// an explicit indent, never per typed character (see the ticket-25 cost note in `docs/editor-follow-ups.md`).
    pub(super) fn indent_unit(&self) -> IndentUnit {
        let Some(source) = self.state.current_source() else { return IndentUnit::new(crate::preferences::Style::Space, 4, 4) };
        let file = source.source();
        let language = source.language();
        let root = self.state.project.as_ref().map(|p| p.root()).filter(|root| file.starts_with(root));
        let project = root.and(self.state.project_preferences()).and_then(|p| p.indent);
        let resolved = crate::preferences::resolve(file, project, language);
        IndentUnit::new(resolved.style.0, resolved.size.0, resolved.tab_width.0)
    }

    /// Indent (`outdent`: unindent) every line the selections touch by one step of the file's indentation; a caret
    /// alone inserts that step at the caret.
    pub(super) fn indent_selection(&mut self, outdent: bool) -> Effect {
        // A live snippet owns Tab: the key visits its placeholders, and indents only once the snippet is over.
        if let Some(effect) = self.step_snippet(outdent) { return effect; }
        let id = if outdent { CommandId::OutdentSelection } else { CommandId::IndentSelection };
        if self.disabled(id).is_some() { return Effect::default(); }
        let unit = self.indent_unit();
        let Some((tab, expected, selections, _)) = self.structural_target() else { return Effect::default() };
        let Some(view) = self.state.view(tab) else { return Effect::default() };
        let edits = crate::edit_ops::indent(view.snapshot(), &selections, &unit, outdent);
        if edits.is_empty() { return Effect::default(); }
        self.apply_structural(tab, expected, edits, None)
    }

    /// Comment or uncomment every line the selections touch, in the file's language: a line prefix after the
    /// indentation, or one block pair around a span where the language has no line comment.
    pub(super) fn toggle_comment(&mut self) -> Effect {
        if self.disabled(CommandId::ToggleComment).is_some() { return Effect::default(); }
        let Some((tab, expected, selections, _)) = self.structural_target() else { return Effect::default() };
        let Some(view) = self.state.view(tab) else { return Effect::default() };
        let Some(edits) = crate::edit_ops::toggle_comments(view.snapshot(), &selections, crate::edit_ops::comments(view.snapshot().language()))
        else { return Effect::default() };
        self.apply_structural(tab, expected, edits, None)
    }

    /// Move every block of lines the selections touch one line down (or up); each line keeps its own ending where it
    /// stands, so CRLF, LF and mixed endings and the file's final newline are untouched.
    pub(super) fn move_lines(&mut self, down: bool) -> Effect {
        let id = if down { CommandId::MoveLinesDown } else { CommandId::MoveLinesUp };
        if self.disabled(id).is_some() { return Effect::default(); }
        let Some((tab, expected, selections, primary)) = self.structural_target() else { return Effect::default() };
        let Some(view) = self.state.view(tab) else { return Effect::default() };
        let Some((edits, after)) = crate::edit_ops::move_lines(view.snapshot(), &selections, down) else { return Effect::default() };
        self.apply_structural(tab, expected, edits, Some((after, primary)))
    }

    /// Paired delimiters typed at the selections: a selection is wrapped, a caret gets its pair with the caret
    /// between, and a closer already in front of every caret is stepped over. `None` leaves ordinary typing.
    pub(super) fn typed_delimiter(&mut self, typed: &str) -> Option<Effect> {
        if self.text_action_disabled(CommandId::Paste).is_some() { return None }
        let (tab, expected, selections, primary) = self.structural_target()?;
        let view = self.state.view(tab)?;
        match crate::edit_ops::typed(typed, view.snapshot(), &selections, view.snapshot().language()) {
            crate::edit_ops::Pairing::Plain => None,
            crate::edit_ops::Pairing::Step(by) => {
                let moved: Vec<Selection> = selections.iter().map(|selection| {
                    let head = view.snapshot().floor_char_boundary(selection.head + by);
                    Selection { anchor: head, head }
                }).collect();
                let primary = moved.get(primary).copied()?;
                let others: Vec<_> = moved.into_iter().filter(|selection| *selection != primary).collect();
                self.tab_mut(tab)?.set_selections(primary, others);
                Some(Effect { repaint: true, ..Effect::default() })
            }
            crate::edit_ops::Pairing::Plan(edits, after) => {
                Some(self.apply_structural(tab, expected, edits, Some((after, primary))))
            }
        }
    }

    /// Copy every block of lines the selections touch directly below (or above) itself, in one transaction; the
    /// selections stay on the original lines.
    pub(super) fn duplicate_lines(&mut self, down: bool) -> Effect {
        let id = if down { CommandId::DuplicateLinesDown } else { CommandId::DuplicateLinesUp };
        if self.disabled(id).is_some() { return Effect::default(); }
        let Some((tab, expected, selections, _)) = self.structural_target() else { return Effect::default() };
        let Some(view) = self.state.view(tab) else { return Effect::default() };
        let Some(edits) = crate::edit_ops::duplicate_lines(view.snapshot(), &selections, down) else { return Effect::default() };
        self.apply_structural(tab, expected, edits, None)
    }
}
