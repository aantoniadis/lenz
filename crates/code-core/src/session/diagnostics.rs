//! Diagnostics on screen: only those a server published for the exact version shown, so a stale one is never drawn
//! on text it was not computed for. Next and Previous Problem move the caret between them; the list names each one.
use super::*;
use crate::semantics::diagnostics::FileDiagnostics;

impl State {
    /// Exact-version diagnostics; parser results cover syntax even without a running server.
    pub fn diagnostics(&self, tab: TabId) -> Option<Arc<FileDiagnostics>> {
        let view = self.view(tab)?;
        let snapshot = view.snapshot();
        let server = if snapshot.language() == Some(Language::Sql) {
            self.sql_view.problems.get(&snapshot.key().buffer).filter(|(identity, _)| self.sql_dialect_for(view.tab()).and_then(|s| s.dialect) == Some(identity.dialect)).map(|(_, found)| found.clone())
        } else { self.semantics.diagnostics(snapshot.source()) }.filter(|found| found.describes(snapshot));
        let syntax = self.syntax_problems.get(&snapshot.key().buffer).filter(|found| found.describes(snapshot));
        match (server, syntax) {
            (Some(server), Some(syntax)) => {
                let mut cache = self.merged_problems.lock().unwrap_or_else(|e| e.into_inner());
                let entry = cache.entry(snapshot.key().buffer).or_insert_with(|| (std::sync::Weak::new(), std::sync::Weak::new(), server.clone()));
                if !entry.0.ptr_eq(&Arc::downgrade(&server)) || !entry.1.ptr_eq(&Arc::downgrade(syntax)) {
                    *entry = (Arc::downgrade(&server), Arc::downgrade(syntax), Arc::new(FileDiagnostics::with_syntax(&server, syntax)));
                }
                Some(entry.2.clone())
            }
            (Some(server), None) => Some(server),
            (None, Some(syntax)) => Some(syntax.clone()),
            (None, None) => None,
        }
    }

    /// One compact inline annotation for a visible logical line; stale versions never provide text or offsets.
    pub fn inline_problem(&self, tab: TabId, line: usize) -> Option<crate::semantics::diagnostics::InlineProblem> {
        let snapshot = self.view(tab)?.snapshot();
        let found = self.diagnostics(tab)?;
        let (problem, other_count) = found.starting_in(snapshot.line_with_ending(line)?, line + 1 == snapshot.line_count())?;
        let range = snapshot.line_range(line)?;
        let byte = snapshot.floor_char_boundary(problem.range.start).clamp(range.start, range.end) - range.start;
        let mut message = String::new();
        let mut chars = problem.message.chars();
        let mut ended = false;
        for c in chars.by_ref().take(160) {
            if matches!(c, '\n' | '\r') { ended = true; break; }
            message.push(if c.is_control() || lenz_content::display::hides_text(c) { ' ' } else { c });
        }
        if !ended && chars.next().is_some() { message.push('…'); }
        Some(crate::semantics::diagnostics::InlineProblem { severity: problem.severity, message, other_count, byte, empty: problem.range.is_empty() })
    }

    /// Errors in a tab; a previous check remains marked with `?` until current text is checked.
    pub fn problem_badge(&self, tab: TabId) -> String {
        let Some(view) = self.view(tab) else {
            return self.tabs.iter().find(|t| t.id == tab).map(|tab| {
                let count = self.path_problem_count(&tab.source, false);
                if count == 0 { String::new() } else { format!("!{count}? ") }
            }).unwrap_or_default();
        };
        let current = self.diagnostics(tab);
        let stale_server = self.semantics.diagnostics(view.snapshot().source()).filter(|found| !found.describes(view.snapshot()));
        let previous = stale_server.as_ref().or_else(|| self.syntax_problems.get(&view.snapshot().key().buffer));
        let count = |found: &FileDiagnostics| found.items.iter().filter(|d| d.severity == crate::semantics::diagnostics::Severity::Error).count();
        let errors = current.as_ref().map_or(0, |f| count(f)).max(previous.map_or(0, |f| count(f)));
        let pending = current.is_none() || stale_server.is_some();
        if errors > 0 { format!("!{errors}{} ", if pending { "?" } else { "" }) }
        else { String::new() }
    }

    /// A file or folder's known errors, including workspace publications for unopened files.
    pub fn path_problem_count(&self, path: &Path, directory: bool) -> usize {
        // Most painted rows are files: avoid building a workspace map for each visible filename.
        if !directory {
            let count = |items: &[crate::semantics::diagnostics::Diagnostic]| items.iter().filter(|d| d.severity == crate::semantics::diagnostics::Severity::Error).count();
            if let Some((tab, view)) = self.tabs.iter().filter(|t| t.source == path).find_map(|tab| self.view(tab.id).map(|view| (tab, view)))
                && let Some(current) = self.diagnostics(tab.id) {
                let stale = self.semantics.diagnostics(path).filter(|d| !d.describes(view.snapshot())).map_or(0, |d| count(&d.items));
                return count(&current.items).max(stale);
            }
            let syntax = self.syntax_locations.get(path).map_or(0, Vec::len);
            let server = self.semantics.published_problems().filter(|(p, _)| *p == path).map(|(_, items)| items.iter().filter(|d| d.severity == crate::semantics::diagnostics::Severity::Error).count()).max().unwrap_or(0);
            return syntax.max(server);
        }
        let matches = |candidate: &Path| if directory { candidate.starts_with(path) } else { candidate == path };
        let mut counts = std::collections::HashMap::new();
        for (path, items) in self.syntax_locations.iter().filter(|(p, _)| matches(p)) { counts.insert(path.clone(), items.len()); }
        for (path, items) in self.semantics.published_problems().filter(|(p, _)| matches(p)) {
            counts.entry(path.to_path_buf()).and_modify(|n: &mut usize| *n = (*n).max(items.iter().filter(|d| d.severity == crate::semantics::diagnostics::Severity::Error).count()))
                .or_insert_with(|| items.iter().filter(|d| d.severity == crate::semantics::diagnostics::Severity::Error).count());
        }
        for tab in self.tabs.iter().filter(|t| matches(&t.source)) {
            if let Some(view) = self.view(tab.id) {
                let current = self.diagnostics(tab.id);
                let prior = self.syntax_problems.get(&view.snapshot().key().buffer);
                if let Some(found) = current.as_ref().or(prior) {
                    let count = found.items.iter().filter(|d| d.severity == crate::semantics::diagnostics::Severity::Error).count();
                    let stale = self.semantics.diagnostics(view.snapshot().source()).filter(|d| !d.describes(view.snapshot())).map_or(0, |d| d.items.iter().filter(|d| d.severity == crate::semantics::diagnostics::Severity::Error).count());
                    counts.insert(tab.source.clone(), count.max(stale));
                }
            }
        }
        counts.values().sum()
    }

    pub fn row_problem_count(&self, row: &crate::project::TreeRow) -> usize {
        let Some(project) = &self.project else { return 0 };
        self.path_problem_count(&project.root().join(row.path.as_ref()), row.dir.is_some())
    }

    /// The most serious diagnostic at the active caret, as the status line shows it.
    pub fn problem_at_caret(&self) -> Option<String> {
        let tab = self.active()?;
        let at = self.selection()?.head;
        let found = self.diagnostics(tab.id)?;
        let d = found.at(at).into_iter().next()?;
        Some(problem_line(d))
    }
}

/// `Error: message (source code)`.
fn problem_line(d: &crate::semantics::diagnostics::Diagnostic) -> String {
    let origin = match (&d.source, &d.code) {
        (Some(s), Some(c)) => format!(" ({s} {c})"),
        (Some(s), None) => format!(" ({s})"),
        (None, Some(c)) => format!(" ({c})"),
        (None, None) => String::new(),
    };
    format!("{}: {}{origin}", d.severity.label(), d.message.lines().next().unwrap_or_default())
}

impl Session {
    /// Move the caret to the next (or previous) diagnostic of the shown version, wrapping around.
    pub(super) fn step_problem(&mut self, forward: bool) -> Effect {
        let repaint = Effect { repaint: true, ..Effect::default() };
        let Some(tab) = self.state.active else { return Effect::default() };
        let Some(found) = self.state.diagnostics(tab).filter(|f| !f.items.is_empty()) else {
            self.state.messages.push("No problems reported for the text shown".into());
            return repaint;
        };
        let caret = self.state.selection().map_or(0, |s| s.head);
        let starts: Vec<usize> = found.items.iter().map(|d| d.range.start).collect();
        let index = if forward {
            starts.iter().position(|s| *s > caret).unwrap_or(0)
        } else {
            starts.iter().rposition(|s| *s < caret).unwrap_or(starts.len() - 1)
        };
        let d = &found.items[index];
        let from = self.here();
        let Some(doc) = self.state.current() else { return Effect::default() };
        let line = doc.line_of(d.range.start);
        let byte = d.range.start - doc.reader_line_range(line).start;
        let Some(t) = self.active_mut() else { return Effect::default() };
        t.place_selection(Some(Selection { anchor: d.range.start, head: d.range.start }));
        t.expansion = None;
        t.reveal(line);
        let id = t.id;
        self.bring_into_view(id, line, byte);
        if let Some(to) = self.here() { self.visit(from, to); }
        self.state.messages.push(format!("Problem {} of {}: {}", index + 1, found.items.len(), problem_line(d)));
        repaint
    }

    /// The problems of the shown version, one per line, for the adapters' details dialog. Never the source itself.
    pub fn problems_details(&self) -> String {
        let Some(tab) = self.state.active() else { return "No file is open".into() };
        let Some(doc) = self.state.current() else { return "No file is open".into() };
        // A shell script ShellCheck did not check says why (missing, zsh).
        let note = self.state.semantics.shell_lint(doc.source()).and_then(|l| l.note());
        let Some(found) = self.state.diagnostics(tab.id) else {
            if let Some(note) = note { return note; }
            if doc.language() == Some(Language::Sql) {
                return self.state.sql_view.notes.get(&doc.key().buffer).filter(|(i, _)| i.key == doc.key()).map_or_else(|| "Offline SQL syntax check is pending or unavailable; choose a dialect in SQL Dialect…".into(), |(_, why)| format!("Offline SQL syntax check unavailable: {why}"));
            }
            return match self.state.provider() {
                Some(_) => "No problems are reported for the text shown. After an edit they return once the language server has checked the new text.".into(),
                None => "No language server reports problems for this file.".into(),
            };
        };
        if found.items.is_empty() && doc.language() == Some(Language::Bash) {
            if let Some(note) = note { return format!("{note}\n\nThe built-in syntax parser reports no syntax problems in the text shown."); }
            let checked = self.current_shell_lint().and_then(|lint| lint.diagnostics.as_ref()).is_some_and(|found| found.describes(&doc));
            return if checked { "ShellCheck reports no problems in this file.".into() }
                else { "The built-in syntax parser reports no syntax problems. ShellCheck has not checked the text shown yet.".into() };
        }
        if found.items.is_empty() && doc.language() == Some(Language::Sql) { return "The offline SQL parser reports no syntax problems; no schema or database validation was performed.".into(); }
        if found.items.is_empty() {
            return "Current syntax and available language-server checks report no problems. This is not a compiler or workspace validation; server checks may still be pending or unavailable.".into();
        }
        let count = |s| found.items.iter().filter(|d| d.severity == s).count();
        use crate::semantics::diagnostics::Severity as S;
        let mut text = note.map_or_else(String::new, |note| format!("{note}\n\n"));
        text.push_str(&format!("{} errors, {} warnings, {} other\n\n", count(S::Error), count(S::Warning), count(S::Information) + count(S::Hint)));
        for d in &found.items {
            let line = doc.line_of(d.range.start);
            let column = d.range.start - doc.reader_line_range(line).start + 1;
            text.push_str(&format!("{}:{column}  {}\n", line + 1, problem_line(d)));
        }
        text.push_str("\nNext Problem / Previous Problem move the caret between them; Show Quick Fixes offers the server's fixes at the caret.");
        text
    }
}

pub(super) type MergedProblems = std::collections::HashMap<crate::text::BufferId,
    (std::sync::Weak<FileDiagnostics>, std::sync::Weak<FileDiagnostics>, Arc<FileDiagnostics>)>;

pub(super) fn syntax_diagnostics(doc: &Document, errors: Vec<highlight::SyntaxError>) -> FileDiagnostics {
    use crate::semantics::diagnostics::{Diagnostic, Severity};
    FileDiagnostics::new(doc, errors.into_iter().map(|error| Diagnostic {
        range: error.range, severity: if error.limited { Severity::Information } else { Severity::Error }, message: compact_message(&error.message),
        source: Some("syntax parser".into()), code: None, raw: serde_json::Value::Null,
    }).collect())
}

impl Session {
    pub(super) fn show_problems(&mut self) -> Effect {
        self.search_seq += 1;
        let root = self.state.project.as_ref().map(|p| (p.root().to_path_buf(), p.generation()));
        let mut results = self.new_results(self.search_seq, ResultsKind::Problems, String::new(), Scope::Project, root, 0, false);
        let (mut paths, folders_first) = (std::collections::HashSet::new(), self.state.folders_first());
        for tab in &self.state.tabs {
            let Some(snapshot) = self.state.source(tab.id) else { continue };
            if paths.contains(&tab.source) { continue; }
            let current = self.state.diagnostics(tab.id);
            let server = self.state.semantics.diagnostics(snapshot.source());
            let previous = server.as_ref().or_else(|| self.state.syntax_problems.get(&snapshot.key().buffer));
            let mut checks: Vec<(&FileDiagnostics, bool)> = current.as_ref().map(|d| vec![(d.as_ref(), true)]).unwrap_or_default();
            if let Some(previous) = previous.filter(|d| !d.describes(snapshot)) { checks.push((previous.as_ref(), false)); }
            if checks.is_empty() { continue; }
            paths.insert(tab.source.clone());
            results.captured.insert(tab.source.clone(), search::CapturedSource::new(snapshot.clone()));
            let path: Arc<str> = results.root.as_ref().and_then(|(root, _)| tab.source.strip_prefix(root).ok()).unwrap_or(&tab.source).to_string_lossy().into_owned().into();
            // Stale coordinates are not used: their text stays visible with an explicit pending label.
            for (found, current) in checks {
                for d in &found.items {
                    let at = if current { d.range.start.min(snapshot.len_bytes()) } else { 0 };
                    let line = snapshot.line_of(at);
                    let column = at - snapshot.reader_line_range(line).start;
                    let message = format!("{}{}", if current { "" } else { "[previous check; checking current text] " }, problem_line(d));
                    results.hits.push(search::Hit { path: path.clone(), line, range: column..column, text: message.into(), decl: None, origin: None });
                }
            }
        }
        for (source, items) in self.state.semantics.published_problems().chain(self.state.syntax_locations.iter().map(|(p, items)| (p.as_path(), items.as_slice()))) {
            if paths.contains(source) { continue; }
            if results.root.as_ref().is_some_and(|(root, _)| !source.starts_with(root)) { continue; }
            let path: Arc<str> = results.root.as_ref().and_then(|(root, _)| source.strip_prefix(root).ok()).unwrap_or(source).to_string_lossy().into_owned().into();
            for d in items {
                results.hits.push(search::Hit { path: path.clone(), line: d.line, range: 0..0, text: format!("{}: {} [last reported location; open to recheck]", d.severity.label(), d.message).into(), decl: None, origin: None });
            }
        }
        results.hits.sort_by(|a,b| search::tree_order(&a.path, &b.path, folders_first).then(a.line.cmp(&b.line)).then(a.text.cmp(&b.text)));
        results.hits.dedup();
        let mut checked = 0;
        let mut pending = 0;
        let mut unavailable = 0;
        for tab in &self.state.tabs {
            let Some(view) = self.state.view(tab.id) else { continue };
            if self.state.diagnostics(tab.id).is_some() { checked += 1; }
            else if !matches!(view.snapshot().language(), None | Some(Language::Env)) && !view.snapshot().is_large() && (view.snapshot().language() != Some(Language::Sql) || self.state.sql_dialect_for(view.tab()).and_then(|d| d.dialect).is_some()) { pending += 1; }
            else { unavailable += 1; }
        }
        results.provider = Some(format!("{checked} open views checked, {pending} pending, {unavailable} unavailable; known reports may be limited"));
        results.done = true;
        results.rebuild_rows();
        self.replace_results(results);
        Effect { repaint: true, ..Effect::default() }
    }
}

/// Keep diagnostics compact even when a parser includes a whole malformed tag name.
pub(super) fn compact_message(message: &str) -> String {
    let mut chars = message.chars();
    let mut text: String = chars.by_ref().take(512).collect();
    if chars.next().is_some() { text.push('…'); }
    text
}

/// Eligibility shared by current-buffer and explicit project checks; the Bash grammar cannot validate zsh.
pub(super) fn syntax_supported(doc: &Document, shell_setting: Option<&str>) -> bool {
    match doc.language() {
        None | Some(Language::Env | Language::Sql) => false,
        Some(Language::Bash) => crate::semantics::shell::identify(doc.source(), crate::semantics::shell::head(doc.text()), shell_setting)
            .shell.is_ok_and(|shell| shell != crate::semantics::shell::Shell::Zsh),
        Some(_) => true,
    }
}

impl State {
    pub(super) fn syntax_shell_setting(&self) -> Option<String> {
        let provider = crate::semantics::provider::PROVIDERS.iter().find(|provider| provider.id == "bash-language-server")?;
        let view = self.provider().filter(|_| self.current_source().is_some_and(|doc| doc.language() == Some(Language::Bash)));
        let root = view.as_ref().and_then(|view| view.root.as_deref()).or_else(|| self.project.as_ref().map(|p| p.root()));
        self.semantics.settings().initialization(provider, root).get("shell")?.as_str().map(str::to_owned)
    }
}
