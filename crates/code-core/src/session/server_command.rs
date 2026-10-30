//! Server commands the user picked (an allowlisted completion, fix or organize-imports command, see
//! `semantics::provider::runs_command`): the server runs it and sends its edit back. That edit is applied only when
//! it changes this file alone, at the exact version the command was sent for, as one undoable transaction; anything
//! else is refused whole and the server is told so. Nothing else a server sends is ever applied.
use super::*;
use crate::text::EditKind;

/// The command running for the user's pick.
pub(super) struct CommandWait {
    request: u64,
    lane: semantics::Lane,
    session: u64,
    tab: TabId,
    expected: crate::text::SnapshotKey,
    /// What the messages call it: "Code completion", "Quick fixes", "Organize Imports".
    label: &'static str,
    applied: bool,
}

impl CommandWait {
    pub(super) fn answers(&self, reply: &semantics::Reply) -> bool { self.request == reply.request && self.lane == reply.lane }
}

impl Session {
    /// Run `command` on server session `session` for `tab` as it is now; its edit arrives through [`Self::command_reply`].
    pub(super) fn run_server_command(&mut self, session: u64, command: &serde_json::Value, lane: semantics::Lane, tab: TabId, label: &'static str) -> Effect {
        let repaint = Effect { repaint: true, ..Effect::default() };
        let Some(expected) = self.state.view(tab).map(|v| v.snapshot().key()) else { return Effect::default() };
        self.sync_semantics();
        self.semantic_seq += 1;
        let request = self.semantic_seq;
        match self.state.semantics.execute_command(session, command, lane, request) {
            Ok(()) => { self.pending_command = Some(CommandWait { request, lane, session, tab, expected, label, applied: false }); repaint }
            Err(why) => { self.state.messages.push(format!("{label}: {why}; nothing was changed")); repaint }
        }
    }

    /// The command's edit (`editId`), or its final answer.
    pub(super) fn command_reply(&mut self, reply: semantics::Reply) -> Effect {
        let repaint = Effect { repaint: true, ..Effect::default() };
        let Some(wait) = self.pending_command.as_ref().filter(|w| w.answers(&reply)) else { return Effect::default() };
        let (session, tab, expected, label) = (wait.session, wait.tab, wait.expected, wait.label);
        let value = match reply.result {
            Ok(value) => value,
            Err(why) => {
                self.pending_command = None;
                self.state.messages.push(format!("{label}: {why}"));
                return repaint;
            }
        };
        let Some(id) = value.get("editId").cloned() else {
            // The command finished: an edit, if any, was handled when it came.
            let applied = self.pending_command.take().is_some_and(|w| w.applied);
            if !applied { self.state.messages.push(format!("{label}: the language server made no change")); }
            return repaint;
        };
        let current = self.state.view(tab).is_some_and(|v| v.snapshot().key() == expected);
        let checked = if !current {
            Err("the file changed while the language server worked".to_owned())
        } else if let Some(why) = self.state.view(tab).and_then(|v| v.edit_disabled()) {
            Err(why.to_owned())
        } else {
            match self.admitted_document(tab, Priority::Visible).and_then(|doc| self.state.semantics.served(&doc, session)) {
                Some(doc) => code_action::workspace_edit(&value["edit"], &doc, reply.encoding).map_err(str::to_owned)
                    .and_then(|edits| match code_action::template_refusal(&doc, &edits) { Some(why) => Err(why), None => Ok(edits) }),
                None => Err("the file is not ready".to_owned()),
            }
        };
        match checked {
            Ok(edits) => {
                let mut effect = if edits.is_empty() { Effect::default() } else { self.edit(tab, expected, edits, EditKind::Replace) };
                self.state.semantics.answer_edit(session, id, true, "");
                if let Some(w) = self.pending_command.as_mut() { w.applied = true; }
                effect.repaint = true;
                effect
            }
            Err(why) => {
                self.state.semantics.answer_edit(session, id, false, &why);
                self.pending_command = None;
                self.state.messages.push(format!("{label}: {why}; nothing was changed"));
                repaint
            }
        }
    }
}
