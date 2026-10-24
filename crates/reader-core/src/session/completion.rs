//! Code completion: a language server's suggestions for the name at the caret, in one popup fenced to the
//! exact draft version, tab and server session that produced them. Typing that continues the name re-filters
//! the popup against the new version; anything else closes it, and a reply nobody is waiting for is dropped.
use super::*;
use crate::snippet::Snippet;
use crate::text::{EditKind, TextEdit};
use std::ops::Range;

/// What opened the popup: the command (it reports an empty or unavailable answer) or typing (it stays quiet).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompletionTrigger {
    Manual,
    Typing,
}

/// One suggestion: its label, and the plain text replacing its range of the shown version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Suggestion {
    pub label: String,
    pub detail: Option<String>,
    /// Why accepting it is unavailable here; it is listed, never partially applied.
    pub unavailable: Option<&'static str>,
    filter: String,
    text: Arc<str>,
    replace: Range<usize>,
    /// A snippet body (a plain suggestion inserts `text` as written).
    snippet: Option<Snippet>,
}

/// The open completion popup.
#[derive(Clone, Debug)]
pub struct Suggestions {
    pub tab: TabId,
    pub trigger: CompletionTrigger,
    session: u64,
    source: SemanticSource,
    caret: usize,
    word: usize,
    incomplete: bool,
    items: Vec<Suggestion>,
    shown: Vec<usize>,
    selected: usize,
}

impl Suggestions {
    /// The suggestions matching what has been typed, best first.
    pub fn rows(&self) -> impl ExactSizeIterator<Item = &Suggestion> { self.shown.iter().map(|i| &self.items[*i]) }
    pub fn selected(&self) -> usize { self.selected }
    /// The caret it completes at, and the start of the name being completed (where the popup is anchored).
    pub fn caret(&self) -> usize { self.caret }
    pub fn word(&self) -> usize { self.word }

    fn filter(&mut self, typed: &str) {
        let typed = typed.to_lowercase();
        let selected = self.shown.get(self.selected).copied();
        self.shown = (0..self.items.len()).filter(|i| subsequence(&typed, &self.items[*i].filter)).collect();
        self.selected = selected.and_then(|s| self.shown.iter().position(|i| *i == s)).unwrap_or(0);
    }
}

fn subsequence(typed: &str, text: &str) -> bool {
    let mut text = text.chars();
    typed.chars().all(|c| text.any(|t| t == c))
}

pub(super) fn word_char(c: char) -> bool { c.is_alphanumeric() || c == '_' }

/// The request being answered: its tab and exact version, the document converted against, and the caret.
pub(super) struct CompletionRequest { request: u64, tab: TabId, source: SemanticSource, doc: Option<Arc<Document>>, caret: usize, trigger: CompletionTrigger }

impl CompletionRequest {
    pub(super) fn current(&self, state: &State) -> bool {
        state.active == Some(self.tab) && state.view(self.tab).is_some_and(|v| self.source.matches(v.snapshot()))
            && state.selection().is_some_and(|s| s.anchor == self.caret && s.head == self.caret)
    }
    pub(super) fn request(&self) -> u64 { self.request }
    pub(super) fn trigger(&self) -> CompletionTrigger { self.trigger }
    pub(super) fn admitted(&mut self, doc: &Arc<Document>) { self.doc = Some(doc.clone()); }
}

impl State {
    /// The completion popup, while it is about the active tab's version as it is, with the caret where it was.
    pub fn completion(&self) -> Option<&Suggestions> {
        let c = self.completion.as_ref()?;
        (self.active == Some(c.tab) && self.semantics.can_ask(c.session) && self.view(c.tab).is_some_and(|v| c.source.matches(v.snapshot()))
            && self.selection().is_some_and(|s| s.anchor == c.caret && s.head == c.caret)).then_some(c)
    }
    /// Whether typing a name opens completion by itself (the setting; the command always works).
    pub fn auto_completion(&self) -> bool { self.settings.auto_completion }
}

impl Session {
    /// Ask for completion at the active caret, replacing any request before.
    pub(super) fn complete_at_caret(&mut self, trigger: CompletionTrigger) -> Effect {
        if trigger == CompletionTrigger::Manual && let Some(why) = self.disabled(CommandId::Completion) {
            self.state.messages.push(format!("Code completion: {why}"));
            return Effect { repaint: true, ..Effect::default() };
        }
        let (Some(tab), Some(at)) = (self.state.active, self.state.selection()) else { return Effect::default() };
        if at.anchor != at.head { return Effect::default(); }
        let Some(snapshot) = self.state.view(tab).map(|v| v.snapshot().clone()) else { return Effect::default() };
        self.cancel_completion();
        self.sync_semantics();
        self.semantic_seq += 1;
        let request = self.semantic_seq;
        self.semantic_wait.remove(&semantics::Lane::Completion);
        let mut pending = CompletionRequest { request, tab, source: SemanticSource::new(&snapshot), doc: None, caret: at.head, trigger };
        let Some(doc) = self.admitted_document(tab, Priority::Visible) else {
            self.pending_completion = Some(pending);
            self.defer_semantic(tab, request, at.head, semantics::Question::Completion, None);
            return Effect::default();
        };
        match self.state.semantics.ask(&doc, at.head, request, semantics::Question::Completion) {
            Ok(()) => { pending.doc = Some(doc); self.pending_completion = Some(pending); Effect::default() }
            Err(why) => self.completion_unavailable(trigger, why),
        }
    }

    pub(super) fn completion_unavailable(&mut self, trigger: CompletionTrigger, why: String) -> Effect {
        self.cancel_completion();
        if trigger != CompletionTrigger::Manual { return Effect::default(); }
        self.state.messages.push(format!("Code completion: {why}"));
        Effect { repaint: true, ..Effect::default() }
    }

    /// Stop waiting for suggestions (a late reply is then dropped).
    pub(super) fn cancel_completion(&mut self) {
        if self.pending_completion.take().is_some() {
            self.semantic_wait.remove(&semantics::Lane::Completion);
            self.state.semantics.cancel_lane(semantics::Lane::Completion);
        }
    }

    pub(super) fn close_completion(&mut self) -> Effect {
        self.cancel_completion();
        Effect { repaint: self.state.completion.take().is_some(), ..Effect::default() }
    }

    /// Suggestions arrived: shown when they answer the latest request about the active tab's version as it is.
    pub(super) fn completion_reply(&mut self, reply: semantics::Reply) -> Effect {
        let Some(pending) = self.pending_completion.take_if(|p| p.request == reply.request) else { return Effect::default() };
        let (Some(doc), true) = (pending.doc.clone(), pending.current(&self.state) && self.state.semantics.can_ask(reply.session)) else { return Effect::default() };
        let value = match reply.result {
            Ok(v) => v,
            Err(why) => return self.completion_unavailable(pending.trigger, why),
        };
        let Some(snapshot) = self.state.view(pending.tab).map(|v| v.snapshot().clone()) else { return Effect::default() };
        let line = snapshot.line_range(snapshot.line_of(pending.caret)).map_or(pending.caret, |l| l.start);
        let before = snapshot.range(line..pending.caret).unwrap_or_default();
        let word = pending.caret - before.chars().rev().take_while(|c| word_char(*c)).map(char::len_utf8).sum::<usize>();
        let (items, incomplete) = match &value {
            serde_json::Value::Array(items) => (items.as_slice(), false),
            v => (v["items"].as_array().map_or(&[][..], |i| i.as_slice()), v["isIncomplete"].as_bool().unwrap_or(false)),
        };
        let mut items: Vec<(String, Suggestion)> = items.iter().filter_map(|item| suggestion(item, &doc, reply.encoding, word..pending.caret, pending.caret)).collect();
        items.sort_by(|a, b| a.0.cmp(&b.0));
        let mut popup = Suggestions { tab: pending.tab, trigger: pending.trigger, session: reply.session, source: pending.source, caret: pending.caret, word, incomplete,
            items: items.into_iter().map(|(_, s)| s).collect(), shown: Vec::new(), selected: 0 };
        popup.filter(before.get(word - line..).unwrap_or_default());
        if popup.shown.is_empty() {
            self.state.completion = None;
            return self.completion_unavailable(pending.trigger, "No suggestions".into());
        }
        self.state.completion = Some(popup);
        Effect { repaint: true, ..Effect::default() }
    }

    pub(super) fn move_completion(&mut self, by: isize) -> Effect {
        let Some(n) = self.state.completion().map(|c| c.shown.len()) else { return Effect::default() };
        let Some(c) = self.state.completion.as_mut() else { return Effect::default() };
        c.selected = (c.selected as isize + by).rem_euclid(n as isize) as usize;
        Effect { repaint: true, ..Effect::default() }
    }

    pub(super) fn select_completion(&mut self, row: usize) -> Effect {
        if self.state.completion().is_none_or(|c| row >= c.shown.len()) { return Effect::default(); }
        if let Some(c) = self.state.completion.as_mut() { c.selected = row; }
        Effect { repaint: true, ..Effect::default() }
    }

    /// Replace the name with the selected suggestion: one transaction against the version shown. A snippet body is
    /// expanded there instead, with the file's indentation and its first placeholder selected.
    pub(super) fn accept_completion(&mut self) -> Effect {
        let Some(c) = self.state.completion() else { return Effect::default() };
        let Some(item) = c.shown.get(c.selected).map(|i| &c.items[*i]) else { return Effect::default() };
        if let Some(why) = item.unavailable {
            self.state.messages.push(format!("Code completion: {why}"));
            return Effect { repaint: true, ..Effect::default() };
        }
        let (tab, expected) = (c.tab, c.source.key);
        let range = item.replace.start..item.replace.end.max(c.caret);
        let (text, snippet) = (item.text.clone(), item.snippet.clone());
        // A live snippet's placeholders belong to one caret; with several there is no set Tab could visit.
        if snippet.is_some() && self.state.view(tab).is_some_and(|view| !view.tab().extra_selections().is_empty()) {
            self.state.messages.push("Code completion: a snippet needs one caret; Escape collapses several selections".into());
            return Effect { repaint: true, ..Effect::default() };
        }
        self.close_completion();
        match snippet {
            Some(snippet) => self.insert_snippet(tab, expected, range, &snippet),
            None => self.edit(tab, expected, vec![TextEdit { range, text }], EditKind::Replace),
        }
    }

    /// After a typing edit of `tab`: keep the popup when the edit continues the name at its caret (re-filtered,
    /// asked again when its list was incomplete), else close it; then open it by typing when the setting allows.
    pub(super) fn completion_edited(&mut self, tab: TabId, edits: &[TextEdit], kind: EditKind) {
        let typed = match (kind, edits) {
            (EditKind::Typing | EditKind::Delete, [edit]) => Some(edit),
            _ => None,
        };
        let Some(snapshot) = self.state.view(tab).map(|v| v.snapshot().clone()) else { return };
        let mut ask = false;
        if let Some(c) = self.state.completion.as_mut().filter(|c| c.tab == tab) {
            let kept = typed.filter(|e| e.range.end == c.caret && e.range.start >= c.word && e.text.chars().all(word_char));
            match kept {
                Some(e) => {
                    let sticky = |b: usize| if b <= e.range.start { b } else { crate::text::edited_byte(b, edits) };
                    for item in &mut c.items { item.replace = sticky(item.replace.start)..crate::text::edited_byte(item.replace.end, edits); }
                    c.caret = e.range.start + e.text.len();
                    c.source = SemanticSource::new(&snapshot);
                    let typed = snapshot.range(c.word..c.caret).unwrap_or_default();
                    c.filter(&typed);
                    ask = c.incomplete;
                    if c.shown.is_empty() && !c.incomplete { self.state.completion = None; }
                }
                None => { self.state.completion = None; }
            }
        }
        let auto = self.state.settings.auto_completion && self.state.completion.is_none() && typed.is_some_and(|e| kind == EditKind::Typing
            && e.text.chars().last().is_some_and(|last| word_char(last) || self.state.semantics.completion_triggers(snapshot.source()).contains(&last)));
        if ask || auto {
            let trigger = self.state.completion.as_ref().map_or(CompletionTrigger::Typing, |c| c.trigger);
            let open = self.state.completion.take();
            self.complete_at_caret(trigger);
            // An incomplete list keeps showing while its successor is asked.
            if open.is_some() { self.state.completion = open; }
        } else if self.pending_completion.as_ref().is_some_and(|p| p.tab == tab) { self.cancel_completion(); }
    }

    pub(super) fn set_auto_completion(&mut self, on: bool) -> Effect {
        self.state.settings.auto_completion = on;
        self.save_settings();
        Effect { repaint: true, ..Effect::default() }
    }
}

/// An item as a suggestion: its sort key, and its replacement converted against `doc` (the version asked).
fn suggestion(item: &serde_json::Value, doc: &Document, encoding: semantics::lsp::Encoding, word: Range<usize>, caret: usize) -> Option<(String, Suggestion)> {
    let label = item["label"].as_str()?.to_owned();
    let edit = &item["textEdit"];
    let range = if edit.is_object() { Some(&edit["range"]).filter(|r| r.is_object()).or(Some(&edit["insert"])) } else { None };
    let replace = match range {
        Some(r) => semantics::lsp::offset(doc, &r["start"], encoding).zip(semantics::lsp::offset(doc, &r["end"], encoding)).map(|(a, b)| a..b),
        None => Some(word),
    };
    let text: Arc<str> = edit["newText"].as_str().or(item["insertText"].as_str()).unwrap_or(&label).into();
    let client_hint = |c: &serde_json::Value| matches!(c["command"].as_str(), Some("editor.action.triggerParameterHints" | "editor.action.triggerSuggest"));
    // A snippet body is parsed now, so the list can say why one will not be inserted.
    let (snippet, snippet_why) = match item["insertTextFormat"].as_u64() {
        Some(2) => match Snippet::parse(&text) {
            Ok(snippet) => (Some(snippet), None),
            Err(why) => (None, Some(why)),
        },
        _ => (None, None),
    };
    let unavailable = match &replace {
        Some(r) if r.start <= caret && caret <= r.end && semantics::lsp::line_of(doc, r.start) == semantics::lsp::line_of(doc, caret) => None,
        _ => Some("The language server's replacement range does not contain the caret"),
    }.or_else(|| item["additionalTextEdits"].as_array().is_some_and(|e| !e.is_empty()).then_some("Suggestions with import or other extra edits are not supported yet"))
        .or(snippet_why)
        .or_else(|| (item["command"].is_object() && !client_hint(&item["command"])).then_some("Suggestions that run a language-server command are not supported yet"));
    let filter = item["filterText"].as_str().unwrap_or(&label).to_lowercase();
    let sort = item["sortText"].as_str().unwrap_or(&label).to_owned();
    Some((sort, Suggestion { label, detail: item["detail"].as_str().map(str::to_owned), unavailable, filter, text, replace: replace.unwrap_or(caret..caret), snippet }))
}
