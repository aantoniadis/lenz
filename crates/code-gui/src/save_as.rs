//! In-window file paths and scratch names, followed by the core's validated creation and Replace decision.
use super::*;
use code_core::save::ReplaceChoice;

impl ReaderView {
    pub(super) fn prompt_save_as(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.session.state().active().map(|t| t.id()) else { return };
        let (dir, name) = self.session.save_as_start();
        let input = dir.join(name.as_deref().unwrap_or("")).display().to_string();
        let dir = self.session.state().project().map(|p| p.root().to_owned()).unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        self.modal_scroll.set(0);
        self.modal = Some(Modal::SaveAs { tab, dir, input, selected: true, error: None });
        cx.notify();
    }

    /// File prompts own all input; the core still validates and writes, including the captured Save As tab.
    pub(super) fn file_prompt_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        let Some(Modal::SaveAs { input, selected, error, .. } | Modal::NewScratch { input, selected, error } | Modal::RenameTerminal { input, selected, error, .. }) = &mut self.modal else { return false };
        let key = ev.keystroke.key.as_str();
        let ctrl = ev.keystroke.modifiers.control || ev.keystroke.modifiers.platform;
        if key == "escape" { self.modal = None; cx.notify(); return true; }
        if key == "enter" {
            let command = match &mut self.modal {
                Some(Modal::SaveAs { tab, dir, input, error, .. }) => {
                    if input.trim().is_empty() { *error = Some("Enter the new file's path".into()); None }
                    else {
                        let input = input.trim();
                        let path = input.strip_prefix("~/").and_then(|rest| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(rest))).unwrap_or_else(|| PathBuf::from(input));
                        Some(Command::SaveAs { tab: *tab, path: if path.is_absolute() { path } else { dir.join(path) } })
                    }
                }
                Some(Modal::RenameTerminal { id, input, error, .. }) => {
                    match self.session.terminal_rename_refusal(*id, input) {
                        Some(why) => { *error = Some(why); None }
                        None => Some(Command::Terminal(code_core::terminal::Command::Rename { id: *id, label: input.clone() })),
                    }
                }
                Some(Modal::NewScratch { input, error, .. }) => {
                    let name = input.trim();
                    match code_core::create::name_refusal(name) {
                        Some(why) => { *error = Some(why); None }
                        None => Some(Command::NewScratch(name.into())),
                    }
                }
                _ => None,
            };
            if let Some(command) = command { self.modal = None; self.dispatch(command, cx); }
        } else if ctrl && key == "a" { *selected = true; }
        else if ctrl && key == "u" { input.clear(); *selected = false; *error = None; }
        else if key == "backspace" {
            if *selected { input.clear(); } else { input.pop(); }
            *selected = false; *error = None;
        } else {
            let paste = (ctrl && key == "v").then(|| cx.read_from_clipboard().and_then(|c| c.text())).flatten();
            let typed = ev.keystroke.key_char.clone().filter(|t| !ctrl && !ev.keystroke.modifiers.alt && !t.chars().any(char::is_control));
            if let Some(text) = paste.or(typed) {
                if *selected { input.clear(); }
                input.push_str(&text); *selected = false; *error = None;
            }
        }
        cx.notify();
        true
    }

    pub(super) fn sync_replace_prompt(&mut self) {
        match (self.session.save_as_prompt().map(|p| p.text()), &mut self.modal) {
            (Some(now), Some(Modal::StatusDetails { kind: Details::Replace, text, .. })) => *text = now,
            (Some(text), None) => {
                self.modal_scroll.set(0);
                self.modal = Some(Modal::StatusDetails { title: "Replace existing file?", text, logo: None, kind: Details::Replace });
            }
            (None, Some(Modal::StatusDetails { kind: Details::Replace, .. })) => self.modal = None,
            _ => {}
        }
    }

    pub(super) fn choose_replace(&mut self, choice: ReplaceChoice, cx: &mut Context<Self>) {
        if let Some(prompt) = self.session.save_as_prompt() {
            self.dispatch(Command::ChooseReplace { ticket: prompt.ticket, choice }, cx);
        }
    }
}

impl ReaderView {
    pub(super) fn prompt_new_scratch(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if let Err(why) = self.session.scratch_files() { self.dispatch(Command::Notice(format!("Scratch files: {why}")), cx); return; }
        self.modal_scroll.set(0);
        self.modal = Some(Modal::NewScratch { input: "scratch.txt".into(), selected: true, error: None });
        cx.notify();
    }

    pub(super) fn prompt_scratch_files(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        match self.session.scratch_files() {
            Ok(files) => { self.modal_scroll.set(0); self.modal = Some(Modal::ScratchFiles { files, at: 0 }); cx.notify(); }
            Err(why) => self.dispatch(Command::Notice(format!("Scratch files: {why}")), cx),
        }
    }
}
