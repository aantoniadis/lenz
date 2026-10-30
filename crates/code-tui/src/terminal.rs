//! Terminal geometry and host input only; process, selection and scroll state belong to the core.
use crate::app::{App, Details, Modal, Prompt};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Position, Rect};
use code_core::{session::{Command, CommandId}, terminal::{self, Command as T}};
use std::sync::Arc;

#[derive(Default)]
pub(crate) struct Geometry {
    pub area: Rect,
    pub cells: Rect,
    pub tabs: Vec<(terminal::Id, Rect)>,
    pub active: Option<terminal::Id>,
    pub frame: Option<Arc<terminal::Frame>>,
}

impl App {
    pub(crate) fn terminal_command(&mut self, command: T) { self.dispatch(Command::Terminal(command)); }

    pub(crate) fn sync_terminal_prompt(&mut self) {
        let prompt = self.session.state().terminal().prompt().map(|p| (p.ticket, p.title, p.text.clone()));
        match (prompt, &mut self.modal) {
            (Some((ticket, _, text)), Some(Modal { kind: Prompt::Details { kind: Details::Terminal(held), body, .. }, .. })) if *held == ticket => *body = text,
            // A newer question replaces the open one entirely: its title and buttons, and the ticket they answer.
            (Some((ticket, title, body)), None | Some(Modal { kind: Prompt::Details { kind: Details::Terminal(_), .. }, .. })) => {
                self.modal = None;
                self.prompt(Prompt::Details { title, body, logo: false, kind: Details::Terminal(ticket) });
            }
            (None, Some(Modal { kind: Prompt::Details { kind: Details::Terminal(_), .. }, .. })) => self.modal = None,
            _ => {}
        }
    }

    /// A terminal gets ordinary keys before editor commands, including Ctrl+Q and Escape.
    pub(crate) fn terminal_key(&mut self, key: KeyEvent) -> bool {
        if self.session.state().terminal().focused() && key.code == KeyCode::F(10) { self.prompt(Prompt::Commands); return true; }
        if let Some(id) = super::app::key_press(key).and_then(|p| self.session.key_command(&p))
            && id.passes_terminal_focus()
        { self.run(id); return true; }
        if !self.session.state().terminal().focused() { return false; }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        if ctrl && !alt && matches!(key.code, KeyCode::Char('f' | 'F')) { self.run(CommandId::FindTerminal); return true; }
        if self.session.state().terminal().find().is_some() {
            if let KeyCode::Char(c) = key.code && !ctrl && !alt { self.terminal_command(T::Text(c.to_string())); }
            else if let Some(press) = super::app::key_press(key) { self.terminal_command(T::Key(press)); }
            return true;
        }
        if ctrl && shift && !alt && matches!(key.code, KeyCode::Char('c' | 'C')) { self.terminal_command(T::Copy); return true; }
        if let Some(press) = keypad_press(key) { self.terminal_command(T::Key(press)); return true; }
        if shift && !ctrl && !alt {
            let rows = self.layout.terminal.cells.height.max(1) as i32;
            let command = match key.code {
                KeyCode::PageUp => Some(T::Scroll { lines: rows, local: true }),
                KeyCode::PageDown => Some(T::Scroll { lines: -rows, local: true }),
                KeyCode::Home => Some(T::ScrollTo { top: true }),
                KeyCode::End => Some(T::ScrollTo { top: false }),
                _ => None,
            };
            if let Some(command) = command { self.terminal_command(command); return true; }
        }
        if let KeyCode::Char(c) = key.code && !ctrl && !alt {
            self.terminal_command(T::Text(c.to_string()));
        } else if let Some(press) = super::app::key_press(key) {
            self.terminal_command(T::Key(press));
        }
        true
    }

    pub(crate) fn terminal_mouse(&mut self, event: MouseEvent) -> bool {
        let point = Position::new(event.column, event.row);
        let terminal = self.session.state().terminal();
        let capture = self.terminal_drag.is_some() && matches!(event.kind, MouseEventKind::Drag(_) | MouseEventKind::Up(_));
        if !terminal.shown() || (!capture && !self.layout.terminal.area.contains(point)) { self.terminal_drag = None; return false; }
        if event.kind == MouseEventKind::Down(MouseButton::Left) {
            if let Some((id, _)) = self.layout.terminal.tabs.iter().find(|(_, r)| r.contains(point)) {
                let id = *id;
                if terminal.tabs().iter().any(|t| t.id() == id) { self.terminal_command(T::Activate(id)); }
                return true;
            }
            if let Some((id, _)) = self.layout.controls.iter().find(|(_, r)| r.contains(point)) {
                self.run(*id); return true;
            }
        }
        // The wheel scrolls the history during Find or with Shift, as in the desktop; otherwise it is the program's.
        if matches!(event.kind, MouseEventKind::ScrollUp | MouseEventKind::ScrollDown)
            && (terminal.find().is_some() || event.modifiers.contains(KeyModifiers::SHIFT))
            && self.layout.terminal.cells.contains(point)
        {
            let lines = if event.kind == MouseEventKind::ScrollUp { 3 } else { -3 };
            self.terminal_command(T::Scroll { lines, local: true });
            return true;
        }
        if terminal.find().is_some() { return true; }
        if self.layout.terminal.cells.is_empty() || (!capture && !self.layout.terminal.cells.contains(point)) { return true; }
        // A drag that began outside the cells belongs to the editor or a divider, which also needs its release.
        if !capture && matches!(event.kind, MouseEventKind::Drag(_) | MouseEventKind::Up(_)) { return false; }
        let active_id = terminal.active().map(|t| t.id());
        let current = terminal.active().filter(|t| Some(t.id()) == self.layout.terminal.active && (!capture || self.terminal_drag == Some(t.id())))
            .is_some_and(|active| matches!((active.frame(), self.layout.terminal.frame.as_ref()), (Some(a), Some(b)) if Arc::ptr_eq(a,b)));
        if matches!(event.kind, MouseEventKind::Down(_)) {
            self.log_focus = false;
            self.terminal_command(T::Focus(true));
        }
        if !current { self.terminal_drag = None; return true; }
        match event.kind {
            MouseEventKind::Down(_) => self.terminal_drag = active_id,
            MouseEventKind::Up(_) => self.terminal_drag = None,
            _ => {}
        }
        let kind = match event.kind {
            MouseEventKind::Down(b) => terminal::MouseKind::Press(button(b)),
            MouseEventKind::Up(b) => terminal::MouseKind::Release(button(b)),
            MouseEventKind::Drag(b) => terminal::MouseKind::Move(button(b)),
            MouseEventKind::Moved => terminal::MouseKind::Move(3),
            MouseEventKind::ScrollUp => terminal::MouseKind::Press(64),
            MouseEventKind::ScrollDown => terminal::MouseKind::Press(65),
            _ => return true,
        };
        let row = usize::from(event.row.saturating_sub(self.layout.terminal.cells.y).min(self.layout.terminal.cells.height.saturating_sub(1)));
        let column = usize::from(event.column.saturating_sub(self.layout.terminal.cells.x).min(self.layout.terminal.cells.width.saturating_sub(1)));
        self.terminal_command(T::Mouse { row, column, kind,
            ctrl: event.modifiers.contains(KeyModifiers::CONTROL),
            alt: event.modifiers.contains(KeyModifiers::ALT), shift: event.modifiers.contains(KeyModifiers::SHIFT) });
        true
    }
}
fn button(button: MouseButton) -> u8 { match button { MouseButton::Left => 0, MouseButton::Middle => 1, MouseButton::Right => 2 } }

/// Legacy host terminals omit KEYPAD; those events retain their delivered ordinary key semantics.
fn keypad_press(key: KeyEvent) -> Option<code_core::keymap::KeyPress> {
    if !key.state.contains(crossterm::event::KeyEventState::KEYPAD) { return None; }
    let name = match key.code {
        KeyCode::Char(c @ '0'..='9') => format!("kp{c}"),
        KeyCode::Enter => "kpenter".into(),
        KeyCode::Char('+') => "kpadd".into(),
        KeyCode::Char('-') => "kpsubtract".into(),
        KeyCode::Char('*') => "kpmultiply".into(),
        KeyCode::Char('/') => "kpdivide".into(),
        KeyCode::Char('.') | KeyCode::Delete => "kpdecimal".into(),
        KeyCode::Char('=') => "kpequal".into(),
        KeyCode::Char(',') => "kpseparator".into(),
        KeyCode::Insert => "kp0".into(),
        KeyCode::End => "kp1".into(),
        KeyCode::Down => "kp2".into(),
        KeyCode::PageDown => "kp3".into(),
        KeyCode::Left => "kp4".into(),
        KeyCode::KeypadBegin => "kp5".into(),
        KeyCode::Right => "kp6".into(),
        KeyCode::Home => "kp7".into(),
        KeyCode::Up => "kp8".into(),
        KeyCode::PageUp => "kp9".into(),
        _ => return None,
    };
    code_core::keymap::KeyPress::new(&name,key.modifiers.contains(KeyModifiers::CONTROL),key.modifiers.contains(KeyModifiers::SHIFT),key.modifiers.contains(KeyModifiers::ALT))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEventKind, KeyEventState};

    #[test]
    fn reported_keypad_identity_and_modifiers_reach_core() {
        for (code, name) in [(KeyCode::Char('0'), "kp0"), (KeyCode::Char('9'), "kp9"),
            (KeyCode::Enter,"kpenter"), (KeyCode::Char('+'),"kpadd"),
            (KeyCode::Char('-'),"kpsubtract"), (KeyCode::Char('*'),"kpmultiply"),
            (KeyCode::Char('/'),"kpdivide"), (KeyCode::Char('.'),"kpdecimal"),
            (KeyCode::Char('='),"kpequal"), (KeyCode::Char(','),"kpseparator"),
            (KeyCode::Home,"kp7"), (KeyCode::KeypadBegin,"kp5")] {
            let key = KeyEvent::new_with_kind_and_state(code,KeyModifiers::ALT,KeyEventKind::Press,KeyEventState::KEYPAD);
            let press = keypad_press(key).unwrap();
            assert_eq!(press.key,name);
            assert!(press.alt);
            assert!(!press.ctrl && !press.shift);
            assert!(keypad_press(KeyEvent::new(code,KeyModifiers::NONE)).is_none());
        }
    }
}
