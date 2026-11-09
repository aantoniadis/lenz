//! Focused prompt input, before document shortcuts or global Ctrl+C handling.
use crate::app::{App, Prompt};
use crossterm::event::{Event, KeyEventKind};
use lenz::field_edit::{Command, Cursor};

impl App {
    pub(crate) fn field_event(&mut self, event: &Event) -> bool {
        if self.modal_except_prompt() {
            return false;
        }
        let Some(kind @ (Prompt::Search | Prompt::Line | Prompt::Files)) = self.prompt else {
            return false;
        };
        let command = match event {
            Event::Key(key) if key.kind != KeyEventKind::Release => lenz_terminal_ui::command(*key),
            Event::Paste(text) => Some(Command::Insert(text.clone())),
            _ => None,
        };
        let Some(command) = command else {
            return false;
        };
        let out = self.field_inputs.edit_limited(
            kind as u64,
            0,
            Cursor::default(),
            &mut self.input,
            command,
            4096,
        );
        if out.changed {
            self.selected = 0;
            if kind == Prompt::Search {
                self.search_input();
            }
        }
        if let Some(text) = out.copy {
            self.copy(&text);
        }
        true
    }
}
