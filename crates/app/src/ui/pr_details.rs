//! Pixel layout and input for the shared PR details presentation.
use super::*;
use crate::pr_conversation::Page;
use crate::pr_session::{DetailsTab, PrCommand, PrState};
use crate::rich::{Fill, Target, Tone};
use unicode_width::UnicodeWidthChar;

/// A laid-out PR tab and its scroll; the page is rebuilt when the PR's pages or the width change.
#[derive(Default)]
pub(super) struct Dialog {
    /// PR number, page generation, columns and tab the page was laid out for.
    pub(super) key: Option<(u64, u64, usize, Option<DetailsTab>)>,
    pub(super) page: Arc<Page>,
    pub scroll: usize,
    /// Each tab's scroll, so coming back to a tab shows it as it was.
    pub(super) scrolls: std::collections::HashMap<DetailsTab, usize>,
    /// Selected text, between two (row, column) cells; the press a drag extends.
    pub(super) sel: Option<Selection>,
    pub(super) press: Option<(usize, usize)>,
}
/// The diff area the details fill: x, y, width, height.
pub(super) type Rect = (f32, f32, f32, f32);
/// Selected text: from and to (row, column) cells.
pub(super) type Selection = ((usize, usize), (usize, usize));
/// What a tab paints in a frame: the page, scroll, selection, pane and character width.
pub(super) type Frame = (Arc<Page>, usize, Option<Selection>, Rect, f32);
const PAD_X: f32 = 12.0;
const PAD_Y: f32 = 8.0;

/// The colour a tone shows in.
fn tone(p: &theme::Palette, t: Tone) -> u32 {
    match t {
        Tone::Normal => p.fg, Tone::Dim => p.dim, Tone::Accent => p.accent, Tone::Added | Tone::Good => p.status[0],
        Tone::Removed => p.status[1], Tone::Bad => p.error, Tone::Warn => p.status[2],
    }
}

impl Dialog {
    fn prepare(&mut self, pr: &mut PrState, rect: Rect, char_w: f32) {
        let cols = ((rect.2 - 2.0 * PAD_X).max(0.0) / char_w.max(1.0)).floor().max(1.0) as usize;
        let tab = pr.details().map(|d| d.tab);
        let key = (pr.pr.number, pr.generation(), cols, tab);
        if self.key != Some(key) {
            if self.key.is_some_and(|k| k.0 != key.0) { self.scrolls.clear(); }
            if let Some(old) = self.key.filter(|k| k.0 != key.0 || k.3 != tab) {
                if let Some(t) = old.3 { self.scrolls.insert(t, self.scroll); }
                self.scroll = tab.and_then(|t| self.scrolls.get(&t).copied()).unwrap_or(0);
            }
            // a rebuilt page wraps differently, so a selection's (row, column) cells would name other text
            self.sel = None;
            self.press = None;
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
            self.page = Arc::new(pr.page(cols, now).unwrap_or_default());
            self.key = Some(key);
        }
        if let Some(row) = pr.take_reveal().and_then(|t| self.page.row_of(&t)) { self.scroll = row.saturating_sub(2); }
        self.scroll = self.scroll.min(self.max_scroll(rect));
    }
    fn max_scroll(&self, rect: Rect) -> usize {
        self.page.height().saturating_sub(visible(rect))
    }
    fn key(&mut self, key: &str, rect: Rect) {
        let max = self.max_scroll(rect);
        let page = visible(rect).saturating_sub(1).max(1);
        self.scroll = match key {
            "up" => self.scroll.saturating_sub(1),
            "down" => (self.scroll + 1).min(max),
            "pageup" => self.scroll.saturating_sub(page),
            "pagedown" | "space" => (self.scroll + page).min(max),
            "home" => 0,
            "end" => max,
            _ => self.scroll.min(max),
        };
    }
    /// The selection's text, or `None` when nothing is selected.
    fn selected_text(&self) -> Option<String> {
        let (a, b) = self.sel.filter(|(a, b)| a != b)?;
        Some(self.page.text(a, b))
    }
    /// The page row and column under a point, clamped into the pane (for a drag past its edge).
    fn cell_clamped(&self, rect: Rect, char_w: f32, x: f32, y: f32) -> (usize, usize) {
        let (rx, ry) = ((x - rect.0 - PAD_X).max(0.0), (y - rect.1 - PAD_Y).clamp(0.0, (rect.3 - PAD_Y - 1.0).max(0.0)));
        ((ry / line_h()) as usize + self.scroll, (rx / char_w.max(1.0)).round() as usize)
    }
    /// The page row and column under a point.
    fn cell(&self, rect: Rect, char_w: f32, x: f32, y: f32) -> Option<(usize, usize)> {
        let (rx, ry) = (x - rect.0 - PAD_X, y - rect.1 - PAD_Y);
        (rx >= 0.0 && ry >= 0.0).then(|| ((ry / line_h()) as usize + self.scroll, (rx / char_w.max(1.0)) as usize))
    }
    pub(super) fn paint(&self, p: &theme::Palette, rect: Rect, font: &Font, char_w: f32, window: &mut Window, cx: &mut App) {
        let (x, y, w, h) = rect;
        fill(window, x, y, w, h, p.bg);
        let clip = Bounds { origin: point(px(x), px(y)), size: size(px(w.max(0.0)), px(h.max(0.0))) };
        let rows = visible(rect) + 1;
        let (top, bottom) = (self.scroll, self.scroll + rows);
        let col = |c: usize| x + PAD_X + c as f32 * char_w;
        let row_y = |r: usize| y + PAD_Y + (r as f32 - self.scroll as f32) * line_h();
        let sel = self.sel.filter(|(a, b)| a != b).map(|(a, b)| if a <= b { (a, b) } else { (b, a) });
        window.with_content_mask(Some(ContentMask { bounds: clip }), |window| {
            // cards: a header band and an outline, the innermost drawn last
            // outer cards first, so a card's background never covers the cards inside it
            let mut boxes: Vec<_> = self.page.boxes.iter().filter(|b| b.bottom > top && b.top < bottom).collect();
            boxes.sort_by_key(|b| std::cmp::Reverse(b.right - b.left));
            for b in boxes {
                let (bx, bw) = (col(b.left) + char_w * 0.5, (b.right - b.left) as f32 * char_w - char_w);
                let (by, bh) = (row_y(b.top) + line_h() * 0.5, (b.bottom - b.top - 1) as f32 * line_h());
                fill(window, bx, by, bw, bh, p.bg);
                // a folded card is its header alone
                let body = b.bottom - b.top > 3;
                fill(window, bx, by, bw, if body { line_h() * 1.5 } else { bh }, p.header_bg);
                let edge = match b.tone { Tone::Good | Tone::Bad => tone(p, b.tone), _ => p.border };
                for (ex, ey, ew, eh) in [(bx, by, bw, 1.0), (bx, by + bh, bw, 1.0), (bx, by, 1.0, bh), (bx + bw, by, 1.0, bh + 1.0)] { fill(window, ex, ey, ew, eh, edge); }
                if body { fill(window, bx, by + line_h() * 1.5, bw, 1.0, p.border); }
            }
            let paint_row = |window: &mut Window, cx: &mut App, row: &crate::rich::Row, at: f32, right: f32| {
                let start = col(row.indent);
                match row.fill {
                    Fill::Code => fill(window, start - 4.0, at, (right - start + 4.0).max(0.0), line_h(), p.fold_bg),
                    Fill::Quote => fill(window, start - char_w * 1.5, at, 2.0, line_h(), p.border),
                    Fill::Rule => fill(window, start, at + line_h() * 0.5, (right - start).max(0.0), 1.0, p.border),
                    Fill::None => {}
                }
                rich_text(p, window, cx, font, start, at, &row.spans);
            };
            let right_of = |side: bool| if side || self.page.side_at == 0 { x + w - PAD_X } else { col(self.page.side_at.saturating_sub(2)) };
            // the selection, under the text
            if let Some(((r0, c0), (r1, c1))) = sel {
                for r in r0.max(top)..=r1.min(bottom) {
                    // within the row's text: from its indent to its last character
                    let (indent, end) = self.page.rows.get(r).map_or((0, 0), |row| (row.indent, row.indent + crate::rich::width(&row.text())));
                    let (a, b) = ((if r == r0 { c0 } else { 0 }).max(indent), (if r == r1 { c1 } else { end }).min(end));
                    let (a, b) = (col(a), col(b).min(x + w));
                    if b > a { fill(window, a, row_y(r), b - a, line_h(), p.sel_bg); }
                }
            }
            for (i, row) in self.page.rows.iter().enumerate().skip(top).take(rows) {
                // a row inside a card ends at the card's edge
                let edge = self.page.boxes.iter().filter(|b| (b.top..b.bottom).contains(&i) && row.indent >= b.left).map(|b| col(b.right) - char_w).fold(right_of(false), f32::min);
                paint_row(window, cx, row, row_y(i), edge);
            }
            for (i, row) in self.page.side.iter().enumerate().skip(top).take(rows) { paint_row(window, cx, row, row_y(i), right_of(true)); }
        });
    }
}

/// Spans in their tones and styles, side by side from `x`.
fn rich_text(p: &theme::Palette, window: &mut Window, cx: &mut App, font: &Font, x: f32, y: f32, spans: &[crate::rich::Span]) {
    let text: String = spans.iter().map(|s| crate::document::cap_marks(&s.text).into_owned()).collect();
    if text.is_empty() { return; }
    let runs: Vec<TextRun> = spans.iter().filter(|s| !s.text.is_empty()).map(|s| {
        let mut f = font.clone();
        if s.style.bold { f.weight = gpui::FontWeight::BOLD; }
        if s.style.italic { f.style = gpui::FontStyle::Italic; }
        let color = Hsla::from(rgb(tone(p, s.style.tone)));
        TextRun {
            len: crate::document::cap_marks(&s.text).len(), font: f, color,
            background_color: s.style.code.then(|| Hsla::from(rgb(p.fold_bg))),
            underline: matches!(s.target, Some(crate::rich::Target::Url(_))).then(|| gpui::UnderlineStyle { thickness: px(1.0), color: Some(color), wavy: false }),
            strikethrough: s.style.strike.then(|| gpui::StrikethroughStyle { thickness: px(1.0), color: Some(color) }),
        }
    }).collect();
    let line = window.text_system().shape_line(SharedString::from(text), px(font_size()), &runs, None);
    let o = point(px(x), px(y));
    let _ = line.paint_background(o, px(line_h()), TextAlign::Left, None, window, cx);
    let _ = line.paint(o, px(line_h()), TextAlign::Left, None, window, cx);
}
fn visible(rect: Rect) -> usize {
    ((rect.3 - 8.0).max(0.0) / line_h()) as usize
}
pub(super) fn wrap(text: &str, cols: usize) -> Vec<String> {
    let mut rows = Vec::new();
    for line in text.split('\n') {
        let mut row = String::new();
        let mut width = 0;
        for ch in line.chars() {
            let n = ch.width().unwrap_or(0);
            if width + n > cols && !row.is_empty() {
                rows.push(std::mem::take(&mut row));
                width = 0;
            }
            row.push(ch);
            width += n;
        }
        rows.push(row);
    }
    rows
}

impl Viewer {
    /// A pull request details tab is on screen (in Focus, in place of the diff).
    pub(super) fn pr_details_shown(&self) -> bool {
        self.repo.as_ref().and_then(|r| r.pr.as_ref()).is_some_and(|p| p.details().is_some())
    }
    /// Where a details tab paints: the diff area below its header.
    pub(super) fn pr_pane(&self) -> Rect {
        let [vw, vh] = self.panes.viewport;
        // over the diff header too: the tabs are in the toolbar, so that row has nothing to say
        let (x, y) = (self.diff_x(), self.top() - hdr_h());
        (x, y, (vw - x).max(0.0), (vh - y).max(0.0))
    }
    fn in_pr_pane(&self, at: gpui::Point<Pixels>) -> bool {
        let (x, y, w, h) = self.pr_pane();
        let (px_, py_) = (f32::from(at.x), f32::from(at.y));
        px_ >= x && px_ < x + w && py_ >= y && py_ < y + h
    }
    pub(super) fn pr_command(&mut self, command: PrCommand, cx: &mut Context<Self>) {
        // details are a Focus tab: showing one from the Overview enters Focus first
        if matches!(command, PrCommand::Show(_) | PrCommand::ShowThreads(_)) && self.repo.is_some() && !self.layout.focused(self.repo.as_ref()) {
            self.layout_command(crate::layout::LayoutCommand::ToggleFocus, cx);
        }
        if matches!(command, PrCommand::Show(_) | PrCommand::ShowThreads(_)) { self.panel_focused = false; }
        if let Some(pr) = self.repo.as_mut().and_then(|r| r.pr.as_mut()) {
            pr.execute(command);
        }
        self.dragging = false;
        self.menu = None;
        self.command_palette = None;
        self.diff_options = None;
        cx.notify();
    }
    fn prepare_pr_dialog(&mut self) {
        let rect = self.pr_pane();
        let char_w = self.char_w;
        if let Some(pr) = self.repo.as_mut().and_then(|r| r.pr.as_mut()).filter(|p| p.details().is_some()) {
            self.pr_dialog.get_or_insert_with(Dialog::default).prepare(pr, rect, char_w);
        }
    }
    /// The details tab's own keys: Esc or d back to Files changed, Tab / Shift-Tab / 1–3 between the tabs, scrolling.
    /// The palette, help, quitting, Save and the Files toggle go on as usual; every other key would act on the
    /// hidden diff, so the tab takes it.
    pub(super) fn pr_dialog_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        let m = &ev.keystroke.modifiers;
        let key = ev.keystroke.key.as_str();
        let cmd = m.control || m.platform;
        let passes = matches!(key, "?" | ":" | "q") && !cmd || cmd && matches!(key, "q" | "w" | "s") || m.alt && key == "1";
        if !self.pr_details_shown() || passes {
            return false;
        }
        self.prepare_pr_dialog();
        match key {
            "c" if cmd => {
                match self.pr_dialog.as_ref().and_then(Dialog::selected_text) {
                    Some(text) => { cx.write_to_clipboard(ClipboardItem::new_string(text)); self.toast("Copied".into(), cx); }
                    None => self.toast("Select text to copy (drag over it)".into(), cx),
                }
            }
            "escape" | "d" => self.pr_command(PrCommand::Close, cx),
            // Conversation, Commits, Files changed in turn
            "tab" => {
                let Some(tab) = self.repo.as_ref().and_then(|r| r.pr.as_ref()).and_then(|p| p.details()).map(|d| d.tab) else { return false };
                let i = DetailsTab::ALL.iter().position(|t| *t == tab).unwrap_or(0);
                let n = DetailsTab::ALL.len() + 1;
                match (i + if m.shift { n - 1 } else { 1 }) % n { i if i < DetailsTab::ALL.len() => self.pr_command(PrCommand::Show(DetailsTab::ALL[i]), cx), _ => self.pr_command(PrCommand::Close, cx) }
            }
            "1" | "2" => {
                let i = (key.as_bytes()[0] - b'1') as usize;
                self.pr_command(PrCommand::Show(DetailsTab::ALL[i]), cx);
            }
            "3" => self.pr_command(PrCommand::Close, cx),
            "up" | "down" | "pageup" | "pagedown" | "home" | "end" | "space" if !cmd && !m.alt => {
                let rect = self.pr_pane();
                if let Some(d) = self.pr_dialog.as_mut() { d.key(key, rect); }
            }
            _ => {}
        }
        cx.notify();
        true
    }
    fn pr_target_at(&mut self, at: gpui::Point<Pixels>) -> Option<(Target, Option<crate::pr_conversation::CardBox>)> {
        self.prepare_pr_dialog();
        let d = self.pr_dialog.as_ref()?;
        let (row, column) = d.cell(self.pr_pane(), self.char_w, f32::from(at.x), f32::from(at.y))?;
        Some((d.page.target_at(row, column)?.clone(), None))
    }
    /// A click on a details tab follows what is under it (a link, a thread's line, a fold, a commit); it never reaches
    /// the hidden diff.
    pub(super) fn pr_dialog_click(&mut self, ev: &MouseDownEvent, cx: &mut Context<Self>) -> bool {
        if !self.pr_details_shown() || !self.in_pr_pane(ev.position) { return false; }
        if ev.button != MouseButton::Left { return true; }
        let target = self.pr_target_at(ev.position).map(|(t, _)| t);
        if target.is_none() {
            // anywhere else a press starts a selection
            let (rect, char_w) = (self.pr_pane(), self.char_w);
            if let Some(d) = self.pr_dialog.as_mut() { let at = d.cell_clamped(rect, char_w, f32::from(ev.position.x), f32::from(ev.position.y)); d.press = Some(at); d.sel = Some((at, at)); }
            cx.notify();
            return true;
        }
        if let Some(d) = self.pr_dialog.as_mut() { d.sel = None; }
        match target {
            Some(Target::Url(url)) => self.browse(&url, cx),
            Some(Target::Toggle(key)) => self.pr_command(PrCommand::Toggle(key), cx),
            Some(Target::Commit(sha)) => self.repository_command(crate::repository_session::RepositoryCommand::PrCommit(sha), cx),
            Some(Target::Line { path, line, left }) => {
                if let Some(index) = self.set.comparisons.iter().position(|c| c.path == path) {
                    self.pr_command(PrCommand::Close, cx);
                    self.open(index, Place::Line(if left { Side::Left } else { Side::Right }, line), cx);
                } else {
                    self.toast(format!("{path} is not in this comparison"), cx);
                }
            }
            None => {}
        }
        cx.notify();
        true
    }
    /// A right click on a card: copy its comment or its link.
    pub(super) fn pr_dialog_menu(&mut self, ev: &MouseDownEvent, cx: &mut Context<Self>) -> bool {
        if !self.pr_details_shown() || !self.in_pr_pane(ev.position) { return false; }
        self.prepare_pr_dialog();
        let Some(d) = self.pr_dialog.as_ref() else { return true };
        let card = d.cell(self.pr_pane(), self.char_w, f32::from(ev.position.x), f32::from(ev.position.y)).and_then(|(r, c)| d.page.card_at(r, c)).cloned();
        if let Some(card) = card {
            let mut items = vec![("Copy comment".to_string(), MenuAction::Clipboard(card.body.clone()))];
            if !card.url.is_empty() { items.push(("Copy link to comment".to_string(), MenuAction::Clipboard(card.url.clone()))); }
            self.show_menu(Menu { x: f32::from(ev.position.x), y: f32::from(ev.position.y), items, hover: None });
            cx.notify();
        }
        true
    }
    pub(super) fn pr_dialog_wheel(&mut self, ev: &ScrollWheelEvent, cx: &mut Context<Self>) -> bool {
        if !self.pr_details_shown() || !self.in_pr_pane(ev.position) {
            return false;
        }
        self.prepare_pr_dialog();
        let key = if f32::from(ev.delta.pixel_delta(px(line_h())).y) > 0.0 { "up" } else { "down" };
        let rect = self.pr_pane();
        if let Some(d) = self.pr_dialog.as_mut() {
            for _ in 0..3 { d.key(key, rect); }
        }
        cx.notify();
        true
    }
    /// What a details tab paints this frame: the page, scroll and pane, or `None` on Files changed.
    pub(super) fn pr_details_frame(&mut self) -> Option<Frame> {
        // the dialog stays while the PR is open, with each tab's scroll, for when a tab comes back
        if self.repo.as_ref().is_none_or(|r| r.pr.is_none()) { self.pr_dialog = None; }
        if !self.pr_details_shown() { return None; }
        self.prepare_pr_dialog();
        let dialog = self.pr_dialog.as_ref()?;
        Some((dialog.page.clone(), dialog.scroll, dialog.sel, self.pr_pane(), self.char_w))
    }
    /// Dragging from a press on a tab selects its text.
    pub(super) fn pr_dialog_drag(&mut self, x: f32, y: f32, cx: &mut Context<Self>) -> bool {
        let (rect, char_w, shown) = (self.pr_pane(), self.char_w, self.pr_details_shown());
        let Some(d) = self.pr_dialog.as_mut().filter(|d| d.press.is_some() && shown) else { return false };
        let head = d.cell_clamped(rect, char_w, x, y);
        d.sel = d.press.map(|a| (a, head));
        cx.notify();
        true
    }
    pub(super) fn pr_dialog_release(&mut self) { if let Some(d) = self.pr_dialog.as_mut() { d.press = None; } }

}
