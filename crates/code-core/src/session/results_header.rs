//! The results panel's heading, said the same way by both adapters.
use super::*;
use crate::search::{Header, ResultsKind};

/// Result lists kept in the panel's history.
const RESULTS_HISTORY: usize = 8;

/// `n` of `noun`, plural with `s` (or `es` after an `h`).
fn count(n: usize, noun: &str) -> String {
    match n {
        1 => format!("1 {noun}"),
        _ if noun.ends_with('h') => format!("{n} {noun}es"),
        _ => format!("{n} {noun}s"),
    }
}

impl Session {
    /// Show `results` in the panel. The list they replace joins the panel's history (Previous Results returns to it)
    /// unless it is the same question asked again, or a live search typed further.
    pub(super) fn replace_results(&mut self, results: crate::search::Results) {
        if let Some(old) = self.state.results.take() {
            let live = |k: ResultsKind| matches!(k, ResultsKind::Declarations | ResultsKind::Symbols | ResultsKind::Outline | ResultsKind::FileSymbols | ResultsKind::Hierarchy);
            let again = (old.kind == results.kind && (old.query == results.query || live(old.kind))) || (live(old.kind) && live(results.kind) && matches!(old.scope, Scope::File(_)));
            if !again && !old.hits.is_empty() {
                self.results_back.push(Self::archived(old));
                if self.results_back.len() > RESULTS_HISTORY { self.results_back.remove(0); }
                self.results_forward.clear();
            }
        }
        self.state.results = Some(results);
    }

    /// A list kept in the history: a search still running counts as stopped, and what is rebuilt on showing it
    /// (its preview and line colours) is let go.
    pub(super) fn archived(mut r: crate::search::Results) -> crate::search::Results {
        if !r.done { r.done = true; r.stopped = true; r.rebuild_rows(); }
        r.preview = None;
        // Only the captured sources' identities stay; their text is not pinned by the history.
        for source in r.captured.values_mut() { source.snapshot = None; }
        r.spans.clear();
        r.spans_asked.clear();
        r.hidden = false;
        r
    }

    /// The buttons at the right of the panel's heading, left to right: Stop while a search runs, Rerun, Collapse or
    /// Expand for a tree, Hide and Close.
    pub fn results_controls(&self) -> Vec<(CommandId, &'static str)> {
        let Some(r) = self.state.results() else { return Vec::new() };
        let mut controls = Vec::new();
        if !self.results_back.is_empty() || !self.results_forward.is_empty() {
            controls.extend([(CommandId::PreviousResults, "‹"), (CommandId::NextResults, "›")]);
        }
        if self.disabled(CommandId::StopSearch).is_none() { controls.push((CommandId::StopSearch, "Stop")); }
        if self.disabled(CommandId::RerunSearch).is_none() { controls.push((CommandId::RerunSearch, "Rerun")); }
        if self.disabled(CommandId::FoldAllResults).is_none() {
            controls.push((CommandId::FoldAllResults, if r.all_folded() { "Expand" } else { "Collapse" }));
        }
        controls.extend([(CommandId::ToggleResults, "Hide"), (CommandId::CloseResults, "×")]);
        controls
    }

    /// The status bar's way back to hidden results ("Results (4)"), while the panel is hidden.
    pub fn hidden_results_label(&self) -> Option<String> {
        let r = self.state.results.as_ref().filter(|r| r.hidden)?;
        Some(format!("Results ({})", r.hits().len()))
    }

    /// What the panel says above its results: what was asked, how much was found and how far it got, and the keys.
    pub fn results_header(&self) -> Option<Header> {
        let r = self.state.results()?;
        if let Some(e) = r.error() {
            return Some(Header { title: format!("Search for “{}” failed: {e}", r.query()), error: true, ..Header::default() });
        }
        let provider = r.provider.as_deref().unwrap_or("language server");
        let file = match r.scope() {
            Scope::File(p) => p.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
            Scope::Project => String::new(),
        };
        let named = |what: &str| if r.query().is_empty() { what.to_owned() } else { format!("{what} of {}", r.query()) };
        let in_files = |noun: &str| {
            let n = count(r.hits().len(), noun);
            if r.grouped() && r.matched_files() > 1 { format!("{n} in {} files", r.matched_files()) } else { n }
        };
        let indexing = if r.provider_indexing() { " · may be incomplete (still indexing)" } else { "" };
        let open = "Enter opens · Esc hides";
        let mut h = Header { hint: open.into(), ..Header::default() };
        match r.results_kind() {
            ResultsKind::Outline => {
                h.title = format!("Outline of {file}");
                h.detail = format!("{} (syntactic)", count(r.hits().len(), "declaration"));
                if !r.done() { h.detail += " · building…"; }
                if r.syntax_errors() { h.detail += " · the file has syntax errors, so declarations may be missing"; }
                if self.disabled(CommandId::FileSymbols).is_none() { h.hint = format!("Tab: language server symbols · {open}"); }
            }
            ResultsKind::FileSymbols => {
                h.title = format!("Symbols in {file}");
                h.detail = format!("{} · {provider}{indexing}", count(r.hits().len(), "symbol"));
                h.hint = format!("Tab: syntactic outline · {open}");
            }
            ResultsKind::Declarations => {
                h.title = format!("Declarations “{}”", r.query());
                h.detail = format!("{} (syntactic)", count(r.hits().len(), "declaration"));
                if r.limited() { h.detail += &format!(" · best {}; refine the name", crate::index::DECL_LIMIT); }
                if let Some(c) = r.coverage() {
                    h.detail += &format!(" · indexed {} of {} files", c.indexed, c.candidates);
                    for (n, what) in [(c.too_large, "too large"), (c.unreadable, "unreadable"), (c.syntax_errors, "with syntax errors")] {
                        if n > 0 { h.detail += &format!(", {n} {what}"); }
                    }
                    if c.pending > 0 { h.detail += ", indexing…"; }
                    if c.limited { h.detail += ", index budget reached"; }
                }
                if r.partial_inventory() { h.detail += " · the folder is still being scanned"; }
                if r.done() && r.hits().is_empty() && !r.query().is_empty() { h.detail += " · no matches"; }
            }
            ResultsKind::Hierarchy => {
                let other = match r.hierarchy_direction().map(Direction::other) {
                    Some(Direction::Callers) => "callers",
                    Some(Direction::Supertypes) => "supertypes",
                    Some(Direction::Subtypes) => "subtypes",
                    Some(Direction::Callees) | None => "callees",
                };
                h.title = r.kind();
                h.detail = indexing.trim_start_matches(" · ").to_owned();
                h.hint = format!("Right unfolds · Left folds · Tab shows {other}");
            }
            kind @ (ResultsKind::Definitions | ResultsKind::Implementations | ResultsKind::TypeDefinitions | ResultsKind::Peek | ResultsKind::Usages | ResultsKind::Symbols) => {
                let (what, noun) = match kind {
                    ResultsKind::Definitions | ResultsKind::Peek => ("Definitions", "definition"),
                    ResultsKind::Implementations => ("Implementations", "implementation"),
                    ResultsKind::TypeDefinitions => ("Type definitions", "type definition"),
                    ResultsKind::Usages => ("Usages", "usage"),
                    _ => ("Symbols", "symbol"),
                };
                h.title = if kind == ResultsKind::Symbols { format!("Symbols matching “{}”", r.query()) } else { named(what) };
                h.detail = format!("{} · {provider}", in_files(noun));
                if r.limited() {
                    let limit = if kind == ResultsKind::Usages || kind == ResultsKind::Symbols { crate::semantics::USAGE_LIMIT } else { crate::semantics::TARGET_LIMIT };
                    h.detail += &format!(" · the first {limit}; refine");
                }
                if !r.done() { h.detail += " · asking…"; }
                h.detail += indexing;
                if r.done() && r.hits().is_empty() { h.detail += " · none found"; }
            }
            kind @ (ResultsKind::Problems | ResultsKind::ProjectSyntax) => {
                h.title = if kind == ResultsKind::Problems { "Problems".into() } else { "Project syntax".into() };
                h.detail = format!("{} · {}", in_files("problem"), r.kind().split_once('(').map_or("", |(_, rest)| rest.trim_end_matches(')')));
                h.hint = format!("{open} · rerun to refresh");
            }
            ResultsKind::Text => {
                let o = r.options;
                let on: Vec<&str> = [(o.case, "case"), (o.word, "word"), (o.regex, "regex")].into_iter().filter(|(on, _)| *on).map(|(_, n)| n).collect();
                h.title = format!("“{}”", r.query());
                if !on.is_empty() { h.title += &format!(" ({})", on.join(", ")); }
                match r.scope() {
                    Scope::File(_) => h.title += &format!(" in {file}"),
                    Scope::Project if !r.pattern().is_empty() => h.title += &format!(" in “{}”", r.pattern()),
                    Scope::Project => {}
                }
                h.detail = in_files("match");
                if r.limited() {
                    h.detail += &format!(" · stopped at {}; refine the query", crate::search::RESULT_LIMIT);
                } else if r.stopped {
                    h.detail += &format!(" · stopped after {} of {} files", r.searched(), r.files());
                } else if !r.done() {
                    h.detail += &format!(" · searching {} of {} files…", r.searched(), r.files());
                    h.hint = "Esc stops · Enter opens".into();
                } else if r.scope() == &Scope::Project {
                    h.detail += &format!(" · {} searched", count(r.files(), "file"));
                }
                if r.failed() > 0 { h.detail += &format!(" · {} could not be read", count(r.failed(), "file")); }
                if r.partial_inventory() { h.detail += " · the folder was still being scanned, so later files are not included"; }
                if r.done() && r.hits().is_empty() { h.detail += " · no matches"; }
            }
        }
        if r.done() && r.hits().is_empty() { h.hint = "Esc hides".into(); }
        Some(h)
    }
}
