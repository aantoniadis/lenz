//! Bounded workspace edits: prepare and validate off-thread, then publish draft transactions.
use super::*;
use crate::text::{TextEdit, TextSnapshot, EditKind};
use std::collections::{BTreeMap, HashMap, HashSet};

const FILE_LIMIT: usize = 100;
const BYTE_LIMIT: usize = 32 << 20;
const FILE_BYTES: usize = 4 << 20;
const EDIT_LIMIT: usize = 10_000;

#[derive(Default)]
pub(super) struct Work {
    request: u64,
    latest: Arc<AtomicU64>,
    generation: u64,
    pending: bool,
    plan: Option<WorkspacePlan>,
    rename: Option<RenameRequest>,
}
struct RenameRequest {
    request: u64,
    tab: TabId,
    key: crate::text::SnapshotKey,
    caret: usize,
    root: PathBuf,
    captured: HashMap<PathBuf, Arc<TextSnapshot>>,
}
#[derive(Clone, Debug)]
pub struct WorkspacePlan {
    root: PathBuf,
    title: String,
    current: usize,
    notice: Option<String>,
    rendered: std::sync::OnceLock<String>,
    files: Vec<File>,
}
#[derive(Clone, Debug)]
struct File {
    path: PathBuf,
    doc: Arc<Document>,
    snapshot: Option<Arc<TextSnapshot>>,
    stamp: Option<Stamp>,
    disk: Arc<Document>,
    edits: Vec<TextEdit>,
    selected: bool,
}
impl WorkspacePlan {
    pub fn files(&self) -> Vec<(String, bool)> { self.files.iter().map(|f| (f.path.strip_prefix(&self.root).unwrap_or(&f.path).to_string_lossy().escape_debug().to_string(), f.selected)).collect() }
    pub fn text(&self) -> String { self.rendered.get_or_init(||self.render()).clone() }
    fn render(&self) -> String {
        let mut out = format!("{}\nN/P selects a file · Space includes/excludes · A applies · Esc cancels\n{} files · {} edits\nApply changes drafts only. Save each file to write it; Undo works per file.\n\n", self.title, self.files.len(), self.files.iter().map(|f| f.edits.len()).sum::<usize>());
        if let Some(notice) = &self.notice { out.push_str(&format!("{notice}\n\n")); }
        out.push_str(&format!("{} files included\n", self.files.iter().filter(|f|f.selected).count()));
        out.push_str(&format!("Selected file: {} of {}\n", self.current + 1, self.files.len()));
        if let Some(file) = self.files.get(self.current) { out.push_str(&format!("{} {}\n\n", if file.selected { "[x] Included:" } else { "[ ] Excluded:" }, file.path.strip_prefix(&self.root).unwrap_or(&file.path).to_string_lossy().escape_debug())); }
        let ordered = self.files.iter().enumerate().skip(self.current).chain(self.files.iter().enumerate().take(self.current));
        for (i, file) in ordered {
            out.push_str(&format!("{} {}. {}\n", if file.selected { "[x]" } else { "[ ]" }, i+1, file.path.strip_prefix(&self.root).unwrap_or(&file.path).to_string_lossy().escape_debug()));
            for edit in &file.edits {
                let before = file.doc.text().get(edit.range.clone()).unwrap_or("[range unavailable]");
                out.push_str(&format!("  line {}: {:?}\n  - {:?}\n  + {:?}\n", file.doc.line_of(edit.range.start)+1, file.doc.line(file.doc.line_of(edit.range.start)).chars().take(240).collect::<String>(), before.chars().take(240).collect::<String>(), edit.text.chars().take(240).collect::<String>()));
                if out.len() > 256 << 10 { out.push_str("\nPreview display truncated; narrow the search for a smaller preview.\n"); return out; }
            }
        }
        out
    }
}
impl Session {
    pub fn workspace_preview(&self) -> Option<&WorkspacePlan> { self.workspace_edit.plan.as_ref() }
    pub fn workspace_busy(&self) -> bool { self.workspace_edit.pending }
    pub(super) fn workspace_disabled(&self, id: CommandId) -> Option<&'static str> {
        if id == CommandId::CancelWorkspaceEdit { return (!self.workspace_edit.pending && self.workspace_edit.plan.is_none()).then_some("No edit preview is open"); }
        if !self.text_input_enabled { return Some("Text input is gated on this unvalidated platform/backend"); }
        if self.workspace_edit.pending { return Some("Preparing workspace edits; cancel the preview to stop"); }
        if id == CommandId::ApplyWorkspaceEdit { return self.workspace_edit.plan.as_ref().is_none_or(|p| !p.files.iter().any(|f| f.selected)).then_some("No files selected in an edit preview"); }
        if self.state.project.is_none() { return Some("Open a project folder first"); }
        if id == CommandId::RenameSymbol {
            let Some(view) = self.state.active().and_then(|t| self.state.view(t.id)) else { return Some("Open a source file first"); };
            if view.snapshot().source_meta().archive().is_some() || crate::template::Kind::of(view.snapshot().source()).is_some() { return Some("Rename requires a standalone source file with direct language-server coordinates"); }
            if self.state.active().is_some_and(|t|t.untitled()) { return Some("Save this file before renaming a symbol"); }
            if let Some(why) = view.edit_disabled() { return Some(why); }
            match self.state.provider() {
                None => return Some("No language server for this language"),
                Some(provider) if matches!(provider.status, semantics::Status::Indexing(_)) => return Some("Wait for the language server to finish indexing before renaming"),
                Some(provider) if matches!(provider.status, semantics::Status::Missing(_)) => return Some("The language server is unavailable; open Language Server settings"),
                Some(provider) if matches!(provider.status, semantics::Status::Failed(_)) => return Some("The language server stopped; restart it from Language Server settings"),
                Some(provider) if matches!(provider.status, semantics::Status::Disabled) => return Some("Enable the language server for this folder first"),
                Some(provider) if !provider.supports.is_empty() && !provider.offers(semantics::Question::Rename) => return Some("The language server does not offer symbol rename"),
                _ => {}
            }
            return None;
        }
        match self.state.results.as_ref() {
            Some(r) if r.kind == ResultsKind::Text && r.scope == Scope::Project => {
                if !r.done { Some("Wait for project search to finish") }
                else if r.limited || r.stopped || r.partial_inventory || r.failed > 0 || r.error.is_some() || !r.stale.is_empty() { Some("Rerun a complete project search before replacing") }
                else if r.hits.is_empty() { Some("No search matches to replace") } else { None }
            }
            _ => Some("Search in Project first, then choose Replace in Project"),
        }
    }
    fn workspace_notice(&mut self, why: impl std::fmt::Display) -> Effect { if let Some(plan) = &mut self.workspace_edit.plan { plan.notice = Some(why.to_string()); plan.rendered.take(); } self.state.messages.push(format!("Workspace edits: {why}")); Effect { repaint: true, ..Default::default() } }
    pub(super) fn cancel_workspace_edit(&mut self) -> Effect {
        self.workspace_edit.request += 1;
        self.workspace_edit.latest.store(self.workspace_edit.request, Ordering::Relaxed);
        self.workspace_edit.pending = false;
        self.workspace_edit.plan = None;
        self.workspace_edit.rename = None;
        self.state.semantics.cancel_lane(semantics::Lane::Rename);
        Effect { repaint: true, ..Default::default() }
    }
    pub(super) fn move_workspace_file(&mut self, by: isize) -> Effect {
        if let Some(plan) = self.workspace_edit.plan.as_mut().filter(|p|!p.files.is_empty()) {
            plan.current = (plan.current as isize + by).rem_euclid(plan.files.len() as isize) as usize; plan.rendered.take();
        }
        Effect { repaint: true, ..Default::default() }
    }
    pub(super) fn toggle_current_workspace_file(&mut self) -> Effect {
        let index = self.workspace_edit.plan.as_ref().map_or(0, |p|p.current);
        self.toggle_workspace_file(index)
    }
    pub(super) fn toggle_workspace_file(&mut self, index: usize) -> Effect {
        if !self.workspace_edit.pending && let Some(plan) = self.workspace_edit.plan.as_mut() && let Some(f) = plan.files.get_mut(index) { f.selected = !f.selected; plan.rendered.take(); }
        Effect { repaint: true, ..Default::default() }
    }
    pub(super) fn preview_project_replace(&mut self, replacement: String) -> Effect {
        if let Some(why) = self.workspace_disabled(CommandId::ProjectReplace) { return self.workspace_notice(why); }
        if replacement.len() > 64 << 10 { return self.workspace_notice("Replacement is too large"); }
        self.replace_text = replacement.clone();
        let Some(r) = self.state.results.as_ref() else { return Effect::default() };
        let Some((root, _)) = r.root.clone() else { return Effect::default() };
        let paths: Vec<_> = r.hits.iter().map(|h| h.path.clone()).collect::<std::collections::BTreeSet<_>>().into_iter().collect();
        let captured: HashMap<_, _> = r.captured.iter().filter_map(|(p,c)|c.snapshot.clone().map(|s|(p.clone(),s))).collect();
        let disk_sources = r.disk_sources.clone();
        let query = lenz_content::find::Query { text: r.query.clone(), options: r.options };
        let replacement: Arc<str> = replacement.into();
        self.cancel_workspace_edit();
        let request = self.workspace_edit.request;
        self.workspace_edit.generation = self.state.project.as_ref().map_or(0, |p|p.generation());
        let latest = self.workspace_edit.latest.clone();
        self.workspace_edit.pending = true;
        let versions = self.versions.clone();
        let tx = self.tx.clone();
        self.jobs.submit(Priority::Visible, move || {
            let result = (|| {
                if paths.len() > FILE_LIMIT { return Err("Preview supports at most 100 files; narrow the search".into()); }
                let mut plan = WorkspacePlan { root: root.clone(), current: 0, notice: None, rendered: Default::default(), title: format!("Replace {:?} with {:?} (literal replacement)", query.text, replacement), files: Vec::new() };
                let (mut bytes, mut edits) = (0, 0);
                let mut canonical = HashSet::new();
                for rel in paths {
                    if latest.load(Ordering::Relaxed) != request { return Err("Preview cancelled".into()); }
                    let path = root.join(rel.as_ref());
                    let snapshot = captured.get(&path).cloned();
                    let (stamp, disk) = load(&path, None, &versions)?;
                    let doc = if let Some(snapshot) = &snapshot { load(&path, Some(snapshot), &versions)?.1 } else { disk.clone() };
                    if !canonical.insert(std::fs::canonicalize(&path).map_err(|e| e.to_string())?) { return Err("Two result paths refer to the same file; narrow the search".into()); }
                    if snapshot.is_none() && disk_sources.get(&rel).is_none_or(|s| s.digest != search::decoded_digest(doc.text().as_bytes())) { return Err(format!("{} changed; rerun search", path.display())); }
                    bytes += doc.bytes();
                    if bytes > BYTE_LIMIT { return Err("Preview exceeds 32 MiB; narrow the search".into()); }
                    let (matches, capped) = crate::find::find_all(&doc, &query, || latest.load(Ordering::Relaxed) == request).ok_or("Search cancelled")?.map_err(|e| e.to_string())?;
                    if capped { return Err("Too many matches; narrow the search".into()); }
                    let mut changes = Vec::new();
                    for (line, range) in matches {
                        let base = doc.line_range(line).start;
                        let range = base+range.start..base+range.end;
                        if doc.text().get(range.clone()) == Some(replacement.as_ref()) { continue; }
                        if bytes + replacement.len() > BYTE_LIMIT { return Err("Replacement exceeds 32 MiB; narrow the search".into()); }
                        if edits >= EDIT_LIMIT { return Err("Preview exceeds 10,000 edits; narrow the search".into()); }
                        bytes += replacement.len(); edits += 1;
                        changes.push(TextEdit { range, text: replacement.clone() });
                    }
                    if !changes.is_empty() { plan.files.push(File { path, doc, snapshot, stamp, disk, edits: changes, selected: true }); }
                }
                Ok(plan)
            })();
            let _ = tx.send_blocking(Completion::WorkspacePrepared { request, apply: false, result });
        });
        self.workspace_notice("Preparing replacement preview… Esc cancels")
    }
    pub(super) fn workspace_prepared(&mut self, request: u64, apply: bool, result: Result<WorkspacePlan, String>) -> Effect {
        if request != self.workspace_edit.request || !self.workspace_edit.pending { return Effect::default(); }
        self.workspace_edit.pending = false;
        let plan = match result { Ok(p) => p, Err(why) => return self.workspace_notice(why) };
        if self.state.project.as_ref().is_none_or(|p| p.root() != plan.root || p.generation() != self.workspace_edit.generation) { return self.workspace_notice("The project changed; prepare a new preview"); }
        if !apply {
            if plan.files.is_empty() { return self.workspace_notice("No changes were proposed"); }
            self.workspace_edit.plan = Some(plan);
            return Effect { repaint: true, ..Default::default() };
        }
        // Validate every buffer before publishing any edits. Aliases cannot create a second competing draft.
        let mut targets = Vec::new();
        for file in plan.files.iter().filter(|f| f.selected) {
            if self.save_as_destination_reserved(&file.path) { return self.workspace_notice("An affected path is reserved by Save As; wait for it to finish"); }
            let canonical = file.doc.source_meta().canonical();
            let tab = self.state.tabs.iter().find(|t| t.read_only_side().is_none() && (t.source == file.path || canonical.is_some() && t.canonical.as_deref() == canonical)).map(|t| t.id);
            if let Some(tab) = tab {
                let Some(view) = self.state.view(tab) else { return self.workspace_notice("An affected file is loading; prepare a new preview"); };
                let current = view.snapshot();
                if view.edit_disabled().is_some() || file.snapshot.as_ref().map_or_else(|| !current.matches_document(&file.doc), |old| old.key() != current.key()) { return self.workspace_notice(format!("{} changed or is busy; prepare a new preview", file.path.display())); }
            } else if file.snapshot.is_some() { return self.workspace_notice("An affected draft was closed; prepare a new preview"); }
            targets.push(tab);
        }
        let count = targets.len();
        let active_changed = targets.iter().flatten().any(|tab| {
            self.state.source(*tab).zip(self.state.current_source())
                .is_some_and(|(affected, active)| affected.key().buffer == active.key().buffer)
        });
        let mut first = None;
        for (file, existing) in plan.files.into_iter().filter(|f| f.selected).zip(targets) {
            let tab = existing.unwrap_or_else(|| {
                self.next_tab += 1; let id = TabId(self.next_tab);
                let mut tab = self.fresh_tab(id, &file.path, &file.path, true, true);
                tab.canonical = file.doc.source_meta().canonical().map(Path::to_owned);
                self.state.tabs.push(tab);
                self.show(id, file.doc, file.stamp, false);
                id
            });
            first.get_or_insert(tab);
            if let Some(key) = self.state.source(tab).map(|s| s.key()) { self.edit(tab, key, file.edits, EditKind::Replace); }
        }
        self.workspace_edit.plan = None;
        if !active_changed && let Some(tab) = first { self.activate(tab); }
        self.workspace_notice(format!("Updated {count} drafts. Save writes each file; Undo reverts its edits."))
    }
    pub(super) fn apply_workspace_edit(&mut self) -> Effect {
        if let Some(why) = self.workspace_disabled(CommandId::ApplyWorkspaceEdit) { return self.workspace_notice(why); }
        let Some(plan) = self.workspace_edit.plan.clone() else { return Effect::default() };
        self.workspace_edit.pending = true;
        let request = self.workspace_edit.request;
        let latest = self.workspace_edit.latest.clone();
        let versions = self.versions.clone();
        let tx = self.tx.clone();
        self.jobs.submit(Priority::Visible, move || {
            let result = (|| {
                for f in plan.files.iter().filter(|f| f.selected) {
                    if latest.load(Ordering::Relaxed) != request { return Err("Preview cancelled".into()); }
                    let (_, now) = load(&f.path, None, &versions)?;
                    if !now.same_content(&f.disk) || now.source_meta().canonical() != f.disk.source_meta().canonical() { return Err(format!("{} changed on disk; prepare a new preview", f.path.display())); }
                }
                Ok(plan)
            })();
            let _ = tx.send_blocking(Completion::WorkspacePrepared { request, apply: true, result });
        });
        Effect { repaint: true, ..Default::default() }
    }
}
fn load(path: &Path, snapshot: Option<&Arc<TextSnapshot>>, versions: &AtomicU64) -> Result<(Option<Stamp>, Arc<Document>), String> {
    if let Some(snapshot) = snapshot {
        if snapshot.source_meta().is_binary() || snapshot.source_meta().is_lossy() { return Err("Binary and lossy drafts cannot be replaced".into()); }
        if snapshot.len_bytes() > FILE_BYTES { return Err("An affected draft exceeds 4 MiB".into()); }
        return Ok((None, Arc::new(snapshot.materialize())));
    }
    if std::fs::metadata(path).map_err(|e| e.to_string())?.len() > FILE_BYTES as u64 { return Err("An affected file exceeds 4 MiB".into()); }
    let read = lenz_content::write::read_exact_bytes_bounded(path, FILE_BYTES as u64, &|| false).map_err(|e| format!("Cannot read {}: {e}", path.display()))?;
    let stamp = Stamp::from(read.metadata());
    let doc = decode_read(path.to_owned(), versions.fetch_add(1, Ordering::Relaxed) + 1, &read);
    if doc.bytes() > FILE_BYTES || doc.source_meta().is_binary() || doc.source_meta().is_lossy() { return Err(format!("{} is too large, binary or lossy", path.display())); }
    Ok((Some(stamp), Arc::new(doc)))
}

impl Session {
    pub(super) fn rename_symbol(&mut self, name: String) -> Effect {
        if let Some(why) = self.workspace_disabled(CommandId::RenameSymbol) { return self.workspace_notice(why); }
        if name.is_empty() || name.len() > 1024 || name.chars().any(char::is_control) { return self.workspace_notice("Enter a nonempty symbol name without control characters"); }
        let Some(tab) = self.state.active else { return self.workspace_notice("Open a source file first"); };
        let Some(doc) = self.admitted_document(tab, Priority::Visible) else { return self.workspace_notice("Wait for the language server document to be ready, then try Rename Symbol again"); };
        if doc.is_template() { return self.workspace_notice("Rename is unavailable for projected template source"); }
        let Some(root) = self.state.project.as_ref().map(|p| p.root().to_owned()) else { return Effect::default() };
        let captured = self.captured_project_sources(&root).into_iter().filter_map(|(p,c)| c.snapshot.map(|s|(p,s))).collect();
        let key = match self.state.source(tab) { Some(s) => s.key(), None => return Effect::default() };
        let offset = self.state.selection().map_or(0, |s| s.head);
        self.cancel_workspace_edit();
        self.sync_semantics();
        self.send_admitted(&doc);
        self.semantic_seq += 1;
        let request = self.semantic_seq;
        self.workspace_edit.generation = self.state.project.as_ref().map_or(0, |p|p.generation());
        self.workspace_edit.rename = Some(RenameRequest { request, tab, key, caret: offset, root, captured });
        self.workspace_edit.pending = true;
        match self.state.semantics.ask_with(&doc, offset, request, semantics::Question::Rename, serde_json::json!({"newName":name})) {
            Ok(()) => self.workspace_notice("Asking the language server for a rename preview… Esc cancels"),
            Err(why) => { self.workspace_edit.pending = false; self.workspace_edit.rename = None; self.workspace_notice(why) }
        }
    }
    pub(super) fn rename_reply(&mut self, reply: semantics::Reply) -> Effect {
        let Some(RenameRequest { tab, key, caret, root, captured, .. }) = self.workspace_edit.rename.take_if(|r| r.request == reply.request) else { return Effect::default() };
        if self.state.active != Some(tab) || self.state.selection().is_none_or(|s|s.head != caret) || self.state.source(tab).is_none_or(|s| s.key() != key) || !self.state.semantics.can_ask(reply.session) {
            self.workspace_edit.pending = false;
            return self.workspace_notice("The source or language server changed; request Rename Symbol again");
        }
        if reply.indexing { self.workspace_edit.pending = false; return self.workspace_notice("The language server is still indexing; request Rename Symbol once it is ready"); }
        let value = match reply.result { Ok(v) => v, Err(why) => { self.workspace_edit.pending = false; return self.workspace_notice(why); } };
        let served = reply.sources;
        let request = self.workspace_edit.request;
        let versions = self.versions.clone();
        let tx = self.tx.clone();
        self.jobs.submit(Priority::Visible, move || {
            let result = rename_plan(root, value, reply.encoding, captured, &versions, &served);
            let _ = tx.send_blocking(Completion::WorkspacePrepared { request, apply: false, result });
        });
        Effect { repaint: true, ..Default::default() }
    }
}
fn rename_plan(root: PathBuf, value: serde_json::Value, encoding: semantics::lsp::Encoding, captured: HashMap<PathBuf, Arc<TextSnapshot>>, versions: &AtomicU64, served: &HashMap<PathBuf, semantics::SourceVersion>) -> Result<WorkspacePlan, String> {
    let mut edits = BTreeMap::new();
    if value.is_null() { return Err("The language server cannot rename this symbol".into()); }
    if value.get("changeAnnotations").is_some() { return Err("Annotated workspace edits are not supported; nothing changed".into()); }
    if let Some(changes) = value.get("changes") {
        for (uri, list) in changes.as_object().ok_or("Malformed rename edits")? { edits.insert(uri.clone(), (None, list.clone())); }
    }
    if let Some(changes) = value.get("documentChanges") {
        if !edits.is_empty() { return Err("Rename contains both edit forms; nothing changed".into()); }
        for change in changes.as_array().ok_or("Malformed rename edits")? {
            if change.get("kind").is_some() { return Err("This rename creates, moves or deletes files; nothing changed".into()); }
            let uri = change["textDocument"]["uri"].as_str().ok_or("Missing rename file URI")?;
            let version = &change["textDocument"]["version"];
            if !version.is_null() && version.as_i64().is_none() { return Err("Malformed rename document version".into()); }
            if edits.insert(uri.into(), (version.as_i64(), change["edits"].clone())).is_some() { return Err("Duplicate rename document; nothing changed".into()); }
        }
    }
    if edits.len() > FILE_LIMIT { return Err("Rename exceeds 100 files".into()); }
    let canonical_root = std::fs::canonicalize(&root).map_err(|e| e.to_string())?;
    let mut plan = WorkspacePlan { root, current: 0, notice: None, rendered: Default::default(), title: "Rename Symbol".into(), files: Vec::new() };
    let (mut bytes, mut count) = (0, 0);
    let mut aliases = HashSet::new();
    for (uri, (version, raw)) in edits {
        let path = semantics::lsp::uri_path(&uri).ok_or("Rename contains a non-file URI")?;
        let canonical = std::fs::canonicalize(&path).map_err(|e| e.to_string())?;
        if !canonical.starts_with(&canonical_root) || !aliases.insert(canonical.clone()) { return Err("Rename reaches outside the project or repeats one physical file".into()); }
        let snapshot = captured.get(&path).or_else(|| captured.get(&canonical)).cloned();
        let (stamp, disk) = load(&path, None, versions)?;
        let doc = if let Some(snapshot) = &snapshot { load(&path, Some(snapshot), versions)?.1 } else { disk.clone() };
        if doc.is_template() { return Err("Rename reaches projected template source; nothing changed".into()); }
        // Provider-open coordinates must describe exactly the captured draft. A
        // not-yet-admitted dirty sibling cannot borrow positions from older server text.
        let sent = served.get(&path).or_else(|| served.get(&canonical));
        let current = match (&snapshot, sent) {
            (Some(snapshot), Some(sent)) => sent.matches_snapshot(snapshot),
            (Some(snapshot), None) => snapshot.matches_document(&disk),
            (None, None) => true,
            (None, Some(_)) => false,
        };
        if !current { return Err(format!("The language server has not received the current text of {}; wait for it to synchronize and rename again", path.display())); }
        if let Some(version) = version && version != doc.version() as i64 { return Err("Rename was computed for another document version".into()); }
        let raw_edits = raw.as_array().ok_or("Malformed rename edits")?;
        if raw_edits.len() > EDIT_LIMIT.saturating_sub(count) { return Err("Rename exceeds preview limits (32 MiB / 10,000 edits)".into()); }
        let mut proposed_bytes = doc.bytes();
        for edit in raw_edits {
            proposed_bytes = proposed_bytes.checked_add(edit["newText"].as_str().ok_or("Malformed rename replacement")?.len()).ok_or("Rename is too large")?;
            if proposed_bytes > BYTE_LIMIT.saturating_sub(bytes) { return Err("Rename exceeds preview limits (32 MiB / 10,000 edits)".into()); }
        }
        let mut changes = code_action::text_edits(&raw, &doc, encoding).map_err(str::to_owned)?;
        changes.sort_by_key(|e| (e.range.start,e.range.end));
        if changes.windows(2).any(|w| w[0].range.end > w[1].range.start || w[0].range.start == w[1].range.start) { return Err("Rename edits overlap; nothing changed".into()); }
        bytes += proposed_bytes; count += changes.len();
        if bytes > BYTE_LIMIT || count > EDIT_LIMIT { return Err("Rename exceeds preview limits (32 MiB / 10,000 edits)".into()); }
        if !changes.is_empty() { plan.files.push(File { path, doc, snapshot, stamp, disk, edits: changes, selected: true }); }
    }
    Ok(plan)
}

#[cfg(test)]
mod rename_tests {
    use super::*;
    use serde_json::json;
    fn change(uri: &str, start: usize, end: usize) -> serde_json::Value {
        json!({"changes":{uri:[{"range":{"start":{"line":0,"character":start},"end":{"line":0,"character":end}},"newText":"renamed"}]}})
    }
    #[test]
    fn rename_preview_uses_utf16_positions_and_rejects_overlaps_and_resource_operations() {
        let dir=tempfile::tempdir().unwrap(); let path=dir.path().join("a.rs");
        std::fs::write(&path,"😀 foo\r\n").unwrap(); let uri=semantics::lsp::file_uri(&path);
        let encoding=semantics::lsp::Encoding::Utf16;
        let plan=rename_plan(dir.path().into(),change(&uri,3,6),encoding,HashMap::new(), &AtomicU64::new(1), &HashMap::new()).unwrap();
        assert_eq!(&plan.files[0].doc.text()[plan.files[0].edits[0].range.clone()],"foo");
        let mut overlap=change(&uri,3,6); let entry=overlap["changes"][&uri][0].clone(); overlap["changes"][&uri].as_array_mut().unwrap().push(entry);
        assert!(rename_plan(dir.path().into(),overlap,encoding,HashMap::new(), &AtomicU64::new(1), &HashMap::new()).unwrap_err().contains("overlap"));
        let resource=json!({"documentChanges":[{"kind":"rename","oldUri":uri,"newUri":"file:///tmp/x"}]});
        assert!(rename_plan(dir.path().into(),resource,encoding,HashMap::new(), &AtomicU64::new(1), &HashMap::new()).unwrap_err().contains("creates, moves or deletes"));
        assert_eq!(std::fs::read(&path).unwrap(),"😀 foo\r\n".as_bytes());
    }
    #[test]
    fn rename_preview_refuses_outside_project_and_versioned_stale_edits() {
        let dir=tempfile::tempdir().unwrap(); let root=dir.path().join("p");std::fs::create_dir(&root).unwrap();
        let path=dir.path().join("outside.rs");std::fs::write(&path,b"foo").unwrap();let uri=semantics::lsp::file_uri(&path);
        assert!(rename_plan(root,change(&uri,0,3),semantics::lsp::Encoding::Utf16,HashMap::new(), &AtomicU64::new(1), &HashMap::new()).unwrap_err().contains("outside"));
        let versioned=json!({"documentChanges":[{"textDocument":{"uri":uri,"version":999},"edits":change(&uri,0,3)["changes"][&uri]}]});
        assert!(rename_plan(dir.path().into(),versioned,semantics::lsp::Encoding::Utf16,HashMap::new(), &AtomicU64::new(1), &HashMap::new()).unwrap_err().contains("another document version"));
    }
    #[test]
    fn preview_displays_control_characters_without_breaking_its_rows() {
        let dir=tempfile::tempdir().unwrap();let path=dir.path().join("a\n.rs");
        std::fs::write(&path,b"foo\tbar\n").unwrap();let uri=semantics::lsp::file_uri(&path);
        let plan=rename_plan(dir.path().into(),change(&uri,0,3),semantics::lsp::Encoding::Utf16,HashMap::new(),&AtomicU64::new(1),&HashMap::new()).unwrap();
        let text=plan.text();assert!(text.contains("a\\n.rs"));assert!(text.contains("foo\\tbar"));assert!(!text.contains('\t'));
        assert_eq!(plan.files()[0].0,"a\\n.rs");
    }
}

#[cfg(test)]
mod rename_snapshot_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn rename_refuses_positions_from_a_server_that_has_not_seen_a_dirty_sibling() {
        let dir=tempfile::tempdir().unwrap();let path=dir.path().join("a.rs");std::fs::write(&path,b"foo").unwrap();
        let old=Document::decode(path.clone(),7,b"foo");
        let snapshot=TextSnapshot::from_document(crate::text::BufferId(1),&old);
        let dirty=Arc::new(snapshot.apply_edits(8,&[TextEdit {range:0..0,text:"prefix ".into()}]).unwrap());
        let uri=semantics::lsp::file_uri(&path);
        let edit=json!({"changes":{uri:[{"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":3}},"newText":"renamed"}]}});
        let captured=HashMap::from([(path.clone(),dirty)]);
        for served in [HashMap::new(),HashMap::from([(path.clone(),semantics::SourceVersion::new(&old))])] {
            let result=rename_plan(dir.path().into(),edit.clone(),semantics::lsp::Encoding::Utf16,captured.clone(),&AtomicU64::new(10),&served);
            assert!(result.unwrap_err().contains("has not received the current text"));
        }
        assert_eq!(std::fs::read(path).unwrap(),b"foo");
    }
}
