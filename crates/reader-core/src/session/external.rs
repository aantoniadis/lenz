//! The core owns consent, Save continuation, and exactly-once launch delivery.
use super::*;
use crate::external::{Choice, Prompt, Request};
use lenz_content::open::{Host, Plan};

pub(super) struct Pending {
    ticket: u64,
    request: Request,
    choice: Option<Choice>,
    save: Option<u64>,
}

impl Session {
    /// Resolve platform configuration without mutating state; the returned command is fenced again on dispatch.
    pub fn external_command(&self, host: &Host, configured: Option<&str>, default_app: bool) -> Result<Command, &'static str> {
        let tab = self.external_tab()?;
        let current = self.editor_plan_at(host, configured, default_app, false)?;
        let saved = if self.state.tab_dirty(tab.id) { self.editor_plan_at(host, configured, default_app, true)? } else { current.clone() };
        Ok(Command::BeginExternal(Box::new(Request {
            tab: tab.id,
            source: self.state.source(tab.id).map(|source| SemanticSource::new(source)),
            permission: self.state.view(tab.id).and_then(|view|view.edit_disabled()),
            project: self.project_generation.load(Ordering::Acquire),
            path: tab.source.clone(), current, saved,
        })))
    }

    pub fn external_prompt(&self) -> Option<Prompt> {
        let pending = self.external_pending.as_ref()?;
        let buffer = pending.request.source.as_ref()?.key.buffer;
        let busy = self.state.sources.buffers.get(&buffer).is_some_and(|entry| entry.save.status.busy());
        Some(Prompt {
            ticket: pending.ticket, path: pending.request.path.clone(),
            waiting: pending.save.is_some() || busy,
            save_disabled: if pending.save.is_some() { Some("Waiting for the submitted Save") } else { self.save_buffer_disabled(buffer) },
            saved_disabled: if pending.save.is_some() { Some("Waiting for the submitted Save") } else { self.external_saved_disabled(&pending.request) },
        })
    }

    fn external_current(&self, request: &Request) -> bool {
        !self.closed && self.closing.is_none() && self.state.active == Some(request.tab)
            && self.project_generation.load(Ordering::Acquire) == request.project
            && self.external_tab().is_ok_and(|tab| tab.id == request.tab && tab.source == request.path)
            && self.state.view(request.tab).and_then(|view|view.edit_disabled()) == request.permission
            && match (&request.source, self.state.source(request.tab)) {
                (Some(captured), Some(current)) => captured.matches(current),
                (None, None) => true,
                _ => false,
            }
    }

    fn external_saved_disabled(&self, request: &Request) -> Option<&'static str> {
        let source = request.source.as_ref()?;
        let entry = self.state.sources.buffers.get(&source.key.buffer)?;
        if entry.save.status.busy() { return Some("Wait for the submitted Save operation"); }
        if entry.save.status.blocked() && !entry.save.revalidated.as_ref().is_some_and(|receipt| &receipt.source == source) {
            return Some("Recheck the uncertain disk outcome before opening its saved version");
        }
        None
    }

    pub(super) fn begin_external(&mut self, request: Request) -> Effect {
        if !self.external_current(&request) { return Effect::default(); }
        self.external_seq += 1;
        let choice = (!self.state.tab_dirty(request.tab)).then_some(Choice::OpenSavedVersion);
        self.external_pending = Some(Pending { ticket: self.external_seq, request, choice, save: None });
        Effect { repaint: true, ..Effect::default() }
    }

    pub(super) fn choose_external(&mut self, ticket: u64, choice: Choice) -> Effect {
        let Some(mut pending) = self.external_pending.take() else { return Effect::default() };
        if pending.ticket != ticket { self.external_pending = Some(pending); return Effect::default(); }
        if choice == Choice::Cancel { return Effect { repaint: true, ..Effect::default() }; }
        if !self.external_current(&pending.request) { return Effect { repaint: true, ..Effect::default() }; }
        if pending.choice.is_some() { self.external_pending = Some(pending); return Effect::default(); }
        let Some(source) = &pending.request.source else { return Effect::default() };
        let buffer = source.key.buffer;
        match choice {
            Choice::OpenSavedVersion => {
                if self.external_saved_disabled(&pending.request).is_none() { pending.choice = Some(choice); }
                self.external_pending = Some(pending);
                Effect { repaint: true, ..Effect::default() }
            }
            Choice::SaveAndOpen => {
                if self.save_buffer_disabled(buffer).is_some() { self.external_pending = Some(pending); return Effect::default(); }
                let effect = self.save_buffer(buffer);
                if let Some(request) = self.state.sources.buffers.get(&buffer).and_then(|entry| entry.save.captured.as_ref())
                    .filter(|capture| !capture.compacting).map(|capture| capture.request)
                {
                    pending.choice = Some(choice); pending.save = Some(request); self.external_pending = Some(pending);
                }
                effect
            }
            Choice::Cancel => Effect::default(),
        }
    }

    pub(super) fn advance_external(&mut self) -> Option<Plan> {
        let pending = self.external_pending.take()?;
        if !self.external_current(&pending.request) {
            self.state.messages.push("External open cancelled: the file or context changed. The draft is retained.".into());
            return None;
        }
        match pending.choice {
            Some(Choice::OpenSavedVersion) => Some(pending.request.saved),
            Some(Choice::SaveAndOpen) => {
                let source = pending.request.source.as_ref()?;
                let entry = self.state.sources.buffers.get(&source.key.buffer)?;
                if pending.save.is_some_and(|request| entry.save.written.as_ref().is_some_and(|(written, captured)| *written == request && captured == source)) {
                    return Some(pending.request.current);
                }
                if entry.save.status.busy() { self.external_pending = Some(pending); }
                None
            }
            None => { self.external_pending = Some(pending); None }
            Some(Choice::Cancel) => None,
        }
    }
}
