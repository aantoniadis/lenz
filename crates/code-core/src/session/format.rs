//! Formatting and organizing imports: the file's language server edits the whole document, fenced to the tab,
//! draft version and server session asked, and the result applies as one undoable transaction of this file only.
//! Python files (Pyright does not format) run the project's configured formatter instead (ruff or black, from the
//! project's configured Python environment, else PATH) on the draft through stdin, fenced the same way; shell scripts
//! run shfmt as the shell Lenz identifies (bash-language-server is never asked to format); ERB templates run the
//! project's erb_lint with `--autocorrect` (see [`semantics::erb`]; neither Ruby LSP nor the HTML server is asked).
//! With the project's format-on-save on, Save formats first and writes the formatted draft; a formatter that fails
//! or answers for text that has changed leaves the draft unwritten, and Save Without Formatting writes it as it is.
use super::*;
use crate::text::{EditKind, TextEdit};
use std::sync::atomic::{AtomicBool, Ordering};

/// What a request asks the server for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormatKind {
    Format,
    OrganizeImports,
}

impl FormatKind {
    fn label(self) -> &'static str {
        match self {
            FormatKind::Format => "Format Document",
            FormatKind::OrganizeImports => "Organize Imports",
        }
    }
    fn question(self) -> semantics::Question {
        match self {
            FormatKind::Format => semantics::Question::Formatting,
            FormatKind::OrganizeImports => semantics::Question::OrganizeImports,
        }
    }
}

/// The request being answered; `save` when Save waits for it; `tool` when the project's formatter runs instead of
/// the language server.
pub(super) struct FormatRequest { request: u64, tab: TabId, source: SemanticSource, doc: Option<Arc<Document>>, kind: FormatKind, save: bool, tool: bool, cancelled: Arc<AtomicBool> }

impl Drop for FormatRequest {
    fn drop(&mut self) { self.cancelled.store(true, Ordering::Relaxed); }
}

/// What a run of the project's formatter came to.
#[derive(Debug)]
pub enum ToolResult {
    /// It printed text: the one edit making the text it was given into it (`None`: nothing to change).
    Formatted { tool: &'static str, edit: Option<TextEdit> },
    /// It cannot be run (none configured, not installed, or it does not do this); a save goes ahead unformatted.
    Unavailable(String),
    /// It ran and failed: nothing changes, and a save waiting for it writes nothing.
    Failed(String),
}

/// How long a project formatter may take.
const TOOL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

impl FormatRequest {
    fn current(&self, state: &State) -> bool { state.view(self.tab).is_some_and(|v| self.source.matches(v.snapshot())) }
    pub(super) fn request(&self) -> u64 { self.request }
    /// Whether the project's formatter runs rather than a language server.
    pub(super) fn tool(&self) -> bool { self.tool }
    /// What the status line says while it waits.
    pub(super) fn status(&self) -> String {
        match (self.kind, self.save) {
            (FormatKind::Format, true) => "Formatting before saving".into(),
            (FormatKind::Format, false) => "Formatting".into(),
            (FormatKind::OrganizeImports, _) => "Organizing imports".into(),
        }
    }
}

impl Session {
    /// Save, formatting first when the file's project asks for it.
    pub(super) fn save_command(&mut self, tab: TabId) -> Effect {
        if self.save_disabled(tab).is_some() || !self.format_on_save(tab) { return self.save(tab); }
        self.format_document(tab, FormatKind::Format, true)
    }

    /// Whether `tab`'s file is in the open project and that project formats on save. An environment file never is:
    /// its values are written exactly as typed.
    fn format_on_save(&self, tab: TabId) -> bool {
        let Some(view) = self.state.view(tab) else { return false };
        if view.snapshot().language() == Some(Language::Env) { return false; }
        let in_project = self.state.project.as_ref().is_some_and(|p| view.snapshot().source().starts_with(p.root()));
        in_project && self.state.project_preferences().is_some_and(|p| p.format_on_save)
    }

    /// Ask `tab`'s server to format the document or organize its imports (`save`: then write it).
    pub(super) fn format_document(&mut self, tab: TabId, kind: FormatKind, save: bool) -> Effect {
        let id = match kind { FormatKind::Format => CommandId::FormatDocument, FormatKind::OrganizeImports => CommandId::OrganizeImports };
        if !save && let Some(why) = self.disabled(id) { return self.format_message(kind, why); }
        let Some(snapshot) = self.state.view(tab).map(|v| v.snapshot().clone()) else { return Effect::default() };
        let tool = project_formats(&snapshot);
        self.cancel_format();
        if !tool { self.sync_semantics(); }
        self.semantic_seq += 1;
        let request = self.semantic_seq;
        self.semantic_wait.remove(&semantics::Lane::Format);
        self.state.formatting = Some(FormatRequest { request, tab, source: SemanticSource::new(&snapshot), doc: None, kind, save, tool, cancelled: Arc::new(AtomicBool::new(false)) });
        let Some(doc) = self.admitted_document(tab, Priority::Visible) else {
            self.defer_semantic(tab, request, 0, kind.question(), None);
            return Effect { repaint: true, ..Effect::default() };
        };
        if !tool { self.send_admitted(&doc); }
        let effect = self.ask_format(doc);
        Effect { repaint: true, ..effect }
    }

    /// Send the pending request about `doc`, with the file's indentation as formatting options (or run the project's
    /// formatter on it).
    pub(super) fn ask_format(&mut self, doc: Arc<Document>) -> Effect {
        let Some((request, kind, tool)) = self.state.formatting.as_ref().map(|f| (f.request, f.kind, f.tool)) else { return Effect::default() };
        if tool { return self.run_project_formatter(doc, request, kind); }
        let unit = self.indent_unit();
        let extra = match kind {
            FormatKind::Format => serde_json::json!({ "options": { "tabSize": unit.width, "insertSpaces": unit.style == crate::preferences::Style::Space } }),
            FormatKind::OrganizeImports => serde_json::Value::Null,
        };
        match self.state.semantics.ask_with(&doc, 0, request, kind.question(), extra) {
            Ok(()) => { if let Some(f) = self.state.formatting.as_mut() { f.doc = Some(doc); } Effect::default() }
            Err(why) => self.format_unavailable(why),
        }
    }

    /// Run the project's formatter on `doc` off the UI thread; its answer comes back as
    /// [`Completion::ProjectFormatted`].
    fn run_project_formatter(&mut self, doc: Arc<Document>, request: u64, kind: FormatKind) -> Effect {
        let project = self.state.project.as_ref().map(|p| p.root().to_owned()).filter(|r| doc.source().starts_with(r));
        let (settings, tx) = (self.state.semantics.settings().clone(), self.tx.clone());
        let organize = kind == FormatKind::OrganizeImports;
        let job = doc.clone();
        if doc.language() == Some(Language::Json) {
            let preference = project.as_ref().and(self.state.project_preferences()).and_then(|p| p.indent);
            let resolved = crate::preferences::resolve(doc.source(), preference, doc.language());
            let unit = crate::edit_ops::IndentUnit::new(resolved.style.0, resolved.size.0, resolved.tab_width.0);
            let Some(cancelled) = self.state.formatting.as_ref().map(|f| f.cancelled.clone()) else { return Effect::default() };
            self.jobs.submit(Priority::Visible, move || {
                let result = if organize { ToolResult::Unavailable("JSON has no imports to organize".into()) }
                    else { json::format(&job, &unit, &cancelled) };
                let _ = tx.send_blocking(Completion::ProjectFormatted { request, result });
            });
            if let Some(f) = self.state.formatting.as_mut() { f.doc = Some(doc); }
            return Effect::default();
        }
        if doc.language() == Some(Language::Sql) {
            return match self.sql_action_disabled(CommandId::FormatDocument) {
                Some(why) => self.format_unavailable(why.into()),
                None => self.sql_request(sql_work::Operation::Format(request)),
            };
        }
        if erb(doc.source()) {
            self.jobs.submit(Priority::Visible, move || {
                let result = run_erb_formatter(&job, organize, project.as_deref(), &settings);
                let _ = tx.send_blocking(Completion::ProjectFormatted { request, result });
            });
            if let Some(f) = self.state.formatting.as_mut() { f.doc = Some(doc); }
            return Effect::default();
        }
        if crate::template::Kind::of(doc.source()) == Some(crate::template::Kind::Haml) {
            self.jobs.submit(Priority::Visible, move || {
                let result = haml_work::format(&job, organize);
                let _ = tx.send_blocking(Completion::ProjectFormatted { request, result });
            });
            if let Some(f) = self.state.formatting.as_mut() { f.doc = Some(doc); }
            return Effect::default();
        }
        if doc.language() == Some(Language::Bash) {
            let unit = self.indent_unit();
            let indent = (unit.style == crate::preferences::Style::Space).then(|| u8::try_from(unit.size).unwrap_or(u8::MAX));
            self.jobs.submit(Priority::Visible, move || {
                let result = run_shell_formatter(&job, organize, project.as_deref(), &settings, indent);
                let _ = tx.send_blocking(Completion::ProjectFormatted { request, result });
            });
            if let Some(f) = self.state.formatting.as_mut() { f.doc = Some(doc); }
            return Effect::default();
        }
        self.jobs.submit(Priority::Visible, move || {
            let result = run_python_formatter(job.source(), job.text(), organize, project.as_deref(), &settings, std::env::var_os("PATH").as_deref());
            let _ = tx.send_blocking(Completion::ProjectFormatted { request, result });
        });
        if let Some(f) = self.state.formatting.as_mut() { f.doc = Some(doc); }
        Effect::default()
    }

    /// The project's formatter answered: applied as one transaction while the text is the version it was given, then
    /// written when Save waited; otherwise nothing changes and nothing is written.
    pub(super) fn project_formatted(&mut self, request: u64, result: ToolResult) -> Effect {
        if self.state.formatting.as_ref().is_none_or(|f| f.request != request || !f.tool) { return Effect::default() }
        let (tool, edit) = match result {
            ToolResult::Formatted { tool, edit } => (tool, edit),
            ToolResult::Unavailable(why) => return self.format_unavailable(why),
            ToolResult::Failed(why) => return self.format_refused(&why),
        };
        let Some(f) = self.state.formatting.as_ref() else { return Effect::default() };
        if !f.current(&self.state) { return self.format_refused(&format!("the file changed while {tool} ran")); }
        if let Some(why) = self.state.view(f.tab).and_then(|v| v.edit_disabled()) { return self.format_refused(why); }
        let Some(f) = self.state.formatting.take() else { return Effect::default() };
        let mut effect = Effect { repaint: true, ..Effect::default() };
        let changes = edit.is_some();
        if let Some(edit) = edit { effect = self.edit(f.tab, f.source.key, vec![edit], EditKind::Replace); effect.repaint = true; }
        if f.save { effect.repaint |= self.save(f.tab).repaint; }
        else if changes { self.state.messages.push(format!("{}: done with {tool}", f.kind.label())); }
        else { self.state.messages.push(format!("{}: nothing to change", f.kind.label())); }
        effect
    }

    /// The pending request's answer is not applied: said, with what happens to a save waiting for it.
    fn format_refused(&mut self, why: &str) -> Effect {
        let Some(f) = self.state.formatting.take() else { return Effect::default() };
        let tail = if f.save { "; nothing was written (Save Without Formatting writes the draft as it is)" } else { "" };
        self.state.messages.push(format!("{}: {why}{tail}", f.kind.label()));
        Effect { repaint: true, ..Effect::default() }
    }

    /// The server could not be asked. A save goes ahead unformatted and says so; nothing was sent to format.
    pub(super) fn format_unavailable(&mut self, why: String) -> Effect {
        let Some(f) = self.state.formatting.take() else { return Effect::default() };
        self.semantic_wait.remove(&semantics::Lane::Format);
        if !f.save { return self.format_message(f.kind, why); }
        self.state.messages.push(format!("Saved without formatting: {why}"));
        let mut effect = self.save(f.tab);
        effect.repaint = true;
        effect
    }

    fn format_message(&mut self, kind: FormatKind, why: impl std::fmt::Display) -> Effect {
        self.state.messages.push(format!("{}: {why}", kind.label()));
        Effect { repaint: true, ..Effect::default() }
    }

    /// Stop waiting (a late answer is then dropped).
    pub(super) fn cancel_format(&mut self) -> bool {
        self.sql_cancel(3);
        if self.state.formatting.take().is_none() { return false; }
        self.semantic_wait.remove(&semantics::Lane::Format);
        self.state.semantics.cancel_lane(semantics::Lane::Format);
        true
    }

    /// Escape while it waits: nothing is applied, and a save that waited for it is not written.
    pub(super) fn cancel_format_by_user(&mut self) -> bool {
        let Some((kind, save)) = self.state.formatting.as_ref().map(|f| (f.kind, f.save)) else { return false };
        self.cancel_format();
        let tail = if save { "; nothing was written" } else { "" };
        self.state.messages.push(format!("{}: cancelled{tail}", kind.label()));
        true
    }

    /// The server answered: applied as one transaction while the text is the version asked about, then written when
    /// Save waited; otherwise nothing changes and nothing is written.
    pub(super) fn format_reply(&mut self, reply: semantics::Reply) -> Effect {
        if self.state.formatting.as_ref().is_none_or(|f| f.request != reply.request) { return Effect::default() }
        let Some(f) = self.state.formatting.take() else { return Effect::default() };
        let tail = if f.save { "; nothing was written (Save Without Formatting writes the draft as it is)" } else { "" };
        let refuse = |s: &mut Self, why: &str| { s.state.messages.push(format!("{}: {why}{tail}", f.kind.label())); Effect { repaint: true, ..Effect::default() } };
        let Some(doc) = f.doc.clone() else { return Effect::default() };
        if !f.current(&self.state) || !self.state.semantics.can_ask(reply.session) {
            return refuse(self, "the file changed while the language server answered");
        }
        // A template's answer is about the view (its Ruby) that server holds.
        let Some(doc) = self.state.semantics.served(&doc, reply.session) else { return refuse(self, "the language server answered for another view of the template") };
        let value = match reply.result {
            Ok(value) => value,
            Err(why) => return refuse(self, &why),
        };
        let edits = match f.kind {
            FormatKind::Format if value.is_null() => Ok((Vec::new(), None)),
            FormatKind::Format => code_action::text_edits(&value, &doc, reply.encoding).and_then(code_action::sorted).map(|e| (e, None)).map_err(str::to_owned),
            FormatKind::OrganizeImports => organize(&value, &doc, reply.encoding, &|c: &str| self.state.semantics.runs_command(reply.session, c)),
        };
        let (edits, command) = match edits {
            Ok(found) => found,
            Err(why) => return refuse(self, &why),
        };
        if let Some(why) = code_action::template_refusal(&doc, &edits) { return refuse(self, &why); }
        if let Some(why) = self.state.view(f.tab).and_then(|v| v.edit_disabled()) { return refuse(self, why); }
        // An edit that leaves the text as it is changes nothing and adds no undo step.
        let changes = edits.iter().any(|e| doc.text().get(e.range.clone()) != Some(&*e.text));
        let mut effect = Effect { repaint: true, ..Effect::default() };
        if changes { effect = self.edit(f.tab, f.source.key, edits, EditKind::Replace); effect.repaint = true; }
        // The server organizes through its own command; its edit is checked when it comes back.
        if let Some(command) = command {
            effect.repaint |= self.run_server_command(reply.session, &command, semantics::Lane::Format, f.tab, "Organize Imports").repaint;
            return effect;
        }
        if f.save { effect.repaint |= self.save(f.tab).repaint; }
        else { self.state.messages.push(format!("{}: {}", f.kind.label(), if changes { "done" } else { "nothing to change" })); }
        effect
    }
}

/// Whether a file formats through its project's formatter rather than its language server: Python, which Pyright
/// does not format, shell scripts, which shfmt formats as the shell Lenz identifies, and ERB templates (erb_lint).
/// JSON uses Lenz's source-preserving offline whitespace formatter.
pub(super) fn project_formats(snapshot: &crate::text::TextSnapshot) -> bool {
    matches!(snapshot.language(), Some(Language::Python | Language::Bash | Language::Sql | Language::Json)) || erb(snapshot.source()) || crate::template::Kind::of(snapshot.source()) == Some(crate::template::Kind::Haml)
}

/// Whether `file` is an ERB template.
pub(super) fn erb(file: &Path) -> bool { crate::template::Kind::of(file) == Some(crate::template::Kind::Erb) }

/// The erb_lint an ERB template's project configures (its root: the open folder holding it, else its Ruby workspace),
/// looked up on the Ruby provider's `PATH`. Reads project files.
pub(crate) fn erb_formatter(file: &Path, project: Option<&Path>, settings: &semantics::provider::Settings) -> (Option<semantics::erb::ErbLint>, PathBuf, Option<std::ffi::OsString>) {
    let root = project.filter(|p| file.starts_with(p)).map(Path::to_owned).or_else(|| semantics::provider::ruby_root(file, None)).unwrap_or_else(|| file.parent().unwrap_or(Path::new("/")).to_owned());
    let path = semantics::provider::PROVIDERS.iter().find(|p| p.id == "ruby-lsp").and_then(|p| settings.launch(p).path().map(std::ffi::OsStr::to_owned)).or_else(|| std::env::var_os("PATH"));
    (semantics::erb::configured(file, &root, path.as_deref()), root, path)
}

/// Run the project's erb_lint on an ERB template's draft (see [`semantics::erb::format`]); templates have no imports to
/// organize. Reads project files and runs erb_lint: off the UI thread.
fn run_erb_formatter(doc: &Document, organize: bool, project: Option<&Path>, settings: &semantics::provider::Settings) -> ToolResult {
    if organize { return ToolResult::Unavailable("ERB templates have no imports to organize".into()); }
    let (_, root, path) = erb_formatter(doc.source(), project, settings);
    match semantics::erb::format(doc.source(), doc.text(), &root, path.as_deref()) {
        semantics::erb::Formatted::Text(out) => ToolResult::Formatted { tool: "erb_lint", edit: difference(doc.text(), out) },
        semantics::erb::Formatted::Unavailable(why) => ToolResult::Unavailable(why),
        semantics::erb::Formatted::Failed(why) => ToolResult::Failed(why),
    }
}

/// bash-language-server's merged settings for `file` (its workspace: the browsing root holding it, else its folder) and
/// the `PATH` its tools are looked up on.
pub(crate) fn shell_settings(file: &Path, project: Option<&Path>, settings: &semantics::provider::Settings) -> (serde_json::Value, Option<std::ffi::OsString>) {
    let Some((_, provider)) = semantics::provider::for_language(Language::Bash) else { return (serde_json::Value::Null, std::env::var_os("PATH")) };
    let root = semantics::provider::web_root(file, project);
    let path = settings.launch(provider).path().map(std::ffi::OsStr::to_owned).or_else(|| std::env::var_os("PATH"));
    (settings.initialization(provider, root.as_deref()), path)
}

/// Run shfmt on `doc`'s text as its shell (see [`semantics::shell::format`]); shell scripts have no imports to organize.
/// Reads project files and runs shfmt: off the UI thread.
fn run_shell_formatter(doc: &Document, organize: bool, project: Option<&Path>, settings: &semantics::provider::Settings, indent: Option<u8>) -> ToolResult {
    if organize { return ToolResult::Unavailable("shell scripts have no imports to organize".into()); }
    let (options, path) = shell_settings(doc.source(), project, settings);
    match semantics::shell::format(doc.source(), doc.text(), &options, path.as_deref(), indent) {
        semantics::shell::Formatted::Text(out) => ToolResult::Formatted { tool: "shfmt", edit: difference(doc.text(), out) },
        semantics::shell::Formatted::Unavailable(why) => ToolResult::Unavailable(why),
        semantics::shell::Formatted::Failed(why) => ToolResult::Failed(why),
    }
}

/// The formatter a Python file's project configures, found in the project's configured Python environment (else, with
/// none configured, the project's `.venv`/`venv`), else on `path`, with the root it was looked up under.
pub(crate) fn python_formatter(file: &Path, project: Option<&Path>, settings: &semantics::provider::Settings, path: Option<&std::ffi::OsStr>) -> Option<crate::preferences::Formatter> {
    let workspace = semantics::provider::python_root(file, project)?;
    let root = project.filter(|p| file.starts_with(p)).unwrap_or(&workspace);
    let pyright = semantics::provider::PROVIDERS.iter().find(|p| p.id == "pyright")?;
    use crate::preferences::Tools;
    let configured = semantics::python::configured(&workspace, &settings.initialization(pyright, Some(&workspace)));
    let folder = configured.as_ref().ok().and_then(|c| c.as_ref()?.tools());
    let tools = match (&configured, &folder) {
        (Ok(Some(_)), Some(folder)) => Tools::Environment(folder),
        (Ok(None), _) => Tools::Project,
        // A configured environment that is missing: never another project environment's tool, only PATH.
        _ => Tools::PathOnly,
    };
    crate::preferences::formatter(file, root, Some(Language::Python), path, tools)
}

/// Run `file`'s project formatter on `text` (`organize`: sort its imports): see [`ToolResult`]. Reads project files
/// and runs the tool: call off the UI thread.
fn run_python_formatter(file: &Path, text: &str, organize: bool, project: Option<&Path>, settings: &semantics::provider::Settings, path: Option<&std::ffi::OsStr>) -> ToolResult {
    let Some(formatter) = python_formatter(file, project, settings, path) else {
        let what = if organize { "no ruff is configured for this project to organize imports (ruff.toml, .ruff.toml or [tool.ruff] in pyproject.toml); Pyright offers no organize imports, and Lenz never installs formatters" }
            else { "no Python formatter is configured for this project (ruff.toml, .ruff.toml or [tool.ruff] in pyproject.toml for ruff; [tool.black] in pyproject.toml for black); Pyright does not format, and Lenz never installs formatters" };
        return ToolResult::Unavailable(what.into());
    };
    let Some(args) = formatter.stdin_arguments(organize, file) else {
        return ToolResult::Unavailable(format!("{} does not organize imports; Pyright offers no organize imports either (configure ruff for this project to use it)", formatter.tool));
    };
    let configured = formatter.config.as_ref().map_or_else(|| "its defaults".into(), |c| c.display().to_string());
    let Some(executable) = formatter.executable else {
        return ToolResult::Unavailable(format!("{} is configured by {configured} but was not found in the project's Python environment or on PATH; Lenz never installs formatters", formatter.tool));
    };
    let cwd = file.parent().unwrap_or(Path::new("/"));
    match semantics::runtime::run_with_input(&executable, &args, text, cwd, TOOL_TIMEOUT) {
        Ok(output) => ToolResult::Formatted { tool: formatter.tool, edit: difference(text, output) },
        Err(why) => ToolResult::Failed(format!("{} failed: {why}", formatter.tool)),
    }
}

/// The single edit turning `old` into `new` (the span between their common start and end), or `None` when they are
/// the same: a formatter's whole-text answer kept as a small Undo record.
pub(super) fn difference(old: &str, new: String) -> Option<TextEdit> {
    if old == new { return None; }
    let mut start = old.bytes().zip(new.bytes()).take_while(|(a, b)| a == b).count();
    while !old.is_char_boundary(start) || !new.is_char_boundary(start) { start -= 1; }
    let limit = old.len().min(new.len()) - start;
    let mut end = old.bytes().rev().zip(new.bytes().rev()).take(limit).take_while(|(a, b)| a == b).count();
    while !old.is_char_boundary(old.len() - end) || !new.is_char_boundary(new.len() - end) { end -= 1; }
    // Both ends are character boundaries of both texts; replacing the whole text is the fallback all the same.
    Some(match new.get(start..new.len() - end) {
        Some(text) => TextEdit { range: start..old.len() - end, text: text.into() },
        None => TextEdit { range: 0..old.len(), text: new.into() },
    })
}

/// The edits of the server's organize-imports action: the first `source.organizeImports` action, when it changes only
/// this file; an empty answer has nothing to change.
fn organize(value: &serde_json::Value, doc: &Document, encoding: semantics::lsp::Encoding, runs: &dyn Fn(&str) -> bool) -> Result<(Vec<TextEdit>, Option<serde_json::Value>), String> {
    let items = value.as_array().map(Vec::as_slice).unwrap_or_default();
    let organizing = items.iter().filter(|i| i["kind"].as_str().is_some_and(|k| k == "source.organizeImports" || k.starts_with("source.organizeImports.")));
    let Some(action) = organizing.filter_map(|i| code_action::action(i, doc, encoding, runs)).next() else { return Ok((Vec::new(), None)) };
    match action.unavailable.clone() {
        Some(why) if why == code_action::NO_CHANGE => Ok((Vec::new(), None)),
        Some(why) => Err(why),
        None => Ok(action.into_parts()),
    }
}

#[cfg(test)]
mod tests {
    use super::difference;

    fn apply(old: &str, new: &str) -> Option<(std::ops::Range<usize>, String)> {
        let edit = difference(old, new.to_owned())?;
        let mut text = old.to_owned();
        text.replace_range(edit.range.clone(), &edit.text);
        assert_eq!(text, new, "the edit makes the new text");
        Some((edit.range, edit.text.to_string()))
    }

    #[test]
    fn a_formatters_answer_is_one_edit_between_the_common_start_and_end() {
        assert_eq!(apply("x = 1\n", "x = 1\n"), None);
        assert_eq!(apply("x  =  1\ny = 2\n", "x = 1\ny = 2\n"), Some((2..5, "=".into())));
        // Characters sharing leading or trailing bytes are replaced whole.
        assert_eq!(apply("s = 'é'\n", "s = 'è'\n"), Some((5..7, "è".into())));
        assert_eq!(apply("a𝄞", "b𝄞"), Some((0..1, "b".into())));
        assert_eq!(apply("𝄞x", "𝄠x"), Some((0..4, "𝄠".into())));
        // Repeated text: the common start and end never overlap.
        assert_eq!(apply("aaa", "aa"), Some((2..3, String::new())));
        assert_eq!(apply("aa", "aaa"), Some((2..2, "a".into())));
        assert_eq!(apply("", "x\n"), Some((0..0, "x\n".into())));
    }
}
