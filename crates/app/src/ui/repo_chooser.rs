//! Native presentation of the shared repository / file / folder opening workflow.
use super::*;
use crate::line_edit::Edit;
use crate::repo_picker::{Choice, Mode, PickerCommand, RepoPicker};
use crate::field_edit::{Command as FieldCommand, Native};
use gpui::{EntityInputHandler, Point, Size, UTF16Selection};
use std::{cell::Cell, ops::Range, rc::Rc, sync::Mutex};

static CHOSEN: Mutex<Option<Choice>> = Mutex::new(None);
#[cfg(all(test, feature = "gui-test"))]
static TEST_CHOOSER: Mutex<()> = Mutex::new(());
pub fn chosen() -> Option<Choice> {
    CHOSEN.lock().ok()?.take()
}
pub struct RepoChooser {
    pub(super) core: RepoPicker,
    focus: FocusHandle,
    font: Font,
    rows: Vec<(usize, f32)>,
    browse: (f32, f32, f32),
    submit: (f32, f32, f32),
    cancel: (f32, f32, f32),
    request: usize,
    native: Native,
    native_owner: (Mode, usize),
    native_stale: bool,
    native_registered: Rc<Cell<bool>>,
    native_suppress: Option<(Mode, usize)>,
}
const PAD: f32 = 16.0;
impl RepoChooser {
    pub fn new(core: RepoPicker, font: Font, cx: &mut Context<Self>) -> Self {
        let native_owner = (core.mode, core.field);
        Self {
            core,
            focus: cx.focus_handle(),
            font,
            rows: Vec::new(),
            browse: (0., 0., 0.),
            submit: (0., 0., 0.),
            cancel: (0., 0., 0.),
            request: 0,
            native: Native::default(),
            native_owner,
            native_stale: false,
            native_registered: Rc::default(),
            native_suppress: None,
        }
    }
    pub fn focus_handle(&self) -> FocusHandle {
        self.focus.clone()
    }
    pub(super) fn command(&mut self, command: PickerCommand, cx: &mut Context<Self>) {
        self.cancel_native();
        if let Some(choice) = self.core.execute(command) {
            if let Ok(mut chosen) = CHOSEN.lock() {
                *chosen = Some(choice);
            }
            cx.quit();
        }
        self.sync_native();
        cx.notify();
    }
    fn cancel_native(&mut self) {
        self.native.cancel();
        self.native_stale = true;
        self.native_suppress = None;
    }
    fn sync_native(&mut self) {
        let owner = (self.core.mode, self.core.field);
        if self.native_owner != owner {
            self.cancel_native();
            self.native_owner = owner;
        }
    }
    fn edit_field(&mut self, command: FieldCommand, cx: &mut Context<Self>) {
        self.native.cancel();
        self.native_suppress = None;
        if let Some(text) = self.core.edit_field(command.first_line()).copy {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
        cx.notify();
    }
    fn finish_native(&mut self, cx: &mut Context<Self>) {
        let field = self.core.active();
        if let Some(command) = self.native.unmark(field.text(), field.cursor()) {
            self.edit_field(command, cx);
            self.native_suppress = Some((self.core.mode, self.core.field));
        }
    }
    fn browse(&mut self, cx: &mut Context<Self>) {
        self.cancel_native();
        self.request = self.request.wrapping_add(1);
        let (request, mode, field) = (self.request, self.core.mode, self.core.field);
        let answer = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: mode == Mode::Files,
            directories: mode != Mode::Files,
            multiple: false,
            prompt: Some("Open".into()),
        });
        cx.spawn(async move |this, cx| {
            let picked = match answer.await {
                Ok(Ok(Some(paths))) => paths.into_iter().next().map(Ok),
                Ok(Err(_)) => Some(Err(
                    "The file dialog could not open. Type or paste a path instead.".to_string(),
                )),
                _ => None,
            };
            this.update(cx, |c, cx| {
                if c.request != request || c.core.mode != mode || c.core.field != field {
                    return;
                }
                match picked {
                    Some(Ok(path)) => c.command(PickerCommand::Choose(path), cx),
                    Some(Err(message)) => {
                        c.core.error = Some(message);
                        cx.notify();
                    }
                    None => {}
                }
            })
            .ok();
        })
        .detach();
    }
    pub(super) fn on_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_native();
        let k = &ev.keystroke;
        let ctrl = k.modifiers.control || k.modifiers.platform;
        let word = ctrl || k.modifiers.alt;
        if self.native.is_marked() && k.key == "escape" {
            self.cancel_native();
            cx.stop_propagation();
            cx.notify();
            return;
        }
        if self.native.is_marked() && matches!(k.key.as_str(), "backspace" | "delete") {
            self.cancel_native();
            cx.stop_propagation();
            cx.notify();
            return;
        }
        if self.native.is_marked() && k.key == "enter" {
            self.finish_native(cx);
            cx.stop_propagation();
            return;
        }
        let printable = k.key_char.as_deref().is_some_and(|t| {
            !t.chars().any(char::is_control) && (!k.modifiers.alt || t != k.key)
        }) && !ctrl;
        if lenz_gpui::field::defer_uncommitted_key(
            self.native_registered.get() && self.focus.is_focused(window), k,
        ) {
            self.native_stale = false;
            self.native_suppress = None;
            return;
        }
        if printable && k.modifiers.alt && lenz_gpui::field::defer_option_input(
            self.native_registered.get() && self.focus.is_focused(window),
            cfg!(target_os = "macos"), self.native.is_marked(),
        ) {
            self.native_stale = false;
            self.native_suppress = None;
            return;
        }
        if printable && k.modifiers.alt && !self.native.is_marked()
            && let Some(FieldCommand::Insert(value)) = lenz_gpui::field::command(ev, cx)
            && !value.is_empty()
        {
            self.edit_field(FieldCommand::Insert(value), cx);
            self.native_stale = false;
            cx.stop_propagation();
            return;
        }
        if printable && self.native_registered.get() && self.focus.is_focused(window) {
            self.native_stale = false;
            self.native_suppress = None;
            // Native input commits the text after key handling. Do not insert it twice.
            return;
        }
        if k.key_char.is_none() && !ctrl && !k.modifiers.alt
            && !matches!(k.key.as_str(), "left" | "right" | "up" | "down" | "tab" | "escape" | "enter" | "home" | "end" | "backspace" | "delete" | "f1" | "f2" | "f3")
            && self.native_registered.get() && self.focus.is_focused(window)
        {
            self.native_stale = false;
            self.native_suppress = None;
            return;
        }
        if self.native.is_marked() && k.key_char.is_none() && !ctrl
            && !matches!(k.key.as_str(), "left" | "right" | "up" | "down" | "tab" | "home" | "end" | "backspace" | "delete" | "f1" | "f2" | "f3")
        {
            return;
        }
        if self.native.is_marked() {
            self.cancel_native();
        }
        cx.stop_propagation();
        if matches!(k.key.as_str(), "f1" | "f2" | "f3") && self.core.comparisons {
            let mode = match k.key.as_str() {
                "f1" => Mode::Files,
                "f2" => Mode::Folders,
                _ => Mode::Repository,
            };
            return self.command(PickerCommand::Mode(mode), cx);
        }
        if k.key == "escape" || ctrl && matches!(k.key.as_str(), "q" | "w") {
            self.cancel_native();
            cx.quit();
            return;
        }
        if let Some(command) = lenz_gpui::field::command(ev, cx) {
            if printable && matches!(&command, FieldCommand::Insert(_)) {
                self.native_stale = false;
            }
            self.edit_field(command, cx);
            return;
        }
        if ctrl {
            match k.key.as_str() {
                "o" => return self.browse(cx),
                "a" => return self.command(PickerCommand::SelectAll, cx),
                "v" => {
                    if let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) {
                        self.command(PickerCommand::InsertText(text), cx);
                    }
                    return;
                }
                "c" | "x" => {
                    if let Some(text) = self.core.active().selection() {
                        cx.write_to_clipboard(ClipboardItem::new_string(text.into()));
                        if k.key == "x" {
                            self.command(PickerCommand::Edit(Edit::Delete), cx);
                        }
                    }
                    return;
                }
                "space" => return self.command(PickerCommand::Complete, cx),
                _ => {}
            }
        }
        let edit = match k.key.as_str() {
            "left" | "right"
                if k.modifiers.platform && !k.modifiers.control && !k.modifiers.alt =>
            {
                Some(if k.key == "left" {
                    Edit::Start
                } else {
                    Edit::End
                })
            }
            "left" => Some(if word { Edit::WordLeft } else { Edit::Left }),
            "right" => Some(if word { Edit::WordRight } else { Edit::Right }),
            "home" => Some(Edit::Start),
            "end" => Some(Edit::End),
            "backspace" => Some(if word {
                Edit::BackspaceWord
            } else {
                Edit::Backspace
            }),
            "delete" => Some(if word { Edit::DeleteWord } else { Edit::Delete }),
            _ => None,
        };
        if let Some(edit) = edit {
            self.command(PickerCommand::Edit(edit), cx);
            return;
        }
        let command = match k.key.as_str() {
            "enter" => Some(PickerCommand::Submit),
            "tab" => Some(if self.core.mode == Mode::Repository {
                PickerCommand::Complete
            } else {
                PickerCommand::NextField
            }),
            "up" => Some(PickerCommand::Up),
            "down" => Some(PickerCommand::Down),
            _ if !ctrl && !k.modifiers.alt => k
                .key_char
                .clone()
                .or_else(|| (k.key.chars().count() == 1).then(|| k.key.clone()))
                .map(PickerCommand::InsertText),
            _ => None,
        };
        if let Some(command) = command {
            self.command(command, cx);
        }
    }
    fn compact(&self, height: f32) -> bool {
        self.core.comparisons && height < 2.0 * PAD + 11.0 * line_h()
    }
    fn field_row(&self, height: f32) -> usize {
        if self.compact(height) { 2 } else if self.core.comparisons { 6 } else { 3 }
    }
    fn on_click(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        cx.stop_propagation();
        let (x, y) = (f32::from(ev.position.x), f32::from(ev.position.y));
        let viewport = window.viewport_size();
        let compact = self.compact(f32::from(viewport.height));
        let hit =
            |(bx, by, bw): (f32, f32, f32)| x >= bx && x < bx + bw && y >= by && y < by + line_h();
        if hit(self.browse) {
            return self.browse(cx);
        }
        if hit(self.cancel) {
            self.cancel_native();
            cx.quit();
            return;
        }
        if hit(self.submit) {
            return self.command(PickerCommand::Submit, cx);
        }
        if self.core.comparisons {
            if compact {
                let width = (f32::from(viewport.width) - 2.0 * PAD).max(0.0);
                if width > 0.0 && x >= PAD && x < PAD + width && y >= PAD + line_h() && y < PAD + 2.0 * line_h() {
                    let i = ((x - PAD) / (width / 3.0)).floor() as usize;
                    return self.command(PickerCommand::Mode(Mode::ALL[i.min(2)]), cx);
                }
            } else {
                for (i, mode) in Mode::ALL.into_iter().enumerate() {
                    if y >= PAD + (2 + i) as f32 * line_h() && y < PAD + (3 + i) as f32 * line_h() {
                        return self.command(PickerCommand::Mode(mode), cx);
                    }
                }
            }
        }
        for i in 0..self.core.fields() {
            let top = PAD + (self.field_row(f32::from(viewport.height)) + i) as f32 * line_h();
            if y >= top && y < top + line_h() {
                self.command(PickerCommand::Field(i), cx);
                let Some(bounds) = self.field_bounds(i, window.viewport_size()) else { return };
                let field = self.core.active();
                let at = lenz_gpui::field::hit_test(lenz_gpui::field::View {
                    text: field.text(), cursor: field.cursor(), bounds,
                    char_w: font_size() * 0.6, active: true,
                }, ev.position, window, self.metrics());
                let anchor = if ev.modifiers.shift {
                    field.cursor().anchor.unwrap_or_else(|| field.caret())
                } else { at };
                self.edit_field(FieldCommand::Select { anchor, caret: at }, cx);
                return;
            }
        }
        let row = self
            .rows
            .iter()
            .find(|(_, top)| y >= *top && y < top + line_h())
            .map(|(i, _)| *i);
        if let Some(path) = row.and_then(|i| self.core.matches().get(i).map(|p| p.to_path_buf())) {
            self.command(PickerCommand::Choose(path), cx);
        }
    }
    fn metrics(&self) -> lenz_gpui::Metrics<'_> {
        lenz_gpui::Metrics { font: &self.font, font_size: font_size(), line_h: line_h(), num_w: 0.0 }
    }
    fn field_bounds(&self, index: usize, viewport: Size<Pixels>) -> Option<Bounds<Pixels>> {
        let x = PAD + 7.0 * font_size() * 0.6;
        let y = PAD + (self.field_row(f32::from(viewport.height)) + index) as f32 * line_h();
        let width = (f32::from(viewport.width) - PAD - x).max(0.0);
        let bottom = (f32::from(viewport.height) - PAD - 2.0 * line_h()).max(0.0);
        let height = (bottom - y).clamp(0.0, line_h());
        (width > 0.0 && height > 0.0).then_some(Bounds {
            origin: point(px(x), px(y)), size: size(px(width), px(height)),
        })
    }
    fn lines(
        &self,
        p: &theme::Palette,
        width: f32,
        height: f32,
    ) -> Vec<(String, u32, Option<u32>, Option<usize>)> {
        let mut lines = vec![
            (
                if self.core.comparisons {
                    "New comparison"
                } else {
                    "Open a repository"
                }
                .into(),
                p.accent,
                None,
                None,
            ),
            (
                format!("Paths relative to {}", self.core.cwd().display()),
                p.dim,
                None,
                None,
            ),
        ];
        if self.compact(height) {
            // Keep both paths above the footer even in a short window. The mode row is painted as three buttons.
            lines[1] = (String::new(), p.accent, None, None);
        } else if self.core.comparisons {
            for (i, mode) in Mode::ALL.into_iter().enumerate() {
                lines.push((
                    format!(
                        "{} {}  (F{})",
                        if mode == self.core.mode { "●" } else { "○" },
                        mode.label(),
                        i + 1
                    ),
                    p.accent,
                    (mode == self.core.mode).then_some(p.selected_bg),
                    None,
                ));
            }
        }
        if !self.compact(height) {
            lines.push((String::new(), p.fg, None, None));
        }
        let columns = ((width - 2. * PAD) / (font_size() * 0.62)).max(1.) as usize;
        for i in 0..self.core.fields() {
            let field = if i == 0 {
                &self.core.input
            } else {
                &self.core.right
            };
            let shown = field.visible(columns.saturating_sub(8));
            let mut value = shown.text.to_string();
            if self.core.field == i {
                value.insert(shown.prefix.len(), '▏');
            }
            lines.push((
                format!("{}: {value}", self.core.mode.field(i)),
                p.fg,
                Some(if field.selected() {
                    p.selected_bg
                } else {
                    p.status_bg
                }),
                None,
            ));
        }
        let error = self.core.error.as_deref().unwrap_or_default();
        let starts = crate::wrap::wrap_starts(error, columns);
        let ends = starts.iter().copied().chain(std::iter::once(error.len()));
        for (start, end) in std::iter::once(0)
            .chain(starts.iter().copied())
            .zip(ends)
            .take(3)
        {
            lines.push((
                error.get(start..end).unwrap_or_default().trim_end().into(),
                p.error,
                None,
                None,
            ));
        }
        if self.core.mode == Mode::Repository {
            let matches = self.core.matches();
            lines.push((
                if matches.is_empty() {
                    "No recent repositories"
                } else {
                    "Recent repositories"
                }
                .into(),
                p.accent,
                None,
                None,
            ));
            for (i, path) in matches.iter().enumerate() {
                lines.push((
                    path.display().to_string(),
                    p.fg,
                    (i == self.core.selected).then_some(p.selected_bg),
                    Some(i),
                ));
            }
        }
        lines
    }
}
impl EntityInputHandler for RepoChooser {
    fn text_for_range(&mut self, range: Range<usize>, adjusted: &mut Option<Range<usize>>, window: &mut Window, _: &mut Context<Self>) -> Option<String> {
        self.sync_native();
        if !self.focus.is_focused(window) { return None; }
        let field = self.core.active();
        self.native.text_for_range(field.text(), field.cursor(), range, adjusted)
    }
    fn selected_text_range(&mut self, _: bool, window: &mut Window, _: &mut Context<Self>) -> Option<UTF16Selection> {
        self.sync_native();
        if !self.focus.is_focused(window) { return None; }
        let field = self.core.active();
        let (range, reversed) = self.native.selected(field.text(), field.cursor())?;
        Some(UTF16Selection { range, reversed })
    }
    fn marked_text_range(&self, window: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        if !self.focus.is_focused(window) || self.native_owner != (self.core.mode, self.core.field) { return None; }
        let field = self.core.active();
        self.native.marked_range(field.text(), field.cursor())
    }
    fn unmark_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_native();
        if !self.focus.is_focused(window) || self.native_stale { return; }
        self.finish_native(cx);
    }
    fn replace_text_in_range(&mut self, range: Option<Range<usize>>, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_native();
        if !self.focus.is_focused(window) || self.native_stale { return; }
        if self.native_suppress == Some((self.core.mode, self.core.field)) { return; }
        let field = self.core.active();
        if self.native.cancels_marked_text(field.text(), field.cursor(), range.as_ref(), text, cfg!(target_os = "macos")) {
            self.cancel_native();
            cx.notify();
            return;
        }
        if let Some(command) = self.native.commit(field.text(), field.cursor(), range, text, cfg!(target_os = "macos")) {
            self.edit_field(command, cx);
        }
    }
    fn replace_and_mark_text_in_range(&mut self, range: Option<Range<usize>>, text: &str, selected: Option<Range<usize>>, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_native();
        if !self.focus.is_focused(window) || self.native_stale { return; }
        if self.native_suppress == Some((self.core.mode, self.core.field)) && range.is_some() { return; }
        let field = self.core.active();
        if self.native.mark(field.text(), field.cursor(), range, text, selected, cfg!(target_os = "macos")) {
            self.native_stale = false;
            self.native_suppress = None;
            cx.notify();
        }
    }
    fn bounds_for_range(&mut self, range: Range<usize>, _: Bounds<Pixels>, window: &mut Window, _: &mut Context<Self>) -> Option<Bounds<Pixels>> {
        self.sync_native();
        if !self.focus.is_focused(window) { return None; }
        let bounds = self.field_bounds(self.core.field, window.viewport_size())?;
        let field = self.core.active();
        let cw = font_size() * 0.6;
        let (shown, range) = self.native.snapshot_range(field.text(), field.cursor(), range, (f32::from(bounds.size.width) / cw) as usize)?;
        lenz_gpui::field::bounds_for_range(lenz_gpui::field::View {
            text: &shown.text, cursor: shown.cursor, bounds, char_w: cw, active: true,
        }, range, window, self.metrics())
    }
    fn character_index_for_point(&mut self, point: Point<Pixels>, window: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        self.sync_native();
        if !self.focus.is_focused(window) { return None; }
        let bounds = self.field_bounds(self.core.field, window.viewport_size())?;
        if !bounds.contains(&point) { return None; }
        let field = self.core.active();
        let cw = font_size() * 0.6;
        let shown = self.native.snapshot(field.text(), field.cursor(), (f32::from(bounds.size.width) / cw) as usize);
        let at = lenz_gpui::field::hit_test(lenz_gpui::field::View {
            text: &shown.text, cursor: shown.cursor, bounds, char_w: cw, active: true,
        }, point, window, self.metrics());
        let (selected, reversed) = self.native.selected(field.text(), field.cursor())?;
        let caret = if reversed { selected.start } else { selected.end };
        let local_caret = shown.text.get(..shown.cursor.caret(&shown.text))?.encode_utf16().count();
        Some(caret.saturating_sub(local_caret) + shown.text.get(..at)?.encode_utf16().count())
    }
    fn set_selected_text_range(&mut self, range: Range<usize>, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_native();
        if !self.focus.is_focused(window) || self.native_stale { return; }
        let field = self.core.active();
        if let Some(command) = self.native.set_selection(field.text(), field.cursor(), range) {
            self.edit_field(command, cx);
        }
        cx.notify();
    }
    fn text_length_utf16(&mut self, window: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        self.sync_native();
        if !self.focus.is_focused(window) { return None; }
        let field = self.core.active();
        Some(self.native.len_utf16(field.text(), field.cursor()))
    }
    fn accepts_text_input(&self, window: &mut Window, _: &mut Context<Self>) -> bool {
        self.focus.is_focused(window)
            && lenz_gpui::field::prefers_native_input(self.native_stale, cfg!(target_os = "macos"), window.modifiers().alt)
            && self.native_owner == (self.core.mode, self.core.field)
    }
}
impl Render for RepoChooser {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_native();
        self.native_registered.set(false);
        let native_input = (self.focus.clone(), cx.entity().clone(), self.native_registered.clone());
        let p = cx.global::<themes::GuiThemes>().0.snapshot().palette;
        let viewport = window.viewport_size();
        let (w, h) = (
            f32::from(viewport.width).max(0.),
            f32::from(viewport.height).max(0.),
        );
        let all = self.lines(&p, w, h);
        let fixed = all.iter().take_while(|l| l.3.is_none()).count();
        let room = (((h - 2. * PAD) / line_h()).floor() as usize).saturating_sub(fixed + 3);
        let selected = all
            .iter()
            .position(|l| l.2.is_some() && l.3.is_some())
            .map_or(0, |i| i - fixed);
        let skip = (selected + 1).saturating_sub(room.max(1));
        let lines: Vec<_> = all[..fixed]
            .iter()
            .cloned()
            .chain(all[fixed..].iter().skip(skip).take(room).cloned())
            .collect();
        self.rows = lines
            .iter()
            .enumerate()
            .filter_map(|(row, l)| l.3.map(|i| (i, PAD + row as f32 * line_h())))
            .collect();
        let by = (h - PAD - line_h()).max(0.);
        let button_w = ((w - 2. * PAD - 16.) / 3.).clamp(0., 160.);
        self.browse = (PAD, by, button_w);
        self.cancel = ((w - PAD - 2. * button_w - 8.).max(0.), by, button_w);
        self.submit = ((w - PAD - button_w).max(0.), by, button_w);
        let (browse, cancel, submit, font) =
            (self.browse, self.cancel, self.submit, self.font.clone());
        let input_row=self.field_row(h);let active=self.core.field;
        let compact = self.compact(h);
        let mode = self.core.mode;
        let fields:Vec<_>=(0..self.core.fields()).map(|i|{
            let field=if i==0{&self.core.input}else{&self.core.right};
            let bounds = self.field_bounds(i, viewport);
            let columns = bounds.map_or(0, |b| (f32::from(b.size.width) / (font_size() * 0.6)) as usize);
            let shown = if i == active { self.native.snapshot(field.text(), field.cursor(), columns) }
                else { field.cursor().snapshot(field.text(), columns) };
            (i,self.core.mode.field(i),shown,bounds)
        }).collect();
        let label = self.core.mode.submit();
        let hint = if self.core.mode == Mode::Repository {
            "Tab complete · ↑/↓ recent · Ctrl+A select · Enter open · Esc cancel"
        } else {
            "Tab next path · Ctrl+Space complete · Enter compare · Esc cancel"
        };
        div().relative()
            .size_full()
            .bg(rgb(p.bg))
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_click))
            .child(canvas(|_,_,_|{},move|bounds,_,window,cx| {
                window.handle_input(&native_input.0, gpui::ElementInputHandler::new(bounds, native_input.1.clone()), cx);
                native_input.2.set(true);
            }).absolute().size_full())
            .child(
                canvas(
                    |_, _, _| {},
                    move |_, _, window, cx| {
                        let bounds = Bounds {
                            origin: point(px(0.), px(0.)),
                            size: size(px(w), px((by - line_h()).max(0.))),
                        };
                        window.with_content_mask(Some(ContentMask { bounds }), |window| {
                            for (row, (s, color, bg, _)) in lines.iter().enumerate() {
                                let y = PAD + row as f32 * line_h();
                                if let Some(bg) = bg {
                                    fill(
                                        window,
                                        PAD - 4.,
                                        y,
                                        (w - 2. * PAD + 8.).max(0.),
                                        line_h(),
                                        *bg,
                                    );
                                }
                                if compact && row == 1 {
                                    let width = (w - 2.0 * PAD).max(0.0) / 3.0;
                                    for (i, (choice, label)) in Mode::ALL.into_iter().zip(["F1 File", "F2 Dir", "F3 Repo"]).enumerate() {
                                        let x = PAD + i as f32 * width;
                                        if mode == choice { fill(window, x, y, width, line_h(), p.selected_bg); }
                                        text(window, cx, &font, x, y, label, p.accent);
                                    }
                                } else if let Some((i,label,snapshot,bounds))=fields.iter().find(|(i,_,_,_)|row==input_row+*i) {
                                    text(window,cx,&font,PAD,y,&format!("{label}: "),*color);
                                    if let Some(bounds) = bounds {
                                        lenz_gpui::field::paint(lenz_gpui::field::View{text:&snapshot.text,cursor:snapshot.cursor,
                                            bounds:*bounds,char_w:font_size()*0.6,active:*i==active},window,cx,
                                            lenz_gpui::Metrics{font:&font,font_size:font_size(),line_h:line_h(),num_w:0.},&p);
                                    }
                                } else { text(window, cx, &font, PAD, y, s, *color); }
                            }
                        });
                        text(window, cx, &font, PAD, (by - line_h()).max(0.), hint, p.dim);
                        for ((x, y, width), label) in [
                            (
                                browse,
                                if browse.2 < 150. {
                                    "Browse…"
                                } else {
                                    "Browse… (Ctrl+O)"
                                },
                            ),
                            (cancel, if compact { "Cancel" } else { "Cancel (Esc)" }),
                            (submit, label),
                        ] {
                            fill(window, x, y, width, line_h(), p.selected_bg);
                            text(window, cx, &font, x + 4., y, label, p.accent);
                        }
                    },
                )
                .size_full(),
            )
    }
}
#[cfg(all(test, feature = "gui-test"))]
mod gui_tests {
    use super::*;
    use crate::ui::merge_view::gui_tests::event;
    use gpui::{TestAppContext, VisualTestContext};

    fn git(dir: &Path) {
        assert!(
            std::process::Command::new("git")
                .args(["init", "-q"])
                .current_dir(dir)
                .status()
                .unwrap()
                .success()
        );
    }

    fn native_chooser(cx: &mut TestAppContext, core: RepoPicker, width: f32, height: f32) -> (gpui::Entity<RepoChooser>, gpui::WindowHandle<RepoChooser>) {
        cx.update(themes::install);
        let chooser = cx.new(|cx| RepoChooser::new(core, gpui::font("monospace"), cx));
        let window = cx.update(|cx| cx.open_window(gpui::WindowOptions {
            window_bounds: Some(gpui::WindowBounds::Windowed(Bounds {
                origin: point(px(0.0), px(0.0)), size: size(px(width), px(height)),
            })), ..Default::default()
        }, |_, _| chooser.clone())).unwrap();
        window.update(cx, |c, w, cx| w.focus(&c.focus, cx)).unwrap();
        (chooser, window)
    }

    #[gpui::test]
    fn gui_repo_chooser_registered_dead_key_does_not_insert_its_physical_name(cx: &mut TestAppContext) {
        let _guard = TEST_CHOOSER.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("keep"), "unchanged file\n").unwrap();
        let mut core = RepoPicker::with_recent(dir.path().into(), vec![]).with_comparisons(true);
        core.input = crate::line_edit::Input::new("keep".into());
        let (_, window) = native_chooser(cx, core, 320.0, 360.0);
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        visual.update(|w, cx| { let _ = w.draw(cx); });
        visual.simulate_keystrokes("ctrl-a");
        window.update(&mut visual, |c, w, cx| c.replace_and_mark_text_in_range(None, "´", None, w, cx)).unwrap();
        let mut dead = event("'", false, false, false);
        dead.keystroke.key_char = None;
        visual.simulate_event(dead.clone());
        window.update(&mut visual, |c, w, cx| {
            assert_eq!(c.core.input.text(), "keep", "the dead key's physical name is not committed text");
            c.replace_text_in_range(None, "é", w, cx);
            assert_eq!(c.core.input.text(), "é");
        }).unwrap();
        visual.simulate_keystrokes("tab");
        window.update(&mut visual, |c, w, cx| {
            // X11 can publish its first preedit before delivering the raw key after a field change.
            c.replace_and_mark_text_in_range(None, "´", None, w, cx);
            assert_eq!(c.core.right.text(), "");
        }).unwrap();
        visual.simulate_event(dead);
        window.update(&mut visual, |c, w, cx| {
            assert_eq!(c.core.right.text(), "");
            c.replace_text_in_range(None, "é", w, cx);
            assert_eq!(c.core.right.text(), "é", "a preedit-before-key sequence commits only the composed character");
            assert_eq!(c.core.input.text(), "é");
        }).unwrap();
        assert_eq!(std::fs::read(dir.path().join("keep")).unwrap(), b"unchanged file\n");
    }

    #[gpui::test]
    #[cfg(not(target_os = "macos"))]
    fn gui_repo_chooser_linux_cancel_preedit_preserves_the_selected_path(cx: &mut TestAppContext) {
        let _guard = TEST_CHOOSER.lock().unwrap();
        let _ = chosen();
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("keep"), "unchanged file\n").unwrap();
        let mut core = RepoPicker::with_recent(dir.path().into(), vec![]).with_comparisons(true);
        core.input = crate::line_edit::Input::new("keep".into());
        let (_, window) = native_chooser(cx, core, 320.0, 360.0);
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        visual.update(|w, cx| { let _ = w.draw(cx); });
        visual.simulate_keystrokes("ctrl-a");
        visual.simulate_event(event("dead_acute", false, false, false));
        window.update(&mut visual, |c, w, cx| {
            c.replace_and_mark_text_in_range(None, "候補", None, w, cx);
            assert_eq!(c.core.input.text(), "keep");
            let marked = c.marked_text_range(w, cx).unwrap();
            assert_eq!(marked, 0..2);
            // X11 and Wayland erase their projected preedit using this native callback.
            c.replace_text_in_range(Some(marked), "", w, cx);
            assert_eq!(c.core.input.text(), "keep");
            assert_eq!(c.core.input.selection(), Some("keep"));
            assert_eq!(c.marked_text_range(w, cx), None);
            c.replace_text_in_range(None, "late commit", w, cx);
            c.replace_and_mark_text_in_range(None, "late mark", None, w, cx);
            assert_eq!(c.core.input.text(), "keep");
            assert_eq!(c.marked_text_range(w, cx), None);
        }).unwrap();
        assert_eq!(chosen(), None, "cancelling input never launches a comparison");
        visual.simulate_event(event("dead_acute", false, false, false));
        window.update(&mut visual, |c, w, cx| {
            c.replace_and_mark_text_in_range(None, "候補", None, w, cx);
            c.replace_text_in_range(None, "done", w, cx);
            assert_eq!(c.core.input.text(), "done");
            c.replace_text_in_range(Some(0..4), "", w, cx);
            assert_eq!(c.core.input.text(), "", "an ordinary empty replacement still deletes committed field text");
        }).unwrap();
        assert_eq!(std::fs::read(dir.path().join("keep")).unwrap(), b"unchanged file\n");
    }

    #[gpui::test]
    fn gui_repo_chooser_native_composition_preserves_paths_until_commit_and_rejects_stale_input(cx: &mut TestAppContext) {
        let _guard = TEST_CHOOSER.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a😀z"), "untouched left\n").unwrap();
        std::fs::write(dir.path().join("right"), "untouched right\n").unwrap();
        let mut core = RepoPicker::with_recent(dir.path().into(), vec![]).with_comparisons(true);
        core.input = crate::line_edit::Input::new("a😀z".into());
        core.right = crate::line_edit::Input::new("right".into());
        let (chooser, window) = native_chooser(cx, core, 320.0, 360.0);
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        visual.update(|w, cx| { let _ = w.draw(cx); });
        assert!(chooser.read_with(&visual, |c, _| c.native_registered.get()));
        visual.simulate_keystrokes("home right shift-right");
        visual.simulate_event(event("dead_acute", false, false, false));
        window.update(&mut visual, |c, w, cx| {
            c.replace_and_mark_text_in_range(None, "仮", Some(0..1), w, cx);
            assert_eq!(c.core.input.text(), "a😀z", "preedit cannot edit the chosen path");
            assert_eq!(c.marked_text_range(w, cx), Some(1..2));
            assert_eq!(c.selected_text_range(false, w, cx).unwrap().range, 1..2);
            let mut adjusted = None;
            assert_eq!(c.text_for_range(0..3, &mut adjusted, w, cx).as_deref(), Some("a仮z"));
            assert_eq!(adjusted, Some(0..3));
            let bounds = c.bounds_for_range(1..2, Bounds::default(), w, cx).unwrap();
            assert!(bounds.origin.x >= px(0.0) && bounds.origin.y >= px(0.0));
            assert!(bounds.origin.x + bounds.size.width <= w.viewport_size().width);
            c.replace_text_in_range(None, "確定", w, cx);
            assert_eq!(c.core.input.text(), "a確定z");
            c.replace_text_in_range(None, "™", w, cx);
            assert_eq!(c.core.input.text(), "a確定™z", "ordinary OS insertions do not require a preceding key event");
            c.replace_text_in_range(Some(3..4), "", w, cx);
            assert_eq!(c.core.input.text(), "a確定z");
            c.replace_and_mark_text_in_range(None, "続", None, w, cx);
            c.unmark_text(w, cx);
            assert_eq!(c.core.input.text(), "a確定続z");
            c.replace_text_in_range(None, "続", w, cx);
            assert_eq!(c.core.input.text(), "a確定続z", "the unmark tail cannot insert its preedit twice");
            c.replace_and_mark_text_in_range(None, "次", None, w, cx);
            assert!(c.marked_text_range(w, cx).is_some(), "a fresh native preedit can follow unmark without another raw event");
        }).unwrap();
        visual.simulate_keystrokes("tab");
        window.update(&mut visual, |c, w, cx| {
            assert!(!c.accepts_text_input(w, cx), "a changed field waits for a fresh input gesture");
            c.replace_text_in_range(None, "late commit", w, cx);
            c.replace_and_mark_text_in_range(None, "late mark", None, w, cx);
            assert_eq!(c.core.right.text(), "right");
            assert_eq!(c.marked_text_range(w, cx), None);
        }).unwrap();
        visual.simulate_event(event("dead_acute", false, false, false));
        window.update(&mut visual, |c, w, cx| {
            assert!(c.accepts_text_input(w, cx), "a fresh input gesture re-enables native composition");
            c.replace_and_mark_text_in_range(None, "仮", None, w, cx);
            assert!(c.marked_text_range(w, cx).is_some());
        }).unwrap();
        visual.simulate_keystrokes("escape");
        window.update(&mut visual, |c, w, cx| {
            assert_eq!(c.marked_text_range(w, cx), None);
            c.replace_text_in_range(None, "cancelled commit", w, cx);
            assert_eq!(c.core.right.text(), "right");
        }).unwrap();
        visual.simulate_event(event("dead_acute", false, false, false));
        window.update(&mut visual, |c, w, cx| c.replace_and_mark_text_in_range(None, "仮", None, w, cx)).unwrap();
        visual.simulate_keystrokes("f2");
        window.update(&mut visual, |c, w, cx| {
            c.replace_and_mark_text_in_range(None, "late mode mark", None, w, cx);
            c.replace_text_in_range(None, "late mode commit", w, cx);
            assert_eq!(c.core.mode, Mode::Folders);
            assert!(!c.accepts_text_input(w, cx));
            assert_eq!(c.core.input.text(), "a確定続z");
            assert_eq!(c.marked_text_range(w, cx), None);
        }).unwrap();
        assert_eq!(std::fs::read(dir.path().join("a😀z")).unwrap(), b"untouched left\n");
        assert_eq!(std::fs::read(dir.path().join("right")).unwrap(), b"untouched right\n");
    }

    #[gpui::test]
    fn gui_repo_chooser_native_text_and_unicode_mouse_selection_work_in_a_narrow_window(cx: &mut TestAppContext) {
        let _guard = TEST_CHOOSER.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let mut core = RepoPicker::with_recent(dir.path().into(), vec![]).with_comparisons(true);
        core.input = crate::line_edit::Input::new("a😀z".into());
        let (chooser, window) = native_chooser(cx, core, 240.0, 180.0);
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        visual.update(|w, cx| { let _ = w.draw(cx); });
        window.update(&mut visual, |c, w, _| {
            for field in 0..2 {
                let bounds = c.field_bounds(field, w.viewport_size()).unwrap();
                assert_eq!(bounds.size.height, px(line_h()));
                assert!(bounds.origin.y + bounds.size.height < px(c.browse.1 - line_h()), "both field rows must remain above the footer");
            }
        }).unwrap();
        let position = |visual: &mut VisualTestContext, at: usize| window.update(visual, |c, w, _| {
            let field = c.core.active();
            let bounds = c.field_bounds(c.core.field, w.viewport_size()).unwrap();
            let caret = lenz_gpui::field::bounds_for_range(lenz_gpui::field::View {
                text: field.text(), cursor: field.cursor(), bounds,
                char_w: font_size() * 0.6, active: true,
            }, at..at, w, c.metrics()).unwrap();
            point(caret.origin.x + px(0.25), caret.origin.y + px(line_h() / 2.0))
        }).unwrap();
        let a = position(&mut visual, 1);
        visual.simulate_event(MouseDownEvent { button: MouseButton::Left, position: a, ..Default::default() });
        assert_eq!(chooser.read_with(&visual, |c, _| c.core.input.caret()), 1);
        let b = position(&mut visual, 5);
        visual.simulate_event(MouseDownEvent { button: MouseButton::Left, position: b,
            modifiers: gpui::Modifiers { shift: true, ..Default::default() }, ..Default::default() });
        assert_eq!(chooser.read_with(&visual, |c, _| c.core.input.selection().map(str::to_owned)).as_deref(), Some("😀"));
        visual.simulate_keystrokes("ctrl-c");
        assert_eq!(visual.update(|_, cx| cx.read_from_clipboard().and_then(|c| c.text())).as_deref(), Some("😀"));
        window.update(&mut visual, |c, w, cx| {
            c.edit_field(FieldCommand::Select { anchor: 5, caret: 1 }, cx);
            assert_eq!(c.character_index_for_point(b, w, cx), Some(3), "reverse selection still reports UTF-16 coordinates");
        }).unwrap();
        let mut option = event("2", false, false, true);
        option.keystroke.key_char = Some("™".into());
        visual.simulate_event(option);
        if cfg!(target_os = "macos") {
            // The test platform has no Cocoa input context. Its native callback follows the unhandled raw key.
            assert_eq!(chooser.read_with(&visual, |c, _| c.core.input.text().to_string()), "a😀z");
            window.update(&mut visual, |c, w, cx| c.replace_text_in_range(None, "™", w, cx)).unwrap();
        }
        assert_eq!(chooser.read_with(&visual, |c, _| c.core.input.text().to_string()), "a™z", "native text replaces the selection exactly once");
        for (index, mode) in [(1, Mode::Folders), (2, Mode::Repository), (0, Mode::Files)] {
            let position = window.update(&mut visual, |_, w, _| point(
                px(PAD + (f32::from(w.viewport_size().width) - 2.0 * PAD) * (index as f32 + 0.5) / 3.0),
                px(PAD + 1.5 * line_h()),
            )).unwrap();
            visual.simulate_event(MouseDownEvent { button: MouseButton::Left, position, ..Default::default() });
            assert_eq!(chooser.read_with(&visual, |c, _| c.core.mode), mode, "all compact mode choices remain clickable");
        }
    }

    #[gpui::test]
    fn gui_repo_chooser_types_completes_explains_and_opens(cx: &mut TestAppContext) {
        let _guard = TEST_CHOOSER.lock().unwrap();
        let base = tempfile::tempdir().unwrap();
        std::fs::create_dir(base.path().join("project")).unwrap();
        git(&base.path().join("project"));
        std::fs::create_dir(base.path().join("plain")).unwrap();
        let recent = tempfile::tempdir().unwrap();
        git(recent.path());
        cx.update(themes::install);
        let core = RepoPicker::with_recent(base.path().into(), vec![recent.path().into()]);
        let chooser = cx.new(|cx| RepoChooser::new(core, gpui::font("monospace"), cx));
        let window = cx
            .update(|cx| cx.open_window(Default::default(), |_, _| chooser.clone()))
            .unwrap();
        let key = |cx: &mut TestAppContext, name: &str, ctrl: bool| {
            window
                .update(cx, |c, w, cx| {
                    c.on_key(&event(name, ctrl, false, false), w, cx)
                })
                .unwrap();
            cx.executor().run_until_parked();
        };
        for name in ["p", "l", "tab"] {
            key(cx, name, false);
        }
        assert_eq!(
            chooser.read_with(cx, |c, _| c.core.input.text().to_string()),
            "plain/"
        );
        key(cx, "enter", false);
        assert!(
            chooser
                .read_with(cx, |c, _| c.core.error.clone())
                .unwrap()
                .starts_with("Not a Git repository")
        );
        assert_eq!(chosen(), None, "a refused folder keeps the chooser open");
        for _ in 0.."plain/".len() {
            key(cx, "backspace", false);
        }
        key(cx, "x", true);
        assert_eq!(
            chooser.read_with(cx, |c, _| c.core.input.text().to_string()),
            "",
            "modified keys do not type"
        );
        let lines = chooser.read_with(cx, |c, cx| {
            c.lines(
                &cx.global::<themes::GuiThemes>().0.snapshot().palette,
                800.0,
                600.0,
            )
        });
        assert!(
            lines
                .iter()
                .any(|l| l.3 == Some(0) && l.0 == recent.path().display().to_string()),
            "recent row listed"
        );
        for name in ["p", "r", "tab", "enter"] {
            key(cx, name, false);
        }
        assert_eq!(
            chosen().unwrap(),
            Choice::Repository(crate::changeset::toplevel(&base.path().join("project")).unwrap())
        );
    }
    #[gpui::test]
    fn gui_comparison_chooser_preserves_native_and_pasted_path_whitespace(
        cx: &mut TestAppContext,
    ) {
        let _guard = TEST_CHOOSER.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        for name in ["left", "left ", " right"] {
            std::fs::write(dir.path().join(name), name).unwrap();
        }
        cx.update(themes::install);
        let picker = RepoPicker::with_recent(dir.path().into(), vec![]).with_comparisons(true);
        let chooser = cx.new(|cx| RepoChooser::new(picker, gpui::font("monospace"), cx));
        let window = cx
            .update(|cx| cx.open_window(Default::default(), |_, _| chooser.clone()))
            .unwrap();
        // The native Browse result is dispatched through this command, then advances to Right.
        window.update(cx, |c, _, cx| {
            c.command(PickerCommand::Choose(dir.path().join("left ")), cx);
        }).unwrap();
        cx.update(|cx| cx.write_to_clipboard(ClipboardItem::new_string(" right".into())));
        for (name, ctrl) in [("v", true), ("enter", false)] {
            window.update(cx, |c, w, cx| {
                c.on_key(&event(name, ctrl, false, false), w, cx);
            }).unwrap();
            cx.executor().run_until_parked();
        }
        assert_eq!(chosen(), Some(Choice::Comparison {
            left: dir.path().join("left "), right: dir.path().join(" right"),
        }));
        assert_eq!(std::fs::read(dir.path().join("left ")).unwrap(), b"left ");
    }

    #[gpui::test]
    fn gui_comparison_chooser_replaces_paths_pastes_and_refuses_wrong_types(
        cx: &mut TestAppContext,
    ) {
        let _guard = TEST_CHOOSER.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        for name in ["left é.txt", "right.txt"] {
            std::fs::write(dir.path().join(name), name).unwrap();
        }
        cx.update(themes::install);
        let picker = RepoPicker::with_recent(dir.path().into(), vec![]).with_comparisons(true);
        let chooser = cx.new(|cx| RepoChooser::new(picker, gpui::font("monospace"), cx));
        let window = cx
            .update(|cx| cx.open_window(Default::default(), |_, _| chooser.clone()))
            .unwrap();
        let key = |cx: &mut TestAppContext, name: &str, ctrl: bool| {
            window
                .update(cx, |c, w, cx| {
                    c.on_key(&event(name, ctrl, false, false), w, cx)
                })
                .unwrap();
            cx.executor().run_until_parked();
        };
        cx.update(|cx| cx.write_to_clipboard(ClipboardItem::new_string("/missing/path".into())));
        key(cx, "v", true);
        key(cx, "enter", false);
        assert!(chooser.read_with(cx, |c, _| c.core.error.is_some()));
        key(cx, "a", true);
        cx.update(|cx| cx.write_to_clipboard(ClipboardItem::new_string("left é.txt".into())));
        key(cx, "v", true);
        key(cx, "home", false);
        key(cx, "end", false);
        key(cx, "tab", false);
        cx.update(|cx| cx.write_to_clipboard(ClipboardItem::new_string("right.txt".into())));
        key(cx, "v", true);
        key(cx, "f2", false);
        key(cx, "enter", false);
        assert!(chooser.read_with(cx, |c, _| c.core.error.as_ref().unwrap().contains("folder")));
        key(cx, "f1", false);
        key(cx, "enter", false);
        assert_eq!(
            chosen(),
            Some(Choice::Comparison {
                left: dir.path().join("left é.txt"),
                right: dir.path().join("right.txt")
            })
        );
        assert_eq!(
            std::fs::read(dir.path().join("right.txt")).unwrap(),
            b"right.txt"
        );
    }
}
