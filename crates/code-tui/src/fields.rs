//! Terminal text fields use the same edit commands and ranges as the GUI; the terminal supplies paste events.
use crate::app::{App, Modal, Prompt};
use code_core::{line_edit::Edit, session::Command};
use crossterm::event::{KeyCode, KeyEvent};
use lenz_content::field_edit::{Command as FieldCommand, Cursor};

impl App {
    pub(crate) fn editable_prompt(kind: &Prompt) -> bool {
        !matches!(
            kind,
            Prompt::Help
                | Prompt::Details { .. }
                | Prompt::Mnemonic
                | Prompt::Context { .. }
                | Prompt::Menu(_)
                | Prompt::LanguageServer
                | Prompt::Themes
                | Prompt::ConfirmDelete { .. }
                | Prompt::RecentRoots
                | Prompt::BranchMenu { .. }
        )
    }
    pub(crate) fn field_cursor(modal: &Modal) -> Cursor {
        let field = usize::from(modal.secondary_focus);
        let text = if modal.secondary_focus {
            &modal.secondary
        } else {
            &modal.input
        };
        modal.editors[field].unwrap_or_else(|| {
            Cursor::at(
                if field == 0 { modal.caret } else { None },
                field == 0 && modal.selected,
                text,
            )
        })
    }
    pub(crate) fn field_key(&mut self, key: KeyEvent) -> bool {
        let Some(modal) = self.modal.as_ref() else {
            return false;
        };
        if !Self::editable_prompt(&modal.kind) || modal.capture.is_some() {
            return false;
        }
        let plain = key.modifiers.is_empty();
        let recent = matches!(modal.kind, Prompt::Navigation)
            && self
                .session
                .state()
                .navigation()
                .is_some_and(|n| n.scope == code_core::navigation::Scope::Recent);
        if plain
            && modal.input.is_empty()
            && (key.code == KeyCode::Right && modal.kind == Prompt::Branches
                || key.code == KeyCode::Delete
                    && (recent || matches!(modal.kind, Prompt::Places(_) | Prompt::Keymap)))
        {
            return false;
        }
        let Some(command) = lenz_terminal_ui::command(key) else {
            return false;
        };
        if command == FieldCommand::Paste {
            self.dispatch(Command::Notice("Use terminal Paste (usually Ctrl+Shift+V); bracketed paste edits the focused field".into()));
            return true;
        }
        self.edit_field(command);
        true
    }
    pub(crate) fn edit_field(&mut self, command: FieldCommand) {
        let command = command.first_line();
        let Some(modal) = self.modal.as_mut() else {
            return;
        };
        if !Self::editable_prompt(&modal.kind) || modal.capture.is_some() {
            return;
        }
        let cursor = Self::field_cursor(modal);
        let index = usize::from(modal.secondary_focus);
        let text = if modal.secondary_focus {
            &mut modal.secondary
        } else {
            &mut modal.input
        };
        let editor = modal.editors[index].get_or_insert(cursor);
        let outcome = editor.execute(text, command);
        modal.caret = editor.caret;
        modal.selected = editor.all_selected(text);
        if outcome.changed {
            self.prompt_changed();
        }
        if let Some(copy) = outcome.copy {
            self.dispatch(Command::CopyText(copy));
        }
    }
    pub(crate) fn prompt_changed(&mut self) {
        let Some(modal) = self.modal.as_mut() else {
            return;
        };
        modal.at = 0;
        modal.top = 0;
        modal.reveal_selection = true;
        modal.error = None;
        let (kind, query, replacement, replacing) = (
            modal.kind.clone(),
            modal.input.clone(),
            modal.secondary.clone(),
            modal.secondary_focus,
        );
        match kind {
            Prompt::Navigation => {
                if let Some(search) = self.session.state().navigation() {
                    self.dispatch(Command::SearchEverywhere {
                        scope: search.scope,
                        query,
                    });
                }
            }
            Prompt::Find if replacing => self.dispatch(Command::SetReplaceText(replacement)),
            Prompt::Find => self.dispatch(Command::Find(query)),
            Prompt::Declarations => self.dispatch(Command::FindDeclarations(query)),
            Prompt::Symbols => self.dispatch(Command::Symbols(query)),
            _ => {}
        }
    }
    pub(crate) fn type_field(&mut self, text: &str, backspace: bool) {
        if text.is_empty() && !backspace {
            self.prompt_changed();
        } else {
            self.edit_field(if backspace {
                FieldCommand::Edit {
                    edit: Edit::Backspace,
                    extend: false,
                }
            } else {
                FieldCommand::Insert(text.into())
            });
        }
    }
}
