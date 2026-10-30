//! Parameter information: the signatures of the call around the caret, with the active parameter, fenced like
//! completion to the exact tab, draft version, caret and server session that answered. It is asked by command or
//! by typing a server's trigger character, asked again by typing while it shows, and hidden the moment it is no
//! longer about the text and caret on screen; a late answer never brings back one that was closed.
use super::*;
use std::ops::Range;

/// One parameter: where its name is in the signature's label (an empty range when the server's label for it
/// cannot be found there), and its documentation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Parameter {
    pub label: Range<usize>,
    pub documentation: Option<String>,
}

/// One signature of the called name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Signature {
    pub label: String,
    pub documentation: Option<String>,
    pub parameters: Vec<Parameter>,
    /// The parameter the caret is in, when the server says.
    pub active_parameter: Option<usize>,
}

/// The parameter information shown.
#[derive(Clone, Debug)]
pub struct SignatureHelp {
    pub tab: TabId,
    source: SemanticSource,
    caret: usize,
    session: u64,
    pub signatures: Vec<Signature>,
    /// The signature the server matched to the call (overloads), counting from zero.
    pub active: usize,
}

impl SignatureHelp {
    pub(super) fn local(tab: TabId, source: SemanticSource, caret: usize, signatures: Vec<Signature>) -> Option<Self> {
        (!signatures.is_empty()).then_some(Self { tab, source, caret, session: 0, signatures, active: 0 })
    }
    pub fn caret(&self) -> usize { self.caret }
    pub fn active_signature(&self) -> Option<&Signature> { self.signatures.get(self.active) }
}

/// The request being answered.
pub(super) struct SignatureRequest { request: u64, tab: TabId, source: SemanticSource, doc: Option<Arc<Document>>, caret: usize, manual: bool }

impl SignatureRequest {
    pub(super) fn current(&self, state: &State) -> bool {
        state.active == Some(self.tab) && state.view(self.tab).is_some_and(|v| self.source.matches(v.snapshot()))
            && state.selection().is_some_and(|s| s.anchor == self.caret && s.head == self.caret)
    }
    pub(super) fn request(&self) -> u64 { self.request }
    pub(super) fn manual(&self) -> bool { self.manual }
    pub(super) fn admitted(&mut self, doc: &Arc<Document>) { self.doc = Some(doc.clone()); }
}

impl State {
    /// The parameter information, while it is about the active tab's version as it is, with the caret where it was.
    pub fn signature_help(&self) -> Option<&SignatureHelp> {
        let h = self.signature.as_ref()?;
        (self.active == Some(h.tab) && (h.session == 0 || self.semantics.can_ask(h.session)) && self.view(h.tab).is_some_and(|v| h.source.matches(v.snapshot()))
            && self.selection().is_some_and(|s| s.anchor == h.caret && s.head == h.caret)).then_some(h)
    }
}

impl Session {
    /// Ask for the signatures of the call at the active caret (`manual`: the command, which reports what it finds).
    pub(super) fn signature_at_caret(&mut self, manual: bool) -> Effect {
        if manual && let Some(why) = self.disabled(CommandId::ParameterInfo) {
            self.state.messages.push(format!("Parameter info: {why}"));
            return Effect { repaint: true, ..Effect::default() };
        }
        if self.state.current_source().is_some_and(|d| d.language() == Some(Language::Sql)) { return self.sql_request(sql_work::Operation::Signature(manual)); }
        let (Some(tab), Some(at)) = (self.state.active, self.state.selection()) else { return Effect::default() };
        if at.anchor != at.head { return Effect::default(); }
        let Some(snapshot) = self.state.view(tab).map(|v| v.snapshot().clone()) else { return Effect::default() };
        self.cancel_signature();
        self.sync_semantics();
        self.semantic_seq += 1;
        let request = self.semantic_seq;
        self.semantic_wait.remove(&semantics::Lane::Signature);
        let mut pending = SignatureRequest { request, tab, source: SemanticSource::new(&snapshot), doc: None, caret: at.head, manual };
        let Some(doc) = self.admitted_document(tab, Priority::Visible) else {
            self.pending_signature = Some(pending);
            self.defer_semantic(tab, request, at.head, semantics::Question::SignatureHelp, None);
            return Effect::default();
        };
        self.send_admitted(&doc);
        let asked = self.state.semantics.ask(&doc, at.head, request, semantics::Question::SignatureHelp);
        match asked {
            Ok(()) => { pending.doc = Some(doc); self.pending_signature = Some(pending); Effect::default() }
            Err(why) => self.signature_unavailable(manual, why),
        }
    }

    pub(super) fn signature_unavailable(&mut self, manual: bool, why: String) -> Effect {
        self.cancel_signature();
        if !manual { return Effect::default(); }
        self.state.messages.push(format!("Parameter info: {why}"));
        Effect { repaint: true, ..Effect::default() }
    }

    /// Stop waiting for parameter information (a late reply is then dropped).
    pub(super) fn cancel_signature(&mut self) {
        self.sql_cancel(2);
        if self.pending_signature.take().is_some() {
            self.semantic_wait.remove(&semantics::Lane::Signature);
            self.state.semantics.cancel_lane(semantics::Lane::Signature);
        }
    }

    /// Escape, or the command again: hidden, and an answer still on its way is dropped.
    pub(super) fn close_signature(&mut self) -> Effect {
        self.cancel_signature();
        Effect { repaint: self.state.signature.take().is_some(), ..Effect::default() }
    }

    /// Signatures arrived: shown when they answer the latest request about the active tab's version as it is.
    pub(super) fn signature_reply(&mut self, reply: semantics::Reply) -> Effect {
        let Some(pending) = self.pending_signature.take_if(|p| p.request == reply.request) else { return Effect::default() };
        if !pending.current(&self.state) || !self.state.semantics.can_ask(reply.session) { return Effect::default(); }
        let value = match reply.result {
            Ok(v) => v,
            Err(why) => { self.state.signature = None; return self.signature_unavailable(pending.manual, why); }
        };
        let signatures: Vec<Signature> = value["signatures"].as_array().map(Vec::as_slice).unwrap_or_default().iter()
            .filter_map(|s| signature(s, &value["activeParameter"], reply.encoding)).collect();
        if signatures.is_empty() {
            self.state.signature = None;
            if !pending.manual { return Effect { repaint: true, ..Effect::default() }; }
            return self.signature_unavailable(true, "No parameter information here".into());
        }
        let active = value["activeSignature"].as_u64().map_or(0, |a| a as usize).min(signatures.len() - 1);
        self.state.signature = Some(SignatureHelp { tab: pending.tab, source: pending.source, caret: pending.caret, session: reply.session, signatures, active });
        Effect { repaint: true, ..Effect::default() }
    }

    /// After an edit of `tab`: a typed trigger character asks for parameter information, and any typing while it
    /// shows asks again (the server closes it by answering nothing once the caret has left the call).
    pub(super) fn signature_edited(&mut self, tab: TabId, edits: &[crate::text::TextEdit], kind: crate::text::EditKind) {
        let showing = self.state.signature.as_ref().is_some_and(|h| h.tab == tab) || self.pending_signature.as_ref().is_some_and(|p| p.tab == tab);
        // Typing `(` may insert its pair too, as a second edit: the whole transaction is what was typed.
        if !matches!(kind, crate::text::EditKind::Typing | crate::text::EditKind::Delete) {
            if showing { self.close_signature(); }
            return;
        }
        let typed: String = edits.iter().map(|e| e.text.as_ref()).collect();
        self.signature_typed(tab, &typed);
    }

    /// `typed` was typed in `tab`, however it was applied (a delimiter opening a pair is a structural edit, and one
    /// typed over its own closing pair only moves the caret): a trigger asks for parameter information, and anything
    /// typed while it shows asks again. Asked once the command is over, when the caret is where the command left it.
    pub(super) fn signature_typed(&mut self, tab: TabId, typed: &str) {
        let showing = self.state.signature.as_ref().is_some_and(|h| h.tab == tab) || self.pending_signature.as_ref().is_some_and(|p| p.tab == tab);
        let Some(path) = self.state.view(tab).map(|v| v.snapshot().source().to_owned()) else { return };
        if self.state.view(tab).is_some_and(|v| v.snapshot().language() == Some(Language::Sql)) {
            if showing || typed.ends_with(['(', ',']) { self.signature_due = true; }
            return;
        }
        let (triggers, retriggers) = self.state.semantics.signature_triggers(&path);
        let last = typed.chars().last();
        let trigger = typed.chars().any(|c| triggers.contains(&c));
        if trigger || showing && last.is_none_or(|c| retriggers.contains(&c) || c != '\n') { self.signature_due = true; }
    }
}

/// One signature from the server, its parameter labels as byte ranges of its own label.
pub(super) fn signature(s: &serde_json::Value, active: &serde_json::Value, encoding: semantics::lsp::Encoding) -> Option<Signature> {
    let label = s["label"].as_str()?.to_owned();
    let mut from = 0;
    let parameters = s["parameters"].as_array().map(Vec::as_slice).unwrap_or_default().iter().map(|p| {
        let range = match &p["label"] {
            serde_json::Value::String(name) => label.get(from..).and_then(|rest| rest.find(name.as_str())).map(|at| from + at..from + at + name.len()),
            serde_json::Value::Array(pair) => pair.first().and_then(serde_json::Value::as_u64).zip(pair.get(1).and_then(serde_json::Value::as_u64))
                .and_then(|(a, b)| Some(byte_of(&label, a as usize, encoding)?..byte_of(&label, b as usize, encoding)?)).filter(|r| r.start <= r.end),
            _ => None,
        };
        if let Some(r) = &range { from = r.end; }
        Parameter { label: range.unwrap_or(0..0), documentation: documentation(&p["documentation"]) }
    }).collect::<Vec<_>>();
    let active_parameter = s["activeParameter"].as_u64().or_else(|| active.as_u64()).map(|a| a as usize).filter(|a| *a < parameters.len());
    Some(Signature { documentation: documentation(&s["documentation"]), parameters, active_parameter, label })
}

/// The byte offset of `units` code units into `text` in the negotiated encoding; `None` inside a character or past
/// the end.
fn byte_of(text: &str, units: usize, encoding: semantics::lsp::Encoding) -> Option<usize> {
    let mut counted = 0;
    for (at, c) in text.char_indices() {
        if counted == units { return Some(at); }
        counted += match encoding { semantics::lsp::Encoding::Utf8 => c.len_utf8(), semantics::lsp::Encoding::Utf16 => c.len_utf16() };
        if counted > units { return None; }
    }
    (counted == units).then_some(text.len())
}

/// Documentation as text: a string, or the value of markup content.
pub(super) fn documentation(v: &serde_json::Value) -> Option<String> {
    let text = match v {
        serde_json::Value::String(s) => s.trim(),
        serde_json::Value::Object(o) => o.get("value")?.as_str()?.trim(),
        _ => return None,
    };
    (!text.is_empty()).then(|| text.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parameter_labels_are_found_by_name_or_by_offsets_in_the_negotiated_encoding() {
        let s = serde_json::json!({ "label": "fn größe(a: i32, a2: &str)", "documentation": { "kind": "markdown", "value": "Size." },
            "parameters": [{ "label": "a" }, { "label": [17, 25], "documentation": "Second." }] });
        let sig = signature(&s, &serde_json::json!(1), semantics::lsp::Encoding::Utf16).unwrap();
        // UTF-16 offsets 17..25 cover "a2: &str" after the two-byte characters.
        assert_eq!(&sig.label[sig.parameters[1].label.clone()], "a2: &str");
        assert_eq!(&sig.label[sig.parameters[0].label.clone()], "a", "a name is found after the name it follows, not inside `größe`");
        assert_eq!((sig.active_parameter, sig.documentation.as_deref(), sig.parameters[1].documentation.as_deref()), (Some(1), Some("Size."), Some("Second.")));
        // In UTF-8, byte 6 of `fn größe(x)` is inside `ö`: such offsets are not placed.
        let split = serde_json::json!({ "label": "fn größe(x)", "parameters": [{ "label": [0, 6] }] });
        let utf8 = signature(&split, &serde_json::Value::Null, semantics::lsp::Encoding::Utf8).unwrap();
        assert_eq!(utf8.parameters[0].label, 0..0, "offsets that split a character are not placed");
        assert_eq!(utf8.active_parameter, None);
        let out_of_range = signature(&serde_json::json!({ "label": "f(x)", "parameters": [{ "label": "x" }], "activeParameter": 5 }), &serde_json::Value::Null, semantics::lsp::Encoding::Utf16).unwrap();
        assert_eq!(out_of_range.active_parameter, None);
    }
}
