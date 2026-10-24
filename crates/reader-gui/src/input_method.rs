//! Native callback adapter. Preedit is a disposable projection, never an authoritative draft.
use super::*;
use gpui::{EntityInputHandler, UTF16Selection};
use reader_core::{session::PresentationSource, text::{EditKind, TextEdit, TextSnapshot}};
use std::ops::Range;

const SURROUNDING_BYTES: usize = 4096;
#[derive(Clone, PartialEq)]
struct InputContext { font: Font, layout: Layout, viewport: (u32, u32), wrap: bool, split: Option<Split> }
#[derive(Clone)]
pub(super) struct Capture {
    pub(super) tab: TabId, pane: u8, base: Arc<TextSnapshot>, selection: Option<Selection>, context: InputContext,
    range: Range<usize>, text: String, selected: Selection,
}
struct Ended {
    tab: TabId, source: PresentationSource, selection: Option<Selection>, context: InputContext,
}
#[derive(Default)]
pub(super) struct InputState {
    pub(super) capture: Option<Capture>,
    stale: bool,
    ended: Option<Ended>,
    pub registered: std::rc::Rc<std::cell::Cell<bool>>,
}
fn utf16_bytes(text: &str, range: Range<usize>) -> Option<Range<usize>> {
    let offset = |wanted| {
        let mut units = 0;
        for (byte, c) in text.char_indices() {
            if units == wanted { return Some(byte); }
            units += c.len_utf16(); if units > wanted { return None; }
        }
        (units == wanted).then_some(text.len())
    };
    (range.start <= range.end).then_some(())?;
    Some(offset(range.start)?..offset(range.end)?)
}
fn source_range(source: &TextSnapshot, range: Range<usize>) -> Option<Range<usize>> {
    (range.start <= range.end).then_some(())?;
    Some(source.utf16_to_byte(range.start)?..source.utf16_to_byte(range.end)?)
}
impl ReaderView {
    fn input_context(&self, window: &Window) -> InputContext {
        let size = window.viewport_size();
        InputContext { font: self.font.clone(), layout: self.layout, viewport: (f32::from(size.width).to_bits(), f32::from(size.height).to_bits()), wrap: self.session.state().soft_wrap(), split: self.session.state().split() }
    }
    fn capture_source_current(&self, capture: &Capture) -> bool {
        self.draft_target().is_some_and(|(tab, key, _)| tab == capture.tab && key == capture.base.key())
            && self.session.state().view(capture.tab).is_some_and(|view| view.tab().pane() == capture.pane
                && Arc::ptr_eq(view.snapshot().source_meta(), capture.base.source_meta()) && view.tab().selection() == capture.selection)
            && self.session.state().presentation_view(capture.tab).is_some_and(|view| view.snapshot().key() != capture.base.key())
    }
    fn capture_current(&self, capture: &Capture, window: &Window) -> bool {
        self.capture_source_current(capture) && self.focus.is_focused(window) && capture.context == self.input_context(window)
    }
    pub(super) fn invalidate_input_source(&mut self, cx: &mut Context<Self>) {
        if self.input_state.capture.as_ref().is_some_and(|capture| !self.capture_source_current(capture)) { self.cancel_input(true, cx); }
        if self.input_state.ended.as_ref().is_some_and(|ended| !self.ended_source_current(ended)) { self.input_state.ended = None; }
    }
    pub(super) fn synchronize_input(&mut self, window: &Window, cx: &mut Context<Self>) {
        if self.input_state.capture.as_ref().is_some_and(|capture| !self.capture_current(capture, window)) { self.cancel_input(true, cx); }
        if self.input_state.ended.as_ref().is_some_and(|ended| !self.ended_current(ended, window)) { self.input_state.ended = None; }
    }
    fn ended_context(&self, context: InputContext) -> Option<Ended> {
        let (tab, _, _) = self.draft_target()?; let view = self.session.state().view(tab)?;
        Some(Ended { tab, source: PresentationSource::new(view.snapshot()), selection: view.tab().selection(), context })
    }
    fn ended_source_current(&self, ended: &Ended) -> bool {
        self.draft_target().is_some_and(|(tab, _, _)| tab == ended.tab)
            && self.session.state().view(ended.tab).is_some_and(|view| ended.source.matches(view.snapshot()) && view.tab().selection() == ended.selection)
            && ended.context.font == self.font && ended.context.layout == self.layout
            && ended.context.wrap == self.session.state().soft_wrap() && ended.context.split == self.session.state().split()
    }
    fn ended_current(&self, ended: &Ended, window: &Window) -> bool {
        self.focus.is_focused(window) && self.ended_source_current(ended) && ended.context == self.input_context(window)
    }
    fn cancel_marked_input(&mut self, cx: &mut Context<Self>) {
        let context = self.input_state.capture.as_ref().map(|capture| capture.context.clone());
        self.cancel_input(true, cx);
        self.input_state.ended = context.and_then(|context| self.ended_context(context));
    }
    pub(super) fn cancel_input(&mut self, stale: bool, cx: &mut Context<Self>) {
        if self.input_state.capture.take().is_some() { self.session.dispatch(Command::ClearPresentation); cx.notify(); }
        self.input_state.stale = stale; self.input_state.ended = None;
    }
    fn input_snapshot(&self, window: &Window) -> Option<Arc<TextSnapshot>> {
        if !self.focus.is_focused(window) || self.input_state.stale && !self.input_state.ended.as_ref().is_some_and(|ended| self.ended_current(ended, window)) { return None; }
        self.draft_target()?;
        if let Some(capture) = &self.input_state.capture {
            if !self.capture_current(capture, window) { return None; }
            Some(self.session.state().presentation_view(capture.tab)?.snapshot().clone())
        } else { self.session.state().current_source().cloned() }
    }
    fn begin_input(&self, range: Option<Range<usize>>, window: &Window) -> Option<Capture> {
        if !self.focus.is_focused(window) { return None; }
        let (tab, _, selected) = self.draft_target()?;
        let view = self.session.state().view(tab)?; let base = view.snapshot().clone();
        let range = match range { Some(range) => source_range(&base, range)?, None => selected };
        Some(Capture { tab, pane: view.tab().pane(), selection: view.tab().selection(), base, context: self.input_context(window),
            selected: Selection { anchor: range.start, head: range.start }, range, text: String::new() })
    }
    fn show_input(&mut self, capture: Capture, cx: &mut Context<Self>) {
        if self.input_state.capture.is_none() {
            self.doc_rest = (None, None); self.doc_grace = None; self.doc_popup = None;
            self.session.dispatch(Command::CloseDocumentation);
        }
        let command = Command::PresentText { tab: capture.tab, base: PresentationSource::new(&capture.base), base_selection: capture.selection,
            edit: TextEdit { range: capture.range.clone(), text: capture.text.clone().into() }, selected: capture.selected };
        self.input_state.capture = Some(capture); self.dispatch(command, cx);
    }
    fn commit_input(&mut self, cx: &mut Context<Self>) {
        let Some(capture) = self.input_state.capture.take() else { return };
        if !self.capture_source_current(&capture) { self.input_state.stale = true; self.session.dispatch(Command::ClearPresentation); return; }
        self.session.dispatch(Command::ClearPresentation);
        self.dispatch(Command::Edit { tab: capture.tab, expected: capture.base.key(), edits: vec![TextEdit { range: capture.range, text: capture.text.into() }], kind: EditKind::Replace }, cx);
        // IME selection is in the same virtual frame, which is now the committed source.
        self.dispatch(Command::Select(capture.selected), cx);
        self.input_state.ended = self.ended_context(capture.context);
        self.input_state.stale = true; // terminal callbacks stay consumed; a validated new mark begins a new phase
    }
    /// Positive native-input oracle; bounded and absent after cancellation or source invalidation.
    #[doc(hidden)]
    #[cfg(feature = "test-support")]
    pub fn marked_text_for_test(&self) -> Option<(Range<usize>, String)> {
        let capture = self.input_state.capture.as_ref().filter(|capture| self.capture_source_current(capture))?;
        let source = self.session.state().presentation_view(capture.tab)?.snapshot();
        let marked = self.marked_bytes()?;
        let mut end = capture.text.len().min(SURROUNDING_BYTES);
        while !capture.text.is_char_boundary(end) { end -= 1; }
        Some((source.byte_to_utf16(marked.start)?..source.byte_to_utf16(marked.end)?, capture.text[..end].to_owned()))
    }
    pub(super) fn marked_bytes(&self) -> Option<Range<usize>> {
        let capture = self.input_state.capture.as_ref()?;
        Some(capture.range.start..capture.range.start + capture.text.len())
    }
    fn local_marked_range(&self, capture: &Capture, range: Option<Range<usize>>, marking: bool, window: &Window) -> Option<Range<usize>> {
        let Some(range) = range else { return Some(0..capture.text.len()) };
        #[cfg(target_os = "macos")]
        {
            // AppKit setMarkedText replacementRange is relative to the marked text. insertText
            // accepts that marked region, matching the pinned editor's platform interpretation.
            let _ = window;
            if marking { utf16_bytes(&capture.text, range) }
            else { let _ = utf16_bytes(&capture.text, range)?; Some(0..capture.text.len()) }
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = marking;
            let bytes = source_range(self.input_snapshot(window)?.as_ref(), range)?;
            let start = bytes.start.checked_sub(capture.range.start)?;
            let end = bytes.end.checked_sub(capture.range.start)?;
            (end <= capture.text.len()).then_some(start..end)
        }
    }
    /// A fresh raw input boundary can start a new transaction after an obsolete terminal callback.
    /// Printable text is delivered by GPUI's registered handler; shortcut commands stay in21's path.
    pub(super) fn input_key(&mut self, event: &KeyDownEvent, window: &Window, cx: &mut Context<Self>) -> bool {
        self.synchronize_input(window, cx);
        let key = &event.keystroke;
        if key.key == "escape" && self.input_state.capture.is_some() { self.cancel_marked_input(cx); cx.stop_propagation(); return true; }
        let printable = key.key_char.as_deref().is_some_and(|text| !text.chars().any(char::is_control))
            && !key.modifiers.control && !key.modifiers.platform && !key.modifiers.alt;
        if printable && self.draft_context() && self.input_state.registered.get() {
            self.input_state.stale = false; self.input_state.ended = None;
            return true; // do not stop propagation: Window dispatches the committed text exactly once
        }
        if self.input_state.capture.is_some() {
            if key.key == "enter" { self.commit_input(cx); cx.stop_propagation(); return true; }
            if matches!(key.key.as_str(), "backspace" | "delete") { self.cancel_input(true, cx); cx.stop_propagation(); return true; }
            let movement = matches!(key.key.as_str(), "left" | "right" | "up" | "down" | "home" | "end" | "pageup" | "pagedown" | "tab" | "f1" | "f2" | "f10");
            if movement || key.modifiers.control || key.modifiers.platform || key.modifiers.alt { self.cancel_input(true, cx); }
            else { return true; } // XKB/XIM preedit is followed by a non-character dead-key event
        }
        false
    }
}
impl EntityInputHandler for ReaderView {
    fn text_for_range(&mut self, range: Range<usize>, adjusted: &mut Option<Range<usize>>, window: &mut Window, cx: &mut Context<Self>) -> Option<String> {
        self.synchronize_input(window, cx); let source = self.input_snapshot(window)?;
        // Validate both native endpoints before bounding allocation, including surrogate interiors.
        let bytes = source_range(&source, range)?;
        let caret = self.input_state.capture.as_ref().map(|capture| capture.selected.head)
            .or_else(|| self.session.state().selection().map(|selection| selection.head))
            .or_else(|| self.draft_target().map(|(_, _, range)| range.start))?;
        let start = source.floor_char_boundary(caret.saturating_sub(SURROUNDING_BYTES / 2).max(bytes.start).min(bytes.end));
        let end = source.floor_char_boundary(start.saturating_add(SURROUNDING_BYTES).min(bytes.end));
        *adjusted = Some(source.byte_to_utf16(start)?..source.byte_to_utf16(end)?);
        source.copy_range(start..end)
    }
    fn selected_text_range(&mut self, _: bool, window: &mut Window, cx: &mut Context<Self>) -> Option<UTF16Selection> {
        self.synchronize_input(window, cx); let source = self.input_snapshot(window)?;
        let selected = self.input_state.capture.as_ref().map(|capture| capture.selected).or_else(|| self.session.state().selection())
            .or_else(|| self.draft_target().map(|(_, _, range)| Selection { anchor: range.start, head: range.end }))?;
        Some(UTF16Selection { range: source.byte_to_utf16(selected.range().start)?..source.byte_to_utf16(selected.range().end)?, reversed: selected.head < selected.anchor })
    }
    fn marked_text_range(&self, window: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        let source = self.input_snapshot(window)?; let range = self.marked_bytes()?;
        Some(source.byte_to_utf16(range.start)?..source.byte_to_utf16(range.end)?)
    }
    fn unmark_text(&mut self, window: &mut Window, cx: &mut Context<Self>) { self.synchronize_input(window, cx); self.commit_input(cx); }
    fn replace_text_in_range(&mut self, range: Option<Range<usize>>, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.synchronize_input(window, cx); if self.input_state.stale { return; }
        if let Some(mut capture) = self.input_state.capture.clone() {
            #[cfg(not(target_os = "macos"))]
            if text.is_empty() && range.as_ref().is_some_and(|range| self.marked_text_range(window, cx).as_ref() == Some(range)) {
                self.cancel_marked_input(cx); return; // pinned Linux/XIM cancellation, not an empty commit
            }
            let Some(local) = self.local_marked_range(&capture, range, false, window) else { return };
            let caret = capture.range.start + local.start + text.len(); capture.text.replace_range(local, text);
            capture.selected = Selection { anchor: caret, head: caret };
            self.input_state.capture = Some(capture); self.commit_input(cx);
        } else if let Some(capture) = self.begin_input(range.clone(), window) {
            if range.is_none() {
                // Plain typing changes every selection of the view; the core makes the whole set one transaction.
                self.dispatch(Command::InsertText { text: text.to_owned(), kind: EditKind::Typing }, cx);
            } else {
                self.dispatch(Command::Edit { tab: capture.tab, expected: capture.base.key(), edits: vec![TextEdit { range: capture.range, text: text.into() }], kind: EditKind::Replace }, cx);
            }
        }
    }
    fn replace_and_mark_text_in_range(&mut self, range: Option<Range<usize>>, text: &str, selected: Option<Range<usize>>, window: &mut Window, cx: &mut Context<Self>) {
        self.synchronize_input(window, cx);
        if self.input_state.stale && !(range.is_none() && self.input_state.ended.as_ref().is_some_and(|ended| self.ended_current(ended, window))) { return; }
        let Some(relative_selection) = utf16_bytes(text, selected.unwrap_or_else(|| { let end = text.encode_utf16().count(); end..end })) else { return };
        let active = self.input_state.capture.is_some();
        let Some(mut capture) = self.input_state.capture.clone().or_else(|| self.begin_input(range.clone(), window)) else { return };
        let local = if active { match self.local_marked_range(&capture, range, true, window) { Some(range) => range, None => return } } else { 0..0 };
        let at = capture.range.start + local.start;
        capture.text.replace_range(local, text);
        capture.selected = Selection { anchor: at + relative_selection.start, head: at + relative_selection.end };
        self.input_state.stale = false; self.input_state.ended = None;
        self.show_input(capture, cx);
    }
    fn bounds_for_range(&mut self, range: Range<usize>, _: Bounds<Pixels>, window: &mut Window, cx: &mut Context<Self>) -> Option<Bounds<Pixels>> {
        self.synchronize_input(window, cx); let source = self.input_snapshot(window)?; let bytes = source_range(&source, range)?;
        let state = self.session.state(); let view = state.presentation_view(state.active()?.id())?; let tab = view.tab();
        if !self.current_source_geometry(tab) || window.viewport_size() != gpui::size(px(self.width), px(self.height)) { return None; }
        let geometry = self.panes.iter().find(|pane| pane.pane == tab.pane())?;
        let rows = view.layout_rows(geometry.rows, state.wrap_cols(tab)); if rows.pending.is_some() { return None; }
        let (at, row) = rows.rows.iter().enumerate().find(|(_, row)| row.source.start <= bytes.start && (bytes.start < row.end_byte() || row.last && bytes.start == row.end_byte()))?;
        let projected = self.ready_source_row(tab, row)?; let local = projected.caret(bytes.start)?;
        let start = projected.x_for(window, self.metrics(), local)?;
        let end = projected.caret(bytes.end).and_then(|byte| projected.x_for(window, self.metrics(), byte)).unwrap_or(start);
        let x = lenz_gpui::text_x(self.metrics(), f32::from(geometry.text.origin.x), tab.column() as f32 * self.char_w) + row.indent as f32 * self.char_w + start;
        let y = f32::from(geometry.text.origin.y) + at as f32 * line_h();
        if x < f32::from(geometry.text.left()) + num_w() + 4.0 || x >= f32::from(geometry.text.right())
            || y < f32::from(geometry.text.top()) || y >= f32::from(geometry.text.bottom()) { return None; }
        Some(Bounds { origin: gpui::point(px(x), px(y)), size: gpui::size(px((end - start).max(2.0)), px(line_h())) })
    }
    fn character_index_for_point(&mut self, point: gpui::Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) -> Option<usize> {
        self.synchronize_input(window, cx); let source = self.input_snapshot(window)?;
        source.byte_to_utf16(self.byte_at(window, f32::from(point.x), f32::from(point.y))?)
    }
    fn text_length_utf16(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<usize> {
        self.synchronize_input(window, cx); Some(self.input_snapshot(window)?.len_utf16())
    }
    fn accepts_text_input(&self, window: &mut Window, _: &mut Context<Self>) -> bool {
        self.focus.is_focused(window) && self.draft_target().is_some()
            && self.input_state.capture.as_ref().is_none_or(|capture| self.capture_current(capture, window))
    }
    fn set_selected_text_range(&mut self, range: Range<usize>, window: &mut Window, cx: &mut Context<Self>) {
        self.synchronize_input(window, cx); let Some(source) = self.input_snapshot(window) else { return };
        let Some(range) = source_range(&source, range) else { return }; let selected = Selection { anchor: range.start, head: range.end };
        if let Some(mut capture) = self.input_state.capture.clone() { capture.selected = selected; self.show_input(capture, cx); }
        else { self.dispatch(Command::Select(selected), cx); }
    }
    fn paste(&mut self, item: ClipboardItem, window: &mut Window, cx: &mut Context<Self>) {
        self.synchronize_input(window, cx); if !self.draft_context() { return; }
        self.cancel_input(true, cx); if let Some(text) = item.text() { self.dispatch(Command::Paste(text), cx); }
    }
}
