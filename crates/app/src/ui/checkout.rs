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

#[cfg(all(test, feature = "gui-test"))]
mod gui_tests {
    use super::*;
    use super::super::merge_view::gui_tests::event;
    use gpui::{AppContext, Entity, TestAppContext};
    fn git(root: &std::path::Path, args: &[&str]) -> String {
        let output = std::process::Command::new("git").env("GIT_CONFIG_GLOBAL", "/dev/null").env("GIT_CONFIG_NOSYSTEM", "1")
            .args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"])
            .args(args).current_dir(root).output().unwrap();
        assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
        String::from_utf8_lossy(&output.stdout).trim().into()
    }
    /// A repository window on `main` (`a.txt`, `m.txt`), branch `other` changing `a.txt`, tag `v1`; `m.txt` changed
    /// locally and shown.
    fn fixture(cx: &mut TestAppContext) -> (tempfile::TempDir, Entity<Viewer>, gpui::WindowHandle<Viewer>) {
        cx.skip_drawing();
        let dir = tempfile::Builder::new().prefix(".lenz-gui-checkout-").tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let root = dir.path();
        git(root, &["init", "-q", "-b", "main"]);
        git(root, &["config", "user.name", "t"]);
        git(root, &["config", "user.email", "t@t"]);
        std::fs::write(root.join("a.txt"), "main\n").unwrap();
        std::fs::write(root.join("m.txt"), "base\n".repeat(5)).unwrap();
        git(root, &["add", "."]);
        git(root, &["commit", "-qm", "main"]);
        git(root, &["tag", "v1"]);
        git(root, &["switch", "-q", "-c", "other"]);
        std::fs::write(root.join("a.txt"), "other\n").unwrap();
        git(root, &["commit", "-qam", "other"]);
        git(root, &["switch", "-q", "main"]);
        std::fs::write(root.join("m.txt"), "changed\n".repeat(5)).unwrap();
        let spec = crate::repository_session::local_spec();
        let set = Arc::new(crate::repo::local_changes(root, false).unwrap());
        let doc = Arc::new(Doc::load(&set, 0, &root.join("m.txt"), &root.join("m.txt"), false).unwrap());
        let view = doc.view(&doc.line_diff(&Options::default()), 3.0);
        let mut picker = crate::repository_session::Picker::new(root.into(), true);
        picker.sources = crate::repository_session::Sources::read_repository(root);
        picker.refs = crate::history::refs(root).ok();
        let viewer = cx.new(|cx| {
            let mut v = Viewer::new(set, spec, false, None, doc, view, false, Options::default(), gpui::font("monospace"), Instant::now(), false, cx);
            v.repo = Some(crate::repository_session::RepositorySession::with_picker(picker));
            v
        });
        let window = cx.update(|cx| cx.open_window(Default::default(), |_, _| viewer.clone())).unwrap();
        (dir, viewer, window)
    }
    fn pump(cx: &mut TestAppContext, viewer: &Entity<Viewer>, done: impl Fn(&Viewer) -> bool) {
        let started = Instant::now();
        loop {
            cx.executor().run_until_parked();
            cx.executor().advance_clock(std::time::Duration::from_millis(35));
            if viewer.read_with(cx, |v, _| done(v)) { break; }
            assert!(started.elapsed().as_secs() < 15, "GUI did not settle");
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }
    fn keys(cx: &mut TestAppContext, window: &gpui::WindowHandle<Viewer>, keys: &[&str]) {
        for k in keys {
            window.update(cx, |v, w, cx| v.on_key(&event(k, false, false, false), w, cx)).unwrap();
        }
    }
    fn branch(root: &std::path::Path) -> String {
        git(root, &["branch", "--show-current"])
    }
    fn done(v: &Viewer) -> bool {
        v.repo.as_ref().is_some_and(|r| !r.checking_out() && !r.busy())
    }

    #[test]
    fn gui_the_sidebar_menu_checks_out_a_branch() {
        let mut cx = TestAppContext::single();
        let (dir, viewer, window) = fixture(&mut cx);
        // Right-click on `other`'s sidebar row.
        let (x, y) = viewer.read_with(&cx, |v, _| {
            let p = &v.repo.as_ref().unwrap().picker;
            let rows = p.sources.rows(&v.palette, &p.branches);
            let row = rows.iter().position(|r| matches!(&r.action, Some(crate::repository_session::SourceAction::Branch(n, _)) if n == "refs/heads/other")).unwrap();
            (10.0, toolbar_h() + line_h() * (row as f32 + 1.5 - p.sources.scroll))
        });
        viewer.update(&mut cx, |v, cx| assert!(v.picker_menu(x, y, cx)));
        let labels = viewer.read_with(&cx, |v, _| v.menu.as_ref().unwrap().items.iter().map(|i| i.0.clone()).collect::<Vec<_>>());
        assert_eq!(labels, ["Checkout", "New Branch from Here…", "Compare with Current"]);
        keys(&mut cx, &window, &["enter"]);
        pump(&mut cx, &viewer, done);
        assert_eq!(branch(dir.path()), "other");
        assert_eq!(std::fs::read_to_string(dir.path().join("a.txt")).unwrap(), "other\n");
        assert_eq!(std::fs::read_to_string(dir.path().join("m.txt")).unwrap(), "changed\n".repeat(5), "the local change went along");
        viewer.read_with(&cx, |v, _| {
            assert_eq!(v.repo.as_ref().unwrap().picker.title_branch, "other");
            assert_eq!(v.toast.as_ref().map(|t| t.0.as_str()), Some("Checked out other"));
        });
    }

    #[test]
    fn gui_the_checkout_dialog_offers_smart_and_a_confirmed_force() {
        let mut cx = TestAppContext::single();
        let (dir, viewer, window) = fixture(&mut cx);
        std::fs::write(dir.path().join("a.txt"), "mine\n").unwrap();
        viewer.update(&mut cx, |v, cx| v.git_command(RepositoryCommand::Checkout(lenz_content::branches::Target::Branch("other".into())), cx));
        pump(&mut cx, &viewer, |v| v.modal() == Some(Modal::Checkout));
        // Force: a second step lists what is discarded; Enter and a held f do nothing there; y confirms.
        keys(&mut cx, &window, &["f"]);
        pump(&mut cx, &viewer, |v| v.repo.as_ref().and_then(|r| r.checkout_problem()).is_some_and(|p| p.discards.is_some()));
        keys(&mut cx, &window, &["enter", "f", "s"]);
        assert_eq!(viewer.read_with(&cx, |v, _| v.repo.as_ref().unwrap().checkout_problem().unwrap().choices().len()), 2);
        assert_eq!(std::fs::read_to_string(dir.path().join("a.txt")).unwrap(), "mine\n");
        keys(&mut cx, &window, &["escape"]);
        assert!(viewer.read_with(&cx, |v, _| v.modal().is_none()));
        // Again, then y.
        viewer.update(&mut cx, |v, cx| v.git_command(RepositoryCommand::Checkout(lenz_content::branches::Target::Branch("other".into())), cx));
        pump(&mut cx, &viewer, |v| v.modal() == Some(Modal::Checkout));
        keys(&mut cx, &window, &["f"]);
        keys(&mut cx, &window, &["y"]);
        pump(&mut cx, &viewer, done);
        assert_eq!(branch(dir.path()), "other");
        assert_eq!(std::fs::read_to_string(dir.path().join("a.txt")).unwrap(), "other\n");
    }

    #[test]
    fn gui_new_branch_and_revision_prompts_refuse_in_place() {
        let mut cx = TestAppContext::single();
        let (dir, viewer, window) = fixture(&mut cx);
        window.update(&mut cx, |v, w, cx| v.run_action(crate::catalog::Action::Repository(crate::catalog::RepoAction::NewBranch), w, cx)).unwrap();
        assert_eq!(viewer.read_with(&cx, |v, _| v.modal()), Some(Modal::GitPrompt));
        keys(&mut cx, &window, &["o", "t", "h", "e", "r", "enter"]);
        assert!(viewer.read_with(&cx, |v, _| v.goto_prompt()).unwrap().ends_with("A branch named \"other\" already exists"));
        cx.write_to_clipboard(gpui::ClipboardItem::new_string("-x\nmore".into()));
        window.update(&mut cx, |v, w, cx| v.on_key(&event("v", true, false, false), w, cx)).unwrap();
        assert!(viewer.read_with(&cx, |v, _| v.goto_prompt()).unwrap().contains(": other-x▏"));
        keys(&mut cx, &window, &["tab", "enter"]);
        pump(&mut cx, &viewer, done);
        assert_eq!(branch(dir.path()), "main", "Check out was off");
        assert_eq!(git(dir.path(), &["rev-parse", "other-x"]), git(dir.path(), &["rev-parse", "main"]));
        window.update(&mut cx, |v, w, cx| v.run_action(crate::catalog::Action::Repository(crate::catalog::RepoAction::CheckoutRevision), w, cx)).unwrap();
        keys(&mut cx, &window, &["n", "o", "p", "e", "enter"]);
        assert!(viewer.read_with(&cx, |v, _| v.goto_prompt()).unwrap().ends_with("No commit named \"nope\""));
        keys(&mut cx, &window, &["escape"]);
        assert!(viewer.read_with(&cx, |v, _| v.modal().is_none()));
        window.update(&mut cx, |v, w, cx| v.run_action(crate::catalog::Action::Repository(crate::catalog::RepoAction::CheckoutRevision), w, cx)).unwrap();
        keys(&mut cx, &window, &["v", "1", "enter"]);
        pump(&mut cx, &viewer, done);
        assert_eq!(branch(dir.path()), "");
    }

    #[test]
    fn gui_an_unsaved_draft_blocks_a_checkout() {
        let mut cx = TestAppContext::single();
        let (dir, viewer, window) = fixture(&mut cx);
        viewer.update(&mut cx, |v, _| {
            v.repo.as_mut().unwrap().picker.focus = crate::repository_session::Pane::Diff;
            v.nav_focused = false;
            v.panel_focused = false;
        });
        window.update(&mut cx, |v, w, cx| v.on_key(&event("right", false, true, true), w, cx)).unwrap();
        assert!(viewer.read_with(&cx, |v, _| v.edits.has_dirty()), "a draft");
        viewer.update(&mut cx, |v, cx| v.git_command(RepositoryCommand::Checkout(lenz_content::branches::Target::Branch("other".into())), cx));
        pump(&mut cx, &viewer, done);
        assert_eq!(branch(dir.path()), "main");
        assert_eq!(std::fs::read_to_string(dir.path().join("a.txt")).unwrap(), "main\n");
        assert!(viewer.read_with(&cx, |v, _| v.edits.has_dirty() && v.toast.as_ref().is_some_and(|t| t.0.starts_with("Unsaved changes"))));
    }
}
