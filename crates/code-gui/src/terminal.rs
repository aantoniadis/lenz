//! Native cell geometry and input translation; lifecycle and terminal content belong to the core.
use super::*;
use gpui::{point, size};
use code_core::terminal::{self, Command as TerminalCommand};

#[derive(Default)]
pub(super) struct Ui {
    pub marked: Option<(terminal::Id, String)>,
    pub mark_selection: std::ops::Range<usize>,
    pub suppress_commit: Option<(terminal::Id, String)>,
    pub path_context: Option<Result<Command,String>>,
    pub mouse_capture: Option<(terminal::Id,u8)>,
    pub hover: Option<(terminal::Id,usize,usize)>,
    pub wheel: f32,
    pub external_paths: Option<Vec<PathBuf>>,
    pub explorer_drag: Option<(Arc<str>, f32, f32, bool)>,
}

pub(super) struct TerminalLayout {
    pub bounds: Bounds<Pixels>,
    pub grid: Bounds<Pixels>,
    pub find: Option<Bounds<Pixels>>,
    pub buttons: Vec<(Bounds<Pixels>, CommandId, &'static str)>,
    pub tabs: Vec<(Bounds<Pixels>, terminal::Id, String, bool)>,
}

impl ReaderView {
    pub(super) fn terminal_height(&self) -> f32 {
        if !self.session.state().terminal().shown() { return 0.0; }
        let bottom = (self.height - self.status_h() - self.activity_height()).max(0.0);
        let available = (bottom - toolbar_h() - line_h()).max(0.0);
        if available >= line_h() * 6.0 { (available / 3.0 / line_h()).floor().max(2.0) * line_h() }
        else if self.session.state().terminal().focused() { (bottom - toolbar_h().min(bottom)).max(line_h().min(bottom)).min(bottom) }
        else { 0.0 }
    }

    pub(super) fn terminal_layout(&self) -> Option<TerminalLayout> {
        let height = self.terminal_height();
        if height <= 0.0 { return None; }
        let top = (self.height - self.status_h() - self.activity_height() - height).max(0.0);
        let header = line_h().min(height);
        let bounds = Bounds { origin: point(px(0.0), px(top)), size: size(px(self.width.max(0.0)), px(height)) };
        let find_h = if self.session.state().terminal().find().is_some() { line_h().min((height-header).max(0.0)) } else { 0.0 };
        let find = (find_h > 0.0).then(|| Bounds { origin: point(px(0.0), px(top+header)), size: size(bounds.size.width, px(find_h)) });
        let grid = Bounds { origin: point(px(0.0), px(top + header + find_h)), size: size(bounds.size.width, px((height - header - find_h).max(0.0))) };
        let mut right = self.width.max(0.0);
        let buttons = [(CommandId::ShowTerminal, "Hide"), (CommandId::CloseTerminal, "×"), (CommandId::StopTerminal, "Stop"), (CommandId::NewTerminal, "+"), (CommandId::NextTerminal, "›"), (CommandId::PreviousTerminal, "‹")].into_iter().map(|(id,label)| {
            let width = ((label.chars().count() + 2) as f32 * self.char_w).min(right);
            right -= width;
            (Bounds { origin: point(px(right),px(top)), size: size(px(width),px(header)) }, id, label)
        }).collect();
        let state = self.session.state().terminal();
        let active = state.active().map(|t| t.id());
        let labels: Vec<_> = state.tabs().iter().map(|tab| if !tab.running() && !tab.status().is_empty() {format!("{} · {}",tab.label(),tab.status())} else {tab.label().to_owned()}).collect();
        let widths: Vec<_> = labels.iter().map(|label| ((lenz_content::wrap::width(label) + 2) as f32 * self.char_w).min(right.max(self.char_w))).collect();
        let selected = state.tabs().iter().position(|t| Some(t.id()) == active).unwrap_or(0);
        let end: f32 = widths.iter().take(selected + 1).sum();
        let start: f32 = widths.iter().take(selected).sum();
        let mut x = -(end - right).max(0.0).min(start);
        let tabs = state.tabs().iter().zip(widths).zip(labels).filter_map(|((tab,width),label)| {
            let a = x.max(0.0); let b = (x + width).min(right); x += width;
            if b > a {Some((Bounds { origin: point(px(a),px(top)), size: size(px(b-a),px(header)) },tab.id(),label,Some(tab.id()) == active))} else {None}
        }).collect();
        Some(TerminalLayout { bounds, grid, find, buttons, tabs })
    }

    pub(super) fn terminal_drop_paths(&mut self, paths: &[PathBuf], at: gpui::Point<Pixels>, cx: &mut Context<Self>) -> bool {
        if self.modal.is_some() || paths.is_empty() || !self.terminal_layout().is_some_and(|l| l.grid.contains(&at)) { return false; }
        self.dispatch(Command::Terminal(TerminalCommand::Paths(paths.to_vec())),cx);
        true
    }
}

impl ReaderView {
    pub(super) fn terminal_marked(&self) -> Option<&str> {
        let id = self.session.state().terminal().active()?.id();
        self.terminal_ui.marked.as_ref().filter(|(old,_)| *old == id && self.terminal_active()).map(|(_,text)| text.as_str())
    }
    pub(super) fn terminal_active(&self) -> bool {
        self.modal.is_none() && self.session.state().terminal().focused()
    }

    pub(super) fn blur_terminal(&mut self, cx: &mut Context<Self>) {
        if self.session.state().terminal().focused() { self.dispatch(Command::Terminal(TerminalCommand::Focus(false)), cx); }
    }

    pub(super) fn sync_terminal_prompt(&mut self) {
        match (self.session.state().terminal().prompt(), &mut self.modal) {
            (Some(prompt), Some(Modal::StatusDetails { kind: Details::Terminal { ticket }, text, .. })) if *ticket == prompt.ticket => {
                *text = prompt.text.clone();
            }
            // A newer question replaces the open one entirely: its title and buttons, and the ticket they answer.
            (Some(prompt), None | Some(Modal::StatusDetails { kind: Details::Terminal { .. }, .. })) => {
                self.modal_scroll.set(0);
                self.modal = Some(Modal::StatusDetails { title: prompt.title, text: prompt.text.clone(), logo: None, kind: Details::Terminal { ticket: prompt.ticket } });
            }
            (None, Some(Modal::StatusDetails { kind: Details::Terminal { .. }, .. })) => self.modal = None,
            _ => {}
        }
    }

    pub(super) fn terminal_prompt_scroll(&self, ev: &KeyDownEvent) {
        if let Some(l) = self.modal_layout() {
            let maximum = l.lines.len().saturating_sub(l.visible);
            let step = l.visible.saturating_sub(1).max(1);
            self.modal_scroll.set(match ev.keystroke.key.as_str() {
                "home" => 0, "end" => maximum,
                "up" => l.first.saturating_sub(1), "down" => (l.first + 1).min(maximum),
                "pageup" => l.first.saturating_sub(step), "pagedown" => (l.first + step).min(maximum),
                _ => l.first,
            });
        }
    }

    pub(super) fn terminal_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.terminal_active() { return false; }
        let k = &ev.keystroke;
        // Command (Super on Linux) runs Lenz chords only; programs receive Control alone.
        let command = k.modifiers.control || k.modifiers.platform;
        let ctrl = k.modifiers.control;
        if command && !k.modifiers.alt && matches!(k.key.as_str(), "f" | "F") {
            self.terminal_ui.marked = None;
            self.input_state.stale = true;
            self.run(CommandId::FindTerminal, window, cx);
            self.resize_terminal(cx);
            cx.stop_propagation(); return true;
        }
        if k.key == "f10" {
            self.input_state.stale = true;
            self.terminal_ui.marked = None;
            self.modal = Some(Modal::Commands { at: 0 });
            cx.stop_propagation(); cx.notify(); return true;
        }
        if let Some(id) = self.key_command(ev).filter(|id| id.passes_terminal_focus()) {
            self.run(id, window, cx); cx.stop_propagation(); return true;
        }
        // Plain Ctrl+V stays the program's (quoted insert, visual block), as Ctrl+Shift+C copy implies.
        if ((ctrl && k.modifiers.shift) || k.modifiers.platform) && !k.modifiers.alt && matches!(k.key.as_str(), "v" | "V") {
            self.terminal_ui.marked = None;
            if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                self.dispatch(Command::Terminal(TerminalCommand::Paste(text)), cx);
            }
            cx.stop_propagation(); return true;
        }
        let finding = self.session.state().terminal().find().is_some();
        if !finding && command && k.modifiers.shift && !k.modifiers.alt && matches!(k.key.as_str(), "c" | "C") {
            self.dispatch(Command::Terminal(TerminalCommand::Copy), cx);
            cx.stop_propagation(); return true;
        }
        if !finding && k.modifiers.shift && !command && !k.modifiers.alt {
            let rows = self.terminal_layout().map_or(1,|layout| (f32::from(layout.grid.size.height)/line_h()).floor().max(1.0) as i32);
            let command = match k.key.as_str() {
                "pageup" => Some(TerminalCommand::Scroll {lines:rows,local:true}),
                "pagedown" => Some(TerminalCommand::Scroll {lines:-rows,local:true}),
                "home" => Some(TerminalCommand::ScrollTo {top:true}),
                "end" => Some(TerminalCommand::ScrollTo {top:false}),
                _ => None,
            };
            if let Some(command) = command {
                self.terminal_ui.marked = None; self.input_state.stale = true;
                self.dispatch(Command::Terminal(command),cx);
                cx.stop_propagation(); return true;
            }
        }
        if k.modifiers.platform { cx.stop_propagation(); return true; }
        let printable = !ctrl && !k.modifiers.alt && k.key_char.as_deref().is_some_and(|text| !text.chars().any(char::is_control));
        if printable {
            self.input_state.stale = false;
            self.terminal_ui.suppress_commit = None;
            if !self.input_state.registered.get() {
                self.dispatch(Command::Terminal(TerminalCommand::Text(k.key_char.clone().unwrap_or_default())),cx);
                cx.stop_propagation();
            }
            return true;
        }
        if self.terminal_ui.marked.is_some() {
            match k.key.as_str() {
                "escape" => { self.terminal_ui.marked = None; cx.stop_propagation(); cx.notify(); return true; }
                "enter" => {
                    if let Some((_,text)) = self.terminal_ui.marked.take() { self.dispatch(Command::Terminal(TerminalCommand::Text(text)),cx); }
                    cx.stop_propagation(); return true;
                }
                _ => self.terminal_ui.marked = None,
            }
        }
        if finding { self.input_state.stale = true; }
        if let Some(press) = code_core::keymap::KeyPress::new(&k.key,ctrl,k.modifiers.shift,k.modifiers.alt) {
            self.dispatch(Command::Terminal(TerminalCommand::Key(press)),cx);
        }
        cx.stop_propagation(); true
    }

    pub(super) fn resize_terminal(&mut self, cx: &mut Context<Self>) {
        let Some(frame) = self.session.state().terminal().active().filter(|t| t.running()).and_then(|t| t.frame()) else { return };
        let Some(layout) = self.terminal_layout() else { return };
        let columns = self.columns(f32::from(layout.grid.size.width)).clamp(2,512);
        let rows = ((f32::from(layout.grid.size.height) / line_h()).floor() as usize).clamp(1,256);
        if frame.columns != columns || frame.rows != rows {
            self.dispatch(Command::Terminal(TerminalCommand::Resize { columns,rows }),cx);
        }
    }

    fn terminal_cell(&self, at: gpui::Point<Pixels>, captured: bool) -> Option<(usize,usize)> {
        let layout = self.terminal_layout()?;
        if layout.grid.size.width <= px(0.0) || layout.grid.size.height <= px(0.0) || (!captured && !layout.grid.contains(&at)) { return None; }
        let frame = self.session.state().terminal().active()?.frame()?;
        let column = ((f32::from(at.x-layout.grid.left()) / self.char_w).floor() as usize).min(frame.columns.saturating_sub(1));
        let row = ((f32::from(at.y-layout.grid.top()) / line_h()).floor() as usize).min(frame.rows.saturating_sub(1));
        (frame.columns > 0 && frame.rows > 0).then_some((row,column))
    }

    fn terminal_mouse(&mut self, at: gpui::Point<Pixels>, kind: terminal::MouseKind, modifiers: gpui::Modifiers, captured: bool, cx: &mut Context<Self>) {
        if self.session.state().terminal().find().is_some() { return; }
        if let Some((row,column)) = self.terminal_cell(at,captured) {
            self.dispatch(Command::Terminal(TerminalCommand::Mouse { row,column,kind,ctrl:modifiers.control,alt:modifiers.alt,shift:modifiers.shift }),cx);
        }
    }

    pub(super) fn terminal_click(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(layout) = self.terminal_layout().filter(|l| l.bounds.contains(&ev.position)) else { return false };
        if self.modal.is_some() { return true; }
        self.cancel_input(true,cx);
        self.dispatch(Command::Terminal(TerminalCommand::Focus(true)),cx);
        if let Some((_,id,_)) = layout.buttons.iter().find(|(b,_,_)| b.size.width > px(0.0) && b.contains(&ev.position)) {
            if ev.button == MouseButton::Left { self.run(*id,window,cx); }
        } else if let Some((_,id,_,_)) = layout.tabs.iter().find(|(b,_,_,_)| b.contains(&ev.position)) {
            if ev.button == MouseButton::Left { self.dispatch(Command::Terminal(TerminalCommand::Activate(*id)),cx); }
        } else if layout.grid.contains(&ev.position)
            && let Some(button) = mouse_button(ev.button)
            && self.terminal_cell(ev.position,false).is_some()
        {
            self.terminal_ui.mouse_capture = self.session.state().terminal().active().map(|tab| (tab.id(),button));
            self.terminal_mouse(ev.position,terminal::MouseKind::Press(button),ev.modifiers,false,cx);
        }
        cx.stop_propagation(); true
    }

    pub(super) fn terminal_wheel(&mut self, ev: &ScrollEvent, cx: &mut Context<Self>) -> bool {
        if !self.terminal_layout().is_some_and(|l| l.bounds.contains(&ev.position)) { return false; }
        self.terminal_ui.wheel += match ev.delta { ScrollDelta::Pixels(p) => f32::from(p.y)/(line_h()*3.0), ScrollDelta::Lines(p) => p.y };
        let lines = self.terminal_ui.wheel.trunc().clamp(-64.0,64.0) as i32;
        self.terminal_ui.wheel -= lines as f32;
        if ev.modifiers.shift || self.session.state().terminal().find().is_some() {
            if lines != 0 { self.dispatch(Command::Terminal(TerminalCommand::Scroll { lines:lines*3,local:true }),cx); }
        } else {
            for _ in 0..lines.unsigned_abs() { self.terminal_mouse(ev.position,terminal::MouseKind::Press(if lines > 0 {64} else {65}),ev.modifiers,false,cx); }
        }
        true
    }

    pub(super) fn terminal_drag_move(&mut self, ev: &MouseMoveEvent, cx: &mut Context<Self>) -> bool {
        if let Some((_,x,y,moved)) = &mut self.terminal_ui.explorer_drag {
            if ev.pressed_button == Some(MouseButton::Left) { *moved |= (f32::from(ev.position.x)-*x).hypot(f32::from(ev.position.y)-*y) > 6.0; return true; }
            self.terminal_ui.explorer_drag = None;
        }
        if self.drag.is_some() { return false; }
        if cx.has_active_drag() { return true; }
        let active = self.session.state().terminal().active().map(|tab| tab.id());
        let captured = self.terminal_ui.mouse_capture.is_some_and(|(id,_)| Some(id) == active);
        if !captured {self.terminal_ui.mouse_capture = None;}
        let over = self.terminal_layout().is_some_and(|layout| layout.grid.contains(&ev.position));
        if let Some(button) = ev.pressed_button.and_then(mouse_button) {
            self.terminal_ui.hover = None;
            if captured {self.terminal_mouse(ev.position,terminal::MouseKind::Move(button),ev.modifiers,true,cx);}
        } else {
            self.terminal_ui.mouse_capture = None;
            // Motion reports name cells; a move within the last reported cell sends nothing.
            let cell = over.then(|| self.terminal_cell(ev.position,false)).flatten().zip(active).map(|((row,column),id)| (id,row,column));
            if cell.is_some() && cell != self.terminal_ui.hover {self.terminal_mouse(ev.position,terminal::MouseKind::Move(3),ev.modifiers,false,cx);}
            self.terminal_ui.hover = cell;
        }
        captured || self.terminal_layout().is_some_and(|layout| layout.bounds.contains(&ev.position))
    }

    pub(super) fn terminal_drag_end(&mut self, ev: &gpui::MouseUpEvent, cx: &mut Context<Self>) -> bool {
        let explorer = self.terminal_ui.explorer_drag.take();
        let over = self.terminal_layout().is_some_and(|l| l.grid.contains(&ev.position));
        if let Some((path,_,_,true)) = explorer && over {
            match self.session.terminal_path_command(Some(&path)) {
                Ok(command) => self.dispatch(command,cx), Err(why) => self.dispatch(Command::Notice(why),cx),
            }
            return true;
        }
        if let Some(Drag::Tab { id,moved:true,.. }) = self.drag && over {
            if self.session.state().active().is_some_and(|t| t.id() == id) {
                match self.session.terminal_path_command(None) {
                    Ok(command) => self.dispatch(command,cx), Err(why) => self.dispatch(Command::Notice(why),cx),
                }
            }
            return true;
        }
        // A divider or tab drag released over the cells still commits through end_drag.
        if self.drag.is_some() { return false; }
        let capture = self.terminal_ui.mouse_capture.take();
        let current = self.session.state().terminal().active().map(|tab| tab.id());
        let captured = capture.is_some_and(|(id,button)| Some(id) == current && mouse_button(ev.button) == Some(button));
        if captured && let Some(button) = mouse_button(ev.button) {
            self.terminal_mouse(ev.position,terminal::MouseKind::Release(button),ev.modifiers,true,cx);
        }
        captured || over
    }

    pub(super) fn terminal_input_bounds(&self) -> Option<Bounds<Pixels>> {
        let layout = self.terminal_layout()?;
        if let Some(find) = layout.find {
            return Some(Bounds { origin: find.origin + point(px(self.char_w*6.0), px(0.0)), size: size(px(self.char_w), find.size.height) });
        }
        let (row,column) = self.session.state().terminal().active()?.frame()?.cursor?;
        let at = point(layout.grid.left()+px(column as f32*self.char_w),layout.grid.top()+px(row as f32*line_h()));
        layout.grid.contains(&at).then(|| Bounds { origin: at,size:size(px(self.char_w),px(line_h())) })
    }
}

fn mouse_button(button: MouseButton) -> Option<u8> {
    match button { MouseButton::Left => Some(0),MouseButton::Middle => Some(1),MouseButton::Right => Some(2),_ => None }
}

pub(super) struct Paint {
    layout: TerminalLayout,
    frame: Option<Arc<terminal::Frame>>,
    pub(super) notice: String,
    pub(super) error: bool,
    disabled: Vec<CommandId>,
    focused: bool,
    marked: Option<String>,
    find: Option<(String, String)>,
    char_w: f32,
}
impl ReaderView {
    pub(super) fn terminal_paint(&self) -> Option<Paint> {
        let layout = self.terminal_layout()?;
        let state = self.session.state().terminal();
        let tab = state.active();
        let disabled = layout.buttons.iter().filter(|(_,id,_)| self.session.disabled(*id).is_some()).map(|(_,id,_)| *id).collect();
        Some(Paint {
            layout, frame: tab.and_then(|t| t.frame()).cloned(),
            notice: tab.map_or_else(|| "New Terminal (+)".into(),|t| t.error().unwrap_or(t.status()).to_owned()),
            error: tab.is_some_and(|t| t.error().is_some()),
            disabled,
            focused: self.terminal_active(),
            marked: self.terminal_ui.marked.as_ref().filter(|(id,_)| tab.is_some_and(|t| t.id() == *id)).map(|(_,text)| text.clone()),
            find: state.find().map(|query| (query.to_owned(),state.find_label((self.width/self.char_w).floor().max(0.0) as usize,self.terminal_marked().unwrap_or("")).unwrap_or_default())),
            char_w: self.char_w,
        })
    }
}
impl Paint {
    pub(super) fn paint(&self, window: &mut Window, cx: &mut App, m: Metrics<'_>, palette: &lenz_content::theme::Palette, x0: f32, y0: f32) {
        let offset = |b: Bounds<Pixels>| Bounds { origin: b.origin + point(px(x0),px(y0)),size:b.size };
        let bounds = offset(self.layout.bounds);
        window.with_content_mask(Some(gpui::ContentMask { bounds }),|window| {
            let (x,y,w,h) = (f32::from(bounds.left()),f32::from(bounds.top()),f32::from(bounds.size.width),f32::from(bounds.size.height));
            lenz_gpui::fill(window,x,y,w,h,palette.bg);
            lenz_gpui::fill(window,x,y,w,line_h().min(h),palette.header_bg);
            lenz_gpui::fill(window,x,y,w,1.0,if self.focused {palette.accent} else {palette.border});
            for (b,_,label,active) in &self.layout.tabs {
                let b = offset(*b);
                window.with_content_mask(Some(gpui::ContentMask { bounds:b }),|window| {
                    let (x,y,w) = (f32::from(b.left()),f32::from(b.top()),f32::from(b.size.width));
                    if *active { lenz_gpui::fill(window,x,y+1.0,w,(line_h()-1.0).min(h),palette.selected_bg); }
                    lenz_gpui::text(window,cx,m,x+self.char_w,y,label,if *active {palette.accent} else {palette.fg});
                });
            }
            for (b,id,label) in &self.layout.buttons {
                let b = offset(*b);
                if b.size.width <= px(0.0) { continue; }
                window.with_content_mask(Some(gpui::ContentMask { bounds:b }),|window| {
                    lenz_gpui::text(window,cx,m,f32::from(b.left())+self.char_w,f32::from(b.top()),label,if self.disabled.contains(id) {palette.dim} else {palette.accent});
                });
            }
            if let Some((_,label)) = &self.find && let Some(find) = self.layout.find {
                let find = offset(find);
                window.with_content_mask(Some(gpui::ContentMask { bounds:find }), |window| {
                    lenz_gpui::fill(window,f32::from(find.left()),f32::from(find.top()),f32::from(find.size.width),f32::from(find.size.height),palette.header_bg);
                    lenz_gpui::text(window,cx,m,f32::from(find.left()),f32::from(find.top()),label,palette.accent);
                });
            }
            let grid = offset(self.layout.grid);
            window.with_content_mask(Some(gpui::ContentMask { bounds:grid }),|window| {
                let (x,y) = (f32::from(grid.left()),f32::from(grid.top()));
                if let Some(frame) = &self.frame {
                    let rows = frame.rows.min((f32::from(grid.size.height)/line_h()).ceil() as usize);
                    let columns = frame.columns.min((f32::from(grid.size.width)/self.char_w).ceil() as usize);
                    for row in 0..rows {
                        let mut column = 0;
                        while column < columns {
                            let Some(cell) = frame.cells.get(row*frame.columns+column) else { break };
                            let start = column;
                            let mut text = String::new();
                            while column < columns {
                                let Some(next) = frame.cells.get(row*frame.columns+column) else { break };
                                if !same_style(cell,next) { break; }
                                // Fallback-font and bidi glyphs advance by their own metrics: each starts at its cell.
                                let own = !next.text.is_ascii();
                                if own && column > start { break; }
                                if !next.wide_spacer { text.push_str(if next.text.is_empty() { " " } else { &next.text }); }
                                column += 1;
                                if own {
                                    while frame.cells.get(row*frame.columns+column).is_some_and(|c| c.wide_spacer) && column < columns { column += 1; }
                                    break;
                                }
                            }
                            let fg = if cell.dim { lenz_content::theme::mix(cell.fg.unwrap_or(palette.fg),palette.bg,0.5) } else {cell.fg.unwrap_or(palette.fg)};
                            let bg = if cell.search_match && self.find.as_ref().is_some_and(|(query,_)| frame.search.as_ref().is_some_and(|(q,result)| q == query && result == &Some(true))) {palette.found_bg} else if cell.selected {palette.selected_bg} else {cell.bg.unwrap_or(palette.bg)};
                            let (tx,ty,tw) = (x+start as f32*self.char_w,y+row as f32*line_h(),(column-start) as f32*self.char_w);
                            if bg != palette.bg {lenz_gpui::fill(window,tx,ty,tw,line_h(),bg);}
                            let font = Font { weight:if cell.bold {gpui::FontWeight::BOLD} else {m.font.weight},style:if cell.italic {gpui::FontStyle::Italic} else {m.font.style},..m.font.clone() };
                            if !text.trim().is_empty() {lenz_gpui::text(window,cx,Metrics {font:&font,..m},tx,ty,&text,fg);}
                            if cell.underline {lenz_gpui::fill(window,tx,ty+line_h()-2.0,tw,1.0,fg);}
                            if cell.strikeout {lenz_gpui::fill(window,tx,ty+line_h()*0.5,tw,1.0,fg);}
                        }
                    }
                    if self.focused && self.find.is_none() && let Some((row,column)) = frame.cursor.filter(|(r,c)| *r < rows && *c < columns) {
                        let (tx,ty) = (x+column as f32*self.char_w,y+row as f32*line_h());
                        lenz_gpui::fill(window,tx,ty,2.0,line_h(),palette.caret);
                        if let Some(marked) = &self.marked {
                            lenz_gpui::text(window,cx,m,tx,ty,marked,palette.fg);
                            lenz_gpui::fill(window,tx,ty+line_h()-2.0,lenz_content::wrap::width(marked) as f32*self.char_w,1.0,palette.accent);
                        }
                    }
                } else if !self.error {
                    let shown = code_core::chrome::ellipsize(&self.notice,(w/self.char_w).max(0.0) as usize);
                    lenz_gpui::text(window,cx,m,x+4.0,y,&shown,palette.dim);
                }
                if self.error && grid.size.height > px(0.0) {
                    let bottom = (f32::from(grid.bottom())-line_h()).max(y);
                    lenz_gpui::fill(window,x,bottom,w,line_h(),palette.header_bg);
                    let shown = code_core::chrome::ellipsize(&self.notice,((w-8.0)/self.char_w).max(0.0) as usize);
                    lenz_gpui::text(window,cx,m,x+4.0,bottom,&shown,palette.error);
                }
            });
        });
    }
}
fn same_style(a: &terminal::Cell,b: &terminal::Cell) -> bool {
    a.fg == b.fg && a.bg == b.bg && a.bold == b.bold && a.italic == b.italic && a.underline == b.underline && a.strikeout == b.strikeout && a.dim == b.dim && a.selected == b.selected && a.search_match == b.search_match
}
