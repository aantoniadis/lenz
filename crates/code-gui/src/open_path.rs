//! Native picker failures keep a typed-path route in the same window. Cancellation is not an error.
use super::*;
use code_core::{
    line_edit::{Edit, Input},
    open_path::Kind,
};

impl ReaderView {
    pub(super) fn prompt_open_path(
        &mut self,
        kind: Kind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_path_request = self.open_path_request.wrapping_add(1);
        let request = self.open_path_request;
        let answer = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: kind == Kind::File,
            directories: kind == Kind::Folder,
            multiple: false,
            prompt: Some(kind.title().into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = answer.await;
            let _ = this.update(cx, |v, cx| {
                if v.open_path_request != request {
                    return;
                }
                match result {
                    Ok(Ok(Some(paths))) => {
                        if let Some(path) = paths.into_iter().next() {
                            if kind == Kind::File {
                                v.open(path, cx);
                            } else {
                                v.open_root(path, cx);
                            }
                        }
                    }
                    Ok(Err(_)) | Err(_) if v.modal.is_none() => v.open_path_failed(kind, cx),
                    _ => {}
                }
            });
        })
        .detach();
    }
    pub(super) fn open_path_failed(&mut self, kind: Kind, cx: &mut Context<Self>) {
        self.modal = Some(Modal::OpenPath {
            kind,
            input: Input::default(),
            error: Some(
                "The system file dialog could not open. Type or paste a path below.".into(),
            ),
        });
        cx.notify();
    }
    pub(super) fn open_path_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        let Some(Modal::OpenPath { kind, input, error }) = &mut self.modal else {
            return false;
        };
        let key = ev.keystroke.key.as_str();
        let ctrl = ev.keystroke.modifiers.control || ev.keystroke.modifiers.platform;
        if key == "escape" {
            self.close_modal(cx);
            return true;
        }
        if key == "enter" {
            let (kind, text) = (*kind, input.text().to_string());
            let cwd = self
                .session
                .state()
                .project()
                .map(|p| p.root().to_path_buf())
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
            match kind.resolve(&text, &cwd) {
                Ok(path) => {
                    self.close_modal(cx);
                    if kind == Kind::File {
                        self.open(path, cx);
                    } else {
                        self.open_root(path, cx);
                    }
                }
                Err(why) => {
                    if let Some(Modal::OpenPath { error, .. }) = &mut self.modal {
                        *error = Some(why);
                    }
                }
            }
            cx.notify();
            return true;
        }
        if ctrl && key == "a" {
            input.select_all();
        } else if ctrl && matches!(key, "c" | "x") {
            if let Some(text) = input.selection() {
                cx.write_to_clipboard(ClipboardItem::new_string(text.into()));
                if key == "x" {
                    input.edit(Edit::Delete);
                }
            }
        } else if ctrl && key == "v" {
            if let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) {
                input.insert(text.lines().next().unwrap_or_default());
                *error = None;
            }
        } else if let Some(edit) = navigation::line_edit_key(ev).or(match key {
            "home" => Some(Edit::Start),
            "end" => Some(Edit::End),
            _ => None,
        }) {
            input.edit(edit);
            *error = None;
        } else if !ctrl
            && !ev.keystroke.modifiers.alt
            && let Some(text) = &ev.keystroke.key_char
        {
            input.insert(text);
            *error = None;
        }
        cx.notify();
        true
    }
}
