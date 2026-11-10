//! A preview and one explicitly started, cancellable install. Worker replies retain their original identity.
use super::*;
use semantics::installer;

impl Session {
    /// Missing executables/runtimes open setup directly; missing workspaces still open status.
    pub fn language_server_control(&self) -> (String, CommandId) {
        let Some(view) = self.state.provider() else { return (self.state.language_server_label(), CommandId::LanguageServer) };
        if view.root.is_some() && matches!(view.status, semantics::Status::Missing(_)) {
            return (format!("Install {}…", view.name), CommandId::InstallServer);
        }
        (view.status_label(), CommandId::LanguageServer)
    }

    pub fn language_server_initial_action(&self) -> usize {
        let action = self.language_server_control().1;
        let action = if action == CommandId::LanguageServer { CommandId::ServerDetails } else { action };
        self.language_server_rows().iter().position(|(_, id)| *id == Some(action)).unwrap_or(0)
    }
    pub(super) fn install_provider(&self) -> Option<&'static semantics::provider::Provider> {
        self.state.provider().and_then(|view| semantics::provider::PROVIDERS.iter().find(|p| p.name == view.name))
            .or_else(|| self.state.current_source()?.language().and_then(semantics::provider::for_language).map(|(_,p)|p))
    }

    pub(super) fn open_server_install(&mut self) -> Effect {
        if self.disabled(CommandId::InstallServer).is_some() { return Effect::default(); }
        let Some(doc) = self.state.current_source() else { return Effect::default() };
        let Some(provider) = self.install_provider() else { return Effect::default() };
        let home = self.state.semantics.settings_file().parent().unwrap_or(Path::new("/dev/null")).join("servers");
        self.install_request += 1;
        self.state.server_install = Some(installer::Install {
            request: self.install_request, provider: provider.id, source: doc.source().to_owned(), expected: self.state.semantics.settings().clone(),
            preview: installer::preview(provider, self.state.semantics.settings(), &home),
            running: false, finished: false,
            message: "Choose Start Installation (S or Enter) to download and set up this server for Lenz. Project files stay unchanged. Progress appears here; C cancels, D opens Server Details, and Escape closes and cancels. The exact command and destination are below. ShellCheck and shfmt are separate optional tools.".into(),
            output: String::new(), cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        });
        Effect { repaint: true, ..Effect::default() }
    }

    pub(super) fn start_server_install(&mut self) -> Effect {
        let repaint = Effect { repaint: true, ..Effect::default() };
        if self.state.runtime_saving.is_some() || self.state.provider_saves > 0 || self.install_running.is_some() { return repaint; }
        let source = self.state.current_source().map(|d| d.source().to_owned());
        let provider = self.install_provider().map(|p|p.id);
        let Some(model) = &mut self.state.server_install else { return Effect::default() };
        if source.as_ref() != Some(&model.source) || provider != Some(model.provider) {
            model.message = "The active file or its language provider changed. Close and reopen Install for that file.".into();
            return repaint;
        }
        if model.running || model.finished || model.preview.disabled.is_some() { return repaint; }
        let Some(plan) = model.preview.plan.clone() else { return repaint };
        let (request, cancel, expected) = (model.request, model.cancel.clone(), model.expected.clone());
        self.install_running = Some((request, model.provider, cancel.clone()));
        model.running = true; model.message = "Installation running… Cancel Installation (C) stops its process tree; Escape closes and cancels. Output is bounded; the editor remains responsive.".into();
        let (tx, file) = (self.tx.clone(), self.state.semantics.settings_file().to_owned());
        self.jobs.submit(Priority::Visible, move || {
            let mut kept = 0;
            let result = installer::run(&plan, &cancel, &mut |chunk| {
                let mut end = chunk.len().min(installer::OUTPUT_LIMIT.saturating_sub(kept));
                while !chunk.is_char_boundary(end) { end -= 1; }
                if end > 0 && let Some(chunk) = chunk.get(..end) {
                    let chunk = chunk.to_owned();
                    kept += end;
                    // Backpressure must never keep the installer from observing cancellation and killing its group.
                    let _ = tx.try_send(Completion::ServerInstallOutput { request, text: chunk });
                }
            }).and_then(|lock| {
                let result = installer::persist(&plan, &file, &expected, &cancel);
                drop(lock);
                result
            });
            let _ = tx.send_blocking(Completion::ServerInstalled { request, result });
        });
        repaint
    }

    pub(super) fn cancel_server_install(&mut self) -> Effect {
        if let Some(model) = &mut self.state.server_install && model.running {
            model.cancel.store(true, Ordering::Relaxed); model.message = "Cancelling installation…".into();
        }
        Effect { repaint: true, ..Effect::default() }
    }

    pub(super) fn server_install_output(&mut self, request: u64, text: &str) -> Effect {
        let Some(model) = self.state.server_install.as_mut().filter(|m| m.request == request && m.running) else { return Effect::default() };
        let remaining = installer::OUTPUT_LIMIT.saturating_sub(model.output.len());
        let mut end = text.len().min(remaining);
        while !text.is_char_boundary(end) { end -= 1; }
        if let Some(text) = text.get(..end) { model.output.push_str(text); }
        Effect { repaint: true, ..Effect::default() }
    }

    pub(super) fn server_installed(&mut self, request: u64, result: Result<(), String>) -> Effect {
        let Some((_, provider, _)) = self.install_running.as_ref().filter(|(running,_,_)| *running == request) else { return Effect::default() };
        let provider = *provider;
        self.install_running = None;
        if result.is_ok() {
            // Settings may have finished just before Close: still reload exactly the provider whose launch changed.
            self.state.semantics.reload_runtime(provider);
            self.synced = 0;
        }
        if let Some(model) = self.state.server_install.as_mut().filter(|m| m.request == request && m.running) {
            model.running = false; model.finished = true;
            model.message = match result {
                Ok(()) => format!("{provider} installed and validated. Its launch was saved; enabled workspaces will restart."),
                Err(why) => format!("Installation did not finish: {why}. Choose Install Language Server (S or Enter) to retry. Partial downloads may remain in Lenz's managed directory."),
            };
        }
        Effect { repaint: true, ..Effect::default() }
    }
}

impl Session {
    pub fn server_install_start_action(&self) -> CommandId {
        if self.state.server_install.as_ref().is_some_and(|m| m.finished) { CommandId::InstallServer } else { CommandId::StartServerInstall }
    }

    /// Ordered dialog actions and their labels/availability come from the shared command catalog.
    pub fn server_install_actions(&self) -> Vec<CommandId> {
        let Some(model) = self.state.server_install.as_ref() else { return Vec::new() };
        let mut actions = Vec::new();
        if !model.finished { actions.push(if model.running { CommandId::CancelServerInstall } else { CommandId::StartServerInstall }); }
        else { actions.push(CommandId::InstallServer); }
        if !model.running && model.preview.disabled.is_some() && self.runtime_kind().is_some() { actions.push(CommandId::ServerRuntime); }
        actions.push(CommandId::ServerDetails);
        actions
    }

    pub(super) fn refresh_install_runtime(&mut self, provider: &str) {
        if self.state.server_install.as_ref().is_some_and(|m| !m.running && m.provider == provider
            && self.state.current_source().is_some_and(|source| source.source() == m.source)) {
            self.open_server_install();
        }
    }
}
