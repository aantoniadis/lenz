//! Pixel layout and input for the core's unified navigation search.
use super::*;
use reader_core::navigation::{Chosen, Kind, Scope};

const HEADER: f32 = 3.0;

pub(super) struct Frame {
    bounds: Bounds<Pixels>,
    pub(super) first: usize,
    at: usize,
    char_w: f32,
    scope: Scope,
    query: String,
    status: String,
    rows: Vec<reader_core::navigation::Row>,
}

impl ReaderView {
    pub(super) fn open_navigation(&mut self, scope: Scope, cx: &mut Context<Self>) {
        self.double_shift.reset();
        self.dragging = false;
        self.drag = None;
        self.find_focus = None;
        self.crumb_focus = None;
        self.navigation_drawn = None;
        self.modal_seen.set(None);
        self.modal = Some(Modal::Navigation { at: 0 });
        self.dispatch(Command::SearchEverywhere { scope, query: String::new() }, cx);
    }

    fn update_navigation(&mut self, scope: Scope, query: String, cx: &mut Context<Self>) {
        self.modal = Some(Modal::Navigation { at: 0 });
        self.modal_seen.set(None);
        self.navigation_drawn = None;
        self.dispatch(Command::SearchEverywhere { scope, query }, cx);
    }

    pub(super) fn accept_navigation(&mut self, revision: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Modal::Navigation { at }) = self.modal else { return };
        let Some(choice) = self.session.navigation_choice(revision, at) else { return };
        self.close_modal(cx);
        match choice {
            Chosen::Command(command) => {
                self.tree_focus = false;
                self.results_focus = false;
                self.dispatch(command, cx);
            },
            Chosen::Action(id) => self.run(id, window, cx),
        }
    }

    pub(super) fn navigation_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(search) = self.session.state().navigation() else { return self.close_modal(cx) };
        let (mut query, mut scope, len) = (search.query.clone(), search.scope, search.rows.len());
        let key = ev.keystroke.key.as_str();
        let ctrl = ev.keystroke.modifiers.control || ev.keystroke.modifiers.platform;
        let scope_shortcut = match (key, ctrl, ev.keystroke.modifiers.shift, ev.keystroke.modifiers.alt) {
            ("p", true, false, false) | ("n", true, true, false) => Some(Scope::Files),
            ("n", true, true, true) => Some(Scope::Symbols),
            ("a" | "p", true, true, false) => Some(Scope::Actions),
            ("k", true, false, false) => Some(Scope::All),
            ("e", true, false, false) => Some(Scope::Recent),
            _ => None,
        };
        if let Some(scope) = scope_shortcut { return self.update_navigation(scope, query, cx); }
        let Some(Modal::Navigation { at }) = &mut self.modal else { return };
        *at = (*at).min(len.saturating_sub(1));
        let page = self.navigation_drawn.map_or(5, |(_, _, n)| n.max(1));
        match key {
            "escape" => self.close_modal(cx),
            "enter" => self.modal_accept(window, cx),
            "up" => *at = at.saturating_sub(1),
            "down" => *at = (*at + 1).min(len.saturating_sub(1)),
            "pageup" => *at = at.saturating_sub(page),
            "pagedown" => *at = (*at + page).min(len.saturating_sub(1)),
            "home" => *at = 0,
            "end" => *at = len.saturating_sub(1),
            "delete" if scope == Scope::Recent => {
                if let Some((revision, ..)) = self.navigation_drawn {
                    let index = *at;
                    self.dispatch(Command::ForgetNavigation { revision, index }, cx);
                }
            }
            "tab" if scope != Scope::Recent => {
                let i = Scope::ALL.iter().position(|s| *s == scope).unwrap_or(0);
                scope = Scope::ALL[(i + if ev.keystroke.modifiers.shift { 3 } else { 1 }) % 4];
                self.update_navigation(scope, query, cx);
            }
            "a" if ctrl => { query.clear(); self.update_navigation(scope, query, cx); }
            "v" if ctrl => {
                if let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) {
                    query.extend(text.chars().filter(|c| !c.is_control()));
                    self.update_navigation(scope, query, cx);
                }
            }
            "backspace" => { query.pop(); self.update_navigation(scope, query, cx); }
            _ => {
                if !ctrl && !ev.keystroke.modifiers.alt && let Some(text) = ev.keystroke.key_char.as_ref().filter(|s| !s.chars().any(char::is_control)) {
                    query.push_str(text);
                    self.update_navigation(scope, query, cx);
                }
            }
        }
    }

    pub(super) fn navigation_bounds(&self) -> Bounds<Pixels> {
        let width = (self.width - 16.0).clamp(0.0, 720.0);
        let top = (self.height * 0.12).min(72.0);
        let height = (self.height - top - 8.0).max(0.0).min(HEADER * line_h() + 12.0 * line_h() * 2.0 + 8.0);
        Bounds { origin: gpui::point(px((self.width - width) / 2.0), px(top)), size: gpui::size(px(width), px(height)) }
    }

    pub(super) fn navigation_click(&mut self, point: gpui::Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let bounds = self.navigation_bounds();
        if !bounds.contains(&point) { return self.close_modal(cx); }
        let x = f32::from(point.x - bounds.origin.x);
        let y = f32::from(point.y - bounds.origin.y) - 4.0;
        if y >= 0.0 && y < line_h() && self.session.state().navigation().is_some_and(|n| n.scope != Scope::Recent) {
            let i = (x / (f32::from(bounds.size.width) / 4.0).max(1.0)).floor() as usize;
            if let Some(scope) = Scope::ALL.get(i) && let Some(search) = self.session.state().navigation() {
                self.update_navigation(*scope, search.query.clone(), cx);
            }
        } else if y >= HEADER * line_h() && let Some((revision, first, visible)) = self.navigation_drawn {
            let row = ((y - HEADER * line_h()) / (line_h() * 2.0)).floor() as usize;
            if row < visible {
                if let Some(Modal::Navigation { at }) = &mut self.modal { *at = first + row; }
                self.accept_navigation(revision, window, cx);
            }
        }
        cx.notify();
    }

    pub(super) fn navigation_visible(&self, len: usize) -> usize {
        let rows = (((f32::from(self.navigation_bounds().size.height) - 8.0) / line_h() - HEADER) / 2.0).floor().max(0.0) as usize;
        rows.min(len)
    }

    pub(super) fn navigation_frame(&mut self) -> Option<Frame> {
        let bounds = self.navigation_bounds();
        let visible = self.navigation_visible(self.session.state().navigation()?.rows.len());
        let search = self.session.state().navigation()?;
        let Some(Modal::Navigation { at }) = &mut self.modal else { return None };
        *at = (*at).min(search.rows.len().saturating_sub(1));
        let at = *at;
        let first = self.modal_first(at, at..at + 1, search.rows.len(), visible);
        self.navigation_drawn = Some((search.revision, first, visible));
        Some(Frame { bounds, first, at, char_w: self.char_w, scope: search.scope, query: search.query.clone(), status: self.session.navigation_status(), rows: search.rows.iter().skip(first).take(visible).cloned().collect() })
    }

    pub(super) fn navigation_rows(&self) -> Vec<(String, Option<&'static str>)> {
        let Some(search) = self.session.state().navigation() else { return Vec::new() };
        let heading = if search.scope == Scope::Recent { search.scope.label().to_string() } else { Scope::ALL.iter().map(|s| if *s == search.scope { format!("[{}]", s.label()) } else { s.label().to_string() }).collect::<Vec<_>>().join("  ") };
        let mut rows = vec![(heading, None), (format!("Search › {}", search.query), None), (self.session.navigation_status(), None)];
        rows.extend(search.rows.iter().map(|r| (format!("{}{} — {}{}", r.label, match r.on { Some(true) => " (on)", Some(false) => " (off)", None => "" }, r.detail, r.shortcut.map_or(String::new(), |k| format!(" · {k}"))), r.disabled)));
        rows
    }
}

impl Frame {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn paint(&self, window: &mut Window, cx: &mut App, m: Metrics, p: &lenz_content::theme::Palette, x0: f32, y0: f32, nerd: Option<&gpui::Font>, logos: &std::collections::HashMap<&'static str, Arc<gpui::RenderImage>>) {
        let (x, y, w, h) = (x0 + f32::from(self.bounds.origin.x), y0 + f32::from(self.bounds.origin.y), f32::from(self.bounds.size.width), f32::from(self.bounds.size.height));
        let bounds = Bounds { origin: gpui::point(px(x), px(y)), size: self.bounds.size };
        let (bg, border) = p.raised();
        lenz_gpui::fill(window, x - 1.0, y - 1.0, w + 2.0, h + 2.0, border);
        lenz_gpui::fill(window, x, y, w, h, bg);
        window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
            let scopes = if self.scope == Scope::Recent { &[Scope::Recent][..] } else { &Scope::ALL[..] };
            for (i, scope) in scopes.iter().enumerate() {
                let tx = x + i as f32 * w / scopes.len() as f32;
                if *scope == self.scope { lenz_gpui::fill(window, tx, y + line_h() + 2.0, w / scopes.len() as f32, 2.0, p.accent); }
                lenz_gpui::text(window, cx, m, tx + 12.0, y + 4.0, scope.label(), if *scope == self.scope { p.accent } else { p.dim });
            }
            let input_y = y + 4.0 + line_h();
            lenz_gpui::fill(window, x + 6.0, input_y, (w - 12.0).max(0.0), line_h(), p.bg);
            let query = tail(&self.query, ((w - 36.0) / self.char_w).max(1.0) as usize);
            lenz_gpui::text(window, cx, m, x + 12.0, input_y, &format!("› {query}│"), p.fg);
            let hint = if self.scope == Scope::Recent { format!("{} · Enter open · Esc close", self.status) } else if self.query.is_empty() { format!("{} · Tab category · Enter open · Esc close", self.status) } else { self.status.clone() };
            lenz_gpui::text(window, cx, m, x + 12.0, input_y + line_h(), &hint, p.dim);
            for (i, row) in self.rows.iter().enumerate() {
                let top = y + 4.0 + (HEADER + i as f32 * 2.0) * line_h();
                if self.first + i == self.at { lenz_gpui::fill(window, x, top, w, 2.0 * line_h(), p.selected_bg); }
                match row.kind {
                    Kind::File => paint_icon(window, cx, nerd, logos, bounds, p, icons::file(&row.label), x + 10.0, top),
                    Kind::Symbol => lenz_gpui::text(window, cx, m, x + 10.0, top, "◇", p.accent),
                    Kind::Action => lenz_gpui::text(window, cx, m, x + 10.0, top, "›", p.accent),
                }
                let state = match row.on { Some(true) => "  on", Some(false) => "  off", None => "" };
                let label = format!("{}{state}", row.label);
                let color = if row.disabled.is_some() { p.dim } else { p.fg };
                let matched = highlights(&self.query, &row.label, p.accent);
                lenz_gpui::colored_text(window, cx, m, color, x + 34.0, top, &label, &matched, &[]);
                let detail = match row.disabled { Some(why) => format!("{} · {why}", row.detail), None => row.detail.clone() };
                lenz_gpui::text(window, cx, m, x + 34.0, top + line_h(), &detail, p.dim);
                if let Some(shortcut) = row.shortcut {
                    let width = shortcut.chars().count() as f32 * self.char_w + 16.0;
                    if w > 320.0 {
                        lenz_gpui::fill(window, x + w - width - 8.0, top, width + 8.0, line_h(), if self.first + i == self.at { p.selected_bg } else { bg });
                        lenz_gpui::text(window, cx, m, x + w - width, top, shortcut, p.dim);
                    }
                }
            }
        });
    }
}

/// Byte ranges in the original label, retaining UTF-8 boundaries for the shared text painter.
fn highlights(query: &str, label: &str, color: u32) -> Vec<(std::ops::Range<usize>, u32)> {
    let mut chars = label.char_indices();
    let mut spans = Vec::new();
    for q in query.split(':').next().unwrap_or_default().chars().filter(|c| !c.is_whitespace()) {
        let Some((at, ch)) = chars.find(|(_, c)| c.to_lowercase().eq(q.to_lowercase())) else { return Vec::new() };
        spans.push((at..at + ch.len_utf8(), color));
    }
    spans
}

/// Keep the end of a long query and its caret visible in the fixed input row.
pub(super) fn tail(query: &str, columns: usize) -> String {
    use lenz_content::wrap::width;
    if columns == 0 { return String::new(); }
    if width(query) <= columns { return query.to_owned(); }
    let (mut from, mut used) = (query.len(), 1);
    for (at, ch) in query.char_indices().rev() {
        let cells = width(ch.encode_utf8(&mut [0; 4]));
        if used + cells > columns { break; }
        used += cells;
        from = at;
    }
    let last = query.get(from..).unwrap_or_default().trim_start_matches(|c: char| width(c.encode_utf8(&mut [0; 4])) == 0);
    format!("…{last}")
}

#[cfg(test)]
mod tests {
    #[test]
    fn highlights_subsequences_without_splitting_unicode_and_ignores_position_suffix() {
        assert_eq!(super::highlights("ÉT:3", "Été.rs", 7), vec![(0..2, 7), (2..3, 7)]);
        assert!(super::highlights("zz", "main.rs", 7).is_empty());
        assert_eq!(super::highlights("mr", "main.rs", 7), vec![(0..1, 7), (5..6, 7)]);
    }

    #[test]
    fn query_tail_keeps_unicode_cell_width_and_combining_boundaries() {
        assert_eq!(super::tail("src/Été.rs", 5), "…é.rs");
        assert_eq!(super::tail("file", 4), "file");
        let long = format!("{}終端", "Été/".repeat(40));
        let shown = super::tail(&long, 12);
        assert!(shown.starts_with('…') && shown.ends_with("終端") && lenz_content::wrap::width(&shown) <= 12);
        assert_eq!(super::tail(&long, 1), "…");
        assert_eq!(super::tail(&long, 0), "");
        assert_eq!(super::tail("終端", 3), "…端");
        assert_eq!(super::tail("prefixe\u{301}x", 2), "…x");
        assert_eq!(super::tail("prefixe\u{301}x", 3), "…e\u{301}x");
        for query in [&long, "prefixe\u{301}x"] {
            for columns in 0..15 { assert!(lenz_content::wrap::width(&super::tail(query, columns)) <= columns); }
        }
    }
}
