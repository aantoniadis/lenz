//! Popup text is edited by the shared field model; this module routes results back to the existing workflows.
use super::*;
use lenz_content::field_edit::Cursor;
use std::hash::{Hash, Hasher};

impl ReaderView {
    pub(super) fn popup_field_key(&self) -> Option<u64> {
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        std::mem::discriminant(self.modal.as_ref()?).hash(&mut hash);
        Some(hash.finish())
    }
    pub(super) fn popup_field(&self) -> Option<(&str, Cursor, usize)> {
        let m = self.modal.as_ref()?;
        let (text, caret, selected, field) = match m {
            Modal::OpenPath { input, .. } => return Some((input.text(), input.cursor(), 0)),
            Modal::Navigation { caret, .. } => (
                self.session.state().navigation()?.query.as_str(),
                *caret,
                false,
                0,
            ),
            Modal::Search {
                query,
                files,
                in_files,
                caret,
                selected,
                ..
            } => (
                if *in_files {
                    files.as_str()
                } else {
                    query.as_str()
                },
                *caret,
                *selected,
                usize::from(*in_files),
            ),
            Modal::SaveAs {
                input, selected, ..
            }
            | Modal::WorkspaceInput { input, selected, .. }
            | Modal::RenameTerminal {
                input, selected, ..
            } => (input.as_str(), None, *selected, 0),
            Modal::PasteEntry { name, selected, .. } => (name.as_str(), None, *selected, 0),
            Modal::Create { name, .. } | Modal::NewBranch { name, .. } => {
                (name.as_str(), None, false, 0)
            }
            Modal::Revision { input, .. }
            | Modal::GoTo { input, .. }
            | Modal::Relocate { input, .. } => (input.as_str(), None, false, 0),
            Modal::Root { path }
            | Modal::SqlMetadata { path }
            | Modal::ServerRuntime { path, .. } => (path.as_str(), None, false, 0),
            Modal::Branches { query, .. }
            | Modal::Places { query, .. }
            | Modal::NewScratch { query, .. }
            | Modal::Siblings { query, .. }
            | Modal::Declarations { query, .. } => (query.as_str(), None, false, 0),
            _ => return None,
        };
        let initial = Cursor::at(caret, selected, text);
        let cursor = self
            .popup_fields
            .cursor(self.popup_field_key()?, field)
            .unwrap_or(initial);
        Some((text, cursor, field))
    }
    pub(super) fn popup_text_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        if self.field_alt_action(ev) { return false; }
        let Some((text, _, _)) = self.popup_field() else {
            return false;
        };
        let k = &ev.keystroke;
        // Row actions remain accessible with an empty filter; with text present Delete/Right edit that text.
        let row_action = text.is_empty()
            && !k.modifiers.control
            && !k.modifiers.platform
            && !k.modifiers.alt
            && !k.modifiers.shift
            && (k.key == "right" && matches!(self.modal, Some(Modal::Branches { .. }))
                || k.key == "delete"
                    && matches!(
                        self.modal,
                        Some(Modal::Places { .. } | Modal::Navigation { .. })
                    ));
        if row_action {
            return false;
        }
        let Some(command) = lenz_gpui::field::command(ev, cx) else {
            return false;
        };
        self.popup_text_command(command, cx)
    }
    pub(super) fn popup_text_command(&mut self, command: lenz_content::field_edit::Command, cx: &mut Context<Self>) -> bool {
        let Some((_, cursor, field)) = self.popup_field() else { return false };
        let key = self.popup_field_key().unwrap_or_default();
        let command = command.first_line();
        let outcome;
        if matches!(self.modal, Some(Modal::Navigation { .. })) {
            let Some(search) = self.session.state().navigation() else {
                return true;
            };
            let (scope, mut query) = (search.scope, search.query.clone());
            outcome = self
                .popup_fields
                .edit(key, field, cursor, &mut query, command);
            let caret = self.popup_fields.cursor(key, field).and_then(|c| c.caret);
            if outcome.changed {
                self.update_navigation(scope, query, caret, cx);
            } else if let Some(Modal::Navigation { caret: at, .. }) = &mut self.modal {
                *at = caret;
            }
        } else if let Some(Modal::OpenPath { input, error, .. }) = &mut self.modal {
            outcome = input.command(command);
            if outcome.changed {
                *error = None;
            }
        } else {
            let Some(modal) = self.modal.as_mut() else {
                return false;
            };
            let text = match modal {
                Modal::Search {
                    query,
                    files,
                    in_files,
                    ..
                } => {
                    if *in_files {
                        files
                    } else {
                        query
                    }
                }
                Modal::SaveAs { input, .. }
                | Modal::WorkspaceInput { input, .. }
                | Modal::RenameTerminal { input, .. }
                | Modal::Revision { input, .. }
                | Modal::GoTo { input, .. }
                | Modal::Relocate { input, .. } => input,
                Modal::PasteEntry { name, .. }
                | Modal::Create { name, .. }
                | Modal::NewBranch { name, .. } => name,
                Modal::Root { path }
                | Modal::SqlMetadata { path }
                | Modal::ServerRuntime { path, .. } => path,
                Modal::Branches { query, .. }
                | Modal::Places { query, .. }
                | Modal::NewScratch { query, .. }
                | Modal::Siblings { query, .. }
                | Modal::Declarations { query, .. } => query,
                _ => return false,
            };
            outcome = self.popup_fields.edit(key, field, cursor, text, command);
            let cursor = self.popup_fields.cursor(key, field).unwrap_or_default();
            let all_selected = cursor.all_selected(text);
            match modal {
                Modal::Search {
                    selected, caret, ..
                } => {
                    *selected = all_selected;
                    *caret = cursor.caret;
                }
                Modal::SaveAs { selected, .. }
                | Modal::WorkspaceInput { selected, .. }
                | Modal::RenameTerminal { selected, .. }
                | Modal::PasteEntry { selected, .. } => *selected = all_selected,
                _ => {}
            }
            if outcome.changed {
                match modal {
                    Modal::Search { .. } => {}
                    Modal::SaveAs { error, .. }
                    | Modal::WorkspaceInput { error, .. }
                    | Modal::RenameTerminal { error, .. }
                    | Modal::PasteEntry { error, .. }
                    | Modal::Revision { error, .. }
                    | Modal::GoTo { error, .. }
                    | Modal::Relocate { error, .. }
                    | Modal::Create { error, .. }
                    | Modal::NewBranch { error, .. } => *error = None,
                    Modal::ServerRuntime { at, .. }
                    | Modal::Branches { at, .. }
                    | Modal::Places { at, .. }
                    | Modal::NewScratch { at, .. }
                    | Modal::Siblings { at, .. } => *at = 0,
                    Modal::Declarations { query, semantic } => {
                        let command = if *semantic {
                            Command::Symbols(query.clone())
                        } else {
                            Command::FindDeclarations(query.clone())
                        };
                        self.results_scroll = 0;
                        self.dispatch(command, cx);
                    }
                    _ => {}
                }
            }
        }
        self.field_native_notify |= if outcome.changed { 3 } else { 2 };
        if let Some(text) = outcome.copy {
            self.dispatch(Command::CopyText(text), cx);
        }
        cx.notify();
        true
    }
    pub(super) fn field_find_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        if self.field_alt_action(ev) { return false; }
        let Some(command) = lenz_gpui::field::command(ev, cx) else {
            return false;
        };
        self.field_find_command(command, cx)
    }
    pub(super) fn field_find_command(&mut self, command: lenz_content::field_edit::Command, cx: &mut Context<Self>) -> bool {
        let command = command.first_line();
        let Some(tab) = self.session.state().active() else { return false };
        let key = tab.id().0;
        let Some(find) = tab.find() else {
            return false;
        };
        let mut text = if self.find_replace {
            self.session.replace_text().to_string()
        } else {
            find.query().to_string()
        };
        let field = usize::from(self.find_replace);
        let result = self
            .find_fields
            .edit(key, field, Cursor::default(), &mut text, command);
        self.field_native_notify |= if result.changed { 3 } else { 2 };
        if let Some(value) = result.copy {
            self.dispatch(Command::CopyText(value), cx);
        }
        if result.changed {
            self.dispatch(
                if self.find_replace {
                    Command::SetReplaceText(text)
                } else {
                    Command::Find(text)
                },
                cx,
            );
        }
        cx.notify();
        true
    }
}

/// Native callbacks are owned by exactly the field that had the keys when they began.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Target {
    Popup(u64, usize),
    Find(TabId, usize),
}

impl ReaderView {
    pub(super) fn field_source(&self) -> Option<(Target, &str, Cursor)> {
        if let Some((text, cursor, field)) = self.popup_field() {
            return Some((Target::Popup(self.popup_field_key()?, field), text, cursor));
        }
        if self.modal.is_some() || self.crumb_focus.is_some() { return None; }
        let state = self.session.state();
        let tab = state.active()?;
        if self.find_focus != Some(state.focused_pane()) { return None; }
        let find = tab.find()?;
        let field = usize::from(self.find_replace);
        let text = if self.find_replace { self.session.replace_text() } else { find.query() };
        Some((Target::Find(tab.id(), field), text, self.find_fields.cursor(tab.id().0, field).unwrap_or_default()))
    }
    pub(super) fn field_target(&self) -> Option<Target> { self.field_source().map(|(target, _, _)| target) }
    pub(super) fn cancel_field_native(&mut self) {
        if self.field_native_target.is_some() || self.field_native.is_marked() { self.field_native_notify |= 3; }
        self.field_native_stale |= self.field_native.is_marked() || self.field_native_target.is_some();
        self.field_native.cancel();
        self.field_native_suppress = None;
        self.field_native_target = None;
    }
    pub(super) fn synchronize_field_native(&mut self) {
        let target = self.field_target();
        if target != self.field_native_target {
            self.cancel_field_native();
            self.field_native_target = target;
            self.field_native_notify |= 3;
        }
    }
    pub(super) fn with_field_native<R>(&mut self, f: impl FnOnce(&mut lenz_content::field_edit::Native, &str, Cursor) -> R) -> Option<R> {
        self.synchronize_field_native();
        if self.field_native_stale { return None; }
        let mut native = std::mem::take(&mut self.field_native);
        let result = self.field_source().map(|(_, text, cursor)| f(&mut native, text, cursor));
        self.field_native = native;
        result
    }
    pub(super) fn apply_field_command(&mut self, command: lenz_content::field_edit::Command, cx: &mut Context<Self>) -> bool {
        self.field_native.cancel();
        match self.field_target() {
            Some(Target::Popup(..)) => self.popup_text_command(command, cx),
            Some(Target::Find(..)) => self.field_find_command(command, cx),
            None => false,
        }
    }
    pub(super) fn popup_field_snapshot(&self, columns: usize) -> Option<lenz_content::field_edit::Snapshot> {
        let (text, cursor, _) = self.popup_field()?;
        Some(if self.field_native_target == self.field_target() {
            self.field_native.snapshot(text, cursor, columns)
        } else { cursor.snapshot(text, columns) })
    }
    pub(super) fn field_paint_snapshot(&self, field: usize, text: &str, cursor: Cursor, columns: usize) -> lenz_content::field_edit::Snapshot {
        if self.field_native_target == self.field_target() && self.field_source().is_some_and(|(target, _, _)| match target { Target::Popup(_, active) | Target::Find(_, active) => active == field }) {
            self.field_native.snapshot(text, cursor, columns)
        } else { cursor.snapshot(text, columns) }
    }
    fn field_alt_action(&self, event: &KeyDownEvent) -> bool {
        let k = &event.keystroke;
        k.modifiers.alt && !k.modifiers.control && !k.modifiers.platform && match self.modal {
            Some(Modal::Search { .. }) => matches!(k.key.as_str(), "c" | "w" | "r"),
            None if self.find_focus.is_some() => matches!(k.key.as_str(), "c" | "w" | "r" | "a" | "enter"),
            _ => false,
        }
    }
    pub(super) fn finish_field_mark(&mut self, cx: &mut Context<Self>) {
        let target = self.field_target();
        if let Some(command) = self.with_field_native(|native, text, cursor| native.unmark(text, cursor)).flatten() {
            self.apply_field_command(command, cx);
            self.field_native_suppress = target;
        }
    }
    pub(super) fn replace_field_native(&mut self, range: Option<std::ops::Range<usize>>, text: &str, cx: &mut Context<Self>) {
        if self.field_native_suppress.as_ref().is_some_and(|target| Some(*target) == self.field_target()) { return; }
        let text = text.split(['\r', '\n']).next().unwrap_or_default();
        let ended = self.field_native.is_marked();
        let target = self.field_target();
        if let Some(command) = self.with_field_native(|native, current, cursor| native.commit(current, cursor, range, text, cfg!(target_os = "macos"))).flatten() {
            self.apply_field_command(command, cx);
            if ended { self.field_native_suppress = target; }
        }
    }
    pub(super) fn field_native_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.synchronize_field_native();
        if self.field_target().is_none() {
            // A new raw event belongs to the current document/terminal, never an ended field callback.
            self.field_native_stale = false; self.field_native_suppress = None;
            return false;
        }
        if self.field_alt_action(event) { self.cancel_field_native(); cx.stop_propagation(); return false; }
        let key = &event.keystroke;
        if self.field_native.is_marked() && self.field_source().is_none_or(|(_, text, cursor)| self.field_native.marked_range(text, cursor).is_none()) {
            // A real key starts a new phase after an application-driven replacement invalidated preedit.
            self.cancel_field_native(); self.field_native_target = self.field_target();
        }
        if lenz_gpui::field::defer_uncommitted_key(self.input_state.registered.get(), key) && self.focus.is_focused(window) {
            // XKB can publish preedit before a physical dead key with no committed key_char. Its name is not text.
            self.field_native_stale = false; self.field_native_suppress = None;
            return true;
        }
        if self.field_native.is_marked() {
            match key.key.as_str() {
                "escape" => { self.cancel_field_native(); cx.stop_propagation(); cx.notify(); return true; }
                "enter" => {
                    self.finish_field_mark(cx);
                    cx.stop_propagation(); return true;
                }
                "backspace" | "delete" => { self.cancel_field_native(); cx.stop_propagation(); cx.notify(); return true; }
                "left" | "right" | "up" | "down" | "home" | "end" | "tab" => self.cancel_field_native(),
                _ if key.modifiers.control || key.modifiers.platform => self.cancel_field_native(),
                _ => {}
            }
        }
        let printable = (!key.modifiers.control && !key.modifiers.platform && key.key_char.is_some())
            .then(|| lenz_gpui::field::command(event, cx)).flatten();
        if let Some(lenz_content::field_edit::Command::Insert(value)) = printable
            && self.input_state.registered.get() && self.focus.is_focused(window)
        {
            self.field_native_stale = false;
            self.field_native_suppress = None;
            if key.modifiers.alt && !lenz_gpui::field::defer_option_input(true, cfg!(target_os = "macos"), self.field_native.is_marked()) {
                // Synthetic/non-macOS Option text has no AppKit input context to commit it.
                self.apply_field_command(lenz_content::field_edit::Command::Insert(value), cx);
                cx.stop_propagation();
            }
            // Registered macOS Option input must reach Cocoa: a spacing accent in key_char may be a dead key.
            return true; // Native text commits through the handler exactly once.
        }
        if self.field_native.is_marked() && key.key_char.is_none() { return true; }
        // A fresh dead-key phase can begin composition without committed key text.
        if key.key_char.is_none() && !key.modifiers.control && !key.modifiers.platform
            && !matches!(key.key.as_str(), "left" | "right" | "up" | "down" | "home" | "end" | "tab" | "escape" | "enter" | "backspace" | "delete" | "pageup" | "pagedown") {
            self.field_native_stale = false; self.field_native_suppress = None;
        }
        false
    }
    /// Pixel bounds use the same padding and horizontal viewport as the field painter.
    pub(super) fn field_bounds(&self) -> Option<Bounds<Pixels>> {
        self.field_source()?;
        if let Some(l) = self.form_layout() && let Some(b) = l.field {
            return Some(Bounds { origin: gpui::point(b.origin.x + px(8.0), b.origin.y + px(4.0)), size: gpui::size(px((f32::from(b.size.width) - 20.0).max(0.0)), px(line_h())) });
        }
        if let Some(l) = self.search_layout() && let Some(Modal::Search { in_files, .. }) = &self.modal {
            let b = l.fields[usize::from(*in_files)];
            return Some(Bounds { origin: gpui::point(b.origin.x + px(8.0), b.origin.y + px(4.0)), size: gpui::size(px((f32::from(b.size.width) - 20.0).max(0.0)), px(line_h())) });
        }
        if matches!(self.modal, Some(Modal::Navigation { .. })) {
            let b = self.navigation_bounds();
            return Some(Bounds { origin: gpui::point(b.origin.x + px(12.0 + 2.0 * self.char_w), b.origin.y + px(4.0 + line_h())), size: gpui::size(px((f32::from(b.size.width) - 36.0 - 2.0 * self.char_w).max(0.0)), px(line_h())) });
        }
        if let Some(l) = self.modal_layout() {
            if l.query.is_some() {
                return Some(Bounds { origin: gpui::point(l.bounds.origin.x + px(20.0), l.bounds.origin.y + px(line_h() + 24.0)), size: gpui::size(px((f32::from(l.bounds.size.width) - 44.0).max(0.0)), px(line_h())) });
            }
            if l.first == 0 {
                return Some(Bounds { origin: gpui::point(l.bounds.origin.x + px(12.0 + 2.0 * self.char_w), l.bounds.origin.y + px(l.body_top)), size: gpui::size(px((f32::from(l.bounds.size.width) - 24.0 - 2.0 * self.char_w).max(0.0)), px(line_h())) });
            }
            return None;
        }
        let pane = self.session.state().focused_pane();
        let b = self.find_box(self.panes.iter().find(|g| g.pane == pane)?)?;
        if self.find_replace {
            let r = b.replace_row;
            let width = f32::from(b.bounds.size.width);
            let label = if width < b.replace_label.chars().count() as f32 * self.char_w + 6.0 * self.char_w + 20.0 { "↦" } else { b.replace_label.as_str() };
            let x = f32::from(b.bounds.origin.x) + 12.0 + label.chars().count() as f32 * self.char_w;
            Some(Bounds { origin: gpui::point(px(x), r.origin.y + px(2.0)), size: gpui::size(px((f32::from(b.bounds.right()) - 8.0 - x).max(0.0)), r.size.height) })
        } else { Some(b.query_bounds(gpui::point(px(0.0), px(0.0)), self.char_w)) }
    }
    pub(super) fn field_click(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) -> bool {
        // Activate either search row before resolving its actual text geometry.
        if let Some(l) = self.search_layout() && let Some(i) = l.fields.iter().position(|b| b.contains(&event.position))
            && let Some(Modal::Search { in_files, .. }) = &mut self.modal { *in_files = i == 1; }
        if self.modal.is_none() && let Some((pane, replace)) = self.panes.iter().find_map(|g| self.find_box(g).filter(|b| b.bounds.contains(&event.position) && b.option_at(event.position).is_none()).map(|b| (g.pane, b.replace_at(event.position)))) {
            if pane != self.session.state().focused_pane() { self.dispatch(Command::FocusPane(pane), cx); }
            self.find_focus = Some(pane); self.find_replace = replace;
        }
        let Some(bounds) = self.field_bounds().filter(|b| b.contains(&event.position)) else { return false };
        self.cancel_field_native();
        let Some((_, text, cursor)) = self.field_source() else { return false };
        let at = lenz_gpui::field::hit_test(lenz_gpui::field::View { text, cursor, bounds, char_w: self.char_w, active: true }, event.position, window, self.metrics());
        let anchor = if event.modifiers.shift { cursor.anchor.unwrap_or_else(|| cursor.caret(text)) } else { at };
        self.apply_field_command(lenz_content::field_edit::Command::Select { anchor, caret: at }, cx);
        true
    }
}
