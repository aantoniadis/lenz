//! GUI input and presentation for shared contextual file commands.
use super::*;
use crate::file_actions::FileAction;
impl Viewer {
    pub(super) fn file_menu(&self, index: usize) -> Vec<(String, MenuAction)> {
        self.file_menu_actions(index, FileAction::ALL)
    }
    fn file_menu_actions(&self, index: usize, actions: &[FileAction]) -> Vec<(String, MenuAction)> {
        actions
            .iter()
            .map(|&action| {
                let why = if action == FileAction::UndoLenz {
                    self.comparison.markdown_edit_disabled()
                } else {
                    None
                }
                .map(str::to_string)
                .or_else(|| {
                    self.file_disabled(action,index)
                });
                let label = why.map_or_else(
                    || action.label().into(),
                    |why| format!("{} ({why})", action.label()),
                );
                (label, MenuAction::File(action, index, self.set.clone()))
            })
            .collect()
    }
    pub(super) fn file_action(
        &mut self,
        action: FileAction,
        index: usize,
        set: &Arc<ChangeSet>,
        cx: &mut Context<Self>,
    ) {
        if !Arc::ptr_eq(set, &self.set) {
            self.toast("Files changed — reopen the menu".into(), cx);
            return;
        }
        if let Some(reason) = self.file_disabled(action,index) {
            self.toast(reason, cx);
            return;
        }
        let Some(c) = set.comparisons.get(index) else {
            return;
        };
        match action {
            FileAction::CopyPath => {
                cx.write_to_clipboard(ClipboardItem::new_string(c.path.clone()))
            }
            FileAction::Editor => self.open_file(
                index,
                if c.status == Status::Deleted {
                    Side::Left
                } else {
                    Side::Right
                },
                None,
                None,
                false,
                cx,
            ),
            FileAction::History => {
                let path = c.path.clone();
                if self.repo.is_none()
                    && let Some(root) = set.git_root()
                {
                    let session = crate::repository_session::RepositorySession::open_loaded(root.into(), true, None, None, self.set.clone(), self.spec.clone());
                    self.with_session(session, cx);
                }
                self.show_file_history(path, cx);
            }
            FileAction::UndoLenz => {
                if index == self.comparison.current && self.doc().is_some() {
                    self.edit_current(crate::edit_session::EditCommand::Undo, cx);
                } else {
                    match self.comparison.undo_file(&mut self.edits, set, index) {
                        Ok(written) => self.install_file_undo(index,written,cx),
                        Err(error) => self.toast(error.to_string(), cx),
                    }
                }
            }
            FileAction::Discard => match self.file_actions.request(set, index, &self.spec) {
                Ok(()) => self.poll_file_actions(cx),
                Err(error) => self.toast(error, cx),
            },
            FileAction::UndoDiscard => match self.file_actions.undo(set, index) {
                Ok(()) => self.poll_file_actions(cx),
                Err(error) => self.toast(error, cx),
            },
        }
        cx.notify();
    }
    pub(super) fn file_changed(&mut self, path: std::path::PathBuf, cx: &mut Context<Self>) {
        let mut current_changed = false;
        for (index, c) in self.set.comparisons.iter().enumerate() {
            if self
                .set
                .git_root()
                .is_some_and(|root| root.join(&c.path) == path)
                || [&c.left, &c.right]
                    .iter()
                    .any(|source| matches!(source,Source::File(p)if *p==path))
            {
                current_changed |= index == self.comparison.current;
                self.loaded.remove(&index);
                self.comparison.cache.0.remove(&index);
            }
        }
        if current_changed {
            self.sel = None;
            self.comparison.find = None;
            self.load(
                self.comparison.current,
                Place::Anchor(self.comparison.view.anchor_line()),
                false,
                cx,
            );
        }
        self.refresh_repository(cx);
        self.toast("File updated".into(), cx);
    }
    pub(super) fn poll_file_actions(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                let done = this
                    .update(cx, |viewer, cx| {
                        if let Some(result) = viewer.file_actions.poll() {
                            match result {
                                Ok(Some(path)) => viewer.file_changed(path, cx),
                                Ok(None) => {}
                                Err(error) => viewer.toast(format!("Nothing written: {error}"), cx),
                            }
                            cx.notify();
                        }
                        !viewer.file_actions.busy()
                    })
                    .unwrap_or(true);
                if done {
                    break;
                }
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(20))
                    .await;
            }
        })
        .detach();
    }
    pub(super) fn discard_modal(&self) -> bool {
        self.file_actions.busy() || self.file_actions.confirmation().is_some()
    }
    pub(super) fn confirm_discard(&mut self, cx: &mut Context<Self>) {
        if self.file_actions.pending_path().is_some_and(|path|self.edits.dirty_path(path)) {
            self.file_actions.cancel();
            self.toast("Save or discard unsaved changes first".into(),cx);
            return;
        }
        if let Err(error) = self.file_actions.confirm() {
            self.toast(error, cx);
        } else {
            self.poll_file_actions(cx);
        }
        cx.notify();
    }
    pub(super) fn discard_overlay(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let question = self.file_actions.confirmation()?;
        let p = &self.palette;
        Some(
            div()
                .absolute()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .id("discard-confirmation")
                        .w(px(560.))
                        .max_w_full()
                        .max_h_full()
                        .overflow_y_scroll()
                        .p_4()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .bg(rgb(p.status_bg))
                        .border_1()
                        .border_color(rgb(p.border))
                        .text_color(rgb(p.fg))
                        .child(question)
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .gap_3()
                                .child(
                                    div()
                                        .id("discard-cancel")
                                        .p_2()
                                        .child("Cancel (Esc)")
                                        .on_mouse_down(
                                            MouseButton::Left,
                                            cx.listener(|v, _, _, cx| {
                                                v.file_actions.cancel();
                                                cx.notify();
                                                cx.stop_propagation();
                                            }),
                                        ),
                                )
                                .child(
                                    div()
                                        .id("discard-confirm")
                                        .p_2()
                                        .bg(rgb(p.selected_bg))
                                        .child("Discard (Enter)")
                                        .on_mouse_down(
                                            MouseButton::Left,
                                            cx.listener(|v, _, _, cx| {
                                                v.confirm_discard(cx);
                                                cx.stop_propagation();
                                            }),
                                        ),
                                ),
                        ),
                )
                .into_any_element(),
        )
    }
}
