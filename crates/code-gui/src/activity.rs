use super::*;
use gpui::{point, size};

pub(super) struct ActivityLayout {
    pub bounds: Bounds<Pixels>,
    pub buttons: Vec<(Bounds<Pixels>, CommandId, &'static str)>,
    pub rows: std::sync::Arc<Vec<String>>,
    pub start: usize,
    pub visible: usize,
}
impl ReaderView {
    pub(super) fn activity_height(&self) -> f32 {
        if !self.session.state().activity_log_shown() { return 0.0; }
        let available = (self.height - self.status_h() - toolbar_h() - line_h()).max(0.0);
        ((available / 3.0 / line_h()).floor().min(9.0).max(if available >= 3.0 * line_h() { 2.0 } else { 0.0 })) * line_h()
    }
    pub(super) fn activity_button(&self) -> Option<(Bounds<Pixels>, String)> {
        if self.status_h() == 0.0 || self.width < 100.0 { return None; }
        let right = self.commands_button().map_or(self.width - STATUS_PAD, |(b, _)| f32::from(b.left()) - STATUS_PAD);
        let width = (if self.width < 640.0 { 6.0 } else { 10.0 } * self.char_w).min(right.max(0.0));
        Some((Bounds { origin: point(px(right - width), px(self.height - self.status_h())), size: size(px(width), px(self.status_h())) }, (if self.width < 640.0 { "Logs".into() } else { format!("Logs ({})", self.session.state().activity_log().len()) })))
    }
    /// "Results (4)" left of the log button while the results panel is hidden; a click shows it again.
    pub(super) fn results_button(&self) -> Option<(Bounds<Pixels>, String)> {
        let label = self.session.hidden_results_label()?;
        let label = if self.width < 640.0 { "Results".to_owned() } else { label };
        let right = self.activity_button().map(|(b, _)| f32::from(b.left()) - STATUS_PAD)?;
        let width = ((lenz_content::wrap::width(&label) + 1) as f32 * self.char_w).min(right.max(0.0));
        Some((Bounds { origin: point(px(right - width), px(self.height - self.status_h())), size: size(px(width), px(self.status_h())) }, label))
    }
    pub(super) fn activity_layout(&mut self) -> Option<ActivityLayout> {
        let height = self.activity_height();
        if height == 0.0 { return None; }
        let top = self.height - self.status_h() - self.activity_height();
        let bounds = Bounds { origin: point(px(0.0), px(top)), size: size(px(self.width), px(height)) };
        let mut right = self.width;
        let buttons = [(CommandId::ToggleActivityLog, "Hide"), (CommandId::ClearActivityLog, "Clear"), (CommandId::CopyActivityLog, "Copy")].into_iter().map(|(id, label)| {
            let width = ((label.len() + 2) as f32 * self.char_w).min(right.max(0.0));
            right -= width;
            (Bounds { origin: point(px(right), px(top)), size: size(px(width), px(line_h())) }, id, label)
        }).collect();
        let rows = self.session.state().activity_log_rows(self.columns(self.width - 12.0));
        let visible = ((height / line_h()) as usize).saturating_sub(1);
        let maximum = rows.len().saturating_sub(visible);
        self.log_scroll = self.log_scroll.min(maximum);
        Some(ActivityLayout { bounds, buttons, rows, start: maximum - self.log_scroll, visible })
    }
    pub(super) fn activity_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.log_focus || !self.session.state().activity_log_shown() { return false; }
        if self.key_command(ev) == Some(CommandId::FindAction) { self.log_focus = false; return false; }
        let l = self.activity_layout();
        let page = l.as_ref().map_or(1, |l| l.visible.max(1));
        let maximum = l.as_ref().map_or(0, |l| l.rows.len().saturating_sub(l.visible));
        match ev.keystroke.key.as_str() {
            "escape" => self.run(CommandId::ToggleActivityLog, window, cx),
            "c" if ev.keystroke.modifiers.control || ev.keystroke.modifiers.platform => self.run(CommandId::CopyActivityLog, window, cx),
            "up" => self.log_scroll = self.log_scroll.saturating_add(1).min(maximum),
            "pageup" => self.log_scroll = self.log_scroll.saturating_add(page).min(maximum),
            "down" => self.log_scroll = self.log_scroll.saturating_sub(1),
            "pagedown" => self.log_scroll = self.log_scroll.saturating_sub(page),
            "home" => self.log_scroll = maximum,
            "end" => self.log_scroll = 0,
            "tab" | "f10" => { self.log_focus = false; return false; }
            _ => {}
        }
        cx.notify();
        true
    }
}
