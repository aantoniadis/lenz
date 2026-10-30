//! Viewport-only diagnostic labels, painted separately from source text and its input geometry.
use super::*;
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone)]
pub(super) struct Annotation {
    #[cfg(feature = "test-support")]
    pub line: usize,
    pub row: f32,
    pub x: f32,
    pub width: f32,
    pub text: String,
    pub color: u32,
    pub sticky: bool,
}

impl ReaderView {
    pub(super) fn inline_annotations(&self, frame: &PaneFrame, window: &Window) -> Vec<Annotation> {
        if frame.csv.is_some() || frame.picture.is_some() || frame.markdown.is_some() || frame.pending { return Vec::new(); }
        let state = self.session.state();
        let Some(tab) = state.presentation_shown(frame.geom.pane) else { return Vec::new() };
        let Some(view) = state.presentation_view(tab.id()) else { return Vec::new() };
        if state.view(tab.id()).is_none_or(|base| base.snapshot().key() != view.snapshot().key()) { return Vec::new(); }
        if state.diagnostics(tab.id()).is_none_or(|found| found.items.is_empty()) { return Vec::new(); }
        let logical = view.layout_rows(frame.geom.rows, state.wrap_cols(tab)).rows;
        let band = frame.sticky.iter().map(|(row, _)| row.y + 1.0).reduce(f32::max).unwrap_or(0.0);
        let annotation = |row: &TextRow, logical: &code_core::layout::Row, parts: &[(std::ops::Range<usize>, f32)], sticky| {
            if parts.is_empty() || !logical.last { return None; }
            let problem = state.inline_problem(tab.id(), logical.line)?;
            if !self.source_row(tab, logical)?.complete() { return None; }
            let count = if problem.other_count == 0 { String::new() } else { format!(" (+{})", problem.other_count) };
            let label = format!("{}: {}{count}", problem.severity.label(), problem.message);
            let x = (lenz_gpui::text_x(self.metrics(), 0.0, frame.hscroll)
                + native_x(window, self.metrics(), row, parts, row.text.len()) + 2.0 * self.char_w)
                .max(num_w() + 8.0);
            let width = (f32::from(frame.geom.text.size.width) - x - 4.0).max(0.0);
            let label = display::cap_marks(&label);
            let label = display::show_bidi(&label);
            let text = fit(window, self.metrics(), &label, width);
            if text.is_empty() { return None; }
            use code_core::semantics::diagnostics::Severity;
            let palette = state.palette();
            let color = match problem.severity { Severity::Error => palette.error, Severity::Warning => palette.accent, Severity::Information | Severity::Hint => palette.dim };
            Some(Annotation { #[cfg(feature = "test-support")] line: logical.line, row: row.y, x, width, text, color, sticky })
        };
        let mut annotations = Vec::new();
        for (i, row) in frame.rows.iter().enumerate().filter(|(_, row)| row.y >= band) {
            if let Some(logical) = logical.get(row.y as usize)
                && let Some(annotation) = annotation(row, logical, frame.row_parts.get(i).map_or(&[], Vec::as_slice), false) {
                annotations.push(annotation);
            }
        }
        for (i, (row, _)) in frame.sticky.iter().enumerate() {
            if row.number == 0 { continue; }
            if let code_core::layout::Query::Ready(Some(logical)) = view.layout_row(row.number as usize - 1, 0, state.wrap_cols(tab))
                && let Some(annotation) = annotation(row, &logical, frame.sticky_parts.get(i).map_or(&[], Vec::as_slice), true) {
                annotations.push(annotation);
            }
        }
        annotations
    }
}

fn fit(window: &Window, metrics: Metrics<'_>, text: &str, width: f32) -> String {
    if width <= 0.0 { return String::new(); }
    let shape = |text: &str| {
        let run = gpui::TextRun { len: text.len(), font: metrics.font.clone(), color: gpui::Hsla::default(), background_color: None, underline: None, strikethrough: None };
        window.text_system().shape_line(text.to_owned().into(), px(metrics.font_size), &[run], None)
    };
    let shaped = shape(text);
    if f32::from(shaped.width) <= width { return text.to_owned(); }
    let room = width - f32::from(shape("…").width);
    if room < 0.0 { return String::new(); }
    let end = text.grapheme_indices(true).rev().map(|(at, _)| at)
        .find(|&at| f32::from(shaped.x_for_index(at)) <= room).unwrap_or(0);
    format!("{}…", text.get(..end).unwrap_or_default())
}

pub(super) fn paint(window: &mut Window, cx: &mut App, metrics: Metrics<'_>, area: Bounds<Pixels>, annotations: &[Annotation], sticky: bool) {
    for annotation in annotations.iter().filter(|annotation| annotation.sticky == sticky) {
        let x = f32::from(area.origin.x) + annotation.x;
        let y = f32::from(area.origin.y) + annotation.row * metrics.line_h;
        let mask = Bounds { origin: gpui::point(px(x), px(y)), size: gpui::size(px(annotation.width), px(metrics.line_h)) };
        window.with_content_mask(Some(gpui::ContentMask { bounds: mask }), |window| {
            lenz_gpui::text(window, cx, metrics, x, y, &annotation.text, annotation.color);
        });
    }
}

#[cfg(feature = "test-support")]
pub(super) type PaintedUnderlines = std::rc::Rc<std::cell::RefCell<Vec<(u8, Bounds<Pixels>, u32)>>>;
