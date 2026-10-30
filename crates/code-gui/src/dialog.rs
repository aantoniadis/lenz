//! Form and confirmation layout. Submission uses the existing modal key path and core validation.
use super::*;
use code_core::chrome::forms::{Form, FormText};

pub(super) struct FormLayout {
    pub bounds: Bounds<Pixels>,
    pub field: Option<Bounds<Pixels>>,
    pub checkbox: Option<Bounds<Pixels>>,
    pub buttons: [Bounds<Pixels>; 2],
    pub text: FormText,
    value: String,
    selected: bool,
    checked: bool,
    lines: Vec<(String, bool)>,
    pub first: usize,
    pub visible: usize,
    pub max_scroll: usize,
    body_y: f32,
    char_w: f32,
    pad: f32,
    invalid: bool,
    inline_field: bool,
    label_width: f32,
}

fn rect(x: f32, y: f32, w: f32, h: f32) -> Bounds<Pixels> {
    Bounds { origin: gpui::point(px(x), px(y)), size: gpui::size(px(w.max(0.0)), px(h.max(0.0))) }
}

impl ReaderView {
    pub(super) fn form_layout(&self) -> Option<FormLayout> {
        let (form, value, selected, check, error) = match self.modal.as_ref()? {
            Modal::Create { kind, dir, name, error } => (Form::Create(*kind, dir), name.as_str(), false, None, error.as_deref()),
            Modal::SaveAs { input, selected, error, .. } => (Form::SaveAs, input.as_str(), *selected, None, error.as_deref()),
            Modal::RenameTerminal { input, selected, error, .. } => (Form::RenameTerminal, input.as_str(), *selected, None, error.as_deref()),
            Modal::NewScratch { input, selected, error } => (Form::Scratch, input.as_str(), *selected, None, error.as_deref()),
            Modal::Relocate { kind, from, input, error } => (Form::Relocate(*kind, from), input.as_str(), false, None, error.as_deref()),
            Modal::PasteEntry { request, name, selected, error } => (Form::Paste(request), name.as_str(), *selected, None, error.as_deref()),
            Modal::ConfirmDelete { removal, from, error } => (Form::Delete(*removal, from), "", false, None, error.as_deref()),
            Modal::NewBranch { from, name, checkout, error } => (Form::Branch(from.as_deref()), name.as_str(), false, Some(*checkout), error.as_deref()),
            Modal::Revision { input, error } => (Form::Revision, input.as_str(), false, None, error.as_deref()),
            Modal::GoTo { input, error } => (Form::GoTo, input.as_str(), false, None, error.as_deref()),
            Modal::Root { path } => (Form::Workspace, path.as_str(), false, None, None),
            Modal::SqlMetadata { path } => (Form::SqlMetadata, path.as_str(), false, None, None),
            _ => return None,
        };
        let text = self.session.form_text(form);
        let w = (self.width - 16.0).clamp(0.0, 560.0);
        let compact = self.height < 240.0;
        let pad = (if compact { 4.0_f32 } else { 16.0 }).min(w / 4.0);
        let cols = self.columns(w - 2.0 * pad).max(1);
        let lines: Vec<_> = error.into_iter().map(|s| (s, true)).chain([(text.description.as_str(), false)])
            .flat_map(|(s, error)| s.split('\n').flat_map(move |s| wrap(s, cols).into_iter().map(move |s| (s, error))))
            .collect();
        let lh = line_h();
        let field_h = if text.field.is_some() { if compact { lh + 4.0 } else { 2.0 * lh + 16.0 } } else { 0.0 };
        let check_h = if check.is_some() { lh + 8.0 } else { 0.0 };
        let button_h = lh + if compact { 0.0 } else { 12.0 };
        let stacked = w < 400.0 && !compact;
        let footer_h = if stacked { 2.0 * button_h + 8.0 } else { button_h };
        let header_h = pad + lh + if compact { 0.0 } else { 16.0 };
        let body_top = header_h + field_h + check_h;
        let h = (body_top + lines.len() as f32 * lh + pad + footer_h + pad).min((self.height - 16.0).max(0.0));
        let (x, y) = ((self.width - w) / 2.0, (self.height - h) / 2.0);
        let bounds = rect(x, y, w, h);
        let fit = |rx: f32, ry: f32, rw: f32, rh: f32| {
            let left = rx.clamp(x, x + w); let top = ry.clamp(y, y + h);
            rect(left, top, (rx + rw).clamp(left, x + w) - left, (ry + rh).clamp(top, y + h) - top)
        };
        let footer_y = (y + h - pad - footer_h).max(y);
        let button_w = if stacked { w - 2.0 * pad } else { (w - 2.0 * pad - 12.0) / 2.0 };
        let buttons = if stacked {
            [fit(x + pad, footer_y, button_w, button_h), fit(x + pad, footer_y + button_h + 8.0, button_w, button_h)]
        } else {
            [fit(x + pad, footer_y, button_w, button_h), fit(x + pad + button_w + 12.0, footer_y, button_w, button_h)]
        };
        let body_y = (y + body_top).min(footer_y);
        let visible = (((footer_y - pad - body_y).max(0.0) / lh).floor() as usize).min(lines.len());
        let first = self.modal_first(0, 0..0, lines.len(), visible);
        let label_width = if compact { text.field.map_or(0.0, |label| ((lenz_content::wrap::width(label) as f32 + 1.0) * self.char_w).min((w - 2.0 * pad) / 3.0)) } else { 0.0 };
        Some(FormLayout { bounds, field: text.field.map(|_| { let fy = y + header_h + if compact { 0.0 } else { lh + 4.0 }; fit(x + pad + label_width, fy, w - 2.0 * pad - label_width, (lh + if compact { 4.0 } else { 8.0 }).min((footer_y - 4.0 - fy).max(0.0))) }),
            checkbox: check.map(|_| fit(x + pad, y + header_h + field_h, w - 2.0 * pad, lh + 4.0)),
            buttons, text, value: value.into(), selected, checked: check.unwrap_or(false),
            max_scroll: lines.len().saturating_sub(visible), lines, first, visible, body_y, char_w: self.char_w, pad, invalid: error.is_some(), inline_field: compact, label_width })
    }

    pub(super) fn form_click(&mut self, point: gpui::Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(l) = self.form_layout() else { return false };
        if !l.bounds.contains(&point) || l.buttons[0].contains(&point) { self.close_modal(cx); }
        else if l.buttons[1].contains(&point) {
            if let Ok(keystroke) = gpui::Keystroke::parse("enter") {
                self.modal_key(&KeyDownEvent { keystroke, is_held: false, prefer_character_input: false }, window, cx);
            }
        } else if l.checkbox.is_some_and(|b| b.contains(&point)) && let Some(Modal::NewBranch { checkout, .. }) = &mut self.modal {
            *checkout = !*checkout;
        }
        cx.notify(); true
    }
}

impl FormLayout {
    pub(super) fn paint(&self, window: &mut Window, cx: &mut App, m: Metrics, p: &lenz_content::theme::Palette, x0: f32, y0: f32) {
        let (x, y, w, h) = (x0 + f32::from(self.bounds.origin.x), y0 + f32::from(self.bounds.origin.y), f32::from(self.bounds.size.width), f32::from(self.bounds.size.height));
        let pad = self.pad;
        let (bg, border) = p.raised();
        lenz_gpui::fill(window, x - 1.0, y - 1.0, w + 2.0, h + 2.0, border);
        lenz_gpui::fill(window, x, y, w, h, bg);
        let mask = rect(x, y, w, h);
        window.with_content_mask(Some(gpui::ContentMask { bounds: mask }), |window| {
            let heading = Font { weight: gpui::FontWeight::BOLD, ..m.font.clone() };
            let title = code_core::chrome::ellipsize(self.text.title, ((w - 2.0 * pad) / self.char_w).max(0.0) as usize);
            lenz_gpui::text(window, cx, Metrics { font: &heading, ..m }, x + pad, y + pad, &title, p.fg);
            if let Some(b) = self.field {
                let (fx, fy, fw, fh) = (x0 + f32::from(b.origin.x), y0 + f32::from(b.origin.y), f32::from(b.size.width), f32::from(b.size.height));
                let label = code_core::chrome::ellipsize(self.text.field.unwrap_or_default(), if self.inline_field { (self.label_width / self.char_w).floor().max(0.0) as usize } else { ((w - 2.0 * pad) / self.char_w).floor().max(0.0) as usize });
                lenz_gpui::text(window, cx, m, if self.inline_field { x + pad } else { fx }, if self.inline_field { fy + 2.0 } else { fy - line_h() - 4.0 }, &label, p.fg);
                lenz_gpui::fill(window, fx, fy, fw, fh, if self.invalid { p.error } else { p.accent });
                lenz_gpui::fill(window, fx + 1.0, fy + 1.0, (fw - 2.0).max(0.0), (fh - 2.0).max(0.0), p.bg);
                let shown = navigation::tail(&self.value, ((fw - 20.0) / self.char_w).max(0.0) as usize);
                window.with_content_mask(Some(gpui::ContentMask { bounds: rect(fx + 1.0, fy + 1.0, (fw - 2.0).max(0.0), (fh - 2.0).max(0.0)) }), |window| {
                    let sw = lenz_content::wrap::width(&shown) as f32 * self.char_w;
                    if self.selected { lenz_gpui::fill(window, fx + 8.0, fy + 4.0, sw, line_h(), p.selected_bg); }
                    lenz_gpui::text(window, cx, m, fx + 8.0, fy + 4.0, &shown, p.fg);
                    lenz_gpui::fill(window, fx + 8.0 + sw, fy + 4.0, 2.0, line_h(), p.caret);
                });
            }
            if let Some(b) = self.checkbox {
                let (cx0, cy) = (x0 + f32::from(b.origin.x), y0 + f32::from(b.origin.y));
                lenz_gpui::text(window, cx, m, cx0, cy, if self.checked { "[x] Check out the new branch" } else { "[ ] Check out the new branch" }, p.fg);
            }
            for (i, (line, error)) in self.lines.iter().skip(self.first).take(self.visible).enumerate() {
                lenz_gpui::text(window, cx, m, x + pad, y0 + self.body_y + i as f32 * line_h(), line, if *error { p.readable_on(p.error, bg) } else { p.fg });
            }
            for (i, (b, label)) in self.buttons.iter().zip([if self.inline_field { "Cancel" } else { "Cancel  Esc" }, self.text.submit]).enumerate() {
                let (bx, by, bw, bh) = (x0 + f32::from(b.origin.x), y0 + f32::from(b.origin.y), f32::from(b.size.width), f32::from(b.size.height));
                let hover = rect(bx, by, bw, bh).contains(&window.mouse_position());
                let edge = if i == 1 { if self.text.destructive { p.error } else { p.accent } } else { border };
                lenz_gpui::fill(window, bx, by, bw, bh, edge);
                lenz_gpui::fill(window, bx + 1.0, by + 1.0, (bw - 2.0).max(0.0), (bh - 2.0).max(0.0), if hover || i == 1 && !self.text.destructive { p.selected_bg } else { bg });
                let shown = code_core::chrome::ellipsize(label, ((bw - 16.0) / self.char_w).max(0.0) as usize);
                let tx = bx + ((bw - lenz_content::wrap::width(&shown) as f32 * self.char_w) / 2.0).max(0.0);
                window.with_content_mask(Some(gpui::ContentMask { bounds: rect(bx, by, bw, bh) }), |window| lenz_gpui::text(window, cx, m, tx, by + ((bh - line_h()) / 2.0).max(0.0), &shown, if i == 1 && self.text.destructive { p.readable_on(p.error, bg) } else { p.fg }));
            }
        });
    }
}
