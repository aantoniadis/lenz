//! Git in the Repository Window: the sidebar menu's entries (the core decides them), the New Branch and Checkout Tag
//! or Revision prompts, and the checkout dialog.

use super::*;
use crate::repository_session::{RepositoryCommand, SourceEntry};
use lenz_content::branches::CheckoutChoice;

/// A Git prompt's text so far.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum GitPrompt {
    /// A new branch at `from` (HEAD when `None`), checked out when `checkout` (Tab).
    NewBranch { from: Option<String>, name: String, checkout: bool, error: Option<String> },
    Revision { input: String, error: Option<String> },
}

impl Viewer {
    /// The open Git prompt's line, drawn in the Go to line box.
    pub(super) fn git_prompt_text(&self) -> Option<String> {
        Some(match self.git_prompt.as_ref()? {
            GitPrompt::NewBranch { from, name, checkout, error } => {
                let start = from.as_deref().map_or("HEAD", |r| ["refs/heads/", "refs/remotes/", "refs/tags/"].iter().find_map(|p| r.strip_prefix(p)).unwrap_or(r));
                let mark = if *checkout { "x" } else { " " };
                let hint = error.clone().unwrap_or_else(|| "Enter".into());
                format!("New branch from {start} [{mark}] check out (Tab): {name}▏   {hint}")
            }
            GitPrompt::Revision { input, error } => {
                let hint = error.clone().unwrap_or_else(|| "a tag, branch, commit id or HEAD~2 — Enter".into());
                format!("Checkout tag or revision: {input}▏   {hint}")
            }
        })
    }

    /// Keys while a Git prompt is open; returns whether it handled the key.
    pub(super) fn git_prompt_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        let Some(prompt) = self.git_prompt.as_mut() else { return false };
        let ks = &ev.keystroke;
        let paste = (ks.modifiers.control || ks.modifiers.platform) && ks.key == "v";
        let pasted = paste.then(|| cx.read_from_clipboard().and_then(|c| c.text())).flatten().map(|t| t.lines().next().unwrap_or_default().trim().to_string());
        let (text, error) = match prompt {
            GitPrompt::NewBranch { name, error, .. } => (name, error),
            GitPrompt::Revision { input, error } => (input, error),
        };
        match ks.key.as_str() {
            "escape" => self.git_prompt = None,
            "backspace" => {
                text.pop();
                *error = None;
            }
            "tab" => {
                if let GitPrompt::NewBranch { checkout, .. } = prompt {
                    *checkout = !*checkout;
                }
            }
            "enter" => self.git_prompt_enter(cx),
            _ => {
                if let Some(t) = pasted.or_else(|| Self::typed(ev).filter(|_| !paste)) {
                    text.push_str(&t);
                    *error = None;
                }
            }
        }
        cx.notify();
        true
    }

    /// Enter in a Git prompt: a refusal stays in the prompt; otherwise the prompt closes and the command runs.
    fn git_prompt_enter(&mut self, cx: &mut Context<Self>) {
        let (Some(prompt), Some(r)) = (self.git_prompt.clone(), self.repo.as_ref()) else { return };
        let (command, refused) = match prompt {
            GitPrompt::NewBranch { from, name, checkout, .. } => match r.validate_branch_name(&name) {
                Ok(()) => (Some(RepositoryCommand::NewBranch { name, from, checkout }), None),
                Err(e) => (None, Some(e)),
            },
            GitPrompt::Revision { input, .. } => match r.validate_revision(&input) {
                Ok(_) => (Some(RepositoryCommand::Checkout(lenz_content::branches::Target::Detached(input.trim().to_string()))), None),
                Err(e) => (None, Some(e)),
            },
        };
        if let Some(e) = refused {
            if let Some(GitPrompt::NewBranch { error, .. } | GitPrompt::Revision { error, .. }) = self.git_prompt.as_mut() {
                *error = Some(e);
            }
            return;
        }
        self.git_prompt = None;
        if let Some(command) = command {
            self.git_command(command, cx);
        }
    }

    /// A sidebar menu entry.
    pub(super) fn source_entry(&mut self, entry: SourceEntry, window: &mut Window, cx: &mut Context<Self>) {
        match entry {
            SourceEntry::Command(command) => self.git_command(command, cx),
            SourceEntry::Run(action) => self.run_action(action, window, cx),
            SourceEntry::NewBranchFrom(from) => self.git_prompt = Some(GitPrompt::NewBranch { from: Some(from), name: String::new(), checkout: true, error: None }),
        }
        cx.notify();
    }

    /// A repository command; one that rewrites the working tree waits until no draft is unsaved.
    pub(super) fn git_command(&mut self, command: RepositoryCommand, cx: &mut Context<Self>) {
        let writes = matches!(&command, RepositoryCommand::Checkout(_) | RepositoryCommand::NewBranch { checkout: true, .. }
            | RepositoryCommand::CheckoutChoice(CheckoutChoice::Smart | CheckoutChoice::ConfirmForce));
        if writes && !self.edits_may_close(cx) {
            return;
        }
        self.repository_command(command, cx);
    }

    /// Whether the checkout dialog is open.
    pub(super) fn checkout_modal(&self) -> bool {
        self.repo.as_ref().is_some_and(|r| r.checkout_problem().is_some())
    }

    /// Keys while the checkout dialog is open: s Smart, f Force (then y confirms), Escape cancels.
    pub(super) fn checkout_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        let Some(choices) = self.repo.as_ref().and_then(|r| r.checkout_problem()).map(|p| p.choices()) else { return false };
        let has = |c| choices.iter().any(|(_, x)| *x == c);
        let choice = match ev.keystroke.key.as_str() {
            "escape" => Some(CheckoutChoice::Cancel),
            "s" if has(CheckoutChoice::Smart) => Some(CheckoutChoice::Smart),
            // Held keys and repeats never reach the confirmation.
            "f" if has(CheckoutChoice::Force) && !ev.is_held => Some(CheckoutChoice::Force),
            "y" if has(CheckoutChoice::ConfirmForce) && !ev.is_held => Some(CheckoutChoice::ConfirmForce),
            _ => None,
        };
        if let Some(choice) = choice {
            self.git_command(RepositoryCommand::CheckoutChoice(choice), cx);
        }
        cx.notify();
        true
    }

    /// The checkout dialog: what git refused over, and the choices that are safe.
    pub(super) fn checkout_overlay(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let problem = self.repo.as_ref()?.checkout_problem()?;
        let p = &self.palette;
        let buttons = problem.choices().into_iter().map(|(label, choice)| {
            div()
                .id(label)
                .px_3()
                .py_1()
                .border_1()
                .border_color(rgb(p.border))
                .cursor_pointer()
                .child(label)
                .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _, cx| {
                    this.git_command(RepositoryCommand::CheckoutChoice(choice), cx);
                    cx.stop_propagation();
                    cx.notify();
                }))
        });
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
                        .id("checkout-problem")
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
                        .child(problem.text())
                        .child(div().flex().flex_row().flex_wrap().gap_3().children(buttons)),
                )
                .into_any_element(),
        )
    }
}
