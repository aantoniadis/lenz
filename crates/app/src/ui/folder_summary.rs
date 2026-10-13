//! Pixel layout and input for the Folder Summary, shown in place of the diff; its state is the shared
//! `crate::folders::Folders`.
use super::*;
use crate::folders::{FolderAction, Summary, Tone, stat_parts};

/// Rows above the files: the heading and a gap.
const TOP_ROWS: f32 = 2.0;

/// Badge colour index, the file's text, its count and whether the keyboard is on it.
type FrameRow = (usize, String, Vec<(String, Tone)>, bool);

/// One summary laid out for painting.
pub(super) struct SummaryFrame {
    rect: Rect,
    heading: Vec<(String, Tone)>,
    rows: Vec<FrameRow>,
    scroll: f32,
}

impl Viewer {
    pub(super) fn summary_shown(&self) -> bool {
        self.comparison.folders.summary().is_some()
    }

    /// The File Navigator rows, rebuilt when a folder collapsed or expanded or the Change Set changed.
    pub(super) fn sync_nav(&mut self) {
        if self.comparison.folders.sync(&self.set) || self.nav_revision != self.comparison.folders.revision() {
            self.nav = self.comparison.folders.rows(&self.set);
            self.nav_revision = self.comparison.folders.revision();
            self.nav_scroll = self.nav_scroll.min(self.nav.len().saturating_sub(1) as f32);
        }
    }

    fn summary_rows(&self) -> f32 {
        let (_, _, _, h) = self.pr_pane();
        (h / line_h() - TOP_ROWS - 1.0).floor().max(1.0)
    }

    pub(super) fn summary_frame(&mut self) -> Option<SummaryFrame> {
        let summary = self.comparison.folders.summary()?.clone();
        let rows = self.summary_rows();
        // the keyboard row stays in view unless the wheel moved away from it
        if !self.summary_free {
            let selected = summary.selected as f32;
            if selected < self.summary_scroll { self.summary_scroll = selected; } else if selected >= self.summary_scroll + rows { self.summary_scroll = selected + 1.0 - rows; }
        }
        self.summary_scroll = self.summary_scroll.clamp(0.0, (summary.files.len() as f32 - rows).max(0.0));
        let folders = &self.comparison.folders;
        let first = self.summary_scroll as usize;
        let lines = summary.files.iter().enumerate().skip(first).take(rows as usize + 1).map(|(row, &index)| {
            let c = &self.set.comparisons[index];
            let kind = match navigator::badge(&c.status) { "A" => 0, "D" => 1, "M" => 2, "R" => 3, _ => 4 };
            let name = match c.status {
                crate::changeset::Status::Renamed { .. } => format!("{} ← {}", summary.relative(&c.path), summary.relative(&c.old_path)),
                _ => summary.relative(&c.path).to_string(),
            };
            (kind, format!("{} {name}", navigator::badge(&c.status)), stat_parts(folders.stat(index)), row == summary.selected)
        }).collect();
        Some(SummaryFrame { rect: self.pr_pane(), heading: summary.heading(folders), rows: lines, scroll: self.summary_scroll - first as f32 })
    }

    /// Keys while the summary shows (and Files hasn't the keyboard); the rest reach the window.
    pub(super) fn summary_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        let Some(summary) = self.comparison.folders.summary() else { return false };
        let m = &ev.keystroke.modifiers;
        if m.control || m.alt || m.platform { return false; }
        let page = self.summary_rows() as isize;
        let last = summary.files.len() as isize;
        match ev.keystroke.key.as_str() {
            "down" | "j" => self.comparison.folders.select(1),
            "up" | "k" => self.comparison.folders.select(-1),
            "pagedown" | "space" => self.comparison.folders.select(page),
            "pageup" => self.comparison.folders.select(-page),
            "home" => self.comparison.folders.select(-last),
            "end" => self.comparison.folders.select(last),
            "enter" => self.open_summary_file(cx),
            "escape" => self.close_summary(cx),
            "left" => self.folder_action(FolderAction::Collapse, cx),
            "right" => self.folder_action(FolderAction::Expand, cx),
            _ => return false,
        }
        self.summary_free = false;
        cx.notify();
        true
    }

    /// A click on a summary row opens its file; anywhere else over the summary it only takes the pointer.
    pub(super) fn summary_click(&mut self, x: f32, y: f32, cx: &mut Context<Self>) -> bool {
        let Some(summary) = self.comparison.folders.summary() else { return false };
        let (rx, ry, rw, rh) = self.pr_pane();
        if x < rx || x >= rx + rw || y < ry || y >= ry + rh { return false; }
        let row = ((y - ry) / line_h() - TOP_ROWS + self.summary_scroll).floor();
        if row >= 0.0 && (row as usize) < summary.files.len() && y < ry + rh - line_h() {
            let delta = row as isize - summary.selected as isize;
            self.comparison.folders.select(delta);
            self.open_summary_file(cx);
        }
        self.nav_focused = false;
        cx.notify();
        true
    }

    pub(super) fn summary_wheel(&mut self, x: f32, rows: f32, cx: &mut Context<Self>) -> bool {
        if !self.summary_shown() || x < self.pr_pane().0 { return false; }
        self.summary_scroll += rows;
        self.summary_free = true;
        cx.notify();
        true
    }

    /// A folder heading's name: its summary in place of the comparison.
    pub(super) fn show_summary(&mut self, folder: &str, group: Option<usize>, cx: &mut Context<Self>) {
        if self.pr_details_shown() { self.pr_command(crate::pr_session::PrCommand::Close, cx); }
        self.comparison.folders.show_summary(&self.set, folder, group, Some(self.comparison.current));
        self.summary_scroll = 0.0;
        self.summary_free = false;
        self.after_folders(cx);
        self.reveal_in_nav();
    }

    /// Back from the summary to the file behind it.
    pub(super) fn close_summary(&mut self, cx: &mut Context<Self>) {
        self.comparison.folders.close_summary();
        self.comparison.folders.reveal(&self.set.comparisons, self.comparison.current);
        self.after_folders(cx);
        self.reveal_in_nav();
    }

    pub(super) fn open_summary_file(&mut self, cx: &mut Context<Self>) {
        if let Some(index) = self.comparison.folders.selected_file() {
            if index == self.comparison.current { self.close_summary(cx); } else { self.open(index, Place::Top, cx); }
        }
    }

    pub(super) fn toggle_folder(&mut self, folder: &str, cx: &mut Context<Self>) {
        self.comparison.folders.toggle(folder);
        self.after_folders(cx);
    }

    pub(super) fn folder_action(&mut self, action: FolderAction, cx: &mut Context<Self>) {
        if self.pr_details_shown() { self.pr_command(crate::pr_session::PrCommand::Close, cx); }
        if let Err(why) = self.comparison.folders.execute(action, &self.set, self.comparison.current) {
            self.toast(why.into(), cx);
            return;
        }
        self.after_folders(cx);
    }

    /// Rows and counting follow a folder change.
    pub(super) fn after_folders(&mut self, cx: &mut Context<Self>) {
        self.sync_nav();
        self.poll_folders(cx);
        cx.notify();
    }

    /// Counts arrive in batches: a timer runs only while the summary has files left to count.
    pub(super) fn poll_folders(&mut self, cx: &mut Context<Self>) {
        let set = self.set.clone();
        self.comparison.folders.poll(&set);
        if !self.comparison.folders.busy() { self.folders_task = None; return; }
        if self.folders_task.is_some() { return; }
        self.folders_task = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(std::time::Duration::from_millis(20)).await;
                let pending = this.update(cx, |viewer: &mut Viewer, cx| {
                    let set = viewer.set.clone();
                    if viewer.comparison.folders.poll(&set) { cx.notify(); }
                    viewer.comparison.folders.busy()
                }).unwrap_or(false);
                if !pending { break; }
            }
            let _ = this.update(cx, |viewer: &mut Viewer, _| viewer.folders_task = None);
        }));
    }
}

impl SummaryFrame {
    pub(super) fn paint(&self, palette: &theme::Palette, font: &Font, char_w: f32, window: &mut Window, cx: &mut App) {
        let (x0, y0, w, h) = self.rect;
        fill(window, x0, y0, w, h, palette.bg);
        let tone = |t: &Tone| match t { Tone::Added => palette.status[0], Tone::Removed => palette.status[1], Tone::Dim => palette.dim };
        let bounds = Bounds { origin: point(px(x0), px(y0)), size: size(px(w.max(0.0)), px(h.max(0.0))) };
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            let mut x = x0 + 16.0;
            for (i, (part, t)) in self.heading.iter().enumerate() {
                text(window, cx, font, x, y0 + 6.0, part, if i == 0 { palette.accent } else { tone(t) });
                x += part.chars().count() as f32 * char_w;
            }
            let top = y0 + TOP_ROWS * line_h();
            let list = Bounds { origin: point(px(x0), px(top)), size: size(px(w.max(0.0)), px((h - TOP_ROWS * line_h() - line_h()).max(0.0))) };
            window.with_content_mask(Some(ContentMask { bounds: list }), |window| {
                for (i, (kind, name, stat, selected)) in self.rows.iter().enumerate() {
                    let y = top + (i as f32 - self.scroll) * line_h();
                    if *selected { fill(window, x0, y, w, line_h(), palette.selected_bg); }
                    let stat_w: f32 = stat.iter().map(|(s, _)| (s.chars().count() + 1) as f32 * char_w).sum();
                    let room = Bounds { origin: point(px(x0), px(y)), size: size(px((w - stat_w - 32.0).max(0.0)), px(line_h())) };
                    window.with_content_mask(Some(ContentMask { bounds: room }), |window| text(window, cx, font, x0 + 16.0, y, name, palette.status[*kind]));
                    let mut sx = x0 + w - stat_w - 16.0;
                    for (part, t) in stat {
                        text(window, cx, font, sx, y, part, tone(t));
                        sx += (part.chars().count() + 1) as f32 * char_w;
                    }
                }
            });
            text(window, cx, font, x0 + 16.0, y0 + h - line_h() - 4.0, Summary::HINT, palette.dim);
        });
    }
}

#[cfg(all(test, feature = "gui-test"))]
impl SummaryFrame {
    /// Each row's file and count text, as painted.
    pub(super) fn rows_text(&self) -> Vec<(String, String)> {
        self.rows.iter().map(|(_, name, stat, _)| (name.clone(), stat.iter().map(|(t, _)| t.as_str()).collect::<Vec<_>>().join(" "))).collect()
    }
    /// The painted row the keyboard is on, if it is painted.
    pub(super) fn selected_text(&self) -> Option<String> {
        self.rows.iter().find(|r| r.3).map(|r| r.1.clone())
    }
}
