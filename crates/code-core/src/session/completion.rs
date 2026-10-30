//! Code completion: a language server's suggestions for the name at the caret, in one popup fenced to the
//! exact draft version, tab and server session that produced them. Typing that continues the name re-filters
//! the popup against the new version; anything else closes it, and a reply nobody is waiting for is dropped.
//! Accepting applies the name, its import edits and a snippet body as one transaction of one file, or nothing.
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
    /// The language server's kind, shown without changing its ranking.
    pub kind: Option<&'static str>,
    pub detail: Option<String>,
    /// Its documentation, from the list or asked for once while it is the browsed suggestion.
    pub documentation: Option<String>,
    /// Its documentation was asked for (an empty answer is not asked again).
    described: bool,
    markdown: bool,
    /// Its place in the reply as sent.
    index: usize,
    /// Why accepting it is unavailable here; it is listed, never partially applied.
    pub unavailable: Option<&'static str>,
    filter: String,
    text: Arc<str>,
    replace: Range<usize>,
    /// A snippet body (a plain suggestion inserts `text` as written).
    snippet: Option<Snippet>,
    /// Its other edits of the same file (imports), as ranges of the version the popup was asked about.
    extra: Vec<TextEdit>,
    /// Its place in the reply, when the server fills in its imports only once it is chosen.
    resolve: Option<usize>,
    /// An allowlisted command of its server session, run once its text is applied (the server inserts it then).
    command: Option<(u64, serde_json::Value)>,
    /// Its server asks for suggestions again once it is applied (`editor.action.triggerSuggest`: a CSS property's
    /// values after the property).
    suggest_again: bool,
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
    query: String,
    preview_at: Option<(usize, SemanticSource)>,
    preview: Option<Arc<crate::markdown::Preview>>,
    /// Lenz's own suggestions (a Markdown link's destination), with no server behind them.
    local: bool,
    /// The version asked about, its position encoding, the reply's items as sent, and the typing since, each edit
    /// set in order: what the server's ranges are carried through (no version for Lenz's own suggestions).
    doc: Option<Arc<Document>>,
    encoding: semantics::lsp::Encoding,
    raw: Arc<serde_json::Value>,
    typed: Vec<Vec<TextEdit>>,
}

/// An accepted suggestion waiting for its server to fill in its imports.
pub(super) struct ResolveWait {
    request: u64,
    tab: TabId,
    expected: crate::text::SnapshotKey,
    caret: usize,
    item: Suggestion,
    doc: Arc<Document>,
    encoding: semantics::lsp::Encoding,
    typed: Vec<Vec<TextEdit>>,
}

/// Why an item's extra edits are not applied: they are not edits of this file this client can place exactly.
const MALFORMED: &str = "The language server's extra edits are malformed or overlap";
const OVERLAP: &str = "The language server's extra edits overlap the completed name";

impl Suggestions {
    /// The suggestions matching what has been typed, best first.
    pub fn rows(&self) -> impl ExactSizeIterator<Item = &Suggestion> { self.shown.iter().map(|i| &self.items[*i]) }
    pub fn selected(&self) -> usize { self.selected }
    /// The caret it completes at, and the start of the name being completed (where the popup is anchored).
    pub fn caret(&self) -> usize { self.caret }
    pub fn word(&self) -> usize { self.word }
    /// The selected documentation, parsed and highlighted once by the shared worker.
    pub fn documentation_preview(&self) -> Option<&Arc<crate::markdown::Preview>> { self.preview.as_ref() }
    /// Matching UTF-8 ranges in a label; filtering and ranking remain the server's.
    pub fn matches(&self, label: &str) -> Vec<Range<usize>> {
        let mut letters = label.char_indices().flat_map(|(at, c)| c.to_lowercase().map(move |c2| (at..at + c.len_utf8(), c2)));
        let mut ranges: Vec<Range<usize>> = Vec::new();
        for c in self.query.chars() {
            let Some((range, _)) = letters.find(|(_, letter)| *letter == c) else { return Vec::new() };
            if let Some(last) = ranges.last_mut().filter(|last| last.end >= range.start) { last.end = last.end.max(range.end); }
            else { ranges.push(range); }
        }
        ranges
    }

    fn filter(&mut self, typed: &str) {
        let typed = typed.to_lowercase();
        self.query = typed.clone();
        let selected = self.shown.get(self.selected).copied();
        self.shown = (0..self.items.len()).filter(|i| subsequence(&typed, &self.items[*i].filter)).collect();
        self.selected = selected.and_then(|s| self.shown.iter().position(|i| *i == s)).unwrap_or(0);
    }
}

impl Suggestion {
    /// One of Lenz's own suggestions: `text` replaces `replace` (bytes of the version shown); `again` asks for
    /// suggestions once it is applied (a folder's, to go on into it).
    pub(super) fn local(label: String, detail: Option<String>, text: &str, replace: Range<usize>, filter: &str, again: bool) -> Self {
        Self { label, kind: None, detail, documentation: None, described: true, markdown: false, index: 0, unavailable: None, filter: filter.to_lowercase(), text: text.into(), replace,
            snippet: None, extra: Vec::new(), resolve: None, command: None, suggest_again: again }
    }
}

impl Suggestions {
    /// A popup of Lenz's own suggestions for the name from `word` to `caret` of `snapshot`, filtered by what is typed
    /// there already; `None` when nothing matches.
    pub(super) fn local(tab: TabId, trigger: CompletionTrigger, snapshot: &crate::text::TextSnapshot, (word, caret): (usize, usize), items: Vec<Suggestion>) -> Option<Self> {
        let mut popup = Self { tab, trigger, session: 0, source: SemanticSource::new(snapshot), caret, word, incomplete: false, items, shown: Vec::new(), selected: 0, query: String::new(), preview_at: None, preview: None,
            local: true, doc: None, encoding: semantics::lsp::Encoding::Utf16, raw: Arc::new(serde_json::Value::Null), typed: Vec::new() };
        popup.filter(&snapshot.range(word..caret).unwrap_or_default());
        (!popup.shown.is_empty()).then_some(popup)
    }
}

fn subsequence(typed: &str, text: &str) -> bool {
    let mut text = text.chars();
    typed.chars().all(|c| text.any(|t| t == c))
}

pub(super) fn word_char(c: char) -> bool { c.is_alphanumeric() || c == '_' }

fn blank_prefix(snapshot: &crate::text::TextSnapshot, caret: usize) -> bool {
    let start = snapshot.reader_line_range(snapshot.line_of(caret)).start;
    if caret > start && snapshot.range(snapshot.previous_char_boundary(caret)..caret)
        .is_some_and(|text| text.chars().any(|c| !c.is_whitespace())) { return false; }
    let mut end = caret;
    while end > start {
        // Bound reads even on an unusually long indentation-only line.
        let from = snapshot.floor_char_boundary(end.saturating_sub(1024).max(start));
        let Some(text) = snapshot.range(from..end) else { return true };
        if text.chars().any(|c| !c.is_whitespace()) { return false; }
        end = from;
    }
    true
}

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
        (self.active == Some(c.tab) && (c.local || self.semantics.can_ask(c.session)) && self.view(c.tab).is_some_and(|v| c.source.matches(v.snapshot()))
            && self.selection().is_some_and(|s| s.anchor == c.caret && s.head == c.caret)).then_some(c)
    }
    /// Whether typing a name opens completion by itself (the setting; the command always works).
    pub fn auto_completion(&self) -> bool { self.settings.auto_completion }
}

impl Session {
    /// Ask for completion at the active caret, replacing any request before.
    pub(super) fn complete_at_caret(&mut self, trigger: CompletionTrigger) -> Effect {
        if trigger == CompletionTrigger::Typing && self.state.current_source().zip(self.state.selection())
            .is_none_or(|(snapshot, at)| blank_prefix(snapshot, at.head)) { return self.close_completion(); }
        if self.state.current_source().is_some_and(|d| d.language() == Some(Language::Sql)) {
            if let Some(why) = self.sql_action_disabled(CommandId::Completion) { return self.completion_unavailable(trigger, why.into()); }
            if self.text_action_disabled(CommandId::Completion).is_some() { return Effect::default(); }
            return self.sql_request(sql_work::Operation::Complete(trigger));
        }
        if let Some(effect) = self.haml_completion(trigger) { return effect; }
        if let Some(effect) = self.link_completion(trigger) { return effect; }
        if trigger == CompletionTrigger::Manual && let Some(why) = self.disabled(CommandId::Completion) {
            self.state.messages.push(format!("Code completion: {why}"));
            return Effect { repaint: true, ..Effect::default() };
        }
        let (Some(tab), Some(at)) = (self.state.active, self.state.selection()) else { return Effect::default() };
        if at.anchor != at.head { return Effect::default(); }
        let Some(snapshot) = self.state.view(tab).map(|v| v.snapshot().clone()) else { return Effect::default() };
        // The same question is already on its way (typing opened it): wait for that answer rather than cancelling it
        // and asking again; a server may not survive the repeat (Ruby LSP 0.26.11 then fails to locate the caret).
        if let Some(p) = self.pending_completion.as_mut().filter(|p| p.tab == tab && p.caret == at.head && p.source.matches(&snapshot)) {
            if trigger == CompletionTrigger::Manual { p.trigger = trigger; }
            return Effect::default();
        }
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
        self.send_admitted(&doc);
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
        self.sql_cancel(1);
        self.cancel_link_suggestions();
        if self.pending_completion.take().is_some() {
            self.semantic_wait.remove(&semantics::Lane::Completion);
            self.state.semantics.cancel_lane(semantics::Lane::Completion);
        }
    }

    pub(super) fn close_completion(&mut self) -> Effect {
        self.cancel_completion();
        if let Some(c) = &self.state.completion { self.analysis.cancel_consumer(c.source.key.buffer, crate::analysis::Consumer::CompletionDocumentation); }
        Effect { repaint: self.state.completion.take().is_some(), ..Effect::default() }
    }

    /// Suggestions arrived: shown when they answer the latest request about the active tab's version as it is.
    pub(super) fn completion_reply(&mut self, reply: semantics::Reply) -> Effect {
        if self.pending_resolve.as_ref().is_some_and(|w| w.request == reply.request) { return self.completion_resolved(reply); }
        let Some(pending) = self.pending_completion.take_if(|p| p.request == reply.request) else { return Effect::default() };
        let (Some(doc), true) = (pending.doc.clone(), pending.current(&self.state) && self.state.semantics.can_ask(reply.session)) else { return Effect::default() };
        // A template's suggestions are about the view that server holds: its ranges convert through that projection.
        let Some(doc) = self.state.semantics.served(&doc, reply.session) else { return Effect::default() };
        let value = match reply.result {
            Ok(v) => v,
            Err(why) => return self.completion_unavailable(pending.trigger, why),
        };
        let Some(snapshot) = self.state.view(pending.tab).map(|v| v.snapshot().clone()) else { return Effect::default() };
        let line = snapshot.line_range(snapshot.line_of(pending.caret)).map_or(pending.caret, |l| l.start);
        let before = snapshot.range(line..pending.caret).unwrap_or_default();
        let word = pending.caret - before.chars().rev().take_while(|c| word_char(*c)).map(char::len_utf8).sum::<usize>();
        let raw = Arc::new(value);
        let (items, incomplete) = match &*raw {
            serde_json::Value::Array(items) => (items.as_slice(), false),
            v => (v["items"].as_array().map_or(&[][..], |i| i.as_slice()), v["isIncomplete"].as_bool().unwrap_or(false)),
        };
        let resolves = self.state.semantics.resolves_completion(reply.session);
        // A shell script is offered its own shell's words: an sh script none of Bash's (the shell as Lenz last identified it).
        let shell = (doc.language() == Some(Language::Bash)).then(|| self.state.semantics.shell_lint(doc.source()).and_then(|l| l.shell.shell.clone().ok())).flatten();
        let mut items: Vec<(String, Suggestion)> = items.iter().enumerate()
            .filter(|(_, item)| shell.is_none_or(|s| !semantics::shell::foreign(s, item)))
            // A template projection's own class and method, which Ruby LSP indexes: never offered.
            .filter(|(_, item)| !item["label"].as_str().is_some_and(crate::template::scaffolding))
            .filter_map(|(i, item)| suggestion(item, &doc, reply.encoding, (word..pending.caret, pending.caret), i, resolves && item.get("additionalTextEdits").is_none_or(serde_json::Value::is_null), (reply.session, &|c: &str| self.state.semantics.runs_command(reply.session, c))))
            .collect();
        items.sort_by(|a, b| a.0.cmp(&b.0));
        let mut popup = Suggestions { tab: pending.tab, trigger: pending.trigger, session: reply.session, source: pending.source, caret: pending.caret, word, incomplete,
            items: items.into_iter().map(|(_, s)| s).collect(), shown: Vec::new(), selected: 0, query: String::new(), preview_at: None, preview: None, local: false, doc: Some(doc.clone()), encoding: reply.encoding, raw, typed: Vec::new() };
        popup.filter(before.get(word - line..).unwrap_or_default());
        if popup.shown.is_empty() {
            self.state.completion = None;
            return self.completion_unavailable(pending.trigger, "No suggestions".into());
        }
        self.state.completion = Some(popup);
        self.describe_selected();
        Effect { repaint: true, ..Effect::default() }
    }

    /// The browsed suggestion's documentation: asked of its server once, when the list did not carry it and the
    /// server fills suggestions in. Only the latest such question is waited for.
    pub(super) fn describe_selected(&mut self) {
        self.preview_selected_completion();
        let Some(c) = self.state.completion() else { return };
        let Some(&at) = c.shown.get(c.selected) else { return };
        let item = &c.items[at];
        if item.documentation.is_some() || item.described || !self.state.semantics.resolves_completion(c.session) { return; }
        let raw = match &*c.raw { serde_json::Value::Array(items) => items.get(item.index), v => v["items"].get(item.index) }.cloned();
        let (Some(raw), session, list) = (raw, c.session, c.raw.clone()) else { return };
        if let Some(c) = self.state.completion.as_mut() { c.items[at].described = true; }
        self.semantic_seq += 1;
        let request = self.semantic_seq;
        if self.state.semantics.describe_completion(session, raw, request).is_ok() { self.pending_detail = Some((request, list, at)); }
    }

    /// A suggestion's documentation arrived: kept when the popup it was asked for is still the one open.
    pub(super) fn completion_described(&mut self, reply: semantics::Reply) -> Effect {
        let Some((_, list, at)) = self.pending_detail.take_if(|p| p.0 == reply.request) else { return Effect::default() };
        let Some(c) = self.state.completion.as_mut().filter(|c| Arc::ptr_eq(&c.raw, &list)) else { return Effect::default() };
        let (Ok(value), Some(item)) = (reply.result, c.items.get_mut(at)) else { return Effect::default() };
        item.documentation = super::signature::documentation(&value["documentation"]);
        item.markdown = value["documentation"]["kind"].as_str() == Some("markdown");
        if item.detail.is_none() { item.detail = value["detail"].as_str().map(str::to_owned); }
        self.preview_selected_completion();
        Effect { repaint: true, ..Effect::default() }
    }

    /// Keep one selected item's preview, with the popup and source identities in its worker ticket.
    pub(super) fn preview_selected_completion(&mut self) {
        let Some(c) = self.state.completion() else { return };
        let Some(&at) = c.shown.get(c.selected) else { return };
        let item = &c.items[at];
        if c.preview_at.as_ref().is_some_and(|(item, source)| *item == at && (c.preview.is_some() || *source == c.source)) { return; }
        let text = item.documentation.as_ref().map(|text| if item.markdown { text.clone() } else { format!("```text\n{text}\n```") });
        let (tab, list) = (c.tab, c.raw.clone());
        let Some(snapshot) = self.state.view(tab).map(|v| v.snapshot().clone()) else { return };
        self.analysis.cancel_consumer(snapshot.key().buffer, crate::analysis::Consumer::CompletionDocumentation);
        let Some(c) = self.state.completion.as_mut() else { return };
        c.preview = None;
        c.preview_at = text.as_ref().map(|_| (at, SemanticSource::new(&snapshot)));
        let Some(text) = text else { return };
        self.semantic_seq += 1;
        let request = self.semantic_seq;
        let loaded = self.state.sources.buffers.get(&snapshot.key().buffer).map(|e| e.document.clone()).unwrap_or_default();
        let input = crate::analysis::Input::new(snapshot, loaded, self.project_generation.load(Ordering::Acquire), true);
        self.analysis.request(input, crate::analysis::Consumer::CompletionDocumentation, request, Priority::Visible, move |_, lease| {
            let preview = crate::markdown::render(text.into(), lease.cancellation()).or_else(|why|
                crate::markdown::render(format!("Documentation cannot be displayed: {why}").into(), lease.cancellation()));
            lease.send(Completion::CompletionDocumentationReady { list, at, preview });
        });
    }

    pub(super) fn completion_documentation_ready(&mut self, list: Arc<serde_json::Value>, at: usize, preview: Result<crate::markdown::Preview, String>) -> Effect {
        if self.state.completion().is_none() { return Effect::default(); }
        let Some(c) = self.state.completion.as_mut().filter(|c| Arc::ptr_eq(&c.raw, &list) && c.preview_at.as_ref().is_some_and(|(item, _)| *item == at) && c.shown.get(c.selected) == Some(&at)) else { return Effect::default() };
        c.preview = preview.ok().map(Arc::new);
        Effect { repaint: true, ..Effect::default() }
    }

    pub(super) fn move_completion(&mut self, by: isize) -> Effect {
        let Some(n) = self.state.completion().map(|c| c.shown.len()) else { return Effect::default() };
        let Some(c) = self.state.completion.as_mut() else { return Effect::default() };
        c.selected = (c.selected as isize + by).rem_euclid(n as isize) as usize;
        self.describe_selected();
        Effect { repaint: true, ..Effect::default() }
    }

    pub(super) fn select_completion(&mut self, row: usize) -> Effect {
        if self.state.completion().is_none_or(|c| row >= c.shown.len()) { return Effect::default(); }
        if let Some(c) = self.state.completion.as_mut() { c.selected = row; }
        self.describe_selected();
        Effect { repaint: true, ..Effect::default() }
    }

    /// Replace the name with the selected suggestion, its import edits included: one transaction against the version
    /// shown, or nothing. A snippet body is expanded there instead, with the file's indentation and its first
    /// placeholder selected. A suggestion whose imports the server fills in on request waits for them first.
    pub(super) fn accept_completion(&mut self) -> Effect {
        let Some(c) = self.state.completion() else { return Effect::default() };
        let Some(item) = c.shown.get(c.selected).map(|i| c.items[*i].clone()) else { return Effect::default() };
        if let Some(why) = item.unavailable {
            self.state.messages.push(format!("Code completion: {why}"));
            return Effect { repaint: true, ..Effect::default() };
        }
        let (tab, expected, caret, session) = (c.tab, c.source.key, c.caret, c.session);
        let (doc, encoding, typed) = (c.doc.clone(), c.encoding, c.typed.clone());
        let raw = item.resolve.and_then(|i| match &*c.raw { serde_json::Value::Array(items) => items.get(i), v => v["items"].get(i) }).cloned();
        self.close_completion();
        let (Some(raw), Some(doc)) = (raw, doc.clone()) else { return self.apply_completion(tab, expected, caret, &item, &typed, doc.as_deref()) };
        self.semantic_seq += 1;
        let request = self.semantic_seq;
        match self.state.semantics.resolve_completion(session, raw, request) {
            Ok(()) => {
                self.pending_resolve = Some(ResolveWait { request, tab, expected, caret, item, doc, encoding, typed });
                Effect { repaint: true, ..Effect::default() }
            }
            Err(why) => {
                self.state.messages.push(format!("Code completion: {why}; nothing was inserted"));
                Effect { repaint: true, ..Effect::default() }
            }
        }
    }

    /// The server filled in an accepted suggestion: applied with its imports while the file is still the version it
    /// was accepted in, else nothing changes and the reason is shown.
    fn completion_resolved(&mut self, reply: semantics::Reply) -> Effect {
        let Some(wait) = self.pending_resolve.take() else { return Effect::default() };
        let refuse = |s: &mut Self, why: &str| { s.state.messages.push(format!("Code completion: {why}; nothing was inserted")); Effect { repaint: true, ..Effect::default() } };
        if self.state.view(wait.tab).is_none_or(|v| v.snapshot().key() != wait.expected) {
            return refuse(self, "the file changed while the suggestion's imports were fetched; accept it again");
        }
        let value = match reply.result {
            Ok(value) => value,
            Err(why) => return refuse(self, &why),
        };
        if let Some(why) = command_refusal(&value["command"], &|c: &str| self.state.semantics.runs_command(reply.session, c)) { return refuse(self, why); }
        let mut item = wait.item;
        if value["command"].is_object() && !hint(&value["command"]) { item.command = Some((reply.session, value["command"].clone())); }
        item.suggest_again |= suggests_again(&value["command"]);
        match extra_edits(&value, &wait.doc, wait.encoding) {
            Ok(extra) => item.extra = extra,
            Err(why) => return refuse(self, why),
        }
        self.apply_completion(wait.tab, wait.expected, wait.caret, &item, &wait.typed, Some(&wait.doc))
    }

    /// Apply `item` at the caret (and at every other caret on the same name) with its imports, as one transaction.
    /// `doc`: the version it was offered for (a template's view: every caret's edit must map, see
    /// [`crate::template::Projection::check`], or none is applied).
    fn apply_completion(&mut self, tab: TabId, expected: crate::text::SnapshotKey, caret: usize, item: &Suggestion, typed: &[Vec<TextEdit>], doc: Option<&Document>) -> Effect {
        let repaint = Effect { repaint: true, ..Effect::default() };
        let Some(view) = self.state.view(tab) else { return Effect::default() };
        let snapshot = view.snapshot().clone();
        let others: Vec<Selection> = view.tab().extra_selections().to_vec();
        let Some(extra) = carry(&item.extra, typed) else {
            self.state.messages.push("Code completion: text typed since the suggestion was offered overlaps its import edits; nothing was inserted".into());
            return repaint;
        };
        let range = item.replace.start..item.replace.end.max(caret);
        if item.snippet.is_some() && !others.is_empty() {
            // A live snippet's placeholders belong to one caret; with several there is no set Tab could visit.
            self.state.messages.push("Code completion: a snippet needs one caret; Escape collapses several selections".into());
            return repaint;
        }
        // Every other caret completes the same way only when it sits on the same name text.
        let (before, after) = (caret - range.start, range.end - caret);
        let name = snapshot.range(range.clone()).unwrap_or_default();
        let mut ranges = vec![range.clone()];
        for other in &others {
            let at = other.head;
            let compatible = other.anchor == other.head && at >= before && snapshot.range(at - before..at + after).is_some_and(|t| t == name);
            if !compatible {
                self.state.messages.push("Code completion: the other carets are not on the same name; Escape collapses them to complete here".into());
                return repaint;
            }
            ranges.push(at - before..at + after);
        }
        let mut edits: Vec<TextEdit> = extra.clone();
        if item.snippet.is_none() { edits.extend(ranges.iter().map(|r| TextEdit { range: r.clone(), text: item.text.clone() })); }
        else { edits.push(TextEdit { range: range.clone(), text: Arc::from("") }); }
        edits.sort_by_key(|e| (e.range.start, e.range.end));
        if edits.windows(2).any(|w| w[0].range.end > w[1].range.start) {
            self.state.messages.push(format!("Code completion: {OVERLAP}; nothing was inserted"));
            return repaint;
        }
        if item.snippet.is_none() && let Some(projection) = doc.and_then(Document::embedded)
            && let Err(why) = projection.check(&edits, typed, &|r| snapshot.range(r).map(std::borrow::Cow::into_owned))
        {
            self.state.messages.push(format!("Code completion: {why}; nothing was inserted"));
            return repaint;
        }
        let guard = doc.map(|view| super::snippet::SnippetGuard { view, typed, label: "Code completion" });
        let mut effect = match (&item.snippet, &item.command) {
            (Some(snippet), _) => self.insert_snippet(tab, expected, range, snippet, extra, guard),
            (None, None) => self.edit(tab, expected, edits, EditKind::Replace),
            // Its server inserts it: what the item itself changes first (often nothing), then its command.
            (None, Some((session, command))) => {
                edits.retain(|e| !e.range.is_empty() || !e.text.is_empty());
                let mut effect = if edits.is_empty() { repaint } else { self.edit(tab, expected, edits, EditKind::Replace) };
                effect.repaint |= self.run_server_command(*session, command, semantics::Lane::Completion, tab, "Code completion").repaint;
                effect
            }
        };
        // Applied: its server's further suggestions, asked as typing asks (quiet when there are none).
        if item.suggest_again && effect.repaint && self.state.settings.auto_completion { effect.repaint |= self.complete_at_caret(CompletionTrigger::Typing).repaint; }
        effect
    }

    /// After a typing edit of `tab`: keep the popup when the edit continues the name at its caret (re-filtered,
    /// asked again when its list was incomplete), else close it; then open it by typing when the setting allows.
    pub(super) fn completion_edited(&mut self, tab: TabId, edits: &[TextEdit], kind: EditKind) {
        let typed = match (kind, edits) {
            (EditKind::Typing | EditKind::Delete, [edit]) => Some(edit),
            _ => None,
        };
        let Some(snapshot) = self.state.view(tab).map(|v| v.snapshot().clone()) else { return };
        if self.state.active == Some(tab) && self.state.selection().is_some_and(|at| blank_prefix(&snapshot, at.head))
            && self.state.completion.as_ref().is_none_or(|c| c.trigger == CompletionTrigger::Typing) {
            self.close_completion();
            return;
        }
        let mut ask = false;
        if let Some(c) = self.state.completion.as_mut().filter(|c| c.tab == tab) {
            let continues = if c.local { crate::markdown_authoring::destination_char } else { word_char };
            let kept = typed.filter(|e| e.range.end == c.caret && e.range.start >= c.word && e.text.chars().all(continues));
            match kept {
                Some(e) => {
                    c.typed.push(edits.to_vec());
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
        let markdown = snapshot.language() == Some(Language::Markdown);
        // In Markdown, typing in a link's destination: `(` opens it, `/` and `#` go on to a folder or the headings.
        let auto = self.state.settings.auto_completion && typed.is_some_and(|e| kind == EditKind::Typing && !e.text.contains(['\r', '\n']) && e.text.chars().last().is_some_and(|last| match markdown {
            true => matches!(last, '(' | '/' | '#') || (self.state.completion.is_none() && crate::markdown_authoring::destination_char(last)),
            false => self.state.completion.is_none() && (word_char(last) || (snapshot.language() == Some(Language::Sql) && last == '.') || self.state.semantics.completion_triggers(snapshot.source()).contains(&last)),
        }));
        if ask || auto {
            let trigger = self.state.completion.as_ref().map_or(CompletionTrigger::Typing, |c| c.trigger);
            let open = self.state.completion.take();
            self.complete_at_caret(trigger);
            // An incomplete list keeps showing while its successor is asked.
            if open.is_some() { self.state.completion = open; }
        } else if self.pending_completion.as_ref().is_some_and(|p| p.tab == tab) { self.cancel_completion(); }
        self.describe_selected();
    }

    pub(super) fn set_auto_completion(&mut self, on: bool) -> Effect {
        self.state.settings.auto_completion = on;
        self.save_settings();
        Effect { repaint: true, ..Effect::default() }
    }
}

/// An item as a suggestion: its sort key, and its replacement converted against `doc` (the version asked).
/// `(word, caret)`: the name being completed and the caret in it.
fn suggestion(item: &serde_json::Value, doc: &Document, encoding: semantics::lsp::Encoding, (word, caret): (Range<usize>, usize), index: usize, resolves: bool, (session, runs): (u64, &dyn Fn(&str) -> bool)) -> Option<(String, Suggestion)> {
    let label = item["label"].as_str()?.to_owned();
    let edit = &item["textEdit"];
    let range = if edit.is_object() { Some(&edit["range"]).filter(|r| r.is_object()).or(Some(&edit["insert"])) } else { None };
    let replace = match range {
        Some(r) => semantics::lsp::offset(doc, &r["start"], encoding).zip(semantics::lsp::offset(doc, &r["end"], encoding)).map(|(a, b)| a..b),
        None => Some(word),
    };
    let text: Arc<str> = edit["newText"].as_str().or(item["insertText"].as_str()).unwrap_or(&label).into();
    // A snippet body is parsed now, so the list can say why one will not be inserted.
    let (snippet, snippet_why) = match item["insertTextFormat"].as_u64() {
        Some(2) => match Snippet::parse(&text) {
            Ok(snippet) => (Some(snippet), None),
            Err(why) => (None, Some(why)),
        },
        _ => (None, None),
    };
    let (extra, extra_why) = match extra_edits(item, doc, encoding) {
        Ok(extra) => (extra, None),
        Err(why) => (Vec::new(), Some(why)),
    };
    let unavailable = match &replace {
        Some(r) if r.start <= caret && caret <= r.end && semantics::lsp::line_of(doc, r.start) == semantics::lsp::line_of(doc, caret) => None,
        _ => Some("The language server's replacement range does not contain the caret"),
    }.or(extra_why)
        .or_else(|| replace.as_ref().filter(|r| extra.iter().any(|e| e.range.start < r.end && r.start < e.range.end)).map(|_| OVERLAP))
        .or(snippet_why)
        .or_else(|| command_refusal(&item["command"], runs))
        // A server that inserts through its own command: a snippet body would be inserted twice.
        .or_else(|| (snippet.is_some() && item["command"].is_object() && !hint(&item["command"])).then_some("Snippet suggestions that also run a language-server command are not applied"));
    let command = (item["command"].is_object() && !hint(&item["command"])).then(|| (session, item["command"].clone()));
    let filter = item["filterText"].as_str().unwrap_or(&label).to_lowercase();
    let sort = item["sortText"].as_str().unwrap_or(&label).to_owned();
    let documentation = super::signature::documentation(&item["documentation"]);
    let markdown = item["documentation"]["kind"].as_str() == Some("markdown");
    let kind = completion_kind(item["kind"].as_u64());
    let detail = item["detail"].as_str().map(str::to_owned).or_else(|| label_details(&item["labelDetails"]));
    Some((sort, Suggestion { label, kind, detail, documentation, described: false, markdown, index, unavailable, filter, text,
        replace: replace.unwrap_or(caret..caret), snippet, extra, resolve: resolves.then_some(index), command, suggest_again: suggests_again(&item["command"]) }))
}

/// LSP CompletionItemKind names; unknown extensions stay unlabelled.
fn completion_kind(kind: Option<u64>) -> Option<&'static str> {
    Some(match kind? {
        1 => "Text", 2 => "Method", 3 => "Function", 4 => "Constructor", 5 => "Field", 6 => "Variable",
        7 => "Class", 8 => "Interface", 9 => "Module", 10 => "Property", 11 => "Unit", 12 => "Value",
        13 => "Enum", 14 => "Keyword", 15 => "Snippet", 16 => "Color", 17 => "File", 18 => "Reference",
        19 => "Folder", 20 => "Enum member", 21 => "Constant", 22 => "Struct", 23 => "Event", 24 => "Operator", 25 => "Type parameter",
        _ => return None,
    })
}

/// An item's `labelDetails` (its signature, then its description) as one detail. Lenz does not declare them
/// (`labelDetailsSupport`), so servers that honour the capability keep their detail in `detail`; Ruby LSP and its Rails
/// add-on send them anyway, with no `detail`: the file a method comes from, the column a `where` filters.
fn label_details(details: &serde_json::Value) -> Option<String> {
    let parts: Vec<&str> = [&details["detail"], &details["description"]].into_iter().filter_map(|v| v.as_str()).map(str::trim).filter(|p| !p.is_empty()).collect();
    (!parts.is_empty()).then(|| parts.join(" "))
}

/// Why an item's command keeps it from being accepted: commands the server would run itself (they may edit other
/// files) are not run; the client hint to show parameters is ignored, the one to suggest again asks once the item is
/// applied (see `suggests_again`), and JDT LS's report of which suggestion was chosen is skipped (it only ranks later
/// suggestions; skipping it changes no text).
/// Commands Lenz runs only when the server is allowed them (`runs`; see `semantics::provider::runs_command`).
fn command_refusal(command: &serde_json::Value, runs: &dyn Fn(&str) -> bool) -> Option<&'static str> {
    let allowed = command["command"].as_str().is_some_and(runs);
    (command.is_object() && !hint(command) && !allowed).then_some("Suggestions that run a language-server command are not applied; it could edit other files")
}

/// Whether an item's command is the client hint to suggest again once it is applied.
fn suggests_again(command: &serde_json::Value) -> bool { command["command"].as_str() == Some("editor.action.triggerSuggest") }

/// A command Lenz skips without running: a client hint, or a report that changes no text.
fn hint(command: &serde_json::Value) -> bool {
    matches!(command["command"].as_str(), Some("editor.action.triggerParameterHints" | "editor.action.triggerSuggest" | "java.completion.onDidSelect"))
}

/// An item's `additionalTextEdits` as byte edits of `doc`, sorted; any that cannot be placed exactly, or that
/// overlap, refuse the whole item.
fn extra_edits(item: &serde_json::Value, doc: &Document, encoding: semantics::lsp::Encoding) -> Result<Vec<TextEdit>, &'static str> {
    let list = match item.get("additionalTextEdits") {
        None | Some(serde_json::Value::Null) => return Ok(Vec::new()),
        Some(serde_json::Value::Array(list)) => list,
        Some(_) => return Err(MALFORMED),
    };
    let mut edits = Vec::with_capacity(list.len());
    for edit in list {
        let start = semantics::lsp::offset(doc, &edit["range"]["start"], encoding).ok_or(MALFORMED)?;
        let end = semantics::lsp::offset(doc, &edit["range"]["end"], encoding).ok_or(MALFORMED)?;
        let text = edit["newText"].as_str().ok_or(MALFORMED)?;
        if end < start { return Err(MALFORMED); }
        edits.push(TextEdit { range: start..end, text: text.into() });
    }
    edits.sort_by_key(|e| (e.range.start, e.range.end));
    if edits.windows(2).any(|w| w[0].range.end > w[1].range.start || w[0].range == w[1].range) { return Err(MALFORMED); }
    Ok(edits)
}

/// `extra` (ranges of the version asked about) after the typing since, each edit set in order; `None` when typing
/// touched one of them.
fn carry(extra: &[TextEdit], typed: &[Vec<TextEdit>]) -> Option<Vec<TextEdit>> {
    let mut extra = extra.to_vec();
    for edits in typed {
        for e in &mut extra {
            if edits.iter().any(|t| t.range.start < e.range.end && e.range.start < t.range.end || (t.range.start > e.range.start && t.range.start < e.range.end)) { return None; }
            let start = crate::text::edited_byte(e.range.start, edits);
            e.range = start..crate::text::edited_byte(e.range.end, edits).max(start);
        }
    }
    Some(extra)
}

#[cfg(test)]
mod presentation_tests {
    use super::*;

    #[test]
    fn blank_completion_prefix_handles_indentation_and_only_the_current_line() {
        for (text, expected) in [("",true), (" \t",true), ("code\r\n\t ",true), ("λ ",false),
            ("object.",false), ("return ",false), ("\u{2003}\u{a0}",true)] {
            let doc=Document::decode("prefix.rs".into(),1,text.as_bytes());
            let snapshot=crate::text::TextSnapshot::from_document(crate::text::BufferId(1),&doc);
            assert_eq!(blank_prefix(&snapshot,text.len()),expected,"{text:?}");
        }
        let text=format!("{}suffix", "\u{2003}".repeat(1100));
        let doc=Document::decode("prefix.rs".into(),1,text.as_bytes());
        let snapshot=crate::text::TextSnapshot::from_document(crate::text::BufferId(1),&doc);
        assert!(blank_prefix(&snapshot,3300),"a new line's prefix stays blank even with text after the caret");
    }

    #[test]
    fn completion_keeps_kind_for_rank_preserving_presentation() {
        let doc = Document::decode("Example.java".into(), 1, b"he");
        let (_, row) = suggestion(&serde_json::json!({"label":"hello", "kind":2, "sortText":"001", "detail":"String hello()"}), &doc,
            semantics::lsp::Encoding::Utf16, (0..2, 2), 0, false, (1, &|_| false)).unwrap();
        assert!(format!("{row:?}").contains("kind: Some(\"Method\")"), "LSP kind is needed to distinguish equally ranked methods, types and values");
    }

    #[test]
    fn completion_matches_keep_utf8_ranges_and_fuzzy_gaps() {
        for (query,label,expected) in [("gv","getValue",vec![0..1,3..4]),("i\u{307}v","İValue",std::iter::once(0..3).collect())] {
            let doc=Document::decode("name.rs".into(),1,query.as_bytes());
            let snapshot=crate::text::TextSnapshot::from_document(crate::text::BufferId(1),&doc);
            let row=Suggestion::local(label.into(),None,label,0..query.len(),label,false);
            let popup=Suggestions::local(TabId(1),CompletionTrigger::Manual,&snapshot,(0,query.len()),vec![row]).unwrap();
            assert_eq!(popup.matches(label),expected);
        }
        assert_eq!(completion_kind(Some(999)),None);
    }

}
