//! Diagnostics a language server publishes for a document: each converted once, against the exact text that server
//! was sent, into byte ranges of that version. They describe that version only; a later edit makes them stale until
//! the server publishes again, so nothing is ever placed on text it was not computed for.
use super::{SourceVersion, lsp};
use crate::document::Document;
use serde_json::Value;
use std::ops::Range;

/// How serious a diagnostic is (the protocol's numbering; a missing severity is an error, as editors read it).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    Error,
    Warning,
    Information,
    Hint,
}

impl Severity {
    pub(crate) fn from_lsp(v: &Value) -> Self {
        match v.as_u64() {
            Some(2) => Self::Warning,
            Some(3) => Self::Information,
            Some(4) => Self::Hint,
            _ => Self::Error,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Error => "Error",
            Self::Warning => "Warning",
            Self::Information => "Info",
            Self::Hint => "Hint",
        }
    }
}

/// One diagnostic, its range in bytes of the version it was published for.
#[derive(Clone, Debug, PartialEq)]
pub struct Diagnostic {
    pub range: Range<usize>,
    pub severity: Severity,
    pub message: String,
    /// Who reported it (`rustc`, `clippy`, …) and its code, when the server says.
    pub source: Option<String>,
    pub code: Option<String>,
    /// As the server sent it: what a quick-fix request hands back.
    pub(crate) raw: Value,
}

/// One bounded annotation after a logical source line, for the exact text being shown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InlineProblem {
    pub severity: Severity,
    pub message: String,
    pub other_count: usize,
    /// Diagnostic start, relative to its line and clamped to a UTF-8 caret boundary.
    pub byte: usize,
    pub empty: bool,
}

/// A document's diagnostics, for exactly one of its versions, sorted by position.
#[derive(Debug)]
pub struct FileDiagnostics {
    source: SourceVersion,
    pub items: Vec<Diagnostic>,
}

impl FileDiagnostics {
    /// `items` (byte ranges of `doc`), computed for exactly `doc`: a tool's own findings rather than a server's.
    pub(crate) fn new(doc: &Document, mut items: Vec<Diagnostic>) -> Self {
        items.sort_by_key(|d| (d.range.start, d.range.end, d.severity));
        Self { source: SourceVersion::new(doc), items }
    }
    /// Several servers' problems with one document (a template's views), as one list: those describing the newest
    /// version among them (each server's are of the version it was last sent).
    pub(super) fn merged(parts: &[&std::sync::Arc<FileDiagnostics>]) -> Self {
        let newest = parts.iter().map(|d| d.source.version).max().unwrap_or_default();
        let mut found = parts.iter().filter(|d| d.source.version == newest);
        let source = found.clone().next().map_or_else(|| SourceVersion { version: newest, metadata: std::sync::Weak::new() }, |d| d.source.clone());
        let mut items: Vec<Diagnostic> = found.by_ref().filter(|d| d.source == source).flat_map(|d| d.items.iter().cloned()).collect();
        items.sort_by_key(|d| (d.range.start, d.range.end, d.severity));
        Self { source, items }
    }
    /// Parser recovery errors add coverage without repeating an overlapping server error.
    pub(crate) fn with_syntax(server: &Self, syntax: &Self) -> Self {
        let mut items = server.items.clone();
        items.extend(syntax.items.iter().filter(|syntax| !server.items.iter().any(|server| server.severity == Severity::Error
            && server.range.start <= syntax.range.end && syntax.range.start <= server.range.end)).cloned());
        items.sort_by_key(|d| (d.range.start, d.range.end, d.severity));
        items.dedup_by(|a, b| a.range == b.range && a.severity == b.severity && a.message == b.message);
        Self { source: server.source.clone(), items }
    }
    /// Whether they were computed for `snapshot`'s version of the text.
    pub fn describes(&self, snapshot: &crate::text::TextSnapshot) -> bool {
        self.source.version == snapshot.key().version && self.source.metadata.ptr_eq(&std::sync::Arc::downgrade(snapshot.source_meta()))
    }
    /// Whether they were computed for exactly `doc`.
    pub(super) fn describes_doc(&self, doc: &Document) -> bool { self.source.matches(doc) }
    pub(super) fn newer_than(&self, doc: &Document) -> bool {
        self.source.version > doc.version() && self.source.metadata.ptr_eq(&std::sync::Arc::downgrade(doc.source_meta()))
    }
    /// Only diagnostic starts belonging to this line; multiline findings appear once, on their first line.
    pub(crate) fn starting_in(&self, range: Range<usize>, include_end: bool) -> Option<(&Diagnostic, usize)> {
        let first = self.items.partition_point(|d| d.range.start < range.start);
        let end = self.items.partition_point(|d| d.range.start < range.end || include_end && d.range.start == range.end);
        let items = self.items.get(first..end)?;
        Some((items.iter().min_by_key(|d| d.severity)?, items.len().saturating_sub(1)))
    }
    /// The diagnostics at byte `at` (touching it), most serious first.
    pub fn at(&self, at: usize) -> Vec<&Diagnostic> {
        let mut found: Vec<&Diagnostic> = self.items.iter().filter(|d| d.range.start <= at && at <= d.range.end).collect();
        found.sort_by_key(|d| d.severity);
        found
    }
}

/// `params` of `textDocument/publishDiagnostics` against `doc`, the text the server was sent; `None` when they
/// name another version of it.
pub(super) fn parse(params: &Value, doc: &Document, encoding: lsp::Encoding) -> Option<FileDiagnostics> {
    if params.get("version").filter(|v| !v.is_null()).is_some_and(|v| v.as_u64() != Some(doc.version())) { return None; }
    let mut items: Vec<Diagnostic> = params["diagnostics"].as_array().map(Vec::as_slice).unwrap_or_default().iter().filter_map(|d| {
        // A template's view: only problems describing the template (not its projection's layout), placed on its bytes.
        if let Some(p) = doc.embedded() {
            let at = lsp::offset_at(p.generated(), d["range"]["start"]["line"].as_u64()? as usize, d["range"]["start"]["character"].as_u64()? as usize, encoding);
            if !p.keeps_problem(d["source"].as_str(), d["message"].as_str().unwrap_or_default(), at) { return None; }
        }
        let start = lsp::display_offset(doc, &d["range"]["start"], encoding)?;
        let end = lsp::display_offset(doc, &d["range"]["end"], encoding)?.max(start);
        let code = match &d["code"] { Value::String(s) => Some(s.clone()), Value::Number(n) => Some(n.to_string()), _ => None };
        Some(Diagnostic { range: start..end, severity: Severity::from_lsp(&d["severity"]), message: d["message"].as_str()?.trim().to_owned(),
            source: d["source"].as_str().map(str::to_owned), code, raw: d.clone() })
    }).collect();
    items.sort_by_key(|d| (d.range.start, d.range.end, d.severity));
    Some(FileDiagnostics { source: SourceVersion::new(doc), items })
}

/// Compact server publication for files whose text the editor has not loaded. Coordinates are server positions,
/// never byte underlines on an unknown version.
#[derive(Clone, Debug)]
pub struct PublishedDiagnostic { pub line: usize, pub message: String, pub severity: Severity }

pub(super) fn publication(params: &Value) -> Vec<PublishedDiagnostic> {
    params["diagnostics"].as_array().into_iter().flatten().take(128).filter_map(|d| Some(PublishedDiagnostic {
        line: usize::try_from(d["range"]["start"]["line"].as_u64()?).ok()?,
        message: d["message"].as_str()?.chars().take(512).collect(), severity: Severity::from_lsp(&d["severity"]),
    })).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn java_style_unversioned_diagnostics_use_negotiated_utf16_and_encoded_uri() {
        let doc = Document::decode("/project/App space λ.java".into(), 42, "😀 bad".as_bytes());
        let params = json!({ "uri": lsp::file_uri(doc.source()), "diagnostics": [{ "range": { "start": { "line": 0, "character": 3 }, "end": { "line": 0, "character": 6 } }, "message": "Cannot resolve bad", "severity": 1 }] });
        let found = parse(&params, &doc, lsp::Encoding::Utf16).unwrap();
        assert_eq!(found.items[0].range, 5..8);
        assert!(found.describes_doc(&doc));
        assert_eq!(lsp::uri_path(params["uri"].as_str().unwrap()).as_deref(), Some(doc.source()));
        let stale = json!({ "version": 41, "diagnostics": params["diagnostics"] });
        assert!(parse(&stale, &doc, lsp::Encoding::Utf16).is_none());
        let invalid = json!({ "version": -1, "diagnostics": params["diagnostics"] });
        assert!(parse(&invalid, &doc, lsp::Encoding::Utf16).is_none());
    }

    #[test]
    fn server_errors_suppress_overlapping_parser_recovery_duplicates() {
        let doc = Document::decode("/project/a.java".into(), 1, b"class A {");
        let item = |range, message: &str| Diagnostic { range, message: message.into(), severity: Severity::Error, source: None, code: None, raw: Value::Null };
        let server = FileDiagnostics::new(&doc, vec![item(8..9, "Expected }")]);
        let syntax = FileDiagnostics::new(&doc, vec![item(0..9, "Syntax error"), item(3..4, "Missing token")]);
        let combined = FileDiagnostics::with_syntax(&server, &syntax);
        assert_eq!(combined.items.len(), 2);
        assert!(combined.items.iter().any(|d| d.message == "Missing token"));
        assert!(!combined.items.iter().any(|d| d.message == "Syntax error"));
    }
}
