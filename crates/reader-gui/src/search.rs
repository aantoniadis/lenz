//! Pixel layout and input for the project search box: a centered dialog with its two fields, option toggles and hint.
use super::*;

const PAD: f32 = 14.0;
/// The label column before each field: "Files" and a space.
const LABEL_COLS: f32 = 6.0;

/// The dialog as drawn and clicked; fields and toggles are in window coordinates like `bounds`.
pub(super) struct Layout {
    pub(super) bounds: Bounds<Pixels>,
    /// The text field, then the file-pattern field.
    pub(super) fields: [Bounds<Pixels>; 2],
    pub(super) toggles: [(FindOption, Bounds<Pixels>); 3],
    hint_y: f32,
    hint: Vec<String>,
    /// The hint is why the expression is refused.
    refused: bool,
    query: String,
    files: String,
    in_files: bool,
    on: [bool; 3],
    char_w: f32,
}

fn rect(x: f32, y: f32, w: f32, h: f32) -> Bounds<Pixels> {
    Bounds { origin: gpui::point(px(x), px(y)), size: gpui::size(px(w.max(0.0)), px(h.max(0.0))) }
}

impl ReaderView {
    pub(super) fn search_layout(&self) -> Option<Layout> {
        let Some(Modal::Search { query, files, in_files, .. }) = &self.modal else { return None };
        let (lh, cw) = (line_h(), self.char_w);
        let w = (self.width - 16.0).clamp(0.0, 640.0);
        let x = (self.width - w) / 2.0;
        let right = x + w - PAD;
        let o = self.session.find_options();
        let refused = self.session.search_refusal(query);
        let hint = match &refused {
            Some(e) => format!("{e} — fix the expression to search"),
            None => "Enter search · Tab switch field · Alt+C/W/R options · Esc close".to_string(),
        };
        let hint = wrap(&hint, self.columns(w - 2.0 * PAD).max(1));
        // Rows from the dialog's top: title, the two fields, the toggles (wrapping when narrow), the hint.
        let field_x = x + PAD + LABEL_COLS * cw;
        let field_h = lh + 8.0;
        let fields = [0.0, 1.0].map(|i| rect(field_x, PAD + lh + 10.0 + i * (field_h + 6.0), right - field_x, field_h));
        let (mut tx, mut ty, toggle_h) = (x + PAD, PAD + lh + 10.0 + 2.0 * field_h + 6.0 + 12.0, lh + 6.0);
        let toggles = FindOption::ALL.map(|option| {
            let tw = ((option.symbol().chars().count() + option.label().chars().count() + 4) as f32 * cw).min(right - x - PAD);
            if tx > x + PAD && tx + tw > right {
                (tx, ty) = (x + PAD, ty + toggle_h + 6.0);
            }
            let b = rect(tx, ty, tw, toggle_h);
            tx += tw + 8.0;
            (option, b)
        });
        let hint_y = ty + toggle_h + 12.0;
        let h = hint_y + hint.len() as f32 * lh + PAD;
        // Centered; a window too short for it shows its top.
        let top = ((self.height - h) / 2.0).max(0.0);
        let down = |b: Bounds<Pixels>| Bounds { origin: gpui::point(b.origin.x, b.origin.y + px(top)), size: b.size };
        Some(Layout {
            bounds: rect(x, top, w, h.min(self.height - top)),
            fields: fields.map(down),
            toggles: toggles.map(|(option, b)| (option, down(b))),
            hint_y: top + hint_y,
            hint,
            refused: refused.is_some(),
            query: query.clone(),
            files: files.clone(),
            in_files: *in_files,
            on: [o.case, o.word, o.regex],
            char_w: cw,
        })
    }

    /// A toggle flips its option and a field takes the keys; a click outside closes the dialog.
    pub(super) fn search_click(&mut self, point: gpui::Point<Pixels>, cx: &mut Context<Self>) {
        let Some(l) = self.search_layout() else { return };
        if !l.bounds.contains(&point) {
            return self.close_modal(cx);
        }
        if let Some((option, _)) = l.toggles.iter().find(|(_, b)| b.contains(&point)) {
            self.dispatch(Command::ToggleFindOption(*option), cx);
        } else if let Some(i) = l.fields.iter().position(|b| b.contains(&point))
            && let Some(Modal::Search { in_files, .. }) = &mut self.modal
        {
            *in_files = i == 1;
        }
        cx.notify();
    }
}

impl Layout {
    pub(super) fn paint(&self, window: &mut Window, cx: &mut App, m: Metrics, p: &lenz_content::theme::Palette, x0: f32, y0: f32) {
        let at = |b: &Bounds<Pixels>| (x0 + f32::from(b.origin.x), y0 + f32::from(b.origin.y), f32::from(b.size.width), f32::from(b.size.height));
        let (x, y, w, h) = at(&self.bounds);
        let (bg, border) = p.raised();
        lenz_gpui::fill(window, x - 1.0, y - 1.0, w + 2.0, h + 2.0, border);
        lenz_gpui::fill(window, x, y, w, h, bg);
        let clip = rect(x, y, w, h);
        window.with_content_mask(Some(gpui::ContentMask { bounds: clip }), |window| {
            lenz_gpui::text(window, cx, m, x + PAD, y + PAD, "Search in Project", p.fg);
            let fields = [("Find", &self.query, "Text to search for"), ("Files", &self.files, "All files — e.g. *.rs, src/, !*.test.ts")];
            for (i, (b, (label, value, placeholder))) in self.fields.iter().zip(fields).enumerate() {
                let (fx, fy, fw, fh) = at(b);
                let active = self.in_files == (i == 1);
                lenz_gpui::text(window, cx, m, x + PAD, fy + 4.0, label, if active { p.fg } else { p.dim });
                lenz_gpui::fill(window, fx, fy, fw, fh, if active { p.accent } else { border });
                lenz_gpui::fill(window, fx + 1.0, fy + 1.0, fw - 2.0, fh - 2.0, p.bg);
                let shown = navigation::tail(value, ((fw - 20.0) / self.char_w).max(0.0) as usize);
                let field = rect(fx + 1.0, fy + 1.0, fw - 2.0, fh - 2.0);
                window.with_content_mask(Some(gpui::ContentMask { bounds: field }), |window| {
                    if value.is_empty() {
                        lenz_gpui::text(window, cx, m, fx + 8.0, fy + 4.0, placeholder, p.dim);
                    } else {
                        lenz_gpui::text(window, cx, m, fx + 8.0, fy + 4.0, &shown, p.fg);
                    }
                    if active {
                        let caret_x = fx + 8.0 + lenz_content::wrap::width(&shown) as f32 * self.char_w;
                        lenz_gpui::fill(window, caret_x, fy + 4.0, 2.0, fh - 8.0, p.caret);
                    }
                });
            }
            for ((option, b), on) in self.toggles.iter().zip(self.on) {
                let (tx, ty, tw, th) = at(b);
                lenz_gpui::fill(window, tx, ty, tw, th, if on { p.accent } else { border });
                lenz_gpui::fill(window, tx + 1.0, ty + 1.0, tw - 2.0, th - 2.0, if on { p.selected_bg } else { bg });
                let toggle = rect(tx, ty, tw, th);
                window.with_content_mask(Some(gpui::ContentMask { bounds: toggle }), |window| {
                    lenz_gpui::text(window, cx, m, tx + self.char_w, ty + 3.0, option.symbol(), if on { p.accent } else { p.dim });
                    let lx = tx + (option.symbol().chars().count() + 2) as f32 * self.char_w;
                    lenz_gpui::text(window, cx, m, lx, ty + 3.0, option.label(), p.fg);
                });
            }
            for (i, line) in self.hint.iter().enumerate() {
                lenz_gpui::text(window, cx, m, x + PAD, y0 + self.hint_y + i as f32 * line_h(), line, if self.refused { p.error } else { p.dim });
            }
        });
    }
}
