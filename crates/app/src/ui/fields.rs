//! Native text transport and focused field editing for Diff. Core field commands own every mutation.
use super::*;
use crate::field_edit::{Cursor, Snapshot};
use gpui::{EntityInputHandler, Point, UTF16Selection};

impl Viewer {
    fn field_alt_action(&self, event: &KeyDownEvent) -> bool {
        let k = &event.keystroke;
        k.modifiers.alt
            && !k.modifiers.control
            && !k.modifiers.platform
            && match self.modal() {
                Some(Modal::Find | Modal::MergeFind) => matches!(k.key.as_str(), "c" | "w" | "r"),
                Some(Modal::SearchEditing) => matches!(k.key.as_str(), "c" | "w" | "r" | "o"),
                _ => false,
            }
    }
    pub(super) fn field_input_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        if self.native_field().is_none() {
            return false;
        }
        if self.field_alt_action(event) {
            return false;
        }
        let Some(command) = lenz_gpui::field::command(event, cx) else {
            return false;
        };
        self.field_command(command, cx)
    }
    pub(super) fn field_command(
        &mut self,
        mut command: crate::field_edit::Command,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.native_field().is_none() {
            return false;
        }
        let modal = self.modal();
        self.field_native_changes |= 2;
        self.field_native.cancel();
        self.field_native_suppress = None;
        if self.field_input_modal != modal {
            self.field_inputs.clear();
            self.field_input_modal = modal;
        }
        if modal.is_none() && self.native_field().is_some() {
            let Some(repo) = self.repo.as_ref() else {
                return false;
            };
            let mut query = repo.picker.filter.clone();
            let out = self
                .field_inputs
                .edit(u64::MAX, 0, Cursor::default(), &mut query, command);
            if out.changed {
                self.field_native_changes |= 1;
                self.repository_command(
                    crate::repository_session::RepositoryCommand::Filter(query),
                    cx,
                );
            }
            if let Some(text) = out.copy {
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
            }
            cx.notify();
            return true;
        }
        let Some(kind) = modal else { return false };
        if self.native_field().is_none() {
            return false;
        }
        if kind == Modal::GitPrompt {
            command = command.first_line();
        }
        if kind == Modal::Palette {
            if let Some(p) = &mut self.command_palette {
                let result = p.edit(command);
                if result.changed {
                    self.field_native_changes |= 1;
                }
                if let Some(text) = result.copy {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
                }
                cx.notify();
                return true;
            }
            return false;
        }
        let result;
        if kind == Modal::MergeFind {
            let Some(merge) = self.merge.as_mut() else {
                return false;
            };
            result = merge.edit_find(command);
        } else {
            let text = match kind {
                Modal::GoTo => self.goto.as_mut(),
                Modal::GitPrompt => self.git_prompt.as_mut().map(|p| match p {
                    checkout::GitPrompt::NewBranch { name, .. } => name,
                    checkout::GitPrompt::Revision { input, .. } => input,
                }),
                Modal::Switcher => self.switcher.as_mut().map(|s| &mut s.query),
                Modal::Find => self.comparison.find.as_mut().map(|f| &mut f.query),
                Modal::SearchEditing => self.search.as_mut().map(|s| &mut s.query),
                _ => None,
            };
            let Some(text) = text else { return false };
            result = self
                .field_inputs
                .edit(kind as u64, 0, Cursor::default(), text, command);
            if result.changed {
                match kind {
                    Modal::Switcher => self.update_switcher(),
                    Modal::Find => self.refind(),
                    Modal::SearchEditing => {
                        if let Some(s) = &mut self.search {
                            let q = s.query.clone();
                            s.execute(
                                crate::search_session::SearchCommand::Query(q),
                                &self.set,
                                PANEL_ROWS,
                            );
                        }
                    }
                    Modal::GitPrompt => {
                        if let Some(
                            checkout::GitPrompt::NewBranch { error, .. }
                            | checkout::GitPrompt::Revision { error, .. },
                        ) = &mut self.git_prompt
                        {
                            *error = None;
                        }
                    }
                    _ => {}
                }
            }
        }
        if let Some(text) = result.copy {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
        }
        if result.changed {
            self.field_native_changes |= 1;
        }
        cx.notify();
        true
    }
    pub(super) fn native_field(&self) -> Option<(u64, &str, Cursor)> {
        let Some(kind) = self.modal() else {
            let p = &self.repo.as_ref()?.picker;
            return (!self.nav_focused && p.shown && p.focus == Pane::Log && p.filter_editing)
                .then(|| {
                    (
                        u64::MAX,
                        p.filter.as_str(),
                        self.field_inputs.cursor(u64::MAX, 0).unwrap_or_default(),
                    )
                });
        };
        if kind == Modal::Find
            && (self.nav_focused
                || self
                    .repo
                    .as_ref()
                    .is_some_and(|r| r.picker.shown && r.picker.focus != Pane::Diff))
        {
            return None;
        }
        let (text, cursor) = match kind {
            Modal::Palette => {
                let p = self.command_palette.as_ref()?;
                (p.query(), p.input().cursor())
            }
            Modal::GoTo => (
                self.goto.as_deref()?,
                self.field_inputs.cursor(kind as u64, 0).unwrap_or_default(),
            ),
            Modal::GitPrompt => (
                match self.git_prompt.as_ref()? {
                    checkout::GitPrompt::NewBranch { name, .. } => name.as_str(),
                    checkout::GitPrompt::Revision { input, .. } => input.as_str(),
                },
                self.field_inputs.cursor(kind as u64, 0).unwrap_or_default(),
            ),
            Modal::Switcher => (
                self.switcher.as_ref()?.query.as_str(),
                self.field_inputs.cursor(kind as u64, 0).unwrap_or_default(),
            ),
            Modal::Find => (
                self.comparison.find.as_ref()?.query.as_str(),
                self.field_inputs.cursor(kind as u64, 0).unwrap_or_default(),
            ),
            Modal::SearchEditing => (
                self.search.as_ref()?.query.as_str(),
                self.field_inputs.cursor(kind as u64, 0).unwrap_or_default(),
            ),
            Modal::MergeFind => {
                let m = self.merge.as_ref()?;
                (m.find_query()?, m.find_cursor())
            }
            _ => return None,
        };
        Some((kind as u64, text, cursor))
    }
    pub(super) fn sync_native_field(&mut self) {
        let owner = self.native_field().map(|f| f.0);
        if self.field_native_owner != owner {
            self.field_inputs.clear();
            self.field_input_modal = self.modal();
            self.field_native_changes |= 3;
            self.field_native_stale |=
                self.field_native_owner.is_some() || self.field_native.is_marked();
            self.field_native.cancel();
            self.field_native_suppress = None;
            self.field_native_owner = owner;
        }
    }
    pub(super) fn projected_field(
        &self,
        key: u64,
        text: &str,
        cursor: Cursor,
        columns: usize,
    ) -> Snapshot {
        if self.native_field().is_some_and(|f| f.0 == key) {
            self.field_native.snapshot(text, cursor, columns)
        } else {
            cursor.snapshot(text, columns)
        }
    }
    pub(super) fn field_snapshot(&self, columns: usize) -> Option<Snapshot> {
        let (_, text, cursor) = self.native_field()?;
        Some(self.field_native.snapshot(text, cursor, columns))
    }
    pub(super) fn native_field_key(
        &mut self,
        event: &KeyDownEvent,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> bool {
        self.sync_native_field();
        if self.native_field().is_none() {
            return false;
        }
        let k = &event.keystroke;
        if self.field_native.is_marked() && k.key == "escape" {
            self.field_native.cancel();
            self.field_native_stale = true;
            cx.stop_propagation();
            cx.notify();
            return true;
        }
        if self.field_native.is_marked() && matches!(k.key.as_str(), "backspace" | "delete") {
            self.field_native.cancel();
            self.field_native_stale = true;
            cx.stop_propagation();
            cx.notify();
            return true;
        }
        if self.field_native.is_marked() && k.key == "enter" {
            self.finish_native_field(cx);
            cx.stop_propagation();
            return true;
        }
        if self.field_alt_action(event) {
            self.field_native.cancel();
            self.field_native_stale = true;
            return false;
        }
        if self.focus.is_focused(window)
            && lenz_gpui::field::defer_uncommitted_key(self.field_native_registered.get(), k)
        {
            self.field_native_stale = false;
            self.field_native_suppress = None;
            return true;
        }
        let printable =
            k.key_char.as_deref().is_some_and(|t| {
                !t.chars().any(char::is_control) && (!k.modifiers.alt || t != k.key)
            }) && !k.modifiers.control
                && !k.modifiers.platform;
        if printable
            && k.modifiers.alt
            && !lenz_gpui::field::defer_option_input(
                self.field_native_registered.get(),
                cfg!(target_os = "macos"),
                self.field_native.is_marked(),
            )
            && let Some(command) = lenz_gpui::field::command(event, cx)
        {
            self.field_command(command, cx);
            self.field_native_stale = false;
            cx.stop_propagation();
            return true;
        }
        if printable && self.field_native_registered.get() && self.focus.is_focused(window) {
            if self.native_field().is_some_and(|(_, text, cursor)| {
                self.field_native.is_marked()
                    && self.field_native.marked_range(text, cursor).is_none()
            }) {
                self.field_native.cancel();
            }
            self.field_native_stale = false;
            self.field_native_suppress = None;
            return true;
        }
        if self.field_native.is_marked() {
            if k.key_char.is_none()
                && !k.modifiers.control
                && !k.modifiers.platform
                && !matches!(
                    k.key.as_str(),
                    "left"
                        | "right"
                        | "up"
                        | "down"
                        | "tab"
                        | "home"
                        | "end"
                        | "backspace"
                        | "delete"
                )
            {
                return true;
            }
            self.field_native.cancel();
            self.field_native_stale = true;
        }
        if k.key_char.is_none()
            && !k.modifiers.control
            && !k.modifiers.platform
            && !matches!(
                k.key.as_str(),
                "left"
                    | "right"
                    | "up"
                    | "down"
                    | "home"
                    | "end"
                    | "tab"
                    | "escape"
                    | "enter"
                    | "backspace"
                    | "delete"
                    | "pageup"
                    | "pagedown"
            )
        {
            self.field_native_stale = false;
            self.field_native_suppress = None;
        }
        false
    }
}

impl Viewer {
    fn finish_native_field(&mut self, cx: &mut Context<Self>) {
        let owner = self.native_field().map(|f| f.0);
        let mut native = std::mem::take(&mut self.field_native);
        let command = self
            .native_field()
            .and_then(|(_, text, cursor)| native.unmark(text, cursor));
        self.field_native = native;
        if let Some(command) = command {
            let value = match &command {
                crate::field_edit::Command::Replace { text, .. } => Some(text.clone()),
                _ => None,
            };
            self.field_command(command, cx);
            self.field_native_suppress = owner.zip(value);
        }
    }
    fn native_field_bounds(&self) -> Option<Bounds<Pixels>> {
        let (owner, _, _) = self.native_field()?;
        let cw = self.text_char_w();
        let [vw, _] = self.panes.viewport;
        let rect = |x: f32, y: f32, w: f32| Bounds {
            origin: point(px(x), px(y)),
            size: size(px(w.max(0.0)), px(line_h())),
        };
        if owner == u64::MAX {
            return Some(rect(
                self.picker_w() + 8.0 + 8.0 * cw,
                toolbar_h(),
                vw - self.picker_w() - 16.0 - 8.0 * cw,
            ));
        }
        match self.modal()? {
            Modal::Find => Some(
                self.find_control()?
                    .query_bounds(point(px(0.0), px(0.0)), cw),
            ),
            Modal::Palette => {
                let (x, y, w, _) = self.palette_layout();
                Some(rect(x + 12.0, y + 4.0, w - 24.0))
            }
            Modal::Switcher => {
                let x0 = self.picker_w();
                let w = vw - x0;
                let bw = 720.0f32.min(w - 40.0);
                Some(rect(
                    x0 + (w - bw) / 2.0 + 70.0,
                    toolbar_h() + self.log_h() + 66.0,
                    bw - 80.0,
                ))
            }
            Modal::MergeFind => Some(rect(
                self.diff_x() + 12.0,
                toolbar_h() + self.log_h() + 4.0 + line_h(),
                vw - self.diff_x() - 24.0,
            )),
            Modal::GoTo | Modal::GitPrompt | Modal::SearchEditing => {
                let prompt = self.goto_prompt().or_else(|| self.search_prompt())?;
                let x0 = self.diff_x();
                let w = vw - x0;
                let bw = (prompt.chars().count() as f32 * cw + 24.0).min((w - 24.0).max(0.0));
                Some(rect(
                    x0 + (w - bw) / 2.0 + 12.0,
                    toolbar_h() + self.log_h() + 66.0 + line_h(),
                    bw - 24.0,
                ))
            }
            _ => None,
        }
    }
    pub(super) fn native_field_click(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if event.button != MouseButton::Left {
            return false;
        }
        let Some(bounds) = self
            .native_field_bounds()
            .filter(|b| b.contains(&event.position))
        else {
            return false;
        };
        self.field_native.cancel();
        self.field_native_stale = true;
        let Some((_, text, cursor)) = self.native_field() else {
            return false;
        };
        let at = lenz_gpui::field::hit_test(
            lenz_gpui::field::View {
                text,
                cursor,
                bounds,
                char_w: self.text_char_w(),
                active: true,
            },
            event.position,
            window,
            metrics(&self.font),
        );
        let anchor = if event.modifiers.shift {
            cursor.anchor.unwrap_or(cursor.caret(text))
        } else {
            at
        };
        self.field_command(crate::field_edit::Command::Select { anchor, caret: at }, cx);
        window.focus(&self.focus, cx);
        cx.stop_propagation();
        true
    }
}
impl EntityInputHandler for Viewer {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted: &mut Option<Range<usize>>,
        window: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        if !self.focus.is_focused(window) {
            return None;
        }
        let (_, text, cursor) = self.native_field()?;
        self.field_native
            .text_for_range(text, cursor, range, adjusted)
    }
    fn selected_text_range(
        &mut self,
        _: bool,
        window: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        if !self.focus.is_focused(window) {
            return None;
        }
        let (_, text, cursor) = self.native_field()?;
        let (range, reversed) = self.field_native.selected(text, cursor)?;
        Some(UTF16Selection { range, reversed })
    }
    fn marked_text_range(
        &self,
        window: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        if !self.focus.is_focused(window) {
            return None;
        }
        let (_, text, cursor) = self.native_field()?;
        self.field_native.marked_range(text, cursor)
    }
    fn unmark_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_native_field();
        if !self.focus.is_focused(window) {
            self.field_native.cancel();
            self.field_native_stale = true;
            return;
        }
        if self.field_native_stale {
            return;
        }
        self.finish_native_field(cx);
    }
    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sync_native_field();
        if !self.focus.is_focused(window) {
            self.field_native.cancel();
            self.field_native_stale = true;
            return;
        }
        if self.field_native_stale {
            return;
        }
        if self.native_field().is_some_and(|(_, value, cursor)| {
            self.field_native.cancels_marked_text(
                value,
                cursor,
                range.as_ref(),
                text,
                cfg!(target_os = "macos"),
            )
        }) {
            self.field_native.cancel();
            self.field_native_stale = true;
            self.field_native_changes |= 3;
            cx.notify();
            return;
        }
        if self
            .field_native_suppress
            .as_ref()
            .is_some_and(|(owner, _)| Some(*owner) == self.native_field().map(|f| f.0))
        {
            return;
        }
        let text = if self.modal() == Some(Modal::GitPrompt) {
            text.split(['\r', '\n']).next().unwrap_or_default()
        } else {
            text
        };
        let mut native = std::mem::take(&mut self.field_native);
        let command = self.native_field().and_then(|(_, value, cursor)| {
            native.commit(value, cursor, range, text, cfg!(target_os = "macos"))
        });
        self.field_native = native;
        if let Some(command) = command {
            self.field_command(command, cx);
        }
    }
    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sync_native_field();
        if !self.focus.is_focused(window) {
            self.field_native.cancel();
            self.field_native_stale = true;
            return;
        }
        if self.field_native_stale {
            return;
        }
        let new_phase = range.is_none();
        let text = if self.modal() == Some(Modal::GitPrompt) {
            text.split(['\r', '\n']).next().unwrap_or_default()
        } else {
            text
        };
        let mut native = std::mem::take(&mut self.field_native);
        let changed = self.native_field().is_some_and(|(_, value, cursor)| {
            native.mark(
                value,
                cursor,
                range,
                text,
                selected,
                cfg!(target_os = "macos"),
            )
        });
        self.field_native = native;
        if changed {
            if new_phase {
                self.field_native_suppress = None;
            }
            self.field_native_stale = false;
            cx.notify();
        }
    }
    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        _: Bounds<Pixels>,
        window: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let bounds = self.native_field_bounds()?;
        let (_, text, cursor) = self.native_field()?;
        let (shown, range) = self.field_native.snapshot_range(
            text,
            cursor,
            range,
            (f32::from(bounds.size.width) / self.text_char_w()).max(0.0) as usize,
        )?;
        lenz_gpui::field::bounds_for_range(
            lenz_gpui::field::View {
                text: &shown.text,
                cursor: shown.cursor,
                bounds,
                char_w: self.text_char_w(),
                active: true,
            },
            range,
            window,
            metrics(&self.font),
        )
    }
    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        window: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        let bounds = self.native_field_bounds()?;
        let (_, text, cursor) = self.native_field()?;
        let shown = self.field_native.snapshot(
            text,
            cursor,
            (f32::from(bounds.size.width) / self.text_char_w()).max(0.0) as usize,
        );
        let at = lenz_gpui::field::hit_test(
            lenz_gpui::field::View {
                text: &shown.text,
                cursor: shown.cursor,
                bounds,
                char_w: self.text_char_w(),
                active: true,
            },
            point,
            window,
            metrics(&self.font),
        );
        let (selected, reversed) = self.field_native.selected(text, cursor)?;
        let local_caret = shown
            .text
            .get(..shown.cursor.caret(&shown.text))?
            .encode_utf16()
            .count();
        Some(
            (if reversed {
                selected.start
            } else {
                selected.end
            })
            .saturating_sub(local_caret)
                + shown.text.get(..at)?.encode_utf16().count(),
        )
    }
    fn set_selected_text_range(
        &mut self,
        range: Range<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sync_native_field();
        if !self.focus.is_focused(window) {
            self.field_native.cancel();
            self.field_native_stale = true;
            return;
        }
        if self.field_native_stale {
            return;
        }
        let mut native = std::mem::take(&mut self.field_native);
        let command = self
            .native_field()
            .and_then(|(_, text, cursor)| native.set_selection(text, cursor, range));
        self.field_native = native;
        if let Some(command) = command {
            self.field_command(command, cx);
        }
        cx.notify();
    }
    fn text_length_utf16(&mut self, window: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        if !self.focus.is_focused(window) {
            return None;
        }
        let (_, text, cursor) = self.native_field()?;
        Some(self.field_native.len_utf16(text, cursor))
    }
    fn accepts_text_input(&self, window: &mut Window, _: &mut Context<Self>) -> bool {
        self.focus.is_focused(window)
            && lenz_gpui::field::prefers_native_input(
                self.field_native_stale,
                cfg!(target_os = "macos"),
                window.modifiers().alt,
            )
            && self.native_field().is_some()
    }
}

#[cfg(all(test, feature = "gui-test"))]
mod tests {
    use super::*;
    use crate::ui::{merge_view::gui_tests::event, parity_tests};
    use gpui::TestAppContext;

    #[gpui::test]
    fn gui_native_field_preedit_isolated_commit_once_and_stale_owner_callbacks_ignored(
        cx: &mut TestAppContext,
    ) {
        let f = parity_tests::fixture_named(cx, "old\n", "unchanged\n", "txt");
        f.window
            .update(cx, |v, w, cx| {
                w.focus(&v.focus, cx);
                v.on_key(&event("f", true, false, false), w, cx);
                v.field_native_registered.set(true);
                let mut dead = event("dead_acute", false, false, false);
                dead.keystroke.key_char = None;
                v.replace_and_mark_text_in_range(None, "仮😀", Some(1..3), w, cx);
                assert_eq!(v.comparison.find.as_ref().unwrap().query, "");
                assert_eq!(v.marked_text_range(w, cx), Some(0..3));
                assert_eq!(v.selected_text_range(false, w, cx).unwrap().range, 1..3);
                let mut adjusted = None;
                assert_eq!(
                    v.text_for_range(0..3, &mut adjusted, w, cx).as_deref(),
                    Some("仮😀")
                );
                v.replace_and_mark_text_in_range(None, "確定", None, w, cx);
                v.unmark_text(w, cx);
                assert_eq!(v.comparison.find.as_ref().unwrap().query, "確定");
                v.replace_text_in_range(None, "確定", w, cx);
                assert_eq!(
                    v.comparison.find.as_ref().unwrap().query,
                    "確定",
                    "duplicate terminal callback must not commit twice"
                );
                v.on_key(&dead, w, cx);
                v.replace_and_mark_text_in_range(None, "削除", None, w, cx);
                v.on_key(&event("backspace", false, false, false), w, cx);
                assert_eq!(
                    v.comparison.find.as_ref().unwrap().query,
                    "確定",
                    "deleting a preedit must not edit committed query text"
                );
                v.replace_text_in_range(None, "late delete", w, cx);
                assert_eq!(v.comparison.find.as_ref().unwrap().query, "確定");
                v.on_key(&dead, w, cx);
                v.replace_and_mark_text_in_range(None, "破棄", None, w, cx);
                v.on_key(&event("escape", false, false, false), w, cx);
                v.on_key(&event("escape", false, false, false), w, cx);
                v.goto = Some(String::new());
                v.sync_native_field();
                v.replace_and_mark_text_in_range(None, "obsolete", None, w, cx);
                v.replace_text_in_range(None, "obsolete", w, cx);
                assert_eq!(v.goto.as_deref(), Some(""));
                assert!(!v.field_native.is_marked());
                let mut option = event("2", false, false, true);
                option.keystroke.key_char = Some("™".into());
                assert!(!v.accepts_text_input(w, cx));
                v.on_key(&option, w, cx);
                if cfg!(target_os = "macos") {
                    v.replace_text_in_range(None, "™", w, cx);
                }
                assert_eq!(v.goto.as_deref(), Some("™"));
                assert!(v.accepts_text_input(w, cx));
            })
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(f.dir.path().join("right.txt")).unwrap(),
            "unchanged\n"
        );
        f.viewer.read_with(cx, |v, _| assert!(!v.edits.has_dirty()));
    }

    #[gpui::test]
    fn gui_native_find_preserves_alt_options_and_mouse_uses_grapheme_boundaries(
        cx: &mut TestAppContext,
    ) {
        let f = parity_tests::fixture_named(cx, "old\n", "unchanged\n", "txt");
        f.window
            .update(cx, |v, w, cx| {
                w.focus(&v.focus, cx);
                v.on_key(&event("f", true, false, false), w, cx);
                let before = v.comparison.find_options.case;
                let mut option = event("c", false, false, true);
                option.keystroke.key_char = Some("ç".into());
                v.on_key(&option, w, cx);
                assert_eq!(v.comparison.find_options.case, !before);
                assert!(v.comparison.find.as_ref().unwrap().query.is_empty());
                for value in ["aé👩🏽‍💻b", "abc שלום", "a\u{202e}z"] {
                    v.field_command(
                        crate::field_edit::Command::Select {
                            anchor: 0,
                            caret: usize::MAX,
                        },
                        cx,
                    );
                    v.field_command(crate::field_edit::Command::Insert(value.into()), cx);
                    let bounds = Bounds {
                        origin: point(px(0.0), px(0.0)),
                        size: size(px(280.0), px(line_h())),
                    };
                    let (_, text, cursor) = v.native_field().unwrap();
                    let boundaries: Vec<_> = crate::line_edit::boundaries(text).collect();
                    for x in (0..280).step_by(3) {
                        let at = lenz_gpui::field::hit_test(
                            lenz_gpui::field::View {
                                text,
                                cursor,
                                bounds,
                                char_w: v.text_char_w(),
                                active: true,
                            },
                            point(px(x as f32), px(5.0)),
                            w,
                            metrics(&v.font),
                        );
                        assert!(boundaries.contains(&at), "{value:?} x={x} at={at}");
                    }
                }
            })
            .unwrap();
    }
    #[gpui::test]
    fn gui_native_selection_callback_cannot_retarget_to_a_new_prompt(cx: &mut TestAppContext) {
        let f = parity_tests::fixture_named(cx, "old\n", "unchanged\n", "txt");
        f.window
            .update(cx, |v, w, cx| {
                w.focus(&v.focus, cx);
                v.on_key(&event("f", true, false, false), w, cx);
                v.sync_native_field();
                v.field_command(crate::field_edit::Command::Insert("find".into()), cx);
                v.goto = Some("123".into());
                v.set_selected_text_range(0..1, w, cx);
                let (_, value, cursor) = v.native_field().unwrap();
                assert_eq!(value, "123");
                assert!(cursor.selection(value).is_none());
                assert_eq!(cursor.caret(value), 3);
                assert!(v.field_native_stale);
                v.field_native_registered.set(true);
                v.on_key(&event("1", false, false, false), w, cx);
                v.set_selected_text_range(0..1, w, cx);
                let (_, value, cursor) = v.native_field().unwrap();
                assert_eq!(cursor.selected(value), Some("1"));
            })
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(f.dir.path().join("right.txt")).unwrap(),
            "unchanged\n"
        );
    }

    #[cfg(not(target_os = "macos"))]
    #[gpui::test]
    fn gui_native_linux_cancel_preserves_selected_query_and_rejects_late_callbacks(
        cx: &mut TestAppContext,
    ) {
        let f = parity_tests::fixture_named(cx, "old\n", "unchanged\n", "txt");
        f.window
            .update(cx, |v, w, cx| {
                w.focus(&v.focus, cx);
                v.on_key(&event("f", true, false, false), w, cx);
                v.sync_native_field();
                v.field_command(crate::field_edit::Command::Insert("keep".into()), cx);
                v.field_command(crate::field_edit::Command::SelectAll, cx);
                v.replace_and_mark_text_in_range(None, "候補", None, w, cx);
                let marked = v.marked_text_range(w, cx).unwrap();
                v.replace_text_in_range(Some(marked), "", w, cx);
                assert_eq!(v.comparison.find.as_ref().unwrap().query, "keep");
                let (_, value, cursor) = v.native_field().unwrap();
                assert_eq!(cursor.selected(value), Some("keep"));
                assert!(!v.field_native.is_marked());
                assert!(v.field_native_stale);
                v.replace_text_in_range(None, "obsolete", w, cx);
                v.replace_and_mark_text_in_range(None, "obsolete", None, w, cx);
                assert_eq!(v.comparison.find.as_ref().unwrap().query, "keep");
                let mut dead = event("dead_acute", false, false, false);
                dead.keystroke.key_char = None;
                v.on_key(&dead, w, cx);
                v.replace_and_mark_text_in_range(None, "fresh", None, w, cx);
                v.replace_text_in_range(None, "fresh", w, cx);
                assert_eq!(v.comparison.find.as_ref().unwrap().query, "fresh");
                v.replace_text_in_range(Some(0..5), "", w, cx);
                assert!(
                    v.comparison.find.as_ref().unwrap().query.is_empty(),
                    "unmarked native deletion remains valid"
                );
            })
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(f.dir.path().join("right.txt")).unwrap(),
            "unchanged\n"
        );
        f.viewer.read_with(cx, |v, _| assert!(!v.edits.has_dirty()));
    }
    #[gpui::test]
    fn gui_native_dead_physical_name_waits_for_the_composed_commit(cx: &mut TestAppContext) {
        let f = parity_tests::fixture_named(cx, "old\n", "unchanged\n", "txt");
        f.window
            .update(cx, |v, w, cx| {
                w.focus(&v.focus, cx);
                v.on_key(&event("f", true, false, false), w, cx);
                v.sync_native_field();
                v.field_native_registered.set(true);
                v.replace_and_mark_text_in_range(None, "cancelled", None, w, cx);
                v.on_key(&event("escape", false, false, false), w, cx);
                assert!(v.field_native_stale);
                v.replace_and_mark_text_in_range(None, "'", None, w, cx);
                assert!(
                    !v.field_native.is_marked(),
                    "an obsolete preedit before the fresh key boundary stays ignored"
                );
                let mut dead = event("'", false, false, false);
                dead.keystroke.key_char = None;
                v.on_key(&dead, w, cx);
                assert!(
                    v.comparison.find.as_ref().unwrap().query.is_empty(),
                    "physical apostrophe is not committed text while composing"
                );
                v.replace_text_in_range(None, "é", w, cx);
                assert_eq!(v.comparison.find.as_ref().unwrap().query, "é");
                v.on_key(&event("left", false, false, false), w, cx);
                let (_, text, cursor) = v.native_field().unwrap();
                assert_eq!(
                    cursor.caret(text),
                    0,
                    "ordinary motions retain field editing"
                );
                v.on_key(&event("a", true, false, false), w, cx);
                v.on_key(&event("c", true, false, false), w, cx);
                assert_eq!(
                    cx.read_from_clipboard().and_then(|c| c.text()).as_deref(),
                    Some("é")
                );
            })
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(f.dir.path().join("right.txt")).unwrap(),
            "unchanged\n"
        );
        f.viewer.read_with(cx, |v, _| assert!(!v.edits.has_dirty()));
    }
}
