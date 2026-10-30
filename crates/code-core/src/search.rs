//! Literal text search: case-insensitive, no regular expressions, no fuzziness. A project search reads the files the
//! inventory knows (so ignore rules apply), optionally only those a file pattern picks, folder by folder, and streams
//! matches back in batches; a file search runs over a document already in memory. Matches are text matches, never
//! claims about what a symbol refers to. A project's matches list as a tree: folder, file, then each match.

use std::{
    ops::Range,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use globset::{GlobBuilder, GlobSetBuilder};
use grep_matcher::Matcher;
use lenz_content::find::Query;
use grep_regex::{RegexMatcher, RegexMatcherBuilder};
use grep_searcher::{BinaryDetection, Searcher, SearcherBuilder, Sink, SinkMatch};

/// Results kept at most; past this the search stops and says so.
pub const RESULT_LIMIT: usize = 10_000;
/// Matches sent per batch.
pub const SEARCH_BATCH: usize = 256;
/// Characters of a matching line kept for display.
pub const SNIPPET_CHARS: usize = 240;

/// Ctrl+Backspace in a one-line field: drops the last word and the spaces after it.
pub fn drop_word(text: &mut String) {
    let kept = text.trim_end();
    let start = kept.char_indices().rev().find(|(_, c)| c.is_whitespace()).map_or(0, |(i, c)| i + c.len_utf8());
    text.truncate(start);
}

/// One match: where it is and the line it is on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    /// The file, relative to the root (`/`-separated); for a file search, the document's path.
    pub path: Arc<str>,
    /// 0-based line.
    pub line: usize,
    /// The match's byte range within the line's text (line ending excluded), on character boundaries.
    pub range: Range<usize>,
    /// The line's text (its first [`SNIPPET_CHARS`] characters), for display and validation; for a declaration, its
    /// name.
    pub text: Box<str>,
    /// For a declaration (outline or declaration search): its syntactic kind and nesting depth.
    pub decl: Option<(lenz_content::highlight::DeclKind, u16)>,
    /// For a semantic result merged from several servers: which provider and project gave it.
    pub origin: Option<Arc<str>>,
}

impl Hit {
    /// The bytes of indentation a result row leaves out of [`Hit::text`].
    pub fn indent(&self) -> usize {
        let indent = self.text.len() - self.text.trim_start().len();
        // A match inside the indentation keeps it.
        if self.decl.is_none() && self.range.start < indent { 0 } else { indent }
    }
    /// A declaration at `line` (whose text starts at byte `line_start`): its name's first line, which the source
    /// carries exactly there (a name may span lines, or be cut to its first characters).
    pub fn declaration(path: Arc<str>, line: usize, line_start: usize, decl: &lenz_content::highlight::Declaration) -> Self {
        let shown = decl.name.split('\n').next().unwrap_or_default().trim_end_matches('\r');
        let start = decl.name_range.start as usize - line_start;
        Self { path, line, range: start..start + shown.len(), text: shown.into(), decl: Some((decl.kind, decl.depth)), origin: None }
    }
}

/// A query keeps exact hit sources; processed no-hit sources retain only their compact identity.
pub(crate) struct CapturedSource {
    pub identity: crate::session::SemanticSource,
    pub snapshot: Option<Arc<crate::text::TextSnapshot>>,
}
impl CapturedSource {
    pub fn new(snapshot: Arc<crate::text::TextSnapshot>) -> Self {
        Self { identity: crate::session::SemanticSource::new(&snapshot), snapshot: Some(snapshot) }
    }
}

/// What a result list holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResultsKind {
    Problems,
    ProjectSyntax,
    Text,
    Outline,
    Declarations,
    /// A semantic provider's definition candidates.
    Definitions,
    /// A semantic provider's implementations of a trait, interface or abstract member.
    Implementations,
    /// A semantic provider's definitions of a symbol's type.
    TypeDefinitions,
    /// A definition shown in the panel without moving.
    Peek,
    /// A semantic provider's references to a symbol.
    Usages,
    /// A semantic provider's workspace symbols.
    Symbols,
    /// A semantic provider's symbols of one file (its outline).
    FileSymbols,
    /// A call hierarchy: callers (or callees) of a function, level by level.
    Hierarchy,
}

/// How a call-hierarchy row folds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fold {
    /// Known to have no level.
    Leaf,
    Closed,
    Open,
    Loading,
    /// It is one of its own ancestors: not followed.
    Recursive,
    Failed(String),
}

/// A call-hierarchy row: its function, depth, how it folds and how many calls its site stands for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallRow {
    pub name: String,
    /// The language server's symbol kind.
    pub kind: u64,
    /// Its kind, for the row.
    pub label: &'static str,
    pub depth: u16,
    pub fold: Fold,
    pub calls: usize,
    /// Its level had more than the limit: this many are not shown.
    pub more: usize,
}

/// The decoded source that supplied result coordinates; separate from the index cache's raw-byte key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DiskSource { pub stamp: crate::cache::Stamp, pub digest: [u8; 32] }
impl DiskSource {
    pub(crate) fn new(stamp: crate::cache::Stamp, text: &[u8]) -> Self {
        Self { stamp, digest: decoded_digest(text) }
    }
}

pub(crate) fn decoded_digest(text: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    Sha256::digest(text).into()
}

/// A batch of results for search `id`.
#[derive(Debug, Default)]
pub struct Found {
    pub(crate) sources: std::collections::HashMap<Arc<str>, DiskSource>,
    pub hits: Vec<Hit>,
    /// Files fully searched so far (for a file search, 1 when done).
    pub searched: usize,
    pub done: bool,
    /// The search stopped at [`RESULT_LIMIT`].
    pub limited: bool,
    /// Files that could not be read (so their matches are missing).
    pub failed: usize,
    /// The search could not run at all.
    pub error: Option<String>,
    /// An outline's source had syntax errors: it may be incomplete.
    pub syntax_errors: bool,
    /// How many files the search reads, once a file pattern has picked them.
    pub files: Option<usize>,
}

/// The files a project search reads: those `pattern` picks, each folder's files together (folders, then names, in
/// byte order) so its matches arrive grouped as the results' tree shows them.
///
/// The pattern is comma-separated globs (`*`, `?`, `[…]`, `**`), any case; `!` before one excludes what it matches;
/// with no inclusions everything else is included. A glob without `/` matches a file or folder name at any depth
/// (`*.rs`, `tests`); one with `/` matches from the root (`src/ui`, `crates/*/src`). A folder includes everything in
/// it. An empty pattern picks every file.
pub fn select_files(mut paths: Vec<Arc<str>>, pattern: &str, folders_first: bool) -> Result<Vec<Arc<str>>, String> {
    let (mut include, mut exclude, mut includes) = (GlobSetBuilder::new(), GlobSetBuilder::new(), false);
    for part in pattern.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let (excluded, glob) = match part.strip_prefix('!') {
            Some(g) => (true, g.trim()),
            None => (false, part),
        };
        let glob = glob.trim_end_matches('/');
        if glob.is_empty() {
            continue;
        }
        let base = if glob.contains('/') { glob.trim_start_matches('/').to_string() } else { format!("**/{glob}") };
        for g in [base.clone(), format!("{base}/**")] {
            let g = GlobBuilder::new(&g).literal_separator(true).case_insensitive(true).build().map_err(|e| format!("bad file pattern “{part}”: {}", e.kind()))?;
            if excluded {
                exclude.add(g);
            } else {
                include.add(g);
                includes = true;
            }
        }
    }
    let built = |b: GlobSetBuilder| b.build().map_err(|e| format!("bad file pattern: {e}"));
    let (include, exclude) = (built(include)?, built(exclude)?);
    paths.retain(|p| (!includes || include.is_match(&**p)) && !exclude.is_match(&**p));
    paths.sort_unstable_by(|a, b| tree_order(a, b, folders_first));
    Ok(paths)
}

/// Paths in the explorer's order, compared component by component so every folder's contents are one run: within a
/// folder its subfolders come before its files when `folders_first` is set, else both by name.
pub fn tree_order(a: &str, b: &str, folders_first: bool) -> std::cmp::Ordering {
    let (mut x, mut y) = (a.split('/').peekable(), b.split('/').peekable());
    loop {
        match (x.next(), y.next()) {
            (Some(p), Some(q)) => {
                let (folder_p, folder_q) = (x.peek().is_some(), y.peek().is_some());
                if p == q && folder_p && folder_q { continue; }
                let kind = if folders_first { folder_q.cmp(&folder_p) } else { std::cmp::Ordering::Equal };
                return kind.then(p.cmp(q)).then(folder_p.cmp(&folder_q));
            }
            (p, q) => return p.is_some().cmp(&q.is_some()),
        }
    }
}

/// Where a result row starts showing `text` (from `from`, past its indentation) in `columns` cells so the match at
/// `range` stays in view: `from` when the line fits up to the match's end, else a character boundary leaving about a
/// third of the room before the match for context (the row then shows `…` first).
pub fn clip_start(text: &str, from: usize, range: Range<usize>, columns: usize) -> usize {
    let width = lenz_content::wrap::width;
    let Some(upto) = text.get(from..range.end).filter(|_| range.start >= from) else { return from };
    if width(upto) <= columns { return from; }
    let before = (columns / 3).min(columns.saturating_sub(width(text.get(range.clone()).unwrap_or_default()) + 1));
    let (mut start, mut used) = (range.start, 0);
    for (i, c) in text.get(from..range.start).unwrap_or_default().char_indices().rev() {
        used += width(c.encode_utf8(&mut [0; 4]));
        if used > before { break; }
        start = from + i;
    }
    start
}

/// A match line's syntax colours, from its file's language: the line parsed on its own, so a construct spanning lines
/// (a block comment, a multi-line string) may read differently than in the file.
pub fn snippet_spans(hit: &Hit) -> LineSpans {
    match lenz_content::highlight::detect(Path::new(&*hit.path), &hit.text) {
        Some(lang) => lenz_content::highlight::parse_while(lang, hit.text.as_ref().into(), || true).and_then(|p| p.spans(0..1).into_iter().next()).unwrap_or_default(),
        None => Vec::new(),
    }
}

/// The matcher for `query`, with its options (as the find box matches them). `Ok(None)` for an empty query; a
/// query the find box refuses is refused here with the same reason.
pub fn matcher(query: &Query) -> Result<Option<RegexMatcher>, String> {
    if query.text.is_empty() {
        return Ok(None);
    }
    lenz_content::find::Matcher::new(query).map_err(|e| e.to_string())?;
    let g = query.grep_spec();
    RegexMatcherBuilder::new().case_insensitive(g.case_insensitive).word(g.word).fixed_strings(g.fixed_strings).build(&g.pattern).map(Some).map_err(|e| e.to_string())
}

fn searcher() -> Searcher {
    // Binary files are skipped; a BOM selects the encoding (UTF-16 is transcoded), as the reader decodes them.
    SearcherBuilder::new().binary_detection(BinaryDetection::quit(0)).line_number(true).build()
}

/// Every match in `lines` (one file's text); `push` gets each and returns `false` to stop.
fn collect(matcher: &RegexMatcher, path: &Arc<str>, line: usize, bytes: &[u8], push: &mut dyn FnMut(Hit) -> bool) -> bool {
    // The searcher hands over whole lines of UTF-8 (the caller decodes other encodings first); strip the line
    // ending and find each match in the line.
    let text = String::from_utf8_lossy(bytes);
    let text = text.strip_suffix('\n').unwrap_or(&text);
    let text = text.strip_suffix('\r').unwrap_or(text);
    let snippet: Box<str> = text.chars().take(SNIPPET_CHARS).collect::<String>().into();
    line_matches(matcher, text, &mut |range| push(Hit { path: path.clone(), line, range, text: snippet.clone(), decl: None, origin: None }))
}

/// Each non-empty match in one line (on character boundaries), as the find box counts them; `each` returns `false`
/// to stop, and so does this.
fn line_matches(matcher: &RegexMatcher, text: &str, each: &mut dyn FnMut(std::ops::Range<usize>) -> bool) -> bool {
    let mut keep = true;
    let _ = matcher.find_iter(text.as_bytes(), |m| {
        let range = m.start()..m.end();
        if !range.is_empty() && text.is_char_boundary(range.start) && text.is_char_boundary(range.end) {
            keep = each(range);
        }
        keep
    });
    keep
}

struct Lines<'a> {
    matcher: &'a RegexMatcher,
    path: Arc<str>,
    push: &'a mut dyn FnMut(Hit) -> bool,
    cancelled: &'a dyn Fn() -> bool,
}

impl Sink for Lines<'_> {
    type Error = std::io::Error;
    fn matched(&mut self, _: &Searcher, m: &SinkMatch<'_>) -> Result<bool, Self::Error> {
        if (self.cancelled)() {
            return Ok(false);
        }
        let line = m.line_number().map_or(0, |n| n as usize - 1);
        // A match can span several lines only for multi-line patterns, which a literal search is not.
        Ok(collect(self.matcher, &self.path, line, m.bytes(), self.push))
    }
}

/// Search `text` (a document in memory) for `query`.
pub fn search_text(query: &Query, path: &Arc<str>, text: &str) -> Found {
    let matcher = match matcher(query) {
        Ok(Some(m)) => m,
        Ok(None) => return Found { done: true, searched: 1, ..Found::default() },
        Err(e) => return Found { done: true, error: Some(e), ..Found::default() },
    };
    let mut hits = Vec::new();
    let mut limited = false;
    let mut push = |h: Hit| {
        if hits.len() == RESULT_LIMIT {
            limited = true;
            return false;
        }
        hits.push(h);
        true
    };
    let mut sink = Lines { matcher: &matcher, path: path.clone(), push: &mut push, cancelled: &|| false };
    let _ = searcher().search_slice(&matcher, text.as_bytes(), &mut sink);
    Found { hits, searched: 1, done: true, limited, ..Found::default() }
}

/// Search `files` (relative to `root`) for `query`, sending batches through `send` (which returns `false` once
/// nobody listens). Stops when `latest` no longer says `id`, or at [`RESULT_LIMIT`].
pub fn search_files(root: &Path, files: &[Arc<str>], query: &Query, id: u64, latest: &AtomicU64, send: &mut dyn FnMut(Found) -> bool) {
    let matcher = match matcher(query) {
        Ok(Some(m)) => m,
        Ok(None) => {
            send(Found { done: true, ..Found::default() });
            return;
        }
        Err(e) => {
            send(Found { done: true, error: Some(e), ..Found::default() });
            return;
        }
    };
    let cancelled = || latest.load(Ordering::Relaxed) != id;
    let mut searcher = searcher();
    let mut sources = std::collections::HashMap::new();
    let (mut batch, mut total, mut searched, mut failed) = (Vec::new(), 0usize, 0usize, 0usize);
    let mut limited = false;
    for rel in files {
        if cancelled() {
            return;
        }
        // Search the text the reader shows: UTF-8 as it is, anything else decoded the way documents are, so
        // ranges and matches agree with the open document (a BOM or a legacy encoding included).
        let path = root.join(&**rel);
        let stamp = crate::cache::Stamp::of(&path);
        let Ok(bytes) = std::fs::read(&path) else {
            failed += 1;
            searched += 1;
            continue;
        };
        if stamp.is_none() || crate::cache::Stamp::of(&path) != stamp { failed += 1; searched += 1; continue; }
        let before = total;
        let decoded;
        let bom = bytes.starts_with(b"\xEF\xBB\xBF") || bytes.starts_with(b"\xFF\xFE") || bytes.starts_with(b"\xFE\xFF");
        let text: &[u8] = match std::str::from_utf8(&bytes) {
            Ok(_) if !bom => &bytes,
            // Binary (a NUL early on, with no BOM to say it is UTF-16): skipped without decoding.
            _ if !bom && bytes[..bytes.len().min(8192)].contains(&0) => {
                searched += 1;
                continue;
            }
            _ => {
                decoded = lenz_content::encoding::decode(&bytes, None).text;
                decoded.as_bytes()
            }
        };
        {
            let mut push = |h: Hit| {
                if total == RESULT_LIMIT {
                    limited = true;
                    return false;
                }
                total += 1;
                batch.push(h);
                true
            };
            let mut sink = Lines { matcher: &matcher, path: rel.clone(), push: &mut push, cancelled: &cancelled };
            let _ = searcher.search_slice(&matcher, text, &mut sink);
        }
        if total > before && let Some(stamp) = stamp { sources.insert(rel.clone(), DiskSource::new(stamp, text)); }
        searched += 1;
        if limited {
            break;
        }
        if batch.len() >= SEARCH_BATCH && !send(Found { sources: std::mem::take(&mut sources), hits: std::mem::take(&mut batch), searched, failed, ..Found::default() }) {
            return;
        }
    }
    if cancelled() {
        return;
    }
    send(Found { sources, hits: batch, searched, done: true, limited, failed, ..Found::default() });
}

/// Whether `hit` still matches `query` in `line` (the current text of its line): the target is valid to open.
pub fn still_matches(hit: &Hit, kind: ResultsKind, query: &Query, line: &str) -> bool {
    // A declaration must still carry its name there exactly; a definition its line as located; a text match its
    // query, with the options it ran with, at the same place.
    match kind {
        ResultsKind::Problems | ResultsKind::ProjectSyntax => true,
        _ if hit.decl.is_some() => line.get(hit.range.clone()) == Some(&*hit.text),
        ResultsKind::Definitions | ResultsKind::Implementations | ResultsKind::TypeDefinitions | ResultsKind::Peek | ResultsKind::Usages | ResultsKind::Symbols | ResultsKind::FileSymbols | ResultsKind::Hierarchy => line.chars().take(SNIPPET_CHARS).eq(hit.text.chars()),
        _ => matcher(query).ok().flatten().is_some_and(|m| {
            let mut found = false;
            line_matches(&m, line, &mut |r| {
                found = r == hit.range;
                !found
            });
            found
        }),
    }
}

/// What a result list holds and how it reads: the presentation outlines and semantic results reuse.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Scope {
    Project,
    /// The document at this path (absolute).
    File(std::path::PathBuf),
}

/// A match line's syntax colours: byte ranges of [`Hit::text`].
pub type LineSpans = Vec<(Range<usize>, lenz_content::highlight::Capture)>;

/// How often a search still streaming rebuilds its rows; its last batch always does.
pub const ROWS_INTERVAL: std::time::Duration = std::time::Duration::from_millis(50);

/// Match lines' colours kept for one result list at most; past this they are made again as needed.
pub const SPANS_KEPT: usize = 4096;

/// The folder of a relative `/`-separated path (empty for the root).
pub fn folder_of(path: &str) -> &str {
    &path[..path.rfind('/').unwrap_or(0)]
}

/// A path's folder and name.
fn split_folder(path: &str) -> (&str, &str) {
    let dir = folder_of(path);
    (dir, path[dir.len()..].trim_start_matches('/'))
}

/// A folder or file of a project's results, which can be folded.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Group {
    /// A folder as shown (relative to the project when inside it), `/`-separated.
    Dir(Arc<str>),
    File(Arc<str>),
}

/// One row of a result list: for results across a project a tree of folders, their files and each file's lines
/// (results on one line share its row); for any other list just its results. [`Results::row_depth`] says how deep.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Row {
    /// A folder: `label` is its path below the row above it, several folders when only one leads on (`src/ui`).
    Dir { dir: Arc<str>, label: Arc<str>, count: usize, open: bool },
    File { path: Arc<str>, count: usize, open: bool },
    /// The result at this index of [`Results::hits`], with the rest of its line's results.
    Hit(usize),
}

/// The panel's heading: what was asked, how much was found and how far it got, and what the keys do.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Header {
    pub title: String,
    pub detail: String,
    pub hint: String,
    /// The search could not run: the title says why.
    pub error: bool,
}

/// A result list in the bottom panel: its query, results as they stream in, completion and limits, which
/// result is selected for preview, and which results' files changed since.
pub struct Results {
    pub(crate) id: u64,
    pub(crate) kind: ResultsKind,
    /// The semantic provider that answered (definitions).
    pub(crate) provider: Option<String>,
    /// Definitions and usages: the file and offset asked about, to ask the same question again.
    pub(crate) asked: Option<(std::path::PathBuf, usize)>,
    /// The provider was still indexing when it answered: results may be incomplete.
    pub(crate) provider_indexing: bool,
    /// Declaration search: files indexed so far of those with a supported language, and why some are not.
    pub(crate) coverage: Option<crate::index::Coverage>,
    pub(crate) query: String,
    /// The options a text search ran with (said in its title).
    pub(crate) options: lenz_content::find::FindOptions,
    /// A project search's file pattern (see [`select_files`]); empty for all files.
    pub(crate) pattern: String,
    pub(crate) scope: Scope,
    /// The root the paths are relative to (project scope), and its generation when the search ran.
    pub(crate) root: Option<(std::path::PathBuf, u64)>,
    /// The project folder, `/`-terminated: absolute result paths inside it are shown relative to it.
    pub(crate) shown_root: Option<String>,
    pub(crate) captured: std::collections::HashMap<std::path::PathBuf, CapturedSource>,
    pub(crate) query_paths: std::collections::HashSet<Arc<str>>,
    pub(crate) disk_sources: std::collections::HashMap<Arc<str>, DiskSource>,
    pub(crate) hits: Vec<Hit>,
    pub(crate) searched: usize,
    pub(crate) failed: usize,
    pub(crate) error: Option<String>,
    pub(crate) syntax_errors: bool,
    /// Files the search covers; the inventory was still being enumerated when it started.
    pub(crate) files: usize,
    pub(crate) partial_inventory: bool,
    pub(crate) done: bool,
    pub(crate) limited: bool,
    pub(crate) selected: Option<usize>,
    pub(crate) preview: Option<Preview>,
    pub(crate) stale: std::collections::HashSet<Arc<str>>,
    pub(crate) collapsed: std::collections::HashSet<Group>,
    /// Match lines' syntax colours made so far (off the UI thread, for lines shown), and the lines asked for.
    pub(crate) spans: std::collections::HashMap<usize, LineSpans>,
    pub(crate) spans_asked: std::collections::HashSet<usize>,
    /// Files with matches (for a project's tree).
    pub(crate) matched_files: usize,
    /// Rebuilt whenever the results or what is folded change.
    pub(crate) rows: Vec<Row>,
    /// Each row's depth in the tree.
    pub(crate) depths: Vec<u16>,
    /// The panel is hidden; the results stay (and keep arriving) until shown again or replaced.
    pub(crate) hidden: bool,
    /// When the rows were last rebuilt: a streaming search rebuilds them at most every [`ROWS_INTERVAL`].
    pub(crate) rows_built: Option<std::time::Instant>,
    /// The search was stopped before it finished: what it found so far stays.
    pub(crate) stopped: bool,
    /// The folders the tree shows as rows (a folder that only leads on shares its child's).
    pub(crate) row_dirs: std::collections::HashSet<Arc<str>>,
    /// A call hierarchy's tree, and the node each result shows.
    pub(crate) hierarchy: Option<crate::semantics::hierarchy::Hierarchy>,
    pub(crate) call_nodes: Vec<usize>,
}

/// The selected result's document, shown in the panel without touching the main reading area. It shares the
/// snapshot a tab or the inactive cache already holds when there is one.
pub struct Preview {
    pub index: usize,
    pub doc: Arc<crate::document::Document>,
    /// Its syntax colours: a tab's when it shows the same snapshot, else parsed off the UI thread.
    pub(crate) highlights: Option<Arc<lenz_content::highlight::Parsed>>,
}

impl Preview {
    pub fn highlights(&self) -> Option<&lenz_content::highlight::Parsed> {
        self.highlights.as_deref()
    }
}

impl Results {
    /// What kind of results these are, for the panel: text matches, syntactic candidates, or a named semantic
    /// provider's answer.
    pub fn kind(&self) -> String {
        let provider = self.provider.as_deref().unwrap_or("language server");
        match self.kind {
            ResultsKind::ProjectSyntax => format!("Project syntax: {}", self.provider.as_deref().unwrap_or("waiting")),
            ResultsKind::Problems => format!("Problems ({}; unopened files are not scanned)", self.provider.as_deref().unwrap_or("known reports")),
            ResultsKind::Text => {
                let o = self.options;
                let on: Vec<&str> = [(o.case, "case"), (o.word, "word"), (o.regex, "regex")].into_iter().filter(|(on, _)| *on).map(|(_, n)| n).collect();
                if on.is_empty() { "Text matches".into() } else { format!("Text matches ({})", on.join(", ")) }
            }
            ResultsKind::Outline => "Outline (syntactic candidates)".into(),
            ResultsKind::Declarations => "Declarations (syntactic candidates)".into(),
            ResultsKind::Definitions => format!("Definitions ({provider})"),
            ResultsKind::Implementations => format!("Implementations ({provider})"),
            ResultsKind::TypeDefinitions => format!("Type definitions ({provider})"),
            ResultsKind::Peek => format!("Peek definition ({provider})"),
            ResultsKind::Usages => format!("Usages ({provider})"),
            ResultsKind::Symbols => format!("Symbols ({provider})"),
            ResultsKind::FileSymbols => format!("File symbols ({provider})"),
            ResultsKind::Hierarchy => match &self.hierarchy {
                Some(h) => {
                    use crate::semantics::hierarchy::Direction;
                    let what = match h.direction {
                        Direction::Callers => "Callers",
                        Direction::Callees => "Callees",
                        Direction::Supertypes => "Supertypes",
                        Direction::Subtypes => "Subtypes",
                    };
                    format!("{what} of {} ({provider})", h.nodes[0].call.name)
                }
                None => format!("Call hierarchy ({provider})"),
            },
        }
    }
    /// The language server was still indexing when it answered, so these may be incomplete.
    pub fn provider_indexing(&self) -> bool {
        self.provider_indexing
    }
    /// Which result list this is: a new id means new results.
    pub fn id(&self) -> u64 {
        self.id
    }
    pub fn results_kind(&self) -> ResultsKind {
        self.kind
    }
    /// For declaration search: how much of the project its index covers.
    pub fn coverage(&self) -> Option<&crate::index::Coverage> {
        self.coverage.as_ref()
    }
    pub fn query(&self) -> &str {
        &self.query
    }
    /// The file pattern a project search was limited to (empty: every file).
    pub fn pattern(&self) -> &str {
        &self.pattern
    }
    /// The rows shown, folded groups hidden.
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }
    /// How deep row `row` is in the tree (0 for a list that is not one).
    pub fn row_depth(&self, row: usize) -> usize {
        self.depths.get(row).map_or(0, |d| *d as usize)
    }
    /// The row showing result `i` (its line's row), unless its file is folded.
    pub fn row_of_hit(&self, i: usize) -> Option<usize> {
        let first = self.line_hits(i).start;
        self.rows.iter().position(|r| *r == Row::Hit(first))
    }
    /// The results on result `i`'s line (one row where rows show source lines by file), in line order.
    pub fn line_hits(&self, i: usize) -> Range<usize> {
        if !self.merges_lines() || i >= self.hits.len() {
            return i..i + 1;
        }
        let same = |j: usize| self.hits[j].path == self.hits[i].path && self.hits[j].line == self.hits[i].line;
        let start = (0..i).rev().take_while(|&j| same(j)).last().unwrap_or(i);
        let end = (i + 1..self.hits.len()).find(|&j| !same(j)).unwrap_or(self.hits.len());
        start..end
    }
    /// How a result path reads: relative to the project when it is inside it.
    pub fn shown_path<'a>(&self, path: &'a str) -> &'a str {
        self.shown_root.as_deref().and_then(|root| path.strip_prefix(root)).unwrap_or(path)
    }
    /// A file's name and the folder it is in, as the panel shows them.
    pub fn file_label<'a>(&self, path: &'a str) -> (&'a str, &'a str) {
        let (dir, name) = split_folder(self.shown_path(path));
        (name, dir)
    }
    /// The folder rows and file holding result `i`, outermost first.
    pub fn groups_of(&self, i: usize) -> Vec<Group> {
        let Some(h) = self.hits.get(i) else { return Vec::new() };
        let mut dir = folder_of(self.shown_path(&h.path));
        let mut groups = vec![Group::File(h.path.clone())];
        while !dir.is_empty() {
            if let Some(key) = self.row_dirs.get(dir) {
                groups.push(Group::Dir(key.clone()));
            }
            dir = folder_of(dir);
        }
        groups.reverse();
        groups
    }
    /// Where result `i` is: its shown path and 1-based line.
    pub fn location(&self, i: usize) -> Option<String> {
        let h = self.hits.get(i)?;
        Some(format!("{}:{}", self.shown_path(&h.path), h.line + 1))
    }
    /// Result `i`'s line colours, once made ([`crate::session::Command::ColourResults`]).
    pub fn line_spans(&self, i: usize) -> Option<&[(Range<usize>, lenz_content::highlight::Capture)]> {
        self.spans.get(&i).map(Vec::as_slice)
    }
    /// How many files have matches.
    pub fn matched_files(&self) -> usize {
        self.matched_files
    }
    /// The name of the folder a project's paths are relative to.
    pub fn root_name(&self) -> Option<String> {
        Some(self.root.as_ref()?.0.file_name()?.to_string_lossy().into_owned())
    }
    /// Rows show source lines by file, so results on one line share its row (problems keep a row each).
    fn merges_lines(&self) -> bool {
        self.grouped() && !matches!(self.kind, ResultsKind::Problems | ResultsKind::ProjectSyntax)
    }
    /// Whether a folder or file of the list is folded.
    /// Every folder and file of the tree is folded (Collapse All has nothing left to fold).
    pub(crate) fn all_folded(&self) -> bool {
        !self.collapsed.is_empty() && self.collapsed.len() >= self.row_dirs.len() + self.matched_files
    }
    pub fn is_folded(&self, group: &Group) -> bool {
        self.collapsed.contains(group)
    }
    /// Results across a project, in file order: listed by file. Ranked lists (symbols, declarations) and trees keep
    /// their own order.
    pub fn grouped(&self) -> bool {
        matches!(self.kind, ResultsKind::Text | ResultsKind::Problems | ResultsKind::ProjectSyntax | ResultsKind::Usages
            | ResultsKind::Definitions | ResultsKind::Implementations | ResultsKind::TypeDefinitions | ResultsKind::Peek) && self.scope == Scope::Project
    }

    /// The call-hierarchy row that result `hit` shows.
    pub fn call(&self, hit: usize) -> Option<CallRow> {
        let n = &self.hierarchy.as_ref()?.nodes[*self.call_nodes.get(hit)?];
        use crate::semantics::hierarchy::Children;
        let fold = match &n.state {
            _ if n.recursive => Fold::Recursive,
            Children::Loading(_) => Fold::Loading,
            Children::Failed(why) => Fold::Failed(why.clone()),
            Children::Loaded { .. } if n.children.is_empty() => Fold::Leaf,
            Children::Loaded { .. } if n.open => Fold::Open,
            Children::Loaded { .. } | Children::NotAsked => Fold::Closed,
        };
        let more = match n.state {
            Children::Loaded { more } if n.open => more,
            _ => 0,
        };
        Some(CallRow { name: n.call.name.clone(), kind: n.call.kind, label: n.call.label(&n.site.path), depth: n.depth, fold, calls: n.call.calls, more })
    }

    /// The direction of the call hierarchy shown, if one is.
    pub fn hierarchy_direction(&self) -> Option<crate::semantics::hierarchy::Direction> {
        self.hierarchy.as_ref().map(|h| h.direction)
    }

    /// The results from the call hierarchy's unfolded nodes, keeping the selected node selected.
    pub(crate) fn rebuild_hierarchy(&mut self) {
        let Some(h) = &self.hierarchy else { return };
        let selected = self.selected.and_then(|i| self.call_nodes.get(i).copied());
        self.call_nodes = h.visible();
        self.hits = self.call_nodes.iter().map(|&i| h.nodes[i].site.clone()).collect();
        self.selected = selected.and_then(|n| self.call_nodes.iter().position(|&i| i == n)).or(self.selected.map(|i| i.min(self.hits.len().saturating_sub(1))));
        self.rebuild_rows();
    }

    pub(crate) fn toggle(&mut self, group: Group) {
        if !self.collapsed.remove(&group) {
            self.collapsed.insert(group);
        }
        self.rebuild_rows();
    }

    /// The rows from the results: a project's matches arrive in tree order ([`select_files`]) and semantic results
    /// are sorted that way, so each folder and file is one run of results and each line one run within a file.
    pub(crate) fn rebuild_rows(&mut self) {
        self.rows_built = Some(std::time::Instant::now());
        self.rows.clear();
        self.depths.clear();
        self.row_dirs.clear();
        self.matched_files = self.hits.iter().zip(self.hits.iter().skip(1)).filter(|(a, b)| a.path != b.path).count() + usize::from(!self.hits.is_empty());
        if !self.grouped() {
            self.rows.extend((0..self.hits.len()).map(Row::Hit));
            self.depths.resize(self.rows.len(), 0);
            return;
        }
        let merge = self.merges_lines();
        let hits = &self.hits;
        // Each file's run of results, and per folder: results below it, files directly in it, its subfolders.
        let mut runs = Vec::new();
        let mut i = 0;
        while i < hits.len() {
            let end = (i + 1..hits.len()).find(|&j| hits[j].path != hits[i].path).unwrap_or(hits.len());
            runs.push((folder_of(self.shown_path(&hits[i].path)), i, end));
            i = end;
        }
        #[derive(Default)]
        struct Folder<'a> { count: usize, files: usize, children: usize, last_child: &'a str }
        let mut folders: std::collections::HashMap<&str, Folder> = Default::default();
        for &(dir, start, end) in &runs {
            folders.entry(dir).or_default().files += 1;
            let mut at = dir;
            while !at.is_empty() {
                folders.entry(at).or_default().count += end - start;
                let parent = folders.entry(folder_of(at)).or_default();
                // In tree order a folder's subfolders arrive one after another: a new one is a different name.
                if parent.last_child != at {
                    parent.children += 1;
                    parent.last_child = at;
                }
                at = folder_of(at);
            }
        }
        // A folder with no files of its own and one subfolder shares its child's row.
        let leads_on = |dir: &str| folders.get(dir).is_some_and(|f| f.files == 0 && f.children == 1);
        let below = |dir: &str, inside: &str| dir == inside || dir.strip_prefix(inside).is_some_and(|rest| rest.starts_with('/'));
        let (rows, depths, row_dirs, collapsed) = (&mut self.rows, &mut self.depths, &mut self.row_dirs, &self.collapsed);
        // The folder rows the current file is under, outermost first, and whether that row is folded.
        let mut stack: Vec<(&str, bool)> = Vec::new();
        for &(dir, start, end) in &runs {
            while stack.last().is_some_and(|(open, _)| !below(dir, open)) {
                stack.pop();
            }
            // The folders from the innermost row shown down to this file's, outermost first.
            let above = stack.last().map_or("", |(d, _)| *d);
            let mut chain = Vec::new();
            let mut at = dir;
            while at != above && !at.is_empty() {
                chain.push(at);
                at = folder_of(at);
            }
            chain.reverse();
            let mut from = above;
            for (k, &folder) in chain.iter().enumerate() {
                if k + 1 < chain.len() && leads_on(folder) {
                    continue;
                }
                let key: Arc<str> = folder.into();
                let open = !collapsed.contains(&Group::Dir(key.clone()));
                if stack.iter().all(|(_, open)| *open) {
                    let label: Arc<str> = folder.strip_prefix(from).unwrap_or(folder).trim_start_matches('/').into();
                    let count = folders.get(folder).map_or(0, |f| f.count);
                    depths.push(stack.len() as u16);
                    rows.push(Row::Dir { dir: key.clone(), label, count, open });
                }
                row_dirs.insert(key);
                stack.push((folder, open));
                from = folder;
            }
            if !stack.iter().all(|(_, open)| *open) {
                continue;
            }
            let path = hits[start].path.clone();
            let open = !collapsed.contains(&Group::File(path.clone()));
            depths.push(stack.len() as u16);
            rows.push(Row::File { path, count: end - start, open });
            if open {
                for j in (start..end).filter(|&j| !merge || j == start || hits[j].line != hits[j - 1].line) {
                    depths.push(stack.len() as u16 + 1);
                    rows.push(Row::Hit(j));
                }
            }
        }
    }

    pub fn scope(&self) -> &Scope {
        &self.scope
    }
    pub fn hits(&self) -> &[Hit] {
        &self.hits
    }
    pub fn done(&self) -> bool {
        self.done
    }
    pub fn limited(&self) -> bool {
        self.limited
    }
    pub fn searched(&self) -> usize {
        self.searched
    }
    /// Files that could not be read: their matches are missing.
    pub fn failed(&self) -> usize {
        self.failed
    }
    /// Why the search could not run.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
    /// The outlined source had syntax errors, so the outline may miss declarations.
    pub fn syntax_errors(&self) -> bool {
        self.syntax_errors
    }
    pub fn files(&self) -> usize {
        self.files
    }
    /// The project was still being enumerated when the search started: files found later are not searched.
    pub fn partial_inventory(&self) -> bool {
        self.partial_inventory
    }
    pub fn selected(&self) -> Option<usize> {
        self.selected
    }
    pub fn preview(&self) -> Option<&Preview> {
        self.preview.as_ref()
    }
    /// The result's file changed after the search read it.
    pub fn is_stale(&self, i: usize) -> bool {
        self.hits.get(i).is_some_and(|h| self.stale.contains(&h.path))
    }
    /// The absolute path of result `i`.
    pub fn target(&self, i: usize) -> Option<std::path::PathBuf> {
        let h = self.hits.get(i)?;
        Some(match &self.root {
            Some((root, _)) => root.join(&*h.path),
            None => std::path::PathBuf::from(&*h.path),
        })
    }
}

#[cfg(test)]
mod tests {
    use lenz_content::find::{FindOptions, Query};

    /// Literal text, case ignored: the default options.
    fn plain(text: &str) -> Query {
        Query { text: text.into(), options: FindOptions::default() }
    }

    /// A line's matches as project search reports them.
    fn grep_ranges(m: &RegexMatcher, line: &str) -> Vec<std::ops::Range<usize>> {
        let mut out = Vec::new();
        line_matches(m, line, &mut |r| {
            out.push(r);
            true
        });
        out
    }

    /// Project search and the find box agree on every option combination.
    #[test]
    fn grep_and_the_find_matcher_find_the_same_ranges() {
        let lines = ["Foo foo food _foo foo_", "a.b axb", "v1 V22 user_id", "café CAFÉ", "bbb", "baab foo bar"];
        let cases = [("a*", false, false, true), ("^", false, false, true), (r"\b", false, false, true), ("foo", false, false, false), ("foo", true, false, false), ("foo", false, true, false), ("foo", true, true, false), ("a.b", false, false, false), (r"v\d+", false, false, true), (r"\w+_id", false, true, true), ("café", false, false, false)];
        for (text, case, word, regex) in cases {
            let q = Query { text: text.into(), options: FindOptions { case, word, regex } };
            let ours: Vec<_> = lines.iter().map(|l| lenz_content::find::Matcher::new(&q).unwrap().ranges(l).collect::<Vec<_>>()).collect();
            let theirs: Vec<_> = lines.iter().map(|l| grep_ranges(&matcher(&q).unwrap().unwrap(), l)).collect();
            assert_eq!(ours, theirs, "{text:?} case={case} word={word} regex={regex}");
        }
    }

    #[test]
    fn a_bad_expression_is_refused_before_searching() {
        let q = Query { text: "(".into(), options: FindOptions { regex: true, ..Default::default() } };
        assert_eq!(matcher(&q).err().as_deref(), Some("unclosed group"));
        let found = search_text(&q, &"x".into(), "((\n");
        assert_eq!(found.error.as_deref(), Some("unclosed group"));
    }

    use super::*;

    fn rel(s: &str) -> Arc<str> {
        s.into()
    }

    #[test]
    fn literal_case_insensitive_with_exact_ranges_on_unicode_and_crlf_lines() {
        let found = search_text(&plain("CAFÉ"), &rel("a.txt"), "un café\r\nCafé, café!\nno\n");
        let got: Vec<_> = found.hits.iter().map(|h| (h.line, h.range.clone(), &*h.text)).collect();
        assert_eq!(got, [(0, 3..8, "un café"), (1, 0..5, "Café, café!"), (1, 7..12, "Café, café!")]);
        assert!(found.done && !found.limited);
        assert!(search_text(&plain("a.c"), &rel("x"), "abc\n").hits.is_empty(), "no regex: `.` is a dot");
        assert_eq!(search_text(&plain("a.c"), &rel("x"), "a.c\n").hits.len(), 1);
        assert!(search_text(&plain(""), &rel("x"), "anything\n").hits.is_empty());
    }

    #[test]
    fn frequent_matches_stop_at_the_limit_and_say_so() {
        let text = "x\n".repeat(RESULT_LIMIT + 5);
        let found = search_text(&plain("x"), &rel("x"), &text);
        assert_eq!((found.hits.len(), found.limited), (RESULT_LIMIT, true));
    }

    #[test]
    fn project_search_streams_skips_binary_and_stops_when_cancelled() {
        let d = tempfile::tempdir().unwrap();
        let mut files = Vec::new();
        for i in 0..40 {
            let name = format!("f{i}.txt");
            std::fs::write(d.path().join(&name), "needle\n".repeat(20)).unwrap();
            files.push(rel(&name));
        }
        std::fs::write(d.path().join("bin.dat"), b"needle\0binary").unwrap();
        files.push(rel("bin.dat"));
        let (latest, mut batches) = (AtomicU64::new(1), Vec::new());
        search_files(d.path(), &files, &plain("NEEDLE"), 1, &latest, &mut |f| {
            batches.push(f);
            true
        });
        let hits: usize = batches.iter().map(|b| b.hits.len()).sum();
        assert_eq!(hits, 800, "binary skipped");
        assert!(batches.len() > 1, "streamed in batches");
        assert!(batches.last().unwrap().done && batches.last().unwrap().searched == 41);
        let mut sent = 0;
        search_files(d.path(), &files, &plain("needle"), 2, &latest, &mut |_| {
            sent += 1;
            true
        });
        assert_eq!(sent, 0, "a superseded search sends nothing");
    }

    #[test]
    fn legacy_encodings_are_searched_as_decoded_and_unreadable_files_counted() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("latin.txt"), b"x \xe9t\xe9, caf\xe9 cr\xe8me br\xfbl\xe9e au lait\n").unwrap();
        std::fs::write(d.path().join("bom.txt"), b"\xEF\xBB\xBFcaf\xC3\xA9\n").unwrap();
        let files = [rel("latin.txt"), rel("bom.txt"), rel("gone.txt")];
        let mut found = Vec::new();
        search_files(d.path(), &files, &plain("café"), 1, &AtomicU64::new(1), &mut |f| {
            found.push(f);
            true
        });
        let last = found.last().unwrap();
        let hits: Vec<_> = found.iter().flat_map(|f| f.hits.iter()).map(|h| (&*h.path, h.range.clone(), h.text.to_string())).collect();
        assert_eq!(hits, [("latin.txt", 9..14, "x été, café crème brûlée au lait".to_string()), ("bom.txt", 0..5, "café".to_string())]);
        assert_eq!((last.failed, last.searched), (1, 3));
        let doc = crate::document::Document::decode("latin.txt".into(), 1, b"x \xe9t\xe9, caf\xe9 cr\xe8me br\xfbl\xe9e au lait\n");
        assert_eq!(&doc.line(0)[9..14], "café", "ranges agree with the decoded document");
    }

    #[test]
    fn file_patterns_include_and_exclude_and_files_are_searched_folder_by_folder() {
        let paths: Vec<Arc<str>> = ["src/b.rs", "a.rs", "src/a/x.rs", "src/a.rs", "web/app.ts", "src/a/x.test.rs", "README.md"].map(rel).to_vec();
        let ordered = |pattern: &str, folders_first: bool| select_files(paths.clone(), pattern, folders_first).map(|v| v.iter().map(|p| p.to_string()).collect::<Vec<_>>());
        let picked = |pattern: &str| ordered(pattern, true);
        // Each folder's contents together (the results' tree), in the explorer's order: subfolders, then files.
        assert_eq!(picked("").unwrap(), ["src/a/x.rs", "src/a/x.test.rs", "src/a.rs", "src/b.rs", "web/app.ts", "README.md", "a.rs"]);
        assert_eq!(ordered("", false).unwrap(), ["README.md", "a.rs", "src/a/x.rs", "src/a/x.test.rs", "src/a.rs", "src/b.rs", "web/app.ts"], "or folders among files by name");
        assert_eq!(picked("*.rs, !*.test.rs").unwrap(), ["src/a/x.rs", "src/a.rs", "src/b.rs", "a.rs"], "a name pattern matches at any depth");
        assert_eq!(picked("src/").unwrap(), ["src/a/x.rs", "src/a/x.test.rs", "src/a.rs", "src/b.rs"], "a folder name means everything in it");
        assert_eq!(picked("src/a").unwrap(), ["src/a/x.rs", "src/a/x.test.rs"], "a path with a slash is from the root");
        assert_eq!(picked("!web").unwrap().len(), 6, "only exclusions: everything else");
        assert_eq!(picked("*.MD").unwrap(), ["README.md"], "any case");
        assert!(picked("src/[").unwrap_err().contains("src/["), "a bad pattern says which");
    }

    #[test]
    fn a_long_line_is_shown_from_a_little_before_its_match() {
        let text = format!("    {}needle tail", "x".repeat(200));
        let at = text.find("needle").unwrap();
        assert_eq!(clip_start(&text, 4, at..at + 6, 300), 4, "a line that fits starts after its indentation");
        let start = clip_start(&text, 4, at..at + 6, 60);
        assert_eq!(at - start, 20, "a third of the room is context before the match");
        assert_eq!(clip_start(&text, 4, 10..300, 60), 4, "a range past the kept text changes nothing");
        let wide = format!("{}界needle", "界".repeat(100));
        let at = wide.find("needle").unwrap();
        assert!(wide.is_char_boundary(clip_start(&wide, 0, at..at + 6, 30)));
        for (before, after) in [("foo bar", "foo "), ("foo bar  ", "foo "), ("bar", ""), ("漢\u{3000}字", "漢\u{3000}"), ("", "")] {
            let mut text = before.to_owned();
            drop_word(&mut text);
            assert_eq!(text, after);
        }
    }

    #[test]
    fn a_match_line_is_coloured_by_its_file_language() {
        let hit = |path: &str, text: &str| Hit { path: rel(path), line: 0, range: 0..1, text: text.into(), decl: None, origin: None };
        let spans = snippet_spans(&hit("src/a.rs", "let s = \"needle\"; // note"));
        let kinds: Vec<_> = spans.iter().map(|(r, c)| ("let s = \"needle\"; // note".get(r.clone()).unwrap().to_string(), *c)).collect();
        assert!(kinds.iter().any(|(t, c)| t == "let" && *c == lenz_content::highlight::Capture::Keyword), "{kinds:?}");
        assert!(kinds.iter().any(|(t, c)| t.contains("needle") && *c == lenz_content::highlight::Capture::String), "{kinds:?}");
        assert!(snippet_spans(&hit("notes.unknown", "let x")).is_empty(), "no language, no colours");
    }

    #[test]
    fn validation_compares_the_current_line() {
        let hit = Hit { path: rel("a"), line: 0, range: 3..6, text: "fn Foo()".into(), decl: None, origin: None };
        assert!(still_matches(&hit, ResultsKind::Text, &plain("foo"), "fn Foo()"));
        assert!(!still_matches(&hit, ResultsKind::Text, &plain("foo"), "fn Bar()"));
        assert!(!still_matches(&hit, ResultsKind::Text, &plain("foo"), "fn"));
        assert!(!still_matches(&hit, ResultsKind::Text, &plain("foo"), "foo Bar()"), "the match moved");
    }
}
