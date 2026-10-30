//! Quick fixes: the fixes and refactorings a language server offers at the caret, fenced like completion to the
//! exact tab, draft version, caret and server session that answered. One applies as a single undoable edit of this
//! file only; an action that would change another file, create, rename or delete one, or run a server command is
//! listed with the reason it is not applied, never applied in part. A shell script's fixes are ShellCheck's, from
//! Lenz's own check of the version shown ([`semantics::shell`]), fenced the same way.
use super::*;
use crate::text::{EditKind, TextEdit};
use std::path::Path;

const OTHER_FILES: &str = "Changes other files too; Lenz applies fixes to the open file only";
const FILE_OPERATIONS: &str = "Creates, renames or deletes files; Lenz applies fixes to the open file only";
const COMMAND: &str = "Runs a language-server command, which could change other files; Lenz does not run it";
const OTHER_VERSION: &str = "Computed for another version of the file";
const MALFORMED: &str = "The language server's edits are malformed or overlap";
pub(super) const NO_CHANGE: &str = "The language server offers no change for it";
/// A template view's position in the projection's own scaffolding (see [`crate::template`]).
const OUTSIDE_RUBY: &str = "it reaches outside the template's embedded Ruby";
const OUTSIDE_MARKUP: &str = "it reaches outside the template's embedded markup";
/// The session of fixes Lenz computed itself (ShellCheck's): no server answers them.
const LOCAL: u64 = u64::MAX;

/// One offered action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodeAction {
    pub title: String,
    /// `quickfix`, `refactor.extract`, `source.organizeImports`, … when the server says.
    pub kind: Option<String>,
    /// The server's suggested fix for the problem.
    pub preferred: bool,
    /// Why it is listed but not applied.
    pub unavailable: Option<String>,
    edits: Vec<TextEdit>,
    /// An allowlisted server command run after its edits: the edit it sends back is checked like any other.
    command: Option<serde_json::Value>,
}

/// The actions shown.
#[derive(Clone, Debug)]
pub struct CodeActions {
    pub tab: TabId,
    source: SemanticSource,
    caret: usize,
    session: u64,
    pub actions: Vec<CodeAction>,
    pub selected: usize,
}

impl CodeAction {
    pub(super) fn into_parts(self) -> (Vec<TextEdit>, Option<serde_json::Value>) { (self.edits, self.command) }
}

impl CodeActions {
    pub fn caret(&self) -> usize { self.caret }
}

/// The request being answered.
pub(super) struct CodeActionRequest { request: u64, tab: TabId, source: SemanticSource, doc: Option<Arc<Document>>, caret: usize }

impl CodeActionRequest {
    pub(super) fn current(&self, state: &State) -> bool {
        state.active == Some(self.tab) && state.view(self.tab).is_some_and(|v| self.source.matches(v.snapshot()))
            && state.selection().is_some_and(|s| s.head == self.caret)
    }
    pub(super) fn request(&self) -> u64 { self.request }
    pub(super) fn admitted(&mut self, doc: &Arc<Document>) { self.doc = Some(doc.clone()); }
}

impl State {
    /// The quick fixes shown, while they are about the active tab's version as it is, with the caret where it was.
    pub fn code_actions(&self) -> Option<&CodeActions> {
        let c = self.code_actions.as_ref()?;
        (self.active == Some(c.tab) && (c.session == LOCAL || self.semantics.can_ask(c.session)) && self.view(c.tab).is_some_and(|v| c.source.matches(v.snapshot()))
            && self.selection().is_some_and(|s| s.head == c.caret)).then_some(c)
    }
}

impl Session {
    /// Ask for the quick fixes at the active caret, with the diagnostics there.
    pub(super) fn quick_fix_at_caret(&mut self) -> Effect {
        if let Some(why) = self.disabled(CommandId::QuickFix) { return self.code_action_message(why); }
        let (Some(tab), Some(at)) = (self.state.active, self.state.selection()) else { return Effect::default() };
        let Some(snapshot) = self.state.view(tab).map(|v| v.snapshot().clone()) else { return Effect::default() };
        self.close_code_actions();
        if snapshot.language() == Some(Language::Bash) { return self.shell_fixes(tab, snapshot, at.head); }
        self.sync_semantics();
        self.semantic_seq += 1;
        let request = self.semantic_seq;
        self.semantic_wait.remove(&semantics::Lane::CodeAction);
        let mut pending = CodeActionRequest { request, tab, source: SemanticSource::new(&snapshot), doc: None, caret: at.head };
        let Some(doc) = self.admitted_document(tab, Priority::Visible) else {
            self.pending_code_action = Some(pending);
            self.defer_semantic(tab, request, at.head, semantics::Question::CodeAction, None);
            return Effect::default();
        };
        self.send_admitted(&doc);
        match self.state.semantics.ask(&doc, at.head, request, semantics::Question::CodeAction) {
            Ok(()) => { pending.doc = Some(doc); self.pending_code_action = Some(pending); Effect::default() }
            Err(why) => self.code_action_unavailable(why),
        }
    }

    /// ShellCheck's fixes for the problems at `caret`, from Lenz's check of exactly the version shown.
    fn shell_fixes(&mut self, tab: TabId, snapshot: Arc<crate::text::TextSnapshot>, caret: usize) -> Effect {
        let lint = self.state.semantics.shell_lint(snapshot.source()).cloned();
        let Some(lint) = lint else { return self.code_action_message("ShellCheck has not checked this script yet; try again in a moment") };
        if let Some(note) = lint.note() { return self.code_action_message(note); }
        if !lint.diagnostics.as_ref().is_some_and(|d| d.describes(&snapshot)) {
            return self.code_action_message("ShellCheck has not checked the text shown yet; try again in a moment");
        }
        let mut actions: Vec<CodeAction> = Vec::new();
        for fix in lint.fixes.iter().filter(|f| f.range.start <= caret && caret <= f.range.end) {
            if actions.iter().any(|a| a.title == fix.title) { continue; }
            let (edits, unavailable) = match &fix.edits { Ok(e) => (e.clone(), None), Err(why) => (Vec::new(), Some((*why).to_owned())) };
            actions.push(CodeAction { title: fix.title.clone(), kind: Some("quickfix".into()), preferred: fix.title.starts_with("Apply fix"), unavailable, edits, command: None });
        }
        if actions.is_empty() { return self.code_action_message("None offered here"); }
        // ShellCheck's own fixes first, then disabling its rules.
        actions.sort_by_key(|a| (a.unavailable.is_some(), !a.preferred));
        self.state.code_actions = Some(CodeActions { tab, source: SemanticSource::new(&snapshot), caret, session: LOCAL, actions, selected: 0 });
        Effect { repaint: true, ..Effect::default() }
    }

    fn code_action_message(&mut self, why: impl std::fmt::Display) -> Effect {
        self.state.messages.push(format!("Quick fixes: {why}"));
        Effect { repaint: true, ..Effect::default() }
    }

    pub(super) fn code_action_unavailable(&mut self, why: String) -> Effect {
        self.cancel_code_action();
        self.code_action_message(why)
    }

    /// Stop waiting for quick fixes (a late reply is then dropped).
    pub(super) fn cancel_code_action(&mut self) {
        if self.pending_code_action.take().is_some() {
            self.semantic_wait.remove(&semantics::Lane::CodeAction);
            self.state.semantics.cancel_lane(semantics::Lane::CodeAction);
        }
    }

    /// Escape: hidden, and an answer still on its way is dropped.
    pub(super) fn close_code_actions(&mut self) -> Effect {
        self.cancel_code_action();
        Effect { repaint: self.state.code_actions.take().is_some(), ..Effect::default() }
    }

    /// Actions arrived: shown when they answer the latest request about the active tab's version as it is.
    pub(super) fn code_action_reply(&mut self, reply: semantics::Reply) -> Effect {
        let Some(pending) = self.pending_code_action.take_if(|p| p.request == reply.request) else { return Effect::default() };
        let (Some(doc), true) = (pending.doc.clone(), pending.current(&self.state) && self.state.semantics.can_ask(reply.session)) else { return Effect::default() };
        // A template's answers are about the view that server holds.
        let Some(doc) = self.state.semantics.served(&doc, reply.session) else { return Effect::default() };
        let value = match reply.result {
            Ok(v) => v,
            Err(why) => return self.code_action_message(why),
        };
        let runs = |c: &str| self.state.semantics.runs_command(reply.session, c);
        let mut actions: Vec<CodeAction> = value.as_array().map(Vec::as_slice).unwrap_or_default().iter()
            .filter_map(|item| action(item, &doc, reply.encoding, &runs)).collect();
        if actions.is_empty() { return self.code_action_message("None offered here"); }
        // Applicable first, the server's preferred fix leading, then fixes before refactorings; otherwise its order.
        actions.sort_by_key(|a| (a.unavailable.is_some(), !a.preferred, !a.kind.as_deref().is_some_and(|k| k.starts_with("quickfix"))));
        self.state.code_actions = Some(CodeActions { tab: pending.tab, source: pending.source, caret: pending.caret, session: reply.session, actions, selected: 0 });
        Effect { repaint: true, ..Effect::default() }
    }

    pub(super) fn move_code_action(&mut self, by: isize) -> Effect {
        let Some(n) = self.state.code_actions().map(|c| c.actions.len()) else { return Effect::default() };
        let Some(c) = self.state.code_actions.as_mut() else { return Effect::default() };
        c.selected = (c.selected as isize + by).rem_euclid(n as isize) as usize;
        Effect { repaint: true, ..Effect::default() }
    }

    pub(super) fn select_code_action(&mut self, row: usize) -> Effect {
        if self.state.code_actions().is_none_or(|c| row >= c.actions.len()) { return Effect::default(); }
        if let Some(c) = self.state.code_actions.as_mut() { c.selected = row; }
        Effect { repaint: true, ..Effect::default() }
    }

    /// Apply the selected action as one transaction against the version it was computed for, or nothing.
    pub(super) fn apply_code_action(&mut self) -> Effect {
        let Some(c) = self.state.code_actions() else { return Effect::default() };
        let Some(action) = c.actions.get(c.selected).cloned() else { return Effect::default() };
        if let Some(why) = &action.unavailable { return self.code_action_message(format!("{}: {why}", action.title)); }
        let (tab, expected, session) = (c.tab, c.source.key, c.session);
        if let Some(why) = self.state.view(tab).and_then(|v| v.edit_disabled()) { return self.code_action_message(why); }
        self.state.code_actions = None;
        let mut effect = if action.edits.is_empty() { Effect::default() } else { self.edit(tab, expected, action.edits, EditKind::Replace) };
        match &action.command {
            Some(command) => effect.repaint |= self.run_server_command(session, command, semantics::Lane::CodeAction, tab, "Quick fixes").repaint,
            None => self.state.messages.push(format!("Applied: {}", action.title)),
        }
        effect.repaint = true;
        effect
    }
}

/// One item of a `textDocument/codeAction` reply: a bare command, or an action with an edit (and perhaps a command).
/// `runs` says which server commands Lenz runs for this server.
pub(super) fn action(item: &serde_json::Value, doc: &Document, encoding: semantics::lsp::Encoding, runs: &dyn Fn(&str) -> bool) -> Option<CodeAction> {
    let title = item["title"].as_str()?.trim().to_owned();
    let mut a = CodeAction { title, kind: item["kind"].as_str().map(str::to_owned), preferred: item["isPreferred"].as_bool().unwrap_or(false), unavailable: None, edits: Vec::new(), command: None };
    // A bare `Command` is the item itself; an action may carry one after its edit.
    let command = if item["command"].is_string() { Some(item) } else { Some(&item["command"]).filter(|c| c.is_object()) };
    let allowed = command.filter(|c| c["command"].as_str().is_some_and(runs)).cloned();
    // typescript-language-server's fixes carry tsserver's own action, its text changes included: read here, never
    // run, and only when they change this file alone and ask for no further command (installing a package, say).
    let tsserver = command.filter(|c| c["command"] == "_typescript.applyCodeActionCommand").map(|c| &c["arguments"][0]["action"]);
    let unavailable = if let Some(reason) = item["disabled"]["reason"].as_str() {
        Err(format!("Not available: {reason}"))
    } else if let Some(action) = tsserver {
        tsserver_edits(action, doc).map_err(str::to_owned)
    } else if command.is_some() && allowed.is_none() {
        Err(COMMAND.to_owned())
    } else {
        match &item["edit"] {
            serde_json::Value::Null => Ok(Vec::new()),
            edit => workspace_edit(edit, doc, encoding).map_err(str::to_owned),
        }
    };
    match unavailable {
        Ok(edits) if edits.is_empty() && allowed.is_none() => a.unavailable = Some(NO_CHANGE.to_owned()),
        Ok(edits) => match template_refusal(doc, &edits) {
            Some(why) => a.unavailable = Some(why),
            None => { a.edits = edits; a.command = allowed; }
        },
        Err(why) => a.unavailable = Some(why),
    }
    Some(a)
}

/// A tsserver code action's `changes` as byte edits of `doc` (1-based lines and UTF-16 offsets, as tsserver counts),
/// sorted; refused whole when it changes another file or asks for a further command.
fn tsserver_edits(action: &serde_json::Value, doc: &Document) -> Result<Vec<TextEdit>, &'static str> {
    if action["commands"].as_array().is_some_and(|c| !c.is_empty()) { return Err(COMMAND); }
    let this = |name: &str| Path::new(name) == doc.source() || doc.source_meta().canonical() == Some(Path::new(name));
    let at = |p: &serde_json::Value| {
        let (line, offset) = (p["line"].as_u64()?.checked_sub(1)?, p["offset"].as_u64()?.checked_sub(1)?);
        semantics::lsp::offset_at(doc, line as usize, offset as usize, semantics::lsp::Encoding::Utf16)
    };
    let mut edits = Vec::new();
    for change in action["changes"].as_array().ok_or(MALFORMED)? {
        if !change["fileName"].as_str().is_some_and(this) { return Err(OTHER_FILES); }
        for t in change["textChanges"].as_array().ok_or(MALFORMED)? {
            let (start, end) = (at(&t["start"]).ok_or(MALFORMED)?, at(&t["end"]).ok_or(MALFORMED)?);
            if end < start { return Err(MALFORMED); }
            edits.push(TextEdit { range: start..end, text: t["newText"].as_str().ok_or(MALFORMED)?.into() });
        }
    }
    sorted(edits)
}

/// A `WorkspaceEdit` as byte edits of `doc`, sorted; refused in full when any part of it is outside `doc` as it is.
pub(super) fn workspace_edit(edit: &serde_json::Value, doc: &Document, encoding: semantics::lsp::Encoding) -> Result<Vec<TextEdit>, &'static str> {
    let this = |uri: &str| semantics::lsp::uri_path(uri).is_some_and(|p| p == doc.source() || doc.source_meta().canonical() == Some(p.as_path()));
    let mut edits = Vec::new();
    if let Some(changes) = edit["documentChanges"].as_array() {
        for change in changes {
            if change.get("kind").is_some() { return Err(FILE_OPERATIONS); }
            if !change["textDocument"]["uri"].as_str().is_some_and(this) { return Err(OTHER_FILES); }
            if change["textDocument"]["version"].as_u64().is_some_and(|v| v != doc.version()) { return Err(OTHER_VERSION); }
            edits.extend(text_edits(&change["edits"], doc, encoding)?);
        }
    } else if let Some(changes) = edit["changes"].as_object() {
        for (uri, list) in changes {
            if !this(uri) { return Err(OTHER_FILES); }
            edits.extend(text_edits(list, doc, encoding)?);
        }
    }
    sorted(edits)
}

/// `edits` in document order, refused when any two overlap or replace the same range.
pub(super) fn sorted(mut edits: Vec<TextEdit>) -> Result<Vec<TextEdit>, &'static str> {
    edits.sort_by_key(|e| (e.range.start, e.range.end));
    if edits.windows(2).any(|w| w[0].range.end > w[1].range.start || w[0].range == w[1].range) { return Err(MALFORMED); }
    Ok(edits)
}

/// Why a template view's server may not apply `edits` (byte edits of the version it was asked about): see
/// [`crate::template::Projection::check`]. `None` for any other document.
pub(super) fn template_refusal(doc: &Document, edits: &[TextEdit]) -> Option<String> {
    doc.embedded()?.check(edits, &[], &|r| doc.text().get(r).map(str::to_owned)).err()
}

/// Why a position cannot be placed: in a template view, outside its embedded language.
fn unplaced(doc: &Document) -> &'static str {
    match doc.embedded().map(|p| p.route().language) {
        Some(Language::Ruby) => OUTSIDE_RUBY,
        Some(_) => OUTSIDE_MARKUP,
        None => MALFORMED,
    }
}

/// A list of `TextEdit`s as byte edits of `doc`; any that cannot be placed exactly refuse them all.
pub(super) fn text_edits(list: &serde_json::Value, doc: &Document, encoding: semantics::lsp::Encoding) -> Result<Vec<TextEdit>, &'static str> {
    list.as_array().ok_or(MALFORMED)?.iter().map(|edit| {
        let start = semantics::lsp::offset(doc, &edit["range"]["start"], encoding).ok_or_else(|| unplaced(doc))?;
        let end = semantics::lsp::offset(doc, &edit["range"]["end"], encoding).ok_or_else(|| unplaced(doc))?;
        let text = edit["newText"].as_str().ok_or(MALFORMED)?;
        if end < start { return Err(MALFORMED); }
        Ok(TextEdit { range: start..end, text: text.into() })
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tsserver_changes_are_read_for_this_file_only_and_never_with_further_commands() {
        // tsserver lines are 1-based and offsets 1-based UTF-16 units: `𝄞` counts two.
        let doc = Document::decode(std::path::PathBuf::from("/p/a.ts"), 1, "const s = \"𝄞\"; x\n".as_bytes());
        let change = |file: &str| serde_json::json!({ "changes": [{ "fileName": file, "textChanges": [{ "start": { "line": 1, "offset": 17 }, "end": { "line": 1, "offset": 18 }, "newText": "y" }] }] });
        let edits = tsserver_edits(&change("/p/a.ts"), &doc).unwrap();
        assert_eq!(edits.len(), 1);
        assert_eq!(&doc.text()[edits[0].range.clone()], "x");
        assert_eq!(tsserver_edits(&change("/p/b.ts"), &doc), Err(OTHER_FILES));
        let mut install = change("/p/a.ts");
        install["commands"] = serde_json::json!([{ "type": "install package", "packageName": "@types/x" }]);
        assert_eq!(tsserver_edits(&install, &doc), Err(COMMAND));
    }
}
