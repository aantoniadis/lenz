use super::*;

impl App {
    pub(super) fn draw_activity(&mut self, frame: &mut ratatui::Frame, geometry: &mut Geometry, body: &mut Rect, p: &Palette) {
        if !self.session.state().activity_log_shown() { self.log_focus = false; return; }
        let height = (body.height / 3).min(10);
        let area = Rect::new(body.x, body.bottom().saturating_sub(height), body.width, height);
        body.height = body.height.saturating_sub(height);
        geometry.activity = area;
        if area.is_empty() { return; }
        fill(frame.buffer_mut(), area, style(p.fg, p.sidebar_bg));
        let mut x = area.right();
        for (id, label) in [(CommandId::ToggleActivityLog, "Hide"), (CommandId::ClearActivityLog, "Clear"), (CommandId::CopyActivityLog, "Copy")] {
            let width = ((label.len() + 2) as u16).min(x.saturating_sub(area.x));
            x -= width;
            let rect = Rect::new(x, area.y, width, 1);
            line(frame.buffer_mut(), rect, &format!(" {label} "), style(p.accent, p.header_bg));
            geometry.controls.push((id, rect));
        }
        line(frame.buffer_mut(), Rect::new(area.x, area.y, x - area.x, 1), " Activity Log", style(p.fg, p.header_bg));
        let rows = self.session.state().activity_log_rows(area.width.saturating_sub(2) as usize);
        let visible = area.height.saturating_sub(1) as usize;
        let maximum = rows.len().saturating_sub(visible);
        self.log_scroll = self.log_scroll.min(maximum);
        for (i, row) in rows.iter().skip(maximum - self.log_scroll).take(visible).enumerate() {
            line(frame.buffer_mut(), Rect::new(area.x.saturating_add(1), area.y + 1 + i as u16, area.width.saturating_sub(2), 1), row, style(p.fg, p.sidebar_bg));
        }
    }
}
