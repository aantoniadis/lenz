use super::*;
use code_core::terminal::Command as T;

impl App {
    pub(super) fn draw_terminal(&mut self, frame: &mut ratatui::Frame, geometry: &mut Geometry, body: &mut Rect, p: &Palette) {
        let terminal = self.session.state().terminal();
        if !terminal.shown() { return; }
        let height = if body.height < 6 { if terminal.focused() { body.height } else { 0 } } else { (body.height / 3).max(3) };
        let area = Rect::new(body.x, body.bottom().saturating_sub(height), body.width, height);
        body.height = body.height.saturating_sub(height);
        geometry.terminal.area = area;
        if area.is_empty() { return; }
        fill(frame.buffer_mut(), area, style(p.fg, p.bg));
        let mut x = area.right();
        for (id,short,long) in [(CommandId::StopTerminal,"!","Stop"), (CommandId::CloseTerminal,"×","Close"), (CommandId::ShowTerminal,"_","Hide"), (CommandId::NewTerminal,"+","+ New"), (CommandId::NextTerminal,">",">"), (CommandId::PreviousTerminal,"<","<")] {
            let label = if area.width >= 64 { long } else { short };
            let width = (label.width() as u16 + 1).min(x.saturating_sub(area.x));
            if width == 0 { break; }
            x -= width;
            let rect = Rect::new(x,area.y,width,1);
            let fg = if self.session.disabled(id).is_some() { p.dim } else { p.accent };
            line(frame.buffer_mut(),rect,&format!(" {label}"),style(fg,p.header_bg));
            geometry.controls.push((id,rect));
        }
        let active_id = terminal.active().map(|t|t.id());
        // Always reveal the active tab; Previous/Next and F10 reach tabs that do not fit.
        let active_index = terminal.tabs().iter().position(|t|Some(t.id()) == active_id).unwrap_or(0);
        let width = x.saturating_sub(area.x);
        let start = if terminal.tabs().iter().take(active_index + 1).map(|t|t.label().width() + t.error().map_or(0, UnicodeWidthStr::width) + 4).sum::<usize>() > width as usize { active_index } else { 0 };
        let mut at = area.x;
        for tab in terminal.tabs().iter().skip(start) {
            if at >= x { break; }
            let label = if let Some(error) = tab.error() { format!(" {}: {error} ",tab.label()) } else { format!(" {}{} ",tab.label(),if tab.running() { "" } else if tab.starting() { "…" } else { " ·" }) };
            let w = (label.width() as u16).min(x - at);
            let rect = Rect::new(at,area.y,w,1);
            let selected = Some(tab.id()) == active_id;
            line(frame.buffer_mut(),rect,&code_core::chrome::ellipsize(&label,w as usize),style(if tab.error().is_some() { p.error } else if selected { p.accent } else { p.fg },if selected { p.selected_bg } else { p.header_bg }).add_modifier(if selected { Modifier::BOLD } else { Modifier::empty() }));
            geometry.terminal.tabs.push((tab.id(),rect));
            at += w;
        }
        let finding = terminal.find().is_some();
        let find_rows = u16::from(finding && area.height > 1);
        if find_rows > 0 && let Some(label) = terminal.find_label(area.width as usize, "") {
            line(frame.buffer_mut(),Rect::new(area.x,area.y+1,area.width,1),&label,style(p.accent,p.header_bg));
        }
        let cells = Rect::new(area.x,area.y.saturating_add(1+find_rows),area.width,area.height.saturating_sub(1+find_rows));
        geometry.terminal.cells = cells;
        geometry.terminal.active = active_id;
        if let Some(tab) = terminal.active() {
            if let Some(screen) = tab.frame() {
                geometry.terminal.frame = Some(screen.clone());
                for row in 0..screen.rows.min(cells.height as usize) {
                    for column in 0..screen.columns.min(cells.width as usize) {
                        let Some(cell) = screen.cells.get(row.saturating_mul(screen.columns).saturating_add(column)) else { continue };
                        if cell.wide_spacer { continue; }
                        let mut st = style(cell.fg.unwrap_or(p.fg),cell.bg.unwrap_or(p.bg));
                        if cell.search_match && terminal.find().is_some_and(|query| screen.search.as_ref().is_some_and(|(q,result)| q == query && result == &Some(true))) { st = st.bg(color(p.found_bg)); }
                        else if cell.selected { st = st.bg(color(p.selected_bg)); }
                        for (enabled,modifier) in [(cell.bold,Modifier::BOLD),(cell.italic,Modifier::ITALIC),(cell.underline,Modifier::UNDERLINED),(cell.strikeout,Modifier::CROSSED_OUT),(cell.dim,Modifier::DIM)] { if enabled { st = st.add_modifier(modifier); } }
                        line(frame.buffer_mut(),Rect::new(cells.x + column as u16,cells.y + row as u16,(cells.width - column as u16).min(2),1),if cell.text.is_empty() { " " } else { &cell.text },st);
                    }
                }
                if terminal.focused() && !finding && self.modal.is_none() && let Some((row,column)) = screen.cursor
                    && row < cells.height as usize && column < cells.width as usize {
                    frame.set_cursor_position((cells.x + column as u16,cells.y + row as u16));
                }
            } else {
                line(frame.buffer_mut(),cells,tab.error().unwrap_or(tab.status()),style(if tab.error().is_some() { p.error } else { p.dim },p.bg));
            }
        } else { line(frame.buffer_mut(),cells,"Terminal · + New · F10 commands",style(p.dim,p.bg)); }
        let columns = (cells.width as usize).clamp(2, 512);
        let rows = (cells.height as usize).clamp(1, 256);
        // A stopped tab keeps its last screen but has no process to resize.
        if !cells.is_empty() && self.session.state().terminal().active().is_some_and(|t| t.running()) && geometry.terminal.frame.as_ref().is_some_and(|screen| screen.columns != columns || screen.rows != rows) {
            self.session.dispatch(Command::Terminal(T::Resize { columns, rows }));
        }
    }
}
