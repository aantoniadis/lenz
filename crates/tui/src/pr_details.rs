//! The PR reading tabs in the terminal: the shared pages drawn in cells, scrolled, and clicked.
use crate::chrome::{color, style_with};
use crossterm::event::{Event, KeyCode, KeyModifiers, MouseButton, MouseEventKind};
use lenz::pr_conversation::{CardBox, Page};
use lenz::pr_session::{DetailsTab, PrCommand, PrState};
use lenz::rich::{Fill, Target, Tone};
use ratatui::{Frame, layout::Rect, style::Modifier, widgets::Clear};
use std::sync::Arc;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// What a key or click on a tab asks the app to do.
pub enum Outcome { Pr(PrCommand), Commit(Option<String>), Url(String), Line { path: String, line: u32, left: bool }, Copy(String) }

#[derive(Default)]
pub struct Dialog {
    /// PR number, page generation, width and tab the page was laid out for.
    key: Option<(u64, u64, u16, Option<DetailsTab>)>,
    page: Arc<Page>,
    scroll: usize,
    area: Rect,
    /// Each tab's scroll, so coming back to a tab shows it as it was.
    scrolls: std::collections::HashMap<DetailsTab, usize>,
    /// Selected text between two (row, column) cells, and the press a drag extends.
    sel: Option<((usize, usize), (usize, usize))>,
    press: Option<(usize, usize)>,
}

/// The colour a tone shows in.
fn tone(p: &lenz::theme::Palette, t: Tone) -> u32 {
    match t {
        Tone::Normal => p.fg, Tone::Dim => p.dim, Tone::Accent => p.accent, Tone::Added | Tone::Good => p.status[0],
        Tone::Removed => p.status[1], Tone::Bad => p.error, Tone::Warn => p.status[2],
    }
}

impl Dialog {
    fn prepare(&mut self, pr: &mut PrState, area: Rect) {
        self.area = area;
        // a column each side
        let width = area.width.saturating_sub(2).max(1);
        let tab = pr.details().map(|d| d.tab);
        let key = (pr.pr.number, pr.generation(), width, tab);
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
            self.page = Arc::new(pr.page(usize::from(width), now).unwrap_or_default());
            self.key = Some(key);
        }
        if let Some(row) = pr.take_reveal().and_then(|t| self.page.row_of(&t)) { self.scroll = row.saturating_sub(2); }
        self.scroll = self.scroll.min(self.max());
    }
    fn max(&self) -> usize {
        self.page.height().saturating_sub(self.area.height as usize)
    }
    /// The cell under a point, clamped into the area (for a drag past its edge).
    fn cell_clamped(&self, column: u16, row: u16) -> (usize, usize) {
        let row = row.clamp(self.area.y, self.area.bottom().saturating_sub(1));
        (usize::from(row - self.area.y) + self.scroll, usize::from(column.saturating_sub(self.area.x + 1)))
    }
    fn cell(&self, column: u16, row: u16) -> Option<(usize, usize)> {
        (self.area.contains((column, row).into()) && column > self.area.x).then(|| (usize::from(row - self.area.y) + self.scroll, usize::from(column - self.area.x - 1)))
    }
    /// The card under a cell, for the right-click menu.
    pub fn card_at(&self, column: u16, row: u16) -> Option<CardBox> {
        let (r, c) = self.cell(column, row)?;
        self.page.card_at(r, c).cloned()
    }
    /// A details tab in the diff body (`area`): its own keys, the wheel and clicks over it. Returns whether it took the
    /// event, and what to do. The palette, help, quitting, Save, Files (Alt+1) and Back/Forward go on as usual; any
    /// other key would act on the hidden diff, so the tab takes it.
    pub fn handle(&mut self, event: &Event, pr: &mut PrState, area: Rect) -> (bool, Option<Outcome>) {
        self.prepare(pr, area);
        let Some(tab) = pr.details().map(|d| d.tab) else { return (false, None) };
        let max = self.max();
        let page = area.height.saturating_sub(1).max(1) as usize;
        match event {
            Event::Key(k) => {
                let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
                let alt = k.modifiers.contains(KeyModifiers::ALT);
                let passes = matches!(k.code, KeyCode::Char(':' | '?' | 'q') | KeyCode::F(_)) && !ctrl && !alt
                    || ctrl && matches!(k.code, KeyCode::Char('q' | 'c' | 's'))
                    || alt && matches!(k.code, KeyCode::Char('1') | KeyCode::Left | KeyCode::Right);
                // Ctrl+C copies a selection; without one it goes on as usual
                if ctrl && k.code == KeyCode::Char('c') && let Some((a, b)) = self.sel.filter(|(a, b)| a != b) {
                    return (true, Some(Outcome::Copy(self.page.text(a, b))));
                }
                if passes {
                    return (false, None);
                }
                let show = |i: usize| Some(Outcome::Pr(DetailsTab::ALL.get(i).map_or(PrCommand::Close, |t| PrCommand::Show(*t))));
                let command = match k.code {
                    KeyCode::Esc | KeyCode::Char('d') => Some(Outcome::Pr(PrCommand::Close)),
                    // Conversation, Commits, Files changed in turn
                    KeyCode::Tab | KeyCode::BackTab => {
                        let i = DetailsTab::ALL.iter().position(|t| *t == tab).unwrap_or(0);
                        let back = k.code == KeyCode::BackTab || k.modifiers.contains(KeyModifiers::SHIFT);
                        let n = DetailsTab::ALL.len() + 1;
                        show((i + if back { n - 1 } else { 1 }) % n)
                    }
                    KeyCode::Char(c @ '1'..='3') => show((c as u8 - b'1') as usize),
                    KeyCode::Home => { self.scroll = 0; None }
                    KeyCode::End => { self.scroll = max; None }
                    KeyCode::Up => { self.scroll = self.scroll.saturating_sub(1); None }
                    KeyCode::Down => { self.scroll = (self.scroll + 1).min(max); None }
                    KeyCode::PageUp => { self.scroll = self.scroll.saturating_sub(page); None }
                    KeyCode::PageDown | KeyCode::Char(' ') => { self.scroll = (self.scroll + page).min(max); None }
                    _ => None,
                };
                (true, command)
            }
            // a drag keeps selecting past the tab's edge
            Event::Mouse(m) if self.press.is_some() && matches!(m.kind, MouseEventKind::Drag(MouseButton::Left) | MouseEventKind::Up(MouseButton::Left)) => {
                if m.kind == MouseEventKind::Up(MouseButton::Left) { self.press = None; } else { let head = self.cell_clamped(m.column, m.row); self.sel = self.press.map(|a| (a, head)); }
                (true, None)
            }
            Event::Mouse(m) if area.contains((m.column, m.row).into()) => {
                let outcome = match m.kind {
                    MouseEventKind::ScrollUp => { self.scroll = self.scroll.saturating_sub(1); None }
                    MouseEventKind::ScrollDown => { self.scroll = (self.scroll + 1).min(max); None }
                    MouseEventKind::Down(MouseButton::Left) => {
                        let target = self.cell(m.column, m.row).and_then(|(r, c)| self.page.target_at(r, c)).cloned();
                        // a press off a link, fold or hash starts a selection
                        let at = self.cell_clamped(m.column, m.row);
                        (self.press, self.sel) = if target.is_none() { (Some(at), Some((at, at))) } else { (None, None) };
                        target.map(|t| match t {
                            Target::Url(url) => Outcome::Url(url),
                            Target::Toggle(key) => Outcome::Pr(PrCommand::Toggle(key)),
                            Target::Commit(sha) => Outcome::Commit(sha),
                            Target::Line { path, line, left } => Outcome::Line { path, line, left },
                        })
                    }
                    MouseEventKind::Drag(MouseButton::Left) if self.press.is_some() => { let head = self.cell_clamped(m.column, m.row); self.sel = self.press.map(|a| (a, head)); None }
                    MouseEventKind::Up(MouseButton::Left) => { self.press = None; None }
                    // the right click is the app's copy menu
                    MouseEventKind::Down(MouseButton::Right) => return (false, None),
                    _ => return (false, None),
                };
                (true, outcome)
            }
            _ => (false, None),
        }
    }
    /// The tab in the diff body, under menus and the palette.
    pub fn draw(&mut self, frame: &mut Frame, pr: &mut PrState, area: Rect, p: &lenz::theme::Palette) {
        self.prepare(pr, area);
        frame.render_widget(Clear, area);
        let base = style_with(p);
        let buf = frame.buffer_mut();
        buf.set_style(area, base);
        let (top, rows) = (self.scroll, usize::from(area.height));
        let x = |c: usize| area.x.saturating_add(1).saturating_add(c as u16);
        let y = |r: usize| area.y + (r - top) as u16;
        let shown = |r: usize| r >= top && r < top + rows;
        let right = area.right();
        let put = |buf: &mut ratatui::buffer::Buffer, cx: u16, cy: u16, s: &str, style: ratatui::style::Style| {
            if cx < right && cy < area.bottom() { buf.set_stringn(cx, cy, s, usize::from(right - cx), style); }
        };
        for b in self.page.boxes.iter().filter(|b| b.bottom > top && b.top < top + rows) {
            let edge = base.fg(color(match b.tone { Tone::Good | Tone::Bad => tone(p, b.tone), _ => p.border }));
            let (l, r) = (x(b.left), x(b.right.saturating_sub(1)));
            for row in b.top..b.bottom {
                if !shown(row) { continue; }
                let cy = y(row);
                let (a, mid, z) = if row == b.top { ("╭", "─", "╮") } else if row + 1 == b.bottom { ("╰", "─", "╯") } else { ("│", " ", "│") };
                put(buf, l, cy, a, edge);
                if mid == "─" { put(buf, l + 1, cy, &mid.repeat(usize::from(r.saturating_sub(l + 1))), edge); }
                put(buf, r, cy, z, edge);
                if row == b.top + 1 { buf.set_style(Rect::new(l + 1, cy, r.saturating_sub(l + 1).min(right.saturating_sub(l + 1)), 1), base.bg(color(p.header_bg))); }
            }
        }
        // the selection, over the text's cells
        let sel = self.sel.filter(|(a, b)| a != b).map(|(a, b)| if a <= b { (a, b) } else { (b, a) });
        let selected = |r: usize, c: usize| sel.is_some_and(|((r0, c0), (r1, c1))| (r0..=r1).contains(&r) && (r > r0 || c >= c0) && (r < r1 || c < c1));
        let side = |i: usize| self.page.side.get(i);
        for i in top..top + rows {
            for (n, row) in [self.page.rows.get(i), side(i)].into_iter().enumerate().filter_map(|(n, r)| Some((n, r?))) {
                // the timeline's rows stop short of the side panel
                let right = if n == 0 && self.page.side_at > 0 { x(self.page.side_at.saturating_sub(2)).min(right) } else { right };
                // a card's header row sits on its band
                let header = n == 0 && self.page.boxes.iter().any(|b| b.top + 1 == i && row.indent == b.left + 2);
                let cy = y(i);
                let start = x(row.indent);
                match row.fill {
                    Fill::Code => { let w = self.page.boxes.iter().filter(|b| (b.top..b.bottom).contains(&i) && row.indent >= b.left).map(|b| x(b.right.saturating_sub(1))).min().unwrap_or(right).saturating_sub(start); buf.set_style(Rect::new(start, cy, w.min(right.saturating_sub(start)), 1), base.bg(color(p.fold_bg))); }
                    Fill::Quote => put(buf, start.saturating_sub(2), cy, "▏", base.fg(color(p.border))),
                    Fill::Rule => put(buf, start, cy, &"─".repeat(usize::from(right.saturating_sub(start))), base.fg(color(p.border))),
                    Fill::None => {}
                }
                let mut cx = start;
                for span in &row.spans {
                    let mut style = base.fg(color(tone(p, span.style.tone)));
                    if span.style.bold { style = style.add_modifier(Modifier::BOLD); }
                    if span.style.italic { style = style.add_modifier(Modifier::ITALIC); }
                    if span.style.strike { style = style.add_modifier(Modifier::CROSSED_OUT); }
                    if matches!(span.target, Some(Target::Url(_))) { style = style.add_modifier(Modifier::UNDERLINED); }
                    if span.style.code || row.fill == Fill::Code { style = style.bg(color(p.fold_bg)); }
                    if span.style.highlight { style = style.bg(color(lenz::theme::highlight_bg(p))); }
                    if header && !span.style.code { style = style.bg(color(p.header_bg)); }
                    let text = crate::repository::safe_label(&span.text);
                    if cx >= right { break; }
                    put(buf, cx, cy, &text, style);
                    cx = cx.saturating_add(text.width() as u16);
                }
            }
        }
        if sel.is_some() {
            for i in top..(top + rows).min(self.page.height()) {
                for cx in area.x.saturating_add(1)..right {
                    if selected(i, usize::from(cx - area.x - 1)) { buf[(cx, y(i))].set_bg(color(p.sel_bg)); }
                }
            }
        }
    }
}

/// Shared grapheme-safe wrapping for terminal text dialogs.
pub(crate) fn wrap_into(text: &str, width: u16, rows: &mut Vec<String>) {
    for line in text.split('\n') {
        if line.is_ascii() {
            if line.is_empty() {
                rows.push(String::new());
            } else {
                rows.extend(
                    line.as_bytes().chunks(usize::from(width)).map(|part| {
                        std::str::from_utf8(part).expect("ASCII row").to_owned()
                    }),
                );
            }
            continue;
        }
        if line.width() <= usize::from(width) {
            rows.push(line.to_owned());
            continue;
        }

        let mut row = String::new();
        let mut n = 0;
        for ch in line.graphemes(true) {
            let w = ch.width();
            if n + w > usize::from(width) && !row.is_empty() {
                rows.push(std::mem::take(&mut row));
                n = 0;
            }
            row.push_str(ch);
            n += w;
        }
        rows.push(row);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::MouseEvent;

    #[test]
    fn pr_reading_wheel_moves_one_row_and_clamps_after_resize() {
        let mut pr = lenz::pr::PullRequest { number: 1, title: "A change".into(), author: lenz::pr::Account { login: "alex".into() }, state: lenz::pr::State::Open, is_draft: false,
            body: String::new(), base_ref_name: "main".into(), head_ref_name: "feature".into(), head_ref_oid: "head".into(), url: "https://github.com/o/r/pull/1".into(),
            additions: 1, deletions: 0, changed_files: 1, is_cross_repository: false, head_repository: None, head_repository_owner: None };
        pr.body = "Paragraph.\n\n".repeat(80);
        let mut pr = PrState::new(pr, lenz::pr::Fetched { number: 14130, merge_base: "base".into(), head: "head".into() });
        pr.execute(PrCommand::Show(DetailsTab::Conversation));
        let mut dialog = Dialog::default();
        let area = Rect::new(0, 0, 80, 20);
        dialog.handle(&Event::Mouse(MouseEvent { kind: MouseEventKind::ScrollDown, column: 5, row: 5, modifiers: KeyModifiers::NONE }), &mut pr, area);
        assert_eq!(dialog.scroll, 1);
        let key = |code| Event::Key(crossterm::event::KeyEvent::new(code, KeyModifiers::NONE));
        dialog.handle(&key(KeyCode::End), &mut pr, area);
        let end = dialog.scroll;
        dialog.handle(&key(KeyCode::Up), &mut pr, area);
        assert_eq!(dialog.scroll, end - 1);
        for theme in lenz::theme::builtin_entries() {
            for (width, height) in [(120, 30), (28, 8), (8, 3), (1, 1)] {
                let area = Rect::new(0, 0, width, height);
                let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
                terminal.draw(|frame| dialog.draw(frame, &mut pr, area, &theme.palette)).unwrap();
                assert!(dialog.scroll <= dialog.max());
            }
        }
        assert!(matches!(dialog.handle(&key(KeyCode::Esc), &mut pr, area), (true, Some(Outcome::Pr(PrCommand::Close)))));
    }
}
