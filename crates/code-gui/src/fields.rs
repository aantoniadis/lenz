//! Popup text is edited by the shared field model; this module routes results back to the existing workflows.
use super::*;
use lenz_content::field_edit::Cursor;
use std::hash::{Hash, Hasher};

impl ReaderView {
    pub(super) fn popup_field_key(&self) -> Option<u64> {
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        std::mem::discriminant(self.modal.as_ref()?).hash(&mut hash);
        Some(hash.finish())
    }
    pub(super) fn popup_field(&self) -> Option<(&str, Cursor, usize)> {
        let m = self.modal.as_ref()?;
        let (text, caret, selected, field) = match m {
            Modal::OpenPath { input, .. } => return Some((input.text(), input.cursor(), 0)),
            Modal::Navigation { caret, .. } => (
                self.session.state().navigation()?.query.as_str(),
                *caret,
                false,
                0,
            ),
            Modal::Search {
                query,
                files,
                in_files,
                caret,
                selected,
                ..
            } => (
                if *in_files {
                    files.as_str()
                } else {
                    query.as_str()
                },
                *caret,
                *selected,
                usize::from(*in_files),
            ),
            Modal::SaveAs {
                input, selected, ..
            }
            | Modal::NewScratch {
                input, selected, ..
            }
            | Modal::RenameTerminal {
                input, selected, ..
            } => (input.as_str(), None, *selected, 0),
            Modal::PasteEntry { name, selected, .. } => (name.as_str(), None, *selected, 0),
            Modal::Create { name, .. } | Modal::NewBranch { name, .. } => {
                (name.as_str(), None, false, 0)
            }
            Modal::Revision { input, .. }
            | Modal::GoTo { input, .. }
            | Modal::Relocate { input, .. } => (input.as_str(), None, false, 0),
            Modal::Root { path }
            | Modal::SqlMetadata { path }
            | Modal::ServerRuntime { path, .. } => (path.as_str(), None, false, 0),
            Modal::Branches { query, .. }
            | Modal::Places { query, .. }
            | Modal::Siblings { query, .. }
            | Modal::Declarations { query, .. } => (query.as_str(), None, false, 0),
            _ => return None,
        };
        let initial = Cursor::at(caret, selected, text);
        let cursor = self
            .popup_fields
            .cursor(self.popup_field_key()?, field)
            .unwrap_or(initial);
        Some((text, cursor, field))
    }
    pub(super) fn popup_text_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        let Some((text, cursor, field)) = self.popup_field() else {
            return false;
        };
        let k = &ev.keystroke;
        // Row actions remain accessible with an empty filter; with text present Delete/Right edit that text.
        let row_action = text.is_empty()
            && !k.modifiers.control
            && !k.modifiers.platform
            && !k.modifiers.alt
            && !k.modifiers.shift
            && (k.key == "right" && matches!(self.modal, Some(Modal::Branches { .. }))
                || k.key == "delete"
                    && matches!(
                        self.modal,
                        Some(Modal::Places { .. } | Modal::Navigation { .. })
                    ));
        if row_action {
            return false;
        }
        let key = self.popup_field_key().unwrap_or_default();
        let Some(command) = lenz_gpui::field::command(ev, cx) else {
            return false;
        };
        let command = command.first_line();
        let outcome;
        if matches!(self.modal, Some(Modal::Navigation { .. })) {
            let Some(search) = self.session.state().navigation() else {
                return true;
            };
            let (scope, mut query) = (search.scope, search.query.clone());
            outcome = self
                .popup_fields
                .edit(key, field, cursor, &mut query, command);
            let caret = self.popup_fields.cursor(key, field).and_then(|c| c.caret);
            if outcome.changed {
                self.update_navigation(scope, query, caret, cx);
            } else if let Some(Modal::Navigation { caret: at, .. }) = &mut self.modal {
                *at = caret;
            }
        } else if let Some(Modal::OpenPath { input, error, .. }) = &mut self.modal {
            outcome = input.command(command);
            if outcome.changed {
                *error = None;
            }
        } else {
            let Some(modal) = self.modal.as_mut() else {
                return false;
            };
            let text = match modal {
                Modal::Search {
                    query,
                    files,
                    in_files,
                    ..
                } => {
                    if *in_files {
                        files
                    } else {
                        query
                    }
                }
                Modal::SaveAs { input, .. }
                | Modal::NewScratch { input, .. }
                | Modal::RenameTerminal { input, .. }
                | Modal::Revision { input, .. }
                | Modal::GoTo { input, .. }
                | Modal::Relocate { input, .. } => input,
                Modal::PasteEntry { name, .. }
                | Modal::Create { name, .. }
                | Modal::NewBranch { name, .. } => name,
                Modal::Root { path }
                | Modal::SqlMetadata { path }
                | Modal::ServerRuntime { path, .. } => path,
                Modal::Branches { query, .. }
                | Modal::Places { query, .. }
                | Modal::Siblings { query, .. }
                | Modal::Declarations { query, .. } => query,
                _ => return false,
            };
            outcome = self.popup_fields.edit(key, field, cursor, text, command);
            let cursor = self.popup_fields.cursor(key, field).unwrap_or_default();
            let all_selected = cursor.all_selected(text);
            match modal {
                Modal::Search {
                    selected, caret, ..
                } => {
                    *selected = all_selected;
                    *caret = cursor.caret;
                }
                Modal::SaveAs { selected, .. }
                | Modal::NewScratch { selected, .. }
                | Modal::RenameTerminal { selected, .. }
                | Modal::PasteEntry { selected, .. } => *selected = all_selected,
                _ => {}
            }
            if outcome.changed {
                match modal {
                    Modal::Search { .. } => {}
                    Modal::SaveAs { error, .. }
                    | Modal::NewScratch { error, .. }
                    | Modal::RenameTerminal { error, .. }
                    | Modal::PasteEntry { error, .. }
                    | Modal::Revision { error, .. }
                    | Modal::GoTo { error, .. }
                    | Modal::Relocate { error, .. }
                    | Modal::Create { error, .. }
                    | Modal::NewBranch { error, .. } => *error = None,
                    Modal::ServerRuntime { at, .. }
                    | Modal::Branches { at, .. }
                    | Modal::Places { at, .. }
                    | Modal::Siblings { at, .. } => *at = 0,
                    Modal::Declarations { query, semantic } => {
                        let command = if *semantic {
                            Command::Symbols(query.clone())
                        } else {
                            Command::FindDeclarations(query.clone())
                        };
                        self.results_scroll = 0;
                        self.dispatch(command, cx);
                    }
                    _ => {}
                }
            }
        }
        if let Some(text) = outcome.copy {
            self.dispatch(Command::CopyText(text), cx);
        }
        cx.notify();
        true
    }
    pub(super) fn field_find_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        let Some(command) = lenz_gpui::field::command(ev, cx) else {
            return false;
        };
        let command = command.first_line();
        let Some(find) = self.session.state().active().and_then(|t| t.find()) else {
            return false;
        };
        let mut text = if self.find_replace {
            self.session.replace_text().to_string()
        } else {
            find.query().to_string()
        };
        let field = usize::from(self.find_replace);
        let result = self
            .find_fields
            .edit(0, field, Cursor::default(), &mut text, command);
        if let Some(value) = result.copy {
            self.dispatch(Command::CopyText(value), cx);
        }
        if result.changed {
            self.dispatch(
                if self.find_replace {
                    Command::SetReplaceText(text)
                } else {
                    Command::Find(text)
                },
                cx,
            );
        }
        cx.notify();
        true
    }
}
