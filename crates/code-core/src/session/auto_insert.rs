//! Text a language server inserts after a typed character: VS Code's HTML server closes the element whose `>` was
//! just typed, or the innermost one left open after `</`. Asked once the typing command is over, fenced like
//! completion to the exact tab, draft version, caret and server session; the answer is a snippet inserted as its
//! own Undo step (the typed character stays when it is undone), and an answer for text or a caret that has moved on
//! is dropped. Asking is quiet: a server that cannot answer leaves the typing as it is.
use super::*;
use crate::text::{EditKind, TextEdit};

/// The question on its way: where the character was typed.
pub(super) struct AutoInsertRequest { request: u64, tab: TabId, source: SemanticSource, caret: usize }

impl AutoInsertRequest {
    /// Whether it is still about the active tab's version as it is, with its one caret where it was typed.
    pub(super) fn current(&self, state: &State) -> bool {
        state.active == Some(self.tab) && state.view(self.tab).is_some_and(|v| self.source.matches(v.snapshot()) && v.tab().extra_selections().is_empty())
            && state.selection().is_some_and(|s| s.anchor == self.caret && s.head == self.caret)
    }
    pub(super) fn request(&self) -> u64 { self.request }
}

impl Session {
    /// After an edit of `tab`: one character typed at one caret that its server inserts after is asked about once the
    /// command is over; any other edit drops a question still on its way.
    pub(super) fn auto_insert_edited(&mut self, tab: TabId, edits: &[TextEdit], kind: EditKind) {
        self.cancel_auto_insert();
        let Some(typed) = (match (kind, edits) { (EditKind::Typing, [edit]) => edit.text.chars().last(), _ => None }) else { return };
        let Some(view) = self.state.view(tab) else { return };
        if view.tab().extra_selections().is_empty() && self.state.semantics.auto_inserts(view.snapshot().source(), typed) { self.auto_insert_due = Some(tab); }
    }

    /// Ask `tab`'s server what to insert at its caret, when it is still the active tab with one caret.
    pub(super) fn auto_insert_at_caret(&mut self, tab: TabId) -> Effect {
        if self.state.active != Some(tab) { return Effect::default(); }
        let Some(at) = self.state.selection().filter(|s| s.anchor == s.head) else { return Effect::default() };
        let Some(snapshot) = self.state.view(tab).filter(|v| v.tab().extra_selections().is_empty()).map(|v| v.snapshot().clone()) else { return Effect::default() };
        self.cancel_auto_insert();
        self.semantic_seq += 1;
        let request = self.semantic_seq;
        self.semantic_wait.remove(&semantics::Lane::AutoInsert);
        self.pending_auto_insert = Some(AutoInsertRequest { request, tab, source: SemanticSource::new(&snapshot), caret: at.head });
        // The typed version is usually still converting: asked once it is admitted.
        let Some(doc) = self.admitted_document(tab, Priority::Visible) else {
            self.defer_semantic(tab, request, at.head, semantics::Question::AutoInsert, None);
            return Effect::default();
        };
        self.ask_auto_insert(doc)
    }

    /// Send the pending question about `doc` (the version it was typed in); it is dropped when it cannot be asked.
    pub(super) fn ask_auto_insert(&mut self, doc: Arc<Document>) -> Effect {
        let Some((request, caret)) = self.pending_auto_insert.as_ref().map(|p| (p.request, p.caret)) else { return Effect::default() };
        self.send_admitted(&doc);
        if self.state.semantics.ask(&doc, caret, request, semantics::Question::AutoInsert).is_err() { self.pending_auto_insert = None; }
        Effect::default()
    }

    /// Stop waiting (a late answer is then dropped).
    pub(super) fn cancel_auto_insert(&mut self) {
        if self.pending_auto_insert.take().is_some() {
            self.semantic_wait.remove(&semantics::Lane::AutoInsert);
            self.state.semantics.cancel_lane(semantics::Lane::AutoInsert);
        }
    }

    /// The server answered: its snippet is inserted at the caret, as its own transaction, while the text, the caret
    /// and the server session are those it was asked about; nothing (or anything else) inserts nothing.
    pub(super) fn auto_insert_reply(&mut self, reply: semantics::Reply) -> Effect {
        let Some(pending) = self.pending_auto_insert.take_if(|p| p.request == reply.request) else { return Effect::default() };
        if !pending.current(&self.state) || !self.state.semantics.can_ask(reply.session) { return Effect::default(); }
        let Ok(serde_json::Value::String(body)) = reply.result else { return Effect::default() };
        let Ok(snippet) = crate::snippet::Snippet::parse(&body) else { return Effect::default() };
        if body.is_empty() || self.state.view(pending.tab).is_none_or(|v| v.edit_disabled().is_some()) { return Effect::default(); }
        // In a template: inserted only where its server's view maps, without changing the template's own tags.
        let Some(view) = self.admitted_document(pending.tab, Priority::Visible).and_then(|d| self.state.semantics.served(&d, reply.session)) else { return Effect::default() };
        self.insert_snippet(pending.tab, pending.source.key, pending.caret..pending.caret, &snippet, Vec::new(), Some(super::snippet::SnippetGuard { view: &view, typed: &[], label: "Closing tag" }))
    }
}
