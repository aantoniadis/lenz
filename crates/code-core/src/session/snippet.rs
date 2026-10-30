//! Snippets: one inserted body, its placeholders in visit order, Tab between them and ending it safely.
//!
//! A snippet belongs to one view and one document version. It is live only while that version is shown with the
//! active placeholder selected, so a click, a move, an undo, a reload, another tab or a read-only switch ends it
//! without leaving positions behind; Tab then indents as it otherwise does.
use super::*;
use crate::{
    snippet::{Layout, Snippet},
    text::{EditKind, SnapshotKey, TextEdit},
};
use std::ops::Range;

/// A snippet inserted in `tab`: its stops in visit order, as absolute ranges of that version, and the stop now
/// selected (one range per occurrence of a repeated placeholder).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveSnippet {
    tab: TabId,
    key: SnapshotKey,
    stops: Vec<Vec<Range<usize>>>,
    active: usize,
}

impl LiveSnippet {
    /// The selected stop, counting from one.
    pub fn stop(&self) -> usize { self.active + 1 }
    pub fn stop_count(&self) -> usize { self.stops.len() }
    /// Whether Tab ends the snippet rather than moving on.
    pub fn at_end(&self) -> bool { self.active + 1 >= self.stops.len() }
    fn current(&self) -> &[Range<usize>] { self.stops.get(self.active).map_or(&[], Vec::as_slice) }
}

impl State {
    /// The live snippet: the active tab's, still about the version it was inserted into, still editable, and with
    /// its placeholder selected. Anything else means the snippet is over.
    pub fn snippet(&self) -> Option<&LiveSnippet> {
        let live = self.snippet.as_ref()?;
        if self.active != Some(live.tab) { return None; }
        let view = self.view(live.tab)?;
        if view.snapshot().key() != live.key || view.edit_disabled().is_some() { return None; }
        if matches!(view.rendering(), Some(Rendering::Preview(_) | Rendering::Pending)) { return None; }
        let tab = view.tab();
        let mut selected: Vec<Range<usize>> = tab.selections().map(|selection| selection.range()).collect();
        selected.sort_by_key(|range| (range.start, range.end));
        (selected == live.current()).then_some(live)
    }
}

impl Session {
    /// Insert snippet `body` in place of `range` of `tab`'s `expected` version, saying why when the body cannot be
    /// expanded.
    pub(super) fn insert_snippet_body(&mut self, tab: TabId, expected: SnapshotKey, range: Range<usize>, body: &str) -> Effect {
        match Snippet::parse(body) {
            Ok(snippet) => self.insert_snippet(tab, expected, range, &snippet, Vec::new(), None),
            Err(why) => { self.state.messages.push(format!("Snippet: {why}")); Effect { repaint: true, ..Effect::default() } }
        }
    }

    /// Insert `snippet` in place of `range` of `tab`'s `expected` version as one transaction with `extra` (a
    /// completion's imports, which do not overlap it), with the file's indentation on the inserted lines, and select
    /// its first placeholder. A view that cannot be edited and several carets change nothing and say why.
    pub(super) fn insert_snippet(&mut self, tab: TabId, expected: SnapshotKey, range: Range<usize>, parsed: &Snippet, extra: Vec<TextEdit>, guard: Option<SnippetGuard<'_>>) -> Effect {
        let Some(view) = self.state.view(tab) else { return Effect::default() };
        if let Some(why) = view.edit_disabled() { self.state.messages.push(format!("Snippet: {why}")); return Effect { repaint: true, ..Effect::default() }; }
        if !view.tab().extra_selections().is_empty() {
            self.state.messages.push("Snippet: a snippet needs one caret; Escape collapses several selections".into());
            return Effect { repaint: true, ..Effect::default() };
        }
        let snapshot = view.snapshot().clone();
        let at = snapshot.floor_char_boundary(range.start);
        let base = line_indentation(&snapshot, at);
        let indent = self.indent_unit();
        let expanded = parsed.expand(&Layout { indent: &indent, base: &base, newline: view.input_newline() });
        let inserted = expanded.text.len();
        self.state.snippet = None;
        self.close_completion();
        // Imports before the body move it by their own change in length.
        let shift: isize = extra.iter().filter(|e| e.range.end <= at).map(|e| e.text.len() as isize - e.range.len() as isize).sum();
        let mut edits = extra;
        edits.push(TextEdit { range: at..snapshot.floor_char_boundary(range.end), text: expanded.text.as_str().into() });
        edits.sort_by_key(|e| (e.range.start, e.range.end));
        if let Some(guard) = guard && let Some(projection) = guard.view.embedded()
            && let Err(why) = projection.check(&edits, guard.typed, &|r| snapshot.range(r).map(std::borrow::Cow::into_owned))
        {
            self.state.messages.push(format!("{}: {why}; nothing was inserted", guard.label));
            return Effect { repaint: true, ..Effect::default() };
        }
        let effect = self.edit(tab, expected, edits, EditKind::Replace);
        if !effect.repaint { return effect; }
        let at = at.saturating_add_signed(shift);
        let Some(snapshot) = self.state.view(tab).map(|view| view.snapshot().clone()) else { return effect };
        let stops: Vec<Vec<Range<usize>>> = expanded.stops.iter().map(|stop| {
            stop.iter().map(|range| at.saturating_add(range.start)..at.saturating_add(range.end)).collect()
        }).collect();
        let Some(first) = stops.first().filter(|_| stops.len() > 1) else {
            // Nothing to visit but the final caret: it goes there (`color: $0;` before the `;`), and after the
            // inserted text when the body names none.
            let end = stops.first().and_then(|stop| stop.first()).map_or(at.saturating_add(inserted), |range| range.start);
            self.select_stop(tab, std::slice::from_ref(&(end..end)), &snapshot);
            return effect;
        };
        self.select_stop(tab, first, &snapshot);
        self.state.snippet = Some(LiveSnippet { tab, key: snapshot.key(), stops, active: 0 });
        effect
    }

    /// Tab (`outdent`: Shift+Tab) with a live snippet visits the next (or previous) placeholder; past either end
    /// the snippet is over and the key changes nothing. `None` lets the ordinary indent run.
    pub(super) fn step_snippet(&mut self, outdent: bool) -> Option<Effect> {
        let live = self.state.snippet()?.clone();
        let next = if outdent { live.active.checked_sub(1) } else { (live.active + 1 < live.stops.len()).then_some(live.active + 1) };
        let Some(index) = next else {
            self.state.snippet = None;
            return Some(Effect { repaint: true, ..Effect::default() });
        };
        let Some(stop) = live.stops.get(index).cloned() else {
            self.state.snippet = None;
            return Some(Effect { repaint: true, ..Effect::default() });
        };
        let snapshot = self.state.view(live.tab).map(|view| view.snapshot().clone())?;
        self.select_stop(live.tab, &stop, &snapshot);
        if let Some(current) = self.state.snippet.as_mut() { current.active = index; }
        Some(Effect { repaint: true, ..Effect::default() })
    }

    /// End the live snippet (Escape, a tab change, an undo): Tab indents again.
    pub(super) fn end_snippet(&mut self) -> bool {
        self.state.snippet.take().is_some()
    }

    /// Carry the live snippet's placeholder positions through an edit of its own tab. The transform is the one the
    /// view's selections went through, so a stop stays the selection it was; the version moves on with the text.
    pub(super) fn snippet_edited(&mut self, tab: TabId, key: SnapshotKey, edits: &[TextEdit]) {
        let Some(live) = self.state.snippet.as_mut().filter(|live| live.tab == tab) else { return };
        live.key = key;
        for stop in &mut live.stops {
            for range in stop.iter_mut() {
                range.start = crate::text::edited_byte(range.start, edits);
                range.end = crate::text::edited_byte(range.end, edits).max(range.start);
            }
        }
    }

    /// Select every occurrence of one stop, the leftmost as the primary selection.
    fn select_stop(&mut self, tab: TabId, stop: &[Range<usize>], snapshot: &crate::text::TextSnapshot) {
        let Some((primary, others)) = stop.split_first() else { return };
        let selection = |range: &Range<usize>| Selection { anchor: snapshot.floor_char_boundary(range.start), head: snapshot.floor_char_boundary(range.end) };
        let Some(target) = self.tab_mut(tab) else { return };
        target.set_selections(selection(primary), others.iter().map(selection));
        target.show_head(snapshot);
    }
}

/// A language server's snippet in a template: inserted only when [`crate::template::Projection::check`] accepts it
/// against `view` (the template view the server was asked about) after the `typed` edit sets since; else refused, said
/// under `label`.
pub(super) struct SnippetGuard<'a> {
    pub view: &'a Document,
    pub typed: &'a [Vec<TextEdit>],
    pub label: &'static str,
}

/// The leading whitespace of the line `at` is on: every inserted line after the first starts there.
fn line_indentation(snapshot: &crate::text::TextSnapshot, at: usize) -> String {
    let line = snapshot.line_of(at);
    let Some(start) = snapshot.line_range(line).map(|range| range.start) else { return String::new() };
    let Some(window) = snapshot.line_window(line, start, 4096) else { return String::new() };
    window.text.chars().take_while(|c| *c == ' ' || *c == '\t').collect()
}
