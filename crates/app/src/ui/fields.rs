//! Route focused Diff text fields through the same editor as the reader's popup fields.
use super::*;
use crate::field_edit::{Cursor, Snapshot};

impl Viewer {
    pub(super) fn field_input_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        let modal = self.modal();
        if self.field_input_modal != modal {
            self.field_inputs.clear();
            self.field_input_modal = modal;
        }
        if modal.is_none()
            && !self.nav_focused
            && self.repo.as_ref().is_some_and(|r| {
                r.picker.shown && r.picker.focus == Pane::Log && r.picker.filter_editing
            })
        {
            let Some(command) = lenz_gpui::field::command(event, cx) else {
                return false;
            };
            let Some(repo) = self.repo.as_ref() else {
                return false;
            };
            let mut query = repo.picker.filter.clone();
            let out = self
                .field_inputs
                .edit(u64::MAX, 0, Cursor::default(), &mut query, command);
            if out.changed {
                self.repository_command(
                    crate::repository_session::RepositoryCommand::Filter(query),
                    cx,
                );
            }
            if let Some(text) = out.copy {
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
            }
            cx.notify();
            return true;
        }
        let Some(kind) = modal else { return false };
        if kind == Modal::Find
            && (self.nav_focused
                || self
                    .repo
                    .as_ref()
                    .is_some_and(|r| r.picker.shown && r.picker.focus != Pane::Diff))
        {
            return false;
        }
        if !matches!(
            kind,
            Modal::GitPrompt
                | Modal::GoTo
                | Modal::Switcher
                | Modal::Find
                | Modal::SearchEditing
                | Modal::MergeFind
        ) {
            return false;
        }
        let Some(mut command) = lenz_gpui::field::command(event, cx) else {
            return false;
        };
        if kind == Modal::GitPrompt
            && let crate::field_edit::Command::Insert(text) = &mut command
        {
            *text = text.lines().next().unwrap_or_default().to_owned();
        }
        let result;
        if kind == Modal::MergeFind {
            let Some(merge) = self.merge.as_mut() else {
                return false;
            };
            result = merge.edit_find(command);
        } else {
            let text = match kind {
                Modal::GoTo => self.goto.as_mut(),
                Modal::GitPrompt => self.git_prompt.as_mut().map(|p| match p {
                    checkout::GitPrompt::NewBranch { name, .. } => name,
                    checkout::GitPrompt::Revision { input, .. } => input,
                }),
                Modal::Switcher => self.switcher.as_mut().map(|s| &mut s.query),
                Modal::Find => self.comparison.find.as_mut().map(|f| &mut f.query),
                Modal::SearchEditing => self.search.as_mut().map(|s| &mut s.query),
                _ => None,
            };
            let Some(text) = text else { return false };
            result = self
                .field_inputs
                .edit(kind as u64, 0, Cursor::default(), text, command);
            if result.changed {
                match kind {
                    Modal::Switcher => self.update_switcher(),
                    Modal::Find => self.refind(),
                    Modal::SearchEditing => {
                        if let Some(s) = &mut self.search {
                            let q = s.query.clone();
                            s.execute(
                                crate::search_session::SearchCommand::Query(q),
                                &self.set,
                                PANEL_ROWS,
                            );
                        }
                    }
                    Modal::GitPrompt => {
                        if let Some(
                            checkout::GitPrompt::NewBranch { error, .. }
                            | checkout::GitPrompt::Revision { error, .. },
                        ) = &mut self.git_prompt
                        {
                            *error = None;
                        }
                    }
                    _ => {}
                }
            }
        }
        if let Some(text) = result.copy {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
        }
        cx.notify();
        true
    }
    pub(super) fn field_snapshot(&self, columns: usize) -> Option<Snapshot> {
        let kind = self.modal()?;
        let text = match kind {
            Modal::GoTo => self.goto.as_deref(),
            Modal::GitPrompt => self.git_prompt.as_ref().map(|p| match p {
                checkout::GitPrompt::NewBranch { name, .. } => name.as_str(),
                checkout::GitPrompt::Revision { input, .. } => input.as_str(),
            }),
            Modal::Switcher => self.switcher.as_ref().map(|s| s.query.as_str()),
            Modal::SearchEditing => self.search.as_ref().map(|s| s.query.as_str()),
            Modal::MergeFind => self.merge.as_ref().and_then(|m| m.find_query()),
            _ => None,
        }?;
        let cursor = if kind == Modal::MergeFind {
            self.merge.as_ref()?.find_cursor()
        } else {
            self.field_inputs.cursor(kind as u64, 0).unwrap_or_default()
        };
        Some(cursor.snapshot(text, columns))
    }
}
