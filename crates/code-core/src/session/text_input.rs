//! Ordinary input actions share buffer permissions, source coordinates and transaction history.
//!
//! Typing, Delete, Cut, Paste and Copy act on every selection of the focused view as one transaction: the ranges are
//! sorted, validated and applied together, and each caret follows its own edit.
use super::*;
use crate::text::{EditKind, TextEdit};

impl Session {
    pub fn text_input_enabled(&self) -> bool { self.text_input_enabled }

    /// TestPlatform callbacks can exercise a capability independently of their native backend.
    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn enable_text_input_for_test(&mut self) { self.text_input_enabled = true; }

    pub(super) fn text_action_disabled(&self, id: CommandId) -> Option<&'static str> {
        let Some(view) = self.state.active().and_then(|tab| self.state.view(tab.id)) else { return Some("No loaded file is open") };
        if let Some(why) = view.edit_disabled() { return Some(why); }
        if matches!(view.rendering(), Some(Rendering::Preview(_) | Rendering::Pending)) { return Some("Show source to edit text"); }
        if !self.text_input_enabled { return Some("Text input is gated on this unvalidated platform/backend"); }
        if id == CommandId::Cut && view.tab().selections().all(|selection| selection.range().is_empty()) { return Some("Select text to cut"); }
        None
    }

    /// Availability of the commands that add or drop selections: the same source permissions as editing, and no
    /// place to add one past the document's end.
    pub(super) fn selection_action_disabled(&self, id: CommandId) -> Option<&'static str> {
        let Some(view) = self.state.active().and_then(|tab| self.state.view(tab.id)) else { return Some("No loaded file is open") };
        if let Some(why) = view.edit_disabled() { return Some(why); }
        if matches!(view.rendering(), Some(Rendering::Preview(_) | Rendering::Pending)) { return Some("Show source to edit text"); }
        if !self.text_input_enabled { return Some("Text input is gated on this unvalidated platform/backend"); }
        let (snapshot, tab) = (view.snapshot(), view.tab());
        let Some(primary) = tab.selection() else { return Some("Place the caret in the text first") };
        match id {
            CommandId::AddCursorBelow | CommandId::AddCursorAbove => {
                let below = id == CommandId::AddCursorBelow;
                let line = snapshot.line_of(primary.head);
                let target = tab.visible_for(snapshot).step(line, if below { 1 } else { -1 });
                (target == line).then_some(if below { "The caret is on the last line" } else { "The caret is on the first line" })
            }
            CommandId::SelectNextOccurrence => {
                let range = primary.range();
                (range.is_empty() && snapshot.word_at(primary.head).is_empty()).then_some("Place the caret on a word or select text")
            }
            // A live snippet has its own state to drop even with one selection, so Escape stays its key.
            CommandId::CollapseSelections => (tab.extra_selections().is_empty() && self.state.snippet().is_none()).then_some("One selection is active"),
            _ => None,
        }
    }

    /// The focused view's transaction target: its version and every selection's range in source order. A view without
    /// a selection edits at its top line, as a reader that has not placed a caret does.
    pub(super) fn selection_ranges(&self) -> Option<(TabId, crate::text::SnapshotKey, Vec<std::ops::Range<usize>>)> {
        let view = self.state.active().and_then(|tab| self.state.view(tab.id))?;
        let mut ranges: Vec<_> = view.tab().selections().map(|selection| selection.range()).collect();
        if ranges.is_empty() {
            let start = view.snapshot.reader_line_range(view.tab.top_line).start;
            ranges.push(start..start);
        }
        ranges.sort_by_key(|range| (range.start, range.end));
        Some((view.tab.id, view.snapshot.key(), ranges))
    }

    /// The same text at every selection, as one transaction (typed text, Enter, Tab, bracketed paste).
    pub(super) fn insert_text(&mut self, text: String, kind: EditKind) -> Effect {
        if self.text_action_disabled(CommandId::Paste).is_some() { return Effect::default(); }
        // One typed delimiter can wrap a selection or open a pair; a paste or a multi-character input is literal.
        if kind == EditKind::Typing && text.chars().count() == 1
            && let Some(effect) = self.typed_delimiter(&text)
        { return effect; }
        if kind == EditKind::Typing && (text == "\n" || text == "\r\n")
            && let Some(effect) = self.xml_newline(&text).or_else(|| self.yaml_newline(&text)).or_else(|| self.markdown_newline(&text)).or_else(|| self.haml_newline(&text)).or_else(|| self.indented_newline(&text))
        { return effect; }
        let Some((tab, expected, ranges)) = self.selection_ranges() else { return Effect::default() };
        let edits = ranges.into_iter().map(|range| TextEdit { range, text: text.as_str().into() }).collect();
        self.edit(tab, expected, edits, kind)
    }

    /// Plain Enter preserves each line's existing tabs and spaces before the replacement, without copying its body.
    fn indented_newline(&mut self, newline: &str) -> Option<Effect> {
        let (tab, expected, ranges) = self.selection_ranges()?;
        let doc = self.state.view(tab)?.snapshot();
        let mut edits = Vec::with_capacity(ranges.len());
        for range in ranges {
            let start = doc.reader_line_range(doc.line_of(range.start)).start;
            let mut end = start;
            while end < range.start && matches!(doc.range(end..end + 1).as_deref(), Some(" " | "\t")) { end += 1; }
            let indent = doc.range(start..end)?;
            edits.push(TextEdit { range, text: format!("{newline}{indent}").into() });
        }
        Some(self.edit(tab, expected, edits, EditKind::Typing))
    }

    fn xml_newline(&mut self, newline: &str) -> Option<Effect> {
        let (tab, expected, selections, primary) = self.structural_target()?;
        let doc = self.state.view(tab)?.snapshot();
        if doc.language() != Some(Language::Xml) { return None; }
        let unit = self.indent_unit();
        let mut edits = Vec::with_capacity(selections.len());
        let mut after = Vec::with_capacity(selections.len());
        let mut delta = 0isize;
        for selection in selections {
            let range = selection.range();
            let line = doc.line_range(doc.line_of(range.start))?;
            if range.start - line.start > 65_536 { return None; }
            let before = doc.copy_range(line.start..range.start)?;
            let indent = crate::edit_ops::xml_line_indent(&before, &unit.text);
            let body = before.trim_start_matches([' ', '\t']);
            let base = before.get(..before.len() - body.len())?;
            let closing = if indent.len() > base.len() {
                let name = body.strip_prefix('<')?.split([' ', '\t', '\r', '\n', '/', '>']).next()?;
                doc.copy_range(range.end..doc.line_range(doc.line_of(range.end))?.end)?.starts_with(&format!("</{name}>"))
            } else { false };
            let caret = range.start.checked_add_signed(delta)? + newline.len() + indent.len();
            let text = if closing { format!("{newline}{indent}{newline}{base}") } else { format!("{newline}{indent}") };
            delta += text.len() as isize - range.len() as isize;
            edits.push(TextEdit { range, text: text.into() });
            after.push(Selection { anchor: caret, head: caret });
        }
        Some(self.apply_structural(tab, expected, edits, Some((after, primary))))
    }

    /// Enter in a YAML draft: each caret's new line starts at the indentation its line's structure calls for, all in
    /// one transaction (`None`: not YAML, plain Enter).
    fn yaml_newline(&mut self, newline: &str) -> Option<Effect> {
        let (tab, expected, ranges) = self.selection_ranges()?;
        let mut edits = Vec::with_capacity(ranges.len());
        {
            let doc = self.state.view(tab)?.snapshot();
            if doc.language() != Some(crate::preferences::Language::Yaml) { return None; }
            // The file's step, resolved from its preferences only when a block opens.
            let mut step = None;
            for range in ranges {
                let line = doc.line_range(doc.line_of(range.start))?;
                let before = doc.copy_range(line.start.min(range.start)..range.start)?;
                let indent = crate::edit_ops::yaml_line_indent(&before, || *step.get_or_insert_with(|| self.indent_unit().width));
                edits.push(TextEdit { range, text: format!("{newline}{indent}").as_str().into() });
            }
        }
        Some(self.edit(tab, expected, edits, EditKind::Typing))
    }

    pub(super) fn cut(&mut self) -> Effect {
        if self.text_action_disabled(CommandId::Cut).is_some() { return Effect::default(); }
        let Some((tab, expected, ranges)) = self.selection_ranges() else { return Effect::default() };
        let Some(source) = self.state.source(tab) else { return Effect::default() };
        // The selections' text in source order, one per line, so pasting it elsewhere keeps them apart.
        let copied: Vec<String> = ranges.iter().filter(|range| !range.is_empty()).filter_map(|range| source.copy_range(range.clone())).collect();
        let text = copied.join("\n");
        let edits: Vec<_> = ranges.into_iter().map(|range| TextEdit { range, text: "".into() }).collect();
        let mut effect = self.edit(tab, expected, edits, EditKind::Replace);
        if effect.repaint { effect.copy = Some(text); }
        effect
    }

    pub(super) fn paste(&mut self, text: String) -> Effect {
        if self.text_action_disabled(CommandId::Paste).is_some() { return Effect::default(); }
        let Some((tab, expected, ranges)) = self.selection_ranges() else { return Effect::default() };
        let edits = ranges.into_iter().map(|range| TextEdit { range, text: text.as_str().into() }).collect();
        self.edit(tab, expected, edits, EditKind::Paste)
    }

    pub(super) fn delete(&mut self, backward: bool, word: bool) -> Effect {
        if self.text_action_disabled(CommandId::Delete).is_some() { return Effect::default(); }
        let Some((tab, expected, ranges)) = self.selection_ranges() else { return Effect::default() };
        let Some(source) = self.state.source(tab) else { return Effect::default() };
        // An empty selection takes the neighbouring character (a CRLF pair as one), as a caret alone does.
        let mut removals: Vec<std::ops::Range<usize>> = ranges.into_iter().map(|range| {
            if !range.is_empty() { return range; }
            let at = range.start;
            // Ctrl+Backspace / Ctrl+Delete: to the word stop; at the line's edge, the line break as below.
            let stop = if word { super::word_stop(source, at, backward) } else { at };
            if stop != at { return stop.min(at)..stop.max(at); }
            if backward {
                if source.range(at.saturating_sub(2)..at).as_deref() == Some("\r\n") { at - 2..at } else { source.previous_char_boundary(at)..at }
            } else if source.range(at..at.saturating_add(2).min(source.len_bytes())).as_deref() == Some("\r\n") { at..at + 2 }
            else { at..source.next_char_boundary(at) }
        }).filter(|range| !range.is_empty()).collect();
        // Neighbouring carets can reach for the same character, and a deletion already covered by a selection is
        // removed once; keeping them ascending and disjoint lets the whole set apply as one transaction.
        removals.sort_by_key(|range| (range.start, range.end));
        let mut disjoint: Vec<std::ops::Range<usize>> = Vec::with_capacity(removals.len());
        for range in removals {
            if let Some(last) = disjoint.last_mut().filter(|last| range.start < last.end) {
                last.end = last.end.max(range.end);
            } else {
                disjoint.push(range);
            }
        }
        if disjoint.is_empty() { return Effect::default(); }
        let edits = disjoint.into_iter().map(|range| TextEdit { range, text: "".into() }).collect();
        self.edit(tab, expected, edits, EditKind::Delete)
    }

    /// One more caret on the next (or previous) shown line at the primary caret's column; the new caret becomes the
    /// primary one, so repeating walks on in the same direction.
    pub(super) fn add_cursor(&mut self, below: bool) -> Effect {
        let id = if below { CommandId::AddCursorBelow } else { CommandId::AddCursorAbove };
        if self.disabled(id).is_some() { return Effect::default(); }
        let Some(view) = self.state.active().and_then(|tab| self.state.view(tab.id)) else { return Effect::default() };
        let doc = view.snapshot().clone();
        let Some(primary) = view.tab().selection() else { return Effect::default() };
        let Some(next) = crate::selection::caret_on_adjacent_line(&doc, &view.visible(), primary, below) else { return Effect::default() };
        let selections: Vec<_> = view.tab().selections().collect();
        let Some(tab) = self.tab_mut(view.tab().id()) else { return Effect::default() };
        tab.set_selections(next, selections);
        tab.show_head(&doc);
        Effect { repaint: true, ..Effect::default() }
    }

    /// Select the next occurrence of the primary selection's text as a further selection. A caret first selects the
    /// word it rests on; a press that finds no occurrence the set does not already cover changes nothing and says so.
    pub(super) fn select_next_occurrence(&mut self) -> Effect {
        if self.disabled(CommandId::SelectNextOccurrence).is_some() { return Effect::default(); }
        let Some(view) = self.state.active().and_then(|tab| self.state.view(tab.id)) else { return Effect::default() };
        let doc = view.snapshot().clone();
        let Some(primary) = view.tab().selection() else { return Effect::default() };
        let taken: Vec<_> = view.tab().selections().collect();
        let Some(tab) = self.tab_mut(view.tab().id()) else { return Effect::default() };
        if primary.range().is_empty() {
            let word = doc.word_at(primary.head);
            if word.is_empty() { return Effect::default(); }
            tab.set_selections(Selection { anchor: word.start, head: word.end }, taken);
            return Effect { repaint: true, ..Effect::default() };
        }
        let Some(needle) = doc.copy_range(primary.range()) else { return Effect::default() };
        match crate::selection::next_occurrence(&doc, &needle, primary.range().end, &taken) {
            Some(found) => {
                tab.set_selections(found, taken);
                tab.show_head(&doc);
                Effect { repaint: true, ..Effect::default() }
            }
            None => {
                let shown: String = needle.chars().take(40).collect();
                self.state.messages.push(format!("No further occurrence of “{shown}”"));
                Effect { repaint: true, ..Effect::default() }
            }
        }
    }

    /// Keep only the primary selection: a click or a move does the same, but Escape can drop several at once.
    pub(super) fn collapse_selections(&mut self) -> Effect {
        // Escape ends a live snippet first, whether or not it also has several selections to collapse.
        let live = self.end_snippet();
        if !live && self.disabled(CommandId::CollapseSelections).is_some() { return Effect::default(); }
        if let Some(tab) = self.active_mut() { tab.collapse_selections(); }
        Effect { repaint: true, ..Effect::default() }
    }

    pub(super) fn set_read_only(&mut self, on: bool) -> Effect {
        if self.disabled(CommandId::ReadOnlyMode).is_some() { return Effect::default(); }
        let Some(buffer) = self.state.active().and_then(|t| t.buffer) else { return Effect::default() };
        if let Some(entry) = self.state.sources.buffers.get_mut(&buffer) { entry.read_only = on; if let Some(draft) = &mut entry.draft { draft.end_group(); } }
        Effect { repaint: true, copy: None, ..Effect::default() }
    }
}
