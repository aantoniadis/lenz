//! Terminal state transitions use the same command boundary as editor operations.
use super::*;
use crate::{
    actions::{CatalogAction, InterfaceAction},
    terminal::{self, Command as T, Id},
};
use std::collections::HashMap;

#[derive(Default)]
pub(super) struct Terminals {
    processes: HashMap<Id, terminal::Process>,
    starting: HashMap<Id, (u64, Arc<std::sync::atomic::AtomicBool>)>,
    stopping: HashMap<(Id, u64), terminal::Process>,
    next: u64,
    generation: u64,
    prompt_seq: u64,
    decision: Option<Decision>,
    close_approved: Option<u64>,
}
#[derive(Clone, Copy)]
enum Decision {
    Close(Id, u64),
    Stop(Id, u64),
    View(u64),
}
impl Drop for Terminals {
    fn drop(&mut self) {
        for (_, cancel) in self.starting.values() {
            cancel.store(true, Ordering::Release);
        }
    }
}
impl State {
    pub fn terminal(&self) -> &terminal::State {
        &self.terminal
    }
}
impl Session {
    pub(crate) fn terminal_disabled(&self, id: CommandId) -> Option<&'static str> {
        let state = &self.state.terminal;
        let tab = state.active();
        match id {
            CommandId::ShowTerminal | CommandId::NewTerminal | CommandId::FocusTerminal => {
                (!cfg!(target_os = "linux"))
                    .then_some("Integrated terminals are currently supported on Linux")
            }
            CommandId::FocusEditor => None,
            CommandId::SendSelectionToTerminal => {
                if !tab.is_some_and(|t| t.running()) {
                    return Some("Start a terminal first");
                }
                match self
                    .state
                    .active
                    .and_then(|id| self.state.view(id))
                    .and_then(|v| v.selected_range())
                {
                    Some(range) if range.len() > terminal::MAX_PASTE => {
                        Some("Selection exceeds the 1 MiB terminal paste limit")
                    }
                    Some(range) if !range.is_empty() => None,
                    _ => Some("Select source text to paste into the terminal"),
                }
            }
            CommandId::PasteTerminalPath => {
                (!tab.is_some_and(|t| t.running())).then_some("Start a terminal first")
            }
            CommandId::CloseTerminal | CommandId::CopyTerminalSelection | CommandId::RenameTerminal => {
                tab.is_none().then_some("No terminal is open")
            }
            CommandId::FindTerminal => match tab {
                None => Some("No terminal is open"),
                Some(tab) if tab.starting || self.terminals.processes.get(&tab.id).is_some_and(|p| !p.finished()) => None,
                Some(_) => Some("This terminal's history is unavailable after Stop or a worker failure; restart it first"),
            },
            CommandId::StopTerminal => (!tab.is_some_and(|t| t.running() || t.starting()))
                .then_some("The terminal is not running"),
            CommandId::RestartTerminal if tab.is_some_and(|t| self.terminal_awaiting(t.id)) => {
                Some("Waiting for terminal cleanup to finish")
            }
            CommandId::RestartTerminal => match tab {
                None => Some("No terminal is open"),
                Some(t) if t.running() || t.starting() => {
                    Some("Stop the terminal before restarting it")
                }
                Some(_) => None,
            },
            CommandId::NextTerminal | CommandId::PreviousTerminal => {
                (state.tabs.len() < 2).then_some("Open another terminal tab first")
            }
            _ => None,
        }
    }

    pub(crate) fn terminal_action(&self, id: CommandId) -> CatalogAction {
        let command = match id {
            CommandId::ShowTerminal => {
                if self.state.terminal.shown {
                    T::Hide
                } else {
                    T::Show
                }
            }
            CommandId::NewTerminal => T::New,
            CommandId::FocusTerminal => T::Show,
            CommandId::FocusEditor => T::Focus(false),
            CommandId::SendSelectionToTerminal => {
                if let Some(why) = self.terminal_disabled(id) {
                    return CatalogAction::Disabled(why);
                }
                let text = self
                    .state
                    .active
                    .and_then(|id| self.state.view(id))
                    .and_then(|v| v.copy_selection());
                let Some(text) = text else {
                    return CatalogAction::Disabled(
                        "Select source text to paste into the terminal",
                    );
                };
                T::Paste(text)
            }
            CommandId::PasteTerminalPath => {
                return CatalogAction::Interface(InterfaceAction::PasteTerminalPath);
            }
            CommandId::RenameTerminal => {
                return match self.state.terminal.active() {
                    Some(tab) => CatalogAction::Interface(InterfaceAction::RenameTerminal(tab.id)),
                    None => CatalogAction::Disabled("No terminal is open"),
                };
            }
            CommandId::CloseTerminal => T::Close,
            CommandId::StopTerminal => T::Stop,
            CommandId::RestartTerminal => T::Restart,
            CommandId::NextTerminal => T::Next { backwards: false },
            CommandId::PreviousTerminal => T::Next { backwards: true },
            CommandId::CopyTerminalSelection => T::Copy,
            CommandId::FindTerminal => T::Find,
            _ => return CatalogAction::Disabled("This is not a terminal command"),
        };
        CatalogAction::Command(Command::Terminal(command))
    }

    pub fn terminal_rename_refusal(&self, id: Id, label: &str) -> Option<String> {
        if !self.state.terminal.tabs.iter().any(|tab| tab.id == id) {
            Some("This terminal tab has been closed".into())
        } else if label.chars().any(char::is_control) {
            Some("Terminal names cannot contain control characters".into())
        } else if label.trim().is_empty() {
            Some("Enter a terminal name".into())
        } else if label.trim().chars().count() > 128 {
            Some("Terminal names are limited to 128 characters".into())
        } else {
            None
        }
    }

    pub fn terminal_path_command(&self, path: Option<&str>) -> Result<Command, String> {
        let path = match path {
            Some(path) => self
                .state
                .project
                .as_ref()
                .map(|p| p.root().join(path))
                .ok_or("No project folder is open")?,
            None => {
                let tab = self.state.active().ok_or("No file is open")?;
                if tab.untitled() {
                    return Err("Save the untitled document before pasting its path".into());
                }
                tab.source().to_owned()
            }
        };
        if !path.is_absolute() {
            return Err("The file does not have an absolute path".into());
        }
        Ok(Command::Terminal(T::Paths(vec![path])))
    }

    pub(super) fn terminal_command(&mut self, command: T) -> Effect {
        let previously_focused = self.state.terminal.focused();
        let previous_terminal = self.state.terminal.active;
        let mut effect = Effect {
            repaint: true,
            ..Effect::default()
        };
        if matches!(command, T::New | T::Hide | T::Focus(false) | T::Activate(_) | T::Next { .. } | T::Restart | T::Close | T::Stop) {
            self.terminal_command(T::FindClose);
        }
        // Find owns input at the core boundary too: native commits and paste cannot reach the shell.
        if let Some(query) = self.state.terminal.find.as_ref() {
            match &command {
                T::Text(text) | T::Paste(text) => {
                    if text.len() > 1024usize.saturating_sub(query.len()) || text.chars().any(char::is_control) {
                        return self.command(Command::Notice("Terminal find accepts a single line up to 1024 bytes".into()));
                    }
                    return self.terminal_command(T::Search { query: format!("{query}{text}"), backwards: false, advance: false });
                }
                T::Paths(_) => return self.command(Command::Notice("Close terminal Find before pasting file paths".into())),
                T::Key(key) => {
                    let mut query = query.clone();
                    return match key.key.as_str() {
                        "escape" => self.terminal_command(T::FindClose),
                        "enter" if !key.ctrl && !key.alt => self.terminal_command(T::Search { query, backwards: key.shift, advance: true }),
                        "backspace" if !key.ctrl && !key.alt => {
                            query.pop();
                            self.terminal_command(T::Search { query, backwards: false, advance: false })
                        }
                        "a" if key.ctrl && !key.alt => self.terminal_command(T::Search { query: String::new(), backwards: false, advance: false }),
                        _ => effect,
                    };
                }
                _ => {}
            }
        }
        match command {
            T::Find => {
                if let Some(why) = self.terminal_disabled(CommandId::FindTerminal) {
                    return self.command(Command::Notice(why.into()));
                }
                if self.state.terminal.active().is_some() {
                    self.state.terminal.shown = true;
                    self.state.terminal.focused = true;
                    self.state.terminal.find.get_or_insert_default();
                }
            }
            T::FindClose => {
                if self.state.terminal.find.take().is_some()
                    && let Some(process) = self.state.terminal.active.and_then(|id| self.terminals.processes.get(&id)) {
                    let _ = process.send(T::Search { query: String::new(), backwards: false, advance: false });
                }
            }
            T::Search { query, backwards, advance } => {
                if self.state.terminal.find.is_none() { return effect; }
                if query.len() > 1024 || query.chars().any(char::is_control) {
                    return self.command(Command::Notice("Terminal find accepts a single line up to 1024 bytes".into()));
                }
                self.state.terminal.find = Some(query.clone());
                if let Some(process) = self.state.terminal.active.and_then(|id| self.terminals.processes.get(&id))
                    && let Err(why) = process.send(T::Search { query, backwards, advance }) {
                    return self.command(Command::Notice(why));
                }
            }
            T::Show => {
                if self.state.terminal.tabs.is_empty() {
                    return self.terminal_command(T::New);
                }
                self.state.terminal.shown = true;
                self.state.terminal.focused = true;
            }
            T::Hide => {
                if previously_focused
                    && let Some(process) =
                        previous_terminal.and_then(|id| self.terminals.processes.get(&id))
                {
                    let _ = process.send(T::Focus(false));
                }
                self.state.terminal.shown = false;
                self.state.terminal.focused = false;
            }
            T::New => {
                if !cfg!(target_os = "linux") {
                    return self.command(Command::Notice(
                        "Integrated terminals are currently supported on Linux".into(),
                    ));
                }
                self.cancel_close();
                self.terminals.next += 1;
                let id = Id(self.terminals.next);
                let cwd = self
                    .state
                    .project
                    .as_ref()
                    .map(|p| p.root().to_owned())
                    .or_else(|| {
                        self.state
                            .active()
                            .filter(|t| !t.untitled())
                            .and_then(|t| t.source().parent().map(Path::to_owned))
                    })
                    .unwrap_or_else(|| {
                        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"))
                    });
                self.state.terminal.tabs.push(terminal::Tab {
                    id,
                    generation: 0,
                    label: format!("Terminal {}", id.0),
                    cwd,
                    running: false,
                    starting: false,
                    status: String::new(),
                    error: None,
                    frame: None,
                });
                self.state.terminal.active = Some(id);
                self.state.terminal.shown = true;
                self.state.terminal.focused = true;
                self.start_terminal(id);
            }
            T::Rename { id, label } => {
                if let Some(why) = self.terminal_rename_refusal(id, &label) {
                    return self.command(Command::Notice(why));
                }
                if let Some(tab) = self.state.terminal.tabs.iter_mut().find(|tab| tab.id == id) {
                    tab.label = label.trim().to_owned();
                }
            }
            T::Restart => {
                if self
                    .state
                    .terminal
                    .active
                    .is_some_and(|id| self.terminal_awaiting(id))
                {
                    return effect;
                }
                let Some(tab) = self
                    .state
                    .terminal
                    .active()
                    .filter(|t| !t.running && !t.starting)
                else {
                    return effect;
                };
                let id = tab.id;
                self.cancel_close();
                self.terminals.processes.remove(&id);
                self.start_terminal(id);
                self.state.terminal.shown = true;
                self.state.terminal.focused = true;
            }
            T::Focus(focused) => {
                if focused {
                    return self.terminal_command(T::Show);
                }
                self.state.terminal.focused = false;
                if let Some(process) = self
                    .state
                    .terminal
                    .active
                    .and_then(|id| self.terminals.processes.get(&id))
                {
                    let _ = process.send(T::Focus(false));
                }
            }
            T::Activate(id) => {
                if self.state.terminal.tabs.iter().any(|t| t.id == id) {
                    if let Some(process) = self
                        .state
                        .terminal
                        .active
                        .and_then(|id| self.terminals.processes.get(&id))
                    {
                        let _ = process.send(T::Focus(false));
                    }
                    self.state.terminal.active = Some(id);
                    self.state.terminal.shown = true;
                    self.state.terminal.focused = true;
                }
            }
            T::Next { backwards } => {
                let tabs = &self.state.terminal.tabs;
                if !tabs.is_empty() {
                    let current = tabs
                        .iter()
                        .position(|t| Some(t.id) == self.state.terminal.active)
                        .unwrap_or(0);
                    let index = if backwards {
                        (current + tabs.len() - 1) % tabs.len()
                    } else {
                        (current + 1) % tabs.len()
                    };
                    return self.terminal_command(T::Activate(tabs[index].id));
                }
            }
            T::Close | T::Stop => {
                let Some(tab) = self.state.terminal.active() else {
                    return effect;
                };
                let decision = if command == T::Close {
                    Decision::Close(tab.id, tab.generation)
                } else {
                    Decision::Stop(tab.id, tab.generation)
                };
                if tab.running || tab.starting {
                    let close = command == T::Close;
                    let text = format!(
                        "{} in {}\n\n{} this terminal? Running commands and attached jobs will be terminated.\n\nEnter confirms; Escape cancels and keeps everything running.",
                        tab.label,
                        tab.cwd.display(),
                        if close { "Close" } else { "Stop" }
                    );
                    let (title, accept) = if close { ("Close terminal tab?", "Close tab") } else { ("Stop terminal?", "Stop") };
                    self.terminal_decision(decision, title, text, accept);
                } else {
                    self.finish_terminal_decision(decision);
                }
            }
            T::Confirm { ticket, accept } => {
                if !self
                    .state
                    .terminal
                    .prompt
                    .as_ref()
                    .is_some_and(|p| p.ticket == ticket)
                {
                    return effect;
                }
                self.state.terminal.prompt = None;
                if let Some(decision) = self.terminals.decision.take() {
                    if accept {
                        self.finish_terminal_decision(decision);
                    } else if matches!(decision, Decision::View(_)) {
                        self.cancel_close();
                    }
                }
            }
            command @ (T::Key(_)
            | T::Text(_)
            | T::Paste(_)
            | T::Paths(_)
            | T::Resize { .. }
            | T::Scroll { .. }
            | T::ScrollTo { .. }
            | T::Select { .. }
            | T::Copy
            | T::Mouse { .. }) => {
                let Some(id) = self.state.terminal.active else {
                    return effect;
                };
                if matches!(&command, T::Paste(_) | T::Paths(_)) {
                    self.state.terminal.shown = true;
                    self.state.terminal.focused = true;
                }
                let input = matches!(&command, T::Key(_) | T::Text(_) | T::Paste(_) | T::Paths(_));
                // Resizing a just-starting terminal is repeated after its first frame by the adapter.
                let result = match self.terminals.processes.get(&id) {
                    Some(process) => process.send(command),
                    None => Err("The terminal is still starting or has exited".into()),
                };
                if let Some(tab) = self.state.terminal.tabs.iter_mut().find(|t| t.id == id) {
                    match result {
                        Err(why) => tab.error = Some(why),
                        Ok(()) if input => tab.error = None,
                        // Pointer, scroll and resize traffic keeps a refusal visible; the worker's frame repaints.
                        Ok(()) => return Effect::default(),
                    }
                }
            }
        }
        if self.state.terminal.focused()
            && (!previously_focused || previous_terminal != self.state.terminal.active)
            && let Some(process) = self
                .state
                .terminal
                .active
                .and_then(|id| self.terminals.processes.get(&id))
        {
            let _ = process.send(T::Focus(true));
        }
        effect.repaint = true;
        effect
    }

    fn start_terminal(&mut self, id: Id) {
        self.terminals.generation += 1;
        let generation = self.terminals.generation;
        let Some(tab) = self.state.terminal.tabs.iter_mut().find(|t| t.id == id) else {
            return;
        };
        tab.generation = generation;
        tab.starting = true;
        tab.running = false;
        tab.status = "Starting…".into();
        tab.error = None;
        tab.frame = None;
        let cwd = tab.cwd.clone();
        let tx = self.tx.clone();
        let colors = (self.state.palette.fg, self.state.palette.bg);
        #[cfg(any(test, feature = "test-support"))]
        let shell = std::ffi::OsString::from("/bin/sh");
        #[cfg(not(any(test, feature = "test-support")))]
        let shell = std::env::var_os("SHELL")
            .filter(|p| !p.is_empty())
            .unwrap_or_else(|| "/bin/sh".into());
        let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.terminals
            .starting
            .insert(id, (generation, cancelled.clone()));
        let result = std::thread::Builder::new()
            .name(format!("terminal-start-{}", id.0))
            .spawn(move || {
                let result = if cancelled.load(Ordering::Acquire) {
                    Err("Terminal startup cancelled".into())
                } else {
                    terminal::Process::spawn(id, generation, cwd, shell, tx.clone(), colors)
                };
                // Even cancelled startup returns ownership: normal close awaits its cleanup.
                let _ = tx.send_blocking(Completion::TerminalStarted {
                    id,
                    generation,
                    result,
                });
            });
        if let Err(error) = result {
            self.terminal_started(
                id,
                generation,
                Err(format!("Cannot start terminal worker: {error}")),
            );
        }
    }

    pub(super) fn terminal_started(
        &mut self,
        id: Id,
        generation: u64,
        result: Result<terminal::Process, String>,
    ) -> Effect {
        let Some((wanted, cancelled)) = self.terminals.starting.get(&id) else {
            return Effect::default();
        };
        if *wanted != generation {
            return Effect::default();
        }
        let cancelled = cancelled.load(Ordering::Acquire);
        self.terminals.starting.remove(&id);
        let current = self
            .state
            .terminal
            .tabs
            .iter()
            .any(|t| t.id == id && t.generation == generation && t.starting);
        if cancelled || !current {
            if let Ok(process) = result {
                self.retain_terminal_shutdown(id, generation, process);
            }
            self.terminal_cleanup_status(id, generation);
            return Effect {
                repaint: self.state.terminal.shown,
                ..Effect::default()
            };
        }
        let Some(tab) = self.state.terminal.tabs.iter_mut().find(|t| t.id == id) else {
            return Effect::default();
        };
        tab.starting = false;
        match result {
            Ok(process) => {
                tab.running = true;
                tab.status = "Running".into();
                if self.state.terminal.active == Some(id) && let Some(query) = &self.state.terminal.find {
                    let _ = process.send(T::Search { query: query.clone(), backwards: false, advance: false });
                }
                self.terminals.processes.insert(id, process);
                return self.terminal_wake(id, generation);
            }
            Err(error) => {
                tab.running = false;
                tab.status = "Failed to start".into();
                tab.error = Some(error);
                if self.state.terminal.active == Some(id) { self.state.terminal.find = None; }
            }
        }
        Effect {
            repaint: self.state.terminal.shown,
            ..Effect::default()
        }
    }

    pub(super) fn terminal_wake(&mut self, id: Id, generation: u64) -> Effect {
        if let Some(process) = self.terminals.stopping.get(&(id, generation)) {
            // Acknowledge the old wake before checking; final cleanup must stay observable.
            process.take();
            let finished = process.finished();
            if finished {
                self.terminals.stopping.remove(&(id, generation));
                self.terminal_cleanup_status(id, generation);
            }
            return Effect {
                repaint: self.state.terminal.shown && self.state.terminal.active == Some(id),
                ..Effect::default()
            };
        }
        if !self
            .state
            .terminal
            .tabs
            .iter()
            .any(|t| t.id == id && t.generation == generation)
        {
            return Effect::default();
        }
        let Some(process) = self.terminals.processes.get(&id) else {
            return Effect::default();
        };
        let update = process.take();
        if process.finished() && self.state.terminal.active == Some(id) { self.state.terminal.find = None; }
        let Some(tab) = self.state.terminal.tabs.iter_mut().find(|t| t.id == id) else {
            return Effect::default();
        };
        tab.running = update.running;
        tab.status = update.status;
        if let Some(frame) = update.frame {
            tab.frame = Some(frame);
        }
        if let Some(error) = update.error {
            tab.error = Some(error);
        }
        Effect {
            repaint: self.state.terminal.shown && self.state.terminal.active == Some(id),
            copy: update.copy,
            ..Effect::default()
        }
    }

    fn terminal_decision(&mut self, decision: Decision, title: &'static str, text: String, accept: &'static str) {
        self.terminals.prompt_seq += 1;
        self.terminals.decision = Some(decision);
        self.state.terminal.prompt = Some(terminal::Prompt {
            ticket: self.terminals.prompt_seq,
            title,
            text,
            accept,
        });
    }

    fn finish_terminal_decision(&mut self, decision: Decision) {
        match decision {
            Decision::Close(id, generation) | Decision::Stop(id, generation) => {
                let Some(index) = self
                    .state
                    .terminal
                    .tabs
                    .iter()
                    .position(|t| t.id == id && t.generation == generation)
                else {
                    return;
                };
                self.stop_terminal_process(id, generation);
                if matches!(decision, Decision::Close(_, _)) {
                    self.state.terminal.tabs.remove(index);
                    if self.state.terminal.active == Some(id) {
                        self.state.terminal.active = self
                            .state
                            .terminal
                            .tabs
                            .get(index.min(self.state.terminal.tabs.len().saturating_sub(1)))
                            .map(|t| t.id);
                    }
                    if self.state.terminal.tabs.is_empty() {
                        self.state.terminal.shown = false;
                        self.state.terminal.focused = false;
                    }
                } else if let Some(tab) = self.state.terminal.tabs.get_mut(index) {
                    tab.running = false;
                    tab.starting = false;
                    tab.status = "Stopping…".into();
                }
                self.terminal_cleanup_status(id, generation);
            }
            Decision::View(ticket) => {
                if self.close_ticket() == ticket {
                    self.terminals.close_approved = Some(ticket);
                    self.advance_close();
                }
            }
        }
    }

    pub(super) fn terminal_prepare_close(&mut self, ticket: u64) {
        if self
            .state
            .terminal
            .tabs
            .iter()
            .any(|t| t.running || t.starting)
        {
            let names = self
                .state
                .terminal
                .tabs
                .iter()
                .filter(|t| t.running || t.starting)
                .map(|t| format!("{} — {}", t.label, t.cwd.display()))
                .collect::<Vec<_>>()
                .join("\n");
            self.terminal_decision(Decision::View(ticket), "Close with running terminals?", format!("Close this view and its running terminals?\n\n{names}\n\nRunning commands and attached jobs will be terminated after any unsaved-draft decisions also approve.\n\nEnter confirms; Escape cancels."), "Close view");
        }
    }
    fn terminal_awaiting(&self, id: Id) -> bool {
        self.terminals.starting.contains_key(&id)
            || self
                .terminals
                .stopping
                .keys()
                .any(|(stopped, _)| *stopped == id)
    }
    fn terminal_cleanup_status(&mut self, id: Id, generation: u64) {
        let awaiting = self.terminal_awaiting(id);
        if let Some(tab) = self
            .state
            .terminal
            .tabs
            .iter_mut()
            .find(|t| t.id == id && t.generation == generation && !t.running && !t.starting)
        {
            tab.status = if awaiting { "Stopping…" } else { "Stopped" }.into();
        }
    }
    fn stop_terminal_process(&mut self, id: Id, generation: u64) {
        if let Some((_, cancel)) = self.terminals.starting.get(&id) {
            cancel.store(true, Ordering::Release);
        }
        if let Some(process) = self.terminals.processes.remove(&id) {
            self.retain_terminal_shutdown(id, generation, process);
        }
    }
    fn retain_terminal_shutdown(&mut self, id: Id, generation: u64, process: terminal::Process) {
        process.stop();
        process.take();
        if !process.finished() { self.terminals.stopping.insert((id, generation), process); }
    }
    pub(super) fn terminal_begin_close_stop(&mut self, ticket: u64) {
        if self
            .state
            .terminal
            .tabs
            .iter()
            .any(|t| t.running || t.starting)
            && self.terminals.close_approved != Some(ticket)
        {
            return;
        }
        let targets = self
            .state
            .terminal
            .tabs
            .iter()
            .map(|t| (t.id, t.generation))
            .collect::<Vec<_>>();
        for (id, generation) in targets {
            self.stop_terminal_process(id, generation);
            if let Some(tab) = self.state.terminal.tabs.iter_mut().find(|t| t.id == id) {
                tab.running = false;
                tab.starting = false;
            }
            self.terminal_cleanup_status(id, generation);
        }
    }
    pub(super) fn terminal_consent_ready(&self, ticket: u64) -> bool {
        !self
            .state
            .terminal
            .tabs
            .iter()
            .any(|t| t.running || t.starting)
            || self.terminals.close_approved == Some(ticket)
    }
    pub(super) fn terminal_close_ready(&self, _ticket: u64) -> bool {
        self.terminals.processes.is_empty()
            && self.terminals.starting.is_empty()
            && self.terminals.stopping.is_empty()
    }
    pub(super) fn terminal_cancel_close(&mut self) {
        self.terminals.close_approved = None;
        if matches!(self.terminals.decision, Some(Decision::View(_))) {
            self.terminals.decision = None;
            self.state.terminal.prompt = None;
        }
    }
    pub(super) fn terminal_release_all(&mut self) {
        for (_, cancel) in self.terminals.starting.values() {
            cancel.store(true, Ordering::Release);
        }
        self.terminals.starting.clear();
        self.terminals.processes.clear();
        self.terminals.stopping.clear();
        self.state.terminal.prompt = None;
        self.terminals.decision = None;
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    #[test]
    fn terminal_rename_is_available_in_the_shared_catalog() {
        assert!(CommandId::ALL.iter().any(|id| id.label() == "Rename Terminal Tab…"));
    }
    #[test]
    fn terminal_rename_preserves_sessions_and_rejects_invalid_or_stale_targets() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = session(dir.path());
        assert!(s.disabled(CommandId::RenameTerminal).is_some());
        s.dispatch(Command::Terminal(T::New));
        let first = s.state.terminal.active().unwrap().id;
        s.dispatch(Command::Terminal(T::New));
        let second = s.state.terminal.active().unwrap().id;
        let cwd = s.state.terminal.tabs[0].cwd.clone();
        let generation = s.state.terminal.tabs[0].generation;
        s.dispatch(Command::Terminal(T::Rename { id: first, label: "  Claude — SQL  ".into() }));
        assert_eq!(s.state.terminal.tabs[0].label(), "Claude — SQL");
        assert_eq!(s.state.terminal.active().unwrap().id, second);
        assert_eq!(s.state.terminal.tabs[0].cwd, cwd);
        assert_eq!(s.state.terminal.tabs[0].generation, generation);
        for label in [" ".into(), "bad\nname".into(), "bad\u{1b}name".into(), "a".repeat(129)] {
            assert!(s.terminal_rename_refusal(first, &label).is_some());
            s.dispatch(Command::Terminal(T::Rename { id: first, label }));
            assert_eq!(s.state.terminal.tabs[0].label(), "Claude — SQL");
        }
        s.dispatch(Command::Terminal(T::Rename { id: Id(u64::MAX), label: "stale".into() }));
        assert_eq!(s.state.terminal.tabs[1].label(), "Terminal 2");
        assert!(s.disabled(CommandId::RenameTerminal).is_none());
    }
    #[test]
    fn terminal_pointer_traffic_keeps_a_refusal_visible_without_repainting() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = session(dir.path());
        s.dispatch(Command::Terminal(T::New));
        settle(&mut s, |s| s.state.terminal.active().is_some_and(|t| t.running && t.frame.is_some()));
        s.dispatch(Command::Terminal(T::Paste("injected\u{1b}[201~".into())));
        let refusal = s.state.terminal.active().unwrap().error.clone();
        assert!(refusal.is_some());
        let mouse = T::Mouse { row: 0, column: 0, kind: terminal::MouseKind::Move(3), ctrl: false, alt: false, shift: false };
        assert!(!s.terminal_command(mouse).repaint);
        assert!(!s.terminal_command(T::Scroll { lines: 1, local: true }).repaint);
        assert_eq!(s.state.terminal.active().unwrap().error, refusal);
        s.dispatch(Command::Terminal(T::Text(" ".into())));
        assert_eq!(s.state.terminal.active().unwrap().error, None);
    }

    #[test]
    fn terminal_rename_survives_stop_and_restart() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = session(dir.path());
        s.dispatch(Command::Terminal(T::New));
        settle(&mut s, |s| s.state.terminal.active().is_some_and(|t| t.running));
        let id = s.state.terminal.active().unwrap().id;
        let generation = s.state.terminal.active().unwrap().generation;
        s.dispatch(Command::Terminal(T::Rename { id, label: "SQL".into() }));
        s.dispatch(Command::Terminal(T::Stop));
        let ticket = s.state.terminal.prompt().unwrap().ticket;
        s.dispatch(Command::Terminal(T::Confirm { ticket, accept: true }));
        settle(&mut s, |s| !s.terminal_awaiting(id));
        s.dispatch(Command::Terminal(T::Restart));
        settle(&mut s, |s| s.state.terminal.active().is_some_and(|t| t.running));
        let tab = s.state.terminal.active().unwrap();
        assert_eq!(tab.id, id);
        assert_ne!(tab.generation, generation);
        assert_eq!(tab.label(), "SQL");
    }
    fn session(dir: &Path) -> Session {
        Session::with_text_input(
            Config {
                themes: lenz_content::theme::ThemePaths::under(
                    &dir.join("config"),
                    &dir.join("state"),
                    "lenz-code",
                ),
                recent: dir.join("recent"),
                settings: dir.join("settings"),
                providers: dir.join("providers"),
                cache: dir.join("cache"),
                idle: std::time::Duration::ZERO,
                clock: Arc::new(std::time::Instant::now),
                overrides: Default::default(),
                dark: true,
            },
            true,
        )
    }
    #[test]
    fn terminal_session_new_tabs_have_independent_identity_and_hidden_focus_is_cleared() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = session(dir.path());
        assert!(s.state().terminal().tabs().is_empty());
        assert!(s.disabled(CommandId::SendSelectionToTerminal).is_some());
        s.dispatch(Command::Terminal(T::New));
        let first = s
            .state()
            .terminal()
            .active()
            .expect("New creates a terminal tab")
            .id();
        s.dispatch(Command::Terminal(T::New));
        assert_eq!(s.state().terminal().tabs().len(), 2);
        assert_ne!(s.state().terminal().active().unwrap().id(), first);
        s.dispatch(Command::Terminal(T::Activate(first)));
        assert_eq!(s.state().terminal().active().unwrap().id(), first);
        s.dispatch(Command::Terminal(T::Hide));
        assert!(!s.state().terminal().shown() && !s.state().terminal().focused());
    }

    #[test]
    fn terminal_find_input_bounds_tab_identity_exit_and_stop_availability() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = session(dir.path());
        assert!(s.disabled(CommandId::FindTerminal).is_some());
        s.dispatch(Command::Terminal(T::New));
        settle(&mut s, |s| s.state.terminal.active().is_some_and(|t|t.running && t.frame.is_some()));
        s.dispatch(Command::Terminal(T::Text("stty -echo; printf 'literal.*界\\r\\n'; exit\n".into())));
        settle(&mut s, |s| !s.state.terminal.active().unwrap().running());
        assert!(s.disabled(CommandId::FindTerminal).is_none(), "normal shell exit retains the searchable model");
        s.dispatch(Command::Terminal(T::Find));
        s.dispatch(Command::Terminal(T::Paste("literal.*界".into())));
        settle(&mut s, |s| s.state.terminal.find_status() == "Match");
        assert_eq!(s.state.terminal.find(), Some("literal.*界"));
        s.dispatch(Command::Terminal(T::Paths(vec![dir.path().join("must-not-paste")])));
        assert!(s.status_message().contains("Close terminal Find"));
        s.dispatch(Command::Terminal(T::Text("\n".into())));
        s.dispatch(Command::Terminal(T::Text("x".repeat(1025))));
        assert_eq!(s.state.terminal.find(), Some("literal.*界"));
        s.dispatch(Command::Terminal(T::Key(crate::keymap::KeyPress::new("a",true,false,false).unwrap())));
        assert_eq!(s.state.terminal.find(), Some(""));
        s.dispatch(Command::Terminal(T::New));
        assert!(s.state.terminal.find().is_none());
        settle(&mut s, |s| s.state.terminal.active().is_some_and(|t|t.running && t.frame.is_some()));
        s.dispatch(Command::Terminal(T::Find));
        s.dispatch(Command::Terminal(T::Stop));
        assert!(s.state.terminal.find().is_none());
        let ticket = s.state.terminal.prompt().unwrap().ticket;
        s.dispatch(Command::Terminal(T::Confirm { ticket, accept:true }));
        assert!(s.disabled(CommandId::FindTerminal).is_some());
        s.dispatch(Command::Terminal(T::Find));
        assert!(s.state.terminal.find().is_none());
    }

    fn settle(s: &mut Session, condition: impl Fn(&Session) -> bool) {
        let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !condition(s) {
            assert!(
                std::time::Instant::now() < end,
                "terminal session did not settle"
            );
            while let Ok(c) = s.completions().try_recv() {
                s.dispatch(c);
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }
    fn text(s: &Session) -> String {
        s.state()
            .terminal()
            .active()
            .and_then(|t| t.frame())
            .map(|f| f.cells.iter().map(|c| c.text.as_str()).collect())
            .unwrap_or_default()
    }

    #[test]
    fn terminal_close_confirmation_is_fenced_to_the_original_tab_and_restart_generation() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = session(dir.path());
        s.dispatch(Command::Terminal(T::New));
        let first = s.state().terminal().active().unwrap().id();
        let generation = s.state().terminal().active().unwrap().generation;
        s.dispatch(Command::Terminal(T::Close));
        let ticket = s.state().terminal().prompt().unwrap().ticket;
        s.dispatch(Command::Terminal(T::New));
        let second = s.state().terminal().active().unwrap().id();
        s.dispatch(Command::Terminal(T::Confirm {
            ticket,
            accept: true,
        }));
        assert_eq!(s.state().terminal().tabs().len(), 1);
        assert_eq!(s.state().terminal().active().unwrap().id(), second);
        s.dispatch(Completion::TerminalStarted {
            id: first,
            generation,
            result: Err("stale startup".into()),
        });
        assert!(!s.status_details().contains("stale startup"));
        s.dispatch(Command::Terminal(T::Stop));
        let ticket = s.state().terminal().prompt().unwrap().ticket;
        s.dispatch(Command::Terminal(T::Confirm {
            ticket,
            accept: false,
        }));
        assert!(s.state().terminal().active().unwrap().starting());
        s.dispatch(Command::Terminal(T::Stop));
        let ticket = s.state().terminal().prompt().unwrap().ticket;
        s.dispatch(Command::Terminal(T::Confirm {
            ticket,
            accept: true,
        }));
        settle(&mut s, |s| !s.terminal_awaiting(second));
        s.dispatch(Command::Terminal(T::Restart));
        let before = s.state().terminal().active().unwrap().generation;
        s.dispatch(Completion::TerminalStarted {
            id: second,
            generation: before - 1,
            result: Err("old generation".into()),
        });
        assert!(s.state().terminal().active().unwrap().starting());
        assert!(s.state().terminal().active().unwrap().error().is_none());
    }

    #[test]
    fn terminal_window_close_waits_for_all_terminal_and_draft_consent() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = session(dir.path());
        s.dispatch(Command::NewDocument);
        s.dispatch(Command::InsertText {
            text: "retain draft".into(),
            kind: crate::text::EditKind::Typing,
        });
        s.dispatch(Command::Terminal(T::New));
        let mut coordinator = crate::close::CloseCoordinator::default();
        coordinator.register(s.id());
        coordinator.begin(&mut [&mut s]);
        assert!(matches!(
            coordinator.progress(&mut [&mut s]),
            crate::close::Progress::Pending(_)
        ));
        let ticket = s.state().terminal().prompt().unwrap().ticket;
        s.dispatch(Command::Terminal(T::Confirm {
            ticket,
            accept: true,
        }));
        assert!(matches!(
            coordinator.progress(&mut [&mut s]),
            crate::close::Progress::Pending(_)
        ));
        let prompt = s.close_prompt().unwrap();
        s.dispatch(Command::ChooseClose {
            ticket: prompt.ticket,
            buffer: prompt.buffer,
            choice: crate::close::Choice::Cancel,
        });
        assert_eq!(
            coordinator.progress(&mut [&mut s]),
            crate::close::Progress::Cancelled
        );
        assert!(!s.closed());
        assert_eq!(
            s.state()
                .current_source()
                .unwrap()
                .copy_range(0..12)
                .unwrap(),
            "retain draft"
        );
        coordinator.begin(&mut [&mut s]);
        let ticket = s.state().terminal().prompt().unwrap().ticket;
        s.dispatch(Command::Terminal(T::Confirm {
            ticket,
            accept: false,
        }));
        assert_eq!(
            coordinator.progress(&mut [&mut s]),
            crate::close::Progress::Cancelled
        );
        assert!(!s.closed());
    }

    #[test]
    fn terminal_source_send_uses_unsaved_selection_without_writing_or_changing_undo() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = session(dir.path());
        let source = dir.path().join("query.sql");
        let captured = dir.path().join("capture");
        std::fs::write(&source, "saved\n").unwrap();
        s.dispatch(Command::Open(source.clone()));
        settle(&mut s, |s| s.state().current_source().is_some());
        s.dispatch(Command::SelectAll);
        s.dispatch(Command::InsertText {
            text: "select 1;\nselect 2;".into(),
            kind: crate::text::EditKind::Paste,
        });
        s.dispatch(Command::SelectAll);
        s.dispatch(Command::Terminal(T::New));
        settle(&mut s, |s| {
            s.state()
                .terminal()
                .active()
                .is_some_and(|t| t.running() && t.frame().is_some())
        });
        let expected = b"\x1b[200~select 1;\nselect 2;\x1b[201~";
        let setup = format!(
            "stty -echo -icanon; printf '\\033[?2004hready_%s' paste; dd bs=1 count={} of='{}' 2>/dev/null; printf 'finished_%s' paste\n",
            expected.len(),
            captured.display()
        );
        s.dispatch(Command::Terminal(T::Text(setup)));
        settle(&mut s, |s| text(s).contains("ready_paste"));
        let crate::actions::CatalogAction::Command(command) =
            s.action(CommandId::SendSelectionToTerminal)
        else {
            panic!("selected draft can be sent")
        };
        s.dispatch(command);
        settle(&mut s, |s| text(s).contains("finished_paste"));
        assert_eq!(std::fs::read(captured).unwrap(), expected);
        assert_eq!(std::fs::read(&source).unwrap(), b"saved\n");
        let tab = s.state().active().unwrap().id();
        s.dispatch(Command::Undo { tab });
        assert_eq!(
            s.state()
                .current_source()
                .unwrap()
                .copy_range(0..6)
                .unwrap(),
            "saved\n"
        );
        assert!(
            !s.state()
                .activity_log()
                .iter()
                .any(|line| line.contains("select 2;"))
        );
    }

    #[test]
    fn terminal_final_view_close_waits_for_ignoring_jobs_and_reaps_before_committing() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = session(dir.path());
        let owned = dir.path().join("owned-pids");
        s.dispatch(Command::Terminal(T::New));
        settle(&mut s, |s| {
            s.state()
                .terminal()
                .active()
                .is_some_and(|t| t.running() && t.frame().is_some())
        });
        s.dispatch(Command::Terminal(T::Text(format!("trap '' HUP TERM; sleep 60 & printf '%s %s' \"$$\" \"$!\" > '{}'; printf 'ready_%s' closing\n", owned.display()))));
        settle(&mut s, |s| text(s).contains("ready_closing"));
        let pids = std::fs::read_to_string(&owned)
            .unwrap()
            .split_whitespace()
            .map(|p| p.parse::<u32>().unwrap())
            .collect::<Vec<_>>();
        let mut coordinator = crate::close::CloseCoordinator::default();
        coordinator.register(s.id());
        coordinator.begin(&mut [&mut s]);
        let ticket = s.state().terminal().prompt().unwrap().ticket;
        s.dispatch(Command::Terminal(T::Confirm {
            ticket,
            accept: true,
        }));
        assert!(
            matches!(
                coordinator.progress(&mut [&mut s]),
                crate::close::Progress::Pending(_)
            ),
            "close must await cleanup acknowledgement"
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            while let Ok(c) = s.completions().try_recv() {
                s.dispatch(c);
            }
            if matches!(
                coordinator.progress(&mut [&mut s]),
                crate::close::Progress::Committed(_)
            ) {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "close never acknowledged cleanup"
            );
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(s.closed());
        assert!(
            !PathBuf::from(format!("/proc/{}", pids[0])).exists(),
            "owned shell was not reaped before application exit"
        );
        if let Ok(stat) = std::fs::read_to_string(format!("/proc/{}/stat", pids[1])) {
            assert_eq!(
                stat.rsplit_once(')').unwrap().1.split_whitespace().next(),
                Some("Z"),
                "attached job survived application close"
            );
        }
    }

    #[test]
    fn terminal_quit_all_cancel_keeps_previously_approved_windows_running() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = session(&dir.path().join("a"));
        let mut b = session(&dir.path().join("b"));
        a.dispatch(Command::Terminal(T::New));
        b.dispatch(Command::Terminal(T::New));
        let mut coordinator = crate::close::CloseCoordinator::default();
        coordinator.register(a.id());
        coordinator.register(b.id());
        coordinator.begin(&mut [&mut a, &mut b]);
        let ticket = a.state().terminal().prompt().unwrap().ticket;
        a.dispatch(Command::Terminal(T::Confirm {
            ticket,
            accept: true,
        }));
        assert!(matches!(
            coordinator.progress(&mut [&mut a, &mut b]),
            crate::close::Progress::Pending(_)
        ));
        assert!(
            a.state()
                .terminal()
                .active()
                .is_some_and(|t| t.running() || t.starting()),
            "another window has not consented; earlier terminal must keep running"
        );
        let ticket = b.state().terminal().prompt().unwrap().ticket;
        b.dispatch(Command::Terminal(T::Confirm {
            ticket,
            accept: false,
        }));
        assert_eq!(
            coordinator.progress(&mut [&mut a, &mut b]),
            crate::close::Progress::Cancelled
        );
        assert!(
            a.state()
                .terminal()
                .active()
                .is_some_and(|t| t.running() || t.starting())
        );
        assert!(a.terminals.stopping.is_empty());
        assert!(!a.closed() && !b.closed());
    }

    #[test]
    fn terminal_running_session_does_not_block_closing_an_editor_tab() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = session(dir.path());
        s.dispatch(Command::NewDocument);
        let file = s.state().active().unwrap().id();
        s.dispatch(Command::Terminal(T::New));
        s.dispatch(Command::CloseTab(file));
        assert!(s.state().tabs().is_empty());
        assert!(
            s.state()
                .terminal()
                .active()
                .is_some_and(|t| t.running() || t.starting())
        );
        assert!(!s.closed());
    }

    #[test]
    fn terminal_close_after_reactor_failure_does_not_wait_for_another_wake() {
        let dir = tempfile::tempdir().unwrap(); let mut s = session(dir.path());
        s.dispatch(Command::Terminal(T::New));
        settle(&mut s, |s| s.state().terminal().active().is_some_and(|t| t.running() && t.frame().is_some()));
        s.dispatch(Command::Terminal(T::Text("stty raw -echo; i=0; while [ \"$i\" -lt 100000 ]; do printf '\\033[6n'; i=$((i+1)); done; sleep 60\n".into())));
        settle(&mut s, |s| s.state().terminal().active().is_some_and(|t| !t.running() && t.error().is_some()));
        let id = s.state().terminal().active().unwrap().id();
        assert!(s.terminals.processes.get(&id).unwrap().finished());
        s.dispatch(Command::Terminal(T::Close));
        assert!(s.terminals.stopping.is_empty(), "an already finished worker cannot send another acknowledgement");
        let mut c = crate::close::CloseCoordinator::default(); c.register(s.id()); c.begin(&mut [&mut s]);
        assert!(matches!(c.progress(&mut [&mut s]), crate::close::Progress::Committed(_)));
    }
}
