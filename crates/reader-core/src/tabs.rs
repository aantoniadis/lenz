//! Tabs and reading history. A tab is a view of one source with its own presentation state; at most one tab is
//! the reusable preview. History records reading locations independently of tabs, so Back can reopen a source
//! whose tab has closed. Neither keeps a document snapshot alive beyond the tab showing it.

use std::{collections::VecDeque, ops::Range, path::PathBuf, sync::Arc};

use lenz_content::highlight::Parsed;

use crate::{cache::Stamp, document::Document, session::Selection};

/// History entries kept, oldest dropped first.
pub const HISTORY: usize = 200;
/// Characters of context kept with a location to find it again in a changed document.
pub const ANCHOR_CHARS: usize = 120;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TabId(pub u64);

/// Rows a pane of `rows` rows gives to sticky lines: none below 6, at most a third, at most 5.
pub fn sticky_cap(rows: usize) -> usize {
    if rows < 6 { 0 } else { (rows / 3).min(5) }
}

/// One pinned header: its source line and the row (0 = the pane's top) it is drawn on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StickyLine {
    pub line: usize,
    pub row: usize,
}

/// Sticky lines and what they were made for: the declarations (held, so a new set cannot reuse their address), top
/// line, top row, limit, line count, collapsed generation and wrap width.
type StickyCache = (Arc<lenz_content::highlight::Declarations>, (usize, usize, usize, usize, u64, Option<usize>), Arc<[StickyLine]>);

/// A tab's shown lines and what they were made for: its folds, collapsed generation and line count.
type VisibleCache = (Option<Arc<lenz_content::highlight::Folds>>, u64, usize, Arc<crate::folds::Visible>);

/// One tab: a source and how it is being read. Its document may be released while inactive (reloadable content
/// goes to the inactive cache); positions stay.
pub struct Tab {
    pub(crate) id: TabId,
    /// The file panel's pane it belongs to (0, or 1 while split).
    pub(crate) pane: u8,
    /// The source's identity: absolute and normalized.
    pub(crate) source: PathBuf,
    /// The path as it was requested, for display.
    pub(crate) display: PathBuf,
    /// The resolved path once loaded (symlinks followed), so two spellings of one file share a tab.
    pub(crate) canonical: Option<PathBuf>,
    /// Pinned or double-clicked: not replaced by the next preview.
    pub(crate) retained: bool,
    /// Compatibility worker/layout input; current source reads resolve through State::view.
    pub(crate) doc: Option<Arc<Document>>,
    /// Session-owned source identity, retained when reloadable content is released.
    pub(crate) buffer: Option<crate::text::BufferId>,
    /// Its language as last loaded, kept while its content is released (`None`: never loaded).
    pub(crate) language: Option<Option<lenz_content::highlight::Language>>,
    pub(crate) stamp: Option<Stamp>,
    pub(crate) highlights: Option<Arc<Parsed>>,
    /// Fold regions of its document's version (they arrive with the highlights).
    pub(crate) folds: Option<Arc<lenz_content::highlight::Folds>>,
    /// Declarations of its document's version, from the same parse, while sticky scroll or breadcrumbs is on.
    pub(crate) declarations: Option<Arc<lenz_content::highlight::Declarations>>,
    /// Inlay hints of a document version (shown only while it is the document's).
    pub(crate) hints: Option<(u64, Arc<crate::decor::Hints>)>,
    /// Semantic tokens of a document version.
    pub(crate) tokens: Option<(u64, Arc<crate::decor::Tokens>)>,
    /// The sticky lines last made, and what they were made for.
    sticky: std::sync::Mutex<Option<StickyCache>>,
    /// Collapsed regions by first line and kind; kept across reloads where a region of that kind still starts there.
    pub(crate) collapsed: crate::folds::Collapsed,
    /// The shown lines last made, with the folds, collapsed generation and line count they were made for (the folds
    /// held, so a new set cannot reuse their address).
    visible: std::sync::Mutex<Option<VisibleCache>>,
    pub(crate) selection: Option<Selection>,
    /// Extend Selection's chain while its last selection is the tab's (dropped with the content it was made for).
    pub(crate) expansion: Option<crate::expand::Expansion>,
    /// In-document find: the query, its matches in this document version and the current one (dropped on close).
    pub(crate) find: Option<crate::find::FindState>,
    /// This tab's newest find request; an older one's matching stops (other tabs' finds are unaffected).
    pub(crate) find_latest: std::sync::Arc<std::sync::atomic::AtomicU64>,
    pub(crate) top_line: usize,
    /// While soft-wrapped: the row shown first, and the top line it was set for (any other top line starts at its
    /// first row, so every jump of `top_line` resets it).
    top_row: (usize, usize),
    /// Soft-wrapped rows made so far (for one document version and width).
    wrap: std::sync::Mutex<crate::wrap::WrapCache>,
    /// Horizontal position, in display columns.
    pub(crate) column: usize,
    /// Where to put the view once the document is (re)loaded.
    pub(crate) restore: Option<Location>,
    /// The last reload failed: why, while the last content stays.
    pub(crate) unavailable: Option<String>,
    /// The last restore could not find its context exactly.
    pub(crate) approximate: bool,
    /// The latest refresh check for this tab; older results are dropped.
    pub(crate) refresh: u64,
    /// A move check is in flight; plain refreshes wait for it (it falls back to one).
    pub(crate) moving: bool,
    /// Consecutive checks that raced a writer (0: settled).
    pub(crate) changing: u32,
    /// Its content is released and the file has changed since: it reloads on activation.
    pub(crate) stale_on_disk: bool,
    /// Source a language server returned for a target outside the file system: never read from disk, watched,
    /// released or sent to a server.
    pub(crate) retrieved: Option<crate::semantics::VirtualSource>,
    /// An SVG shown as its source text rather than rendered (a presentation state of the tab).
    pub(crate) image_source: bool,
    /// Open since before the current folder was opened: the folder's places do not record it.
    pub(crate) before_root: bool,
    /// A Markdown file shown as its source rather than its preview (a presentation state of the tab; source
    /// first, the preview on request).
    pub(crate) markdown_source: bool,
    /// The shown version's Markdown preview, or why it cannot be shown (`None`: not made yet).
    pub(crate) preview: Option<Result<Arc<crate::markdown::Preview>, String>>,
    /// Annotate with Git Blame, while on: kept across releases, made again for new content.
    pub(crate) blame: Option<crate::blame::Blame>,
}

impl Tab {
    pub(crate) fn new(id: TabId, source: PathBuf, retained: bool) -> Self {
        Self {
            id,
            pane: 0,
            display: source.clone(),
            canonical: None,
            source,
            retained,
            doc: None,
            buffer: None,
            language: None,
            stamp: None,
            highlights: None,
            folds: None,
            declarations: None,
            hints: None,
            tokens: None,
            sticky: std::sync::Mutex::new(None),
            collapsed: Default::default(),
            visible: Default::default(),
            selection: None,
            expansion: None,
            find: None,
            find_latest: Default::default(),
            top_row: (0, 0),
            wrap: Default::default(),
            top_line: 0,
            column: 0,
            restore: None,
            unavailable: None,
            approximate: false,
            refresh: 0,
            moving: false,
            changing: 0,
            stale_on_disk: false,
            retrieved: None,
            image_source: false,
            before_root: false,
            markdown_source: true,
            preview: None,
            blame: None,
        }
    }

    pub fn id(&self) -> TabId {
        self.id
    }
    pub fn pane(&self) -> u8 {
        self.pane
    }
    /// Its document, when in memory.
    pub fn document(&self) -> Option<&Arc<Document>> {
        self.doc.as_ref()
    }
    /// Syntax spans for its document's version, when ready.
    pub fn highlights(&self) -> Option<&Parsed> {
        self.highlights.as_deref()
    }
    /// Declarations for its document's version, when asked for and ready.
    pub fn declarations(&self) -> Option<&lenz_content::highlight::Declarations> {
        self.declarations.as_deref()
    }
    /// Retained identity for adapter geometry; checking it does not compute sticky layout.
    pub fn declarations_handle(&self) -> Option<&Arc<lenz_content::highlight::Declarations>> {
        self.declarations.as_ref()
    }
    /// Inlay hints for its document's version, when ready.
    pub fn hints(&self) -> Option<&Arc<crate::decor::Hints>> {
        let (version, hints) = self.hints.as_ref()?;
        (self.doc.as_ref()?.version() == *version).then_some(hints)
    }
    /// Semantic tokens for its document's version, when ready.
    pub fn tokens(&self) -> Option<&Arc<crate::decor::Tokens>> {
        let (version, tokens) = self.tokens.as_ref()?;
        (self.doc.as_ref()?.version() == *version).then_some(tokens)
    }
    /// Line `line`'s colours: the syntax spans with the semantic tokens laid over (`None`: the default text colour,
    /// under a modifier-only token), and each span's [`crate::decor::MUTABLE`] / [`crate::decor::DEPRECATED`] bits.
    pub fn spans(&self, line: usize) -> Vec<(Range<usize>, Option<lenz_content::highlight::Capture>, u8)> {
        let syntax = self.highlights().and_then(|p| p.spans(line..line + 1).pop()).unwrap_or_default();
        crate::decor::overlay(&syntax, self.tokens().map_or(&[][..], |t| t.line(line)))
    }
    /// The row of `line` (wrapped at `cols`) the caret at `byte` is drawn on: chosen by where it is drawn (before a type
    /// hint at `byte`, after a parameter hint), so a wrap break inside a hint label keeps it on the row that shows it.
    pub fn caret_row(&self, line: usize, byte: usize, cols: Option<usize>) -> usize {
        let rows = self.line_rows(line, cols);
        if rows.len() < 2 {
            return 0;
        }
        let at = self.shown_line(line).caret(byte);
        rows.iter().rposition(|r| r.display.start <= at).unwrap_or(0)
    }
    /// Line `line` as the text panes draw it, with its inlay hints.
    pub fn shown_line(&self, line: usize) -> crate::shown::Shown {
        let text = self.doc.as_ref().map_or("", |d| d.line(line));
        crate::shown::shown_line(text, self.hints().map_or(&[][..], |h| h.line(line)))
    }
    /// The header lines pinned at the top while `top_line` is first (its continuation rows count when wrapped): the
    /// enclosing declarations' first lines, at most `max`, innermost kept; a header slides up to the shown row (folds
    /// skipped, wrapped at `cols`) its scope ends on. Made again only when the top, the declarations, `max`, the folds
    /// or the wrap width change.
    pub fn sticky(&self, max: usize, cols: Option<usize>) -> Arc<[StickyLine]> {
        let Some(d) = self.declarations.as_ref() else { return Arc::from([]) };
        let lines = self.doc.as_ref().map_or(0, |doc| doc.line_count());
        let key = (self.top_line, self.top_row(), max, lines, self.collapsed.generation(), cols);
        let mut cache = self.sticky.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((_, _, v)) = cache.as_ref().filter(|(held, k, _)| Arc::ptr_eq(held, d) && *k == key) {
            return v.clone();
        }
        let top = u32::try_from(self.top_line).unwrap_or(u32::MAX);
        // The rows a header can slide over: the first `max + 1` shown from the top (a farther end never moves it).
        let shown = self.rows(max + 1, cols);
        let row_of = |line: u32| shown.iter().position(|r| r.line as u32 >= line).map_or(u32::MAX, |i| i as u32);
        let v: Arc<[StickyLine]> = d
            .sticky_rows(top, self.top_row() > 0, max, row_of)
            .into_iter()
            .map(|s| StickyLine { line: s.line as usize, row: s.row as usize })
            .filter(|s| s.line < lines)
            .collect();
        *cache = Some((d.clone(), key, v.clone()));
        v
    }
    /// Fold regions for its document's version, when ready.
    pub fn folds(&self) -> Option<&lenz_content::highlight::Folds> {
        self.folds.as_deref()
    }
    /// The lines it shows: all but those inside its collapsed regions. Made again only when the folds, the collapsed
    /// regions or the line count change (rows, clicks and the pointer ask often).
    pub fn visible(&self) -> Arc<crate::folds::Visible> {
        self.visible_for(self.doc.as_ref().map_or(0, |d| d.line_count()))
    }
    pub(crate) fn visible_for(&self, lines: usize) -> Arc<crate::folds::Visible> {
        let generation = self.collapsed.generation();
        let mut cache = self.visible.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let same = |f: &Option<Arc<lenz_content::highlight::Folds>>| match (f, &self.folds) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        match cache.as_ref() {
            Some((f, g, l, v)) if same(f) && *g == generation && *l == lines => v.clone(),
            _ => {
                let v = Arc::new(crate::folds::Visible::new(self.folds.as_deref(), self.collapsed.map(), lines));
                *cache = Some((self.folds.clone(), generation, lines, v.clone()));
                v
            }
        }
    }
    /// The region starting on `line`, and whether it is collapsed.
    pub fn fold_at(&self, line: usize) -> Option<(lenz_content::highlight::Fold, bool)> {
        let f = self.folds.as_ref()?.starting(line)?;
        Some((f, self.collapsed.get(line) == Some(f.kind)))
    }
    /// Unfold every collapsed region hiding `line`.
    pub(crate) fn reveal(&mut self, line: usize) {
        let Some(folds) = self.folds.clone() else { return };
        self.collapsed.retain(|first, _| !folds.starting(first).is_some_and(|f| first < line && line <= f.last as usize));
    }
    /// After collapsing: a caret on a hidden line moves to the end of its fold's first line (a selection stays: it
    /// still covers the hidden lines), and the top line to the row showing it.
    pub(crate) fn leave_hidden(&mut self) {
        let Some(doc) = self.doc.clone() else { return };
        let v = self.visible();
        if let Some(s) = self.selection.filter(|s| s.anchor == s.head && v.hidden(doc.line_of(s.head))) {
            let at = doc.line_range(v.shown(doc.line_of(s.head))).end;
            self.selection = Some(Selection { anchor: at, head: at });
        }
        self.top_line = v.shown(self.top_line);
    }
    /// Keep the selection's head at or below the top line (the adapter keeps it above the bottom).
    pub(crate) fn show_head(&mut self) {
        let Some(line) = self.selection.zip(self.doc.as_ref()).map(|(s, d)| d.line_of(s.head)) else { return };
        self.top_line = self.top_line.min(self.visible().shown(line));
    }
    /// Unfold what hides the top line and the caret (after a navigation or new folds).
    pub(crate) fn reveal_view(&mut self) {
        let head = self.selection.zip(self.doc.as_ref()).map(|(s, d)| d.line_of(s.head));
        self.reveal(self.top_line);
        if let Some(l) = head {
            self.reveal(l);
        }
    }
    /// The name holding the character at byte `col` of `line` (its range in the line) when it may lead somewhere:
    /// not a keyword, literal or comment.
    pub fn linkable(&self, line: usize, col: usize) -> Option<std::ops::Range<usize>> {
        let doc = self.doc.as_ref()?;
        let word = crate::document::identifier(doc.line(line), col)?;
        use lenz_content::highlight::Capture as C;
        let spans = self.highlights().map(|p| p.spans(line..line + 1)).unwrap_or_default();
        let quiet = spans.first().is_some_and(|s| s.iter().any(|(r, c)| r.contains(&word.start) && matches!(c, C::Keyword | C::String | C::Escape | C::Comment | C::DocComment | C::Number | C::ConstantBuiltin)));
        (!quiet).then_some(word)
    }
    /// The name at a caret at byte `offset` (on it, or just after it), as a range of the document.
    pub fn name_at(&self, offset: usize) -> Option<std::ops::Range<usize>> {
        let doc = self.doc.as_ref()?;
        let line = doc.line_of(offset);
        let start = doc.line_range(line).start;
        let col = offset - start;
        let r = self.linkable(line, col).or_else(|| col.checked_sub(1).filter(|c| doc.line(line).is_char_boundary(*c)).and_then(|c| self.linkable(line, c)))?;
        Some(start + r.start..start + r.end)
    }
    /// How a Markdown file is shown (`None`: not Markdown). A preview that cannot be made shows the source, and a
    /// notice says why.
    pub fn rendering(&self) -> Option<crate::session::Rendering<'_>> {
        use crate::session::Rendering;
        self.doc.as_ref().filter(|d| d.language() == Some(lenz_content::highlight::Language::Markdown))?;
        Some(match &self.preview {
            _ if self.markdown_source => Rendering::Source,
            None => Rendering::Pending,
            Some(Ok(p)) => Rendering::Preview(p),
            Some(Err(_)) => Rendering::Source,
        })
    }
    /// A second view of the same source in pane `pane`: the same snapshot and what was derived from it, at the same
    /// place, kept open.
    pub(crate) fn duplicate(&self, id: TabId, pane: u8) -> Tab {
        Tab {
            id,
            pane,
            source: self.source.clone(),
            display: self.display.clone(),
            canonical: self.canonical.clone(),
            retained: true,
            doc: self.doc.clone(),
            buffer: self.buffer,
            language: self.language,
            stamp: self.stamp,
            highlights: self.highlights.clone(),
            folds: self.folds.clone(),
            declarations: self.declarations.clone(),
            hints: self.hints.clone(),
            tokens: self.tokens.clone(),
            sticky: Default::default(),
            collapsed: self.collapsed.clone(),
            visible: Default::default(),
            selection: self.selection,
            expansion: None,
            find: None,
            find_latest: Default::default(),
            top_row: (0, 0),
            wrap: Default::default(),
            top_line: self.top_line,
            column: self.column,
            restore: self.restore.clone(),
            unavailable: self.unavailable.clone(),
            approximate: self.approximate,
            refresh: 0,
            moving: false,
            changing: 0,
            stale_on_disk: self.stale_on_disk,
            retrieved: self.retrieved.clone(),
            image_source: self.image_source,
            before_root: false,
            markdown_source: self.markdown_source,
            preview: self.preview.clone(),
            // Its own annotations, blamed for itself once shown.
            blame: self.blame.as_ref().map(|_| crate::blame::Blame::new()),
        }
    }
    /// The source's identity: absolute and normalized.
    pub fn source(&self) -> &std::path::Path {
        &self.source
    }
    /// The path as the user asked for it.
    pub fn display_path(&self) -> &std::path::Path {
        &self.display
    }
    /// The file name, for the tab bar.
    pub fn title(&self) -> String {
        self.display.file_name().map_or_else(|| self.display.display().to_string(), |n| n.to_string_lossy().into_owned())
    }
    /// Whether an SVG shows its source text (else it is rendered).
    pub fn image_source(&self) -> bool {
        self.image_source
    }
    /// Whether a Markdown file shows its source (else its preview).
    pub fn markdown_source(&self) -> bool {
        self.markdown_source
    }
    /// Its shown content is replaced or released: what was derived from the old content goes.
    pub(crate) fn new_content(&mut self) {
        self.highlights = None;
        self.folds = None;
        self.declarations = None;
        self.hints = None;
        self.tokens = None;
        self.expansion = None;
        self.preview = None;
    }
    /// The annotation column's width in cells (0 while Annotate with Git Blame is off).
    pub fn annotation_cols(&self) -> usize {
        if self.blame.is_some() { crate::blame::ANNOTATION_COLS } else { 0 }
    }
    /// Line `line`'s blame annotation, once git has reached it.
    pub fn annotation(&self, line: usize) -> Option<crate::blame::Annotation<'_>> {
        crate::blame::annotation(self.blame.as_ref()?, line)
    }
    /// For source a language server returned (not a file): where it came from.
    pub fn retrieved(&self) -> Option<&crate::semantics::VirtualSource> {
        self.retrieved.as_ref()
    }
    /// The content is released and the file changed since; activation reloads it.
    pub fn stale_on_disk(&self) -> bool {
        self.stale_on_disk
    }
    pub fn retained(&self) -> bool {
        self.retained
    }
    /// Whether the content is in memory now (an inactive tab's may have been released).
    pub fn loaded(&self) -> bool {
        self.doc.is_some()
    }
    pub fn selection(&self) -> Option<Selection> {
        self.selection
    }
    pub fn top_row(&self) -> usize {
        if self.top_row.0 == self.top_line { self.top_row.1 } else { 0 }
    }

    /// The top as a line and its first shown row.
    pub(crate) fn set_top(&mut self, line: usize, row: usize) {
        self.top_line = line;
        self.top_row = (line, row);
    }

    /// `line`'s rows: wrapped at `cols` columns, else the whole line as one row.
    pub fn line_rows(&self, line: usize, cols: Option<usize>) -> std::sync::Arc<[crate::wrap::Row]> {
        let Some(doc) = self.doc.as_ref() else { return Vec::new().into() };
        match cols {
            Some(cols) => self.wrap.lock().unwrap_or_else(std::sync::PoisonError::into_inner).line_rows(doc, line, cols, self.hints()),
            None => {
                let text = doc.line(line);
                vec![crate::wrap::Row { line, index: 0, display: 0..lenz_content::display::displayed_bytes(text), source: 0..text.len(), indent: 0 }].into()
            }
        }
    }

    /// The `n` rows shown from the top (collapsed lines skipped), wrapped at `cols` when given.
    pub fn rows(&self, n: usize, cols: Option<usize>) -> Vec<crate::wrap::Row> {
        if self.doc.is_none() || n == 0 {
            return Vec::new();
        }
        let v = self.visible();
        let mut out = Vec::with_capacity(n);
        let (mut line, mut skip) = (self.top_line, if cols.is_some() { self.top_row() } else { 0 });
        loop {
            let rows = self.line_rows(line, cols);
            out.extend(rows.iter().skip(skip).take(n - out.len()).cloned());
            skip = 0;
            let next = v.step(line, 1);
            if out.len() >= n || next == line {
                return out;
            }
            line = next;
        }
    }

    /// `(line, row)` moved by `delta` rows (wrapped rows count; collapsed lines are skipped), within the document.
    pub(crate) fn step_rows(&self, (mut line, mut row): (usize, usize), delta: isize, cols: usize) -> (usize, usize) {
        let v = self.visible();
        let mut left = delta;
        while left > 0 {
            if row + 1 < self.line_rows(line, Some(cols)).len() {
                row += 1;
            } else {
                let next = v.step(line, 1);
                if next == line {
                    break;
                }
                (line, row) = (next, 0);
            }
            left -= 1;
        }
        while left < 0 {
            if row > 0 {
                row -= 1;
            } else {
                let prev = v.step(line, -1);
                if prev == line {
                    break;
                }
                (line, row) = (prev, self.line_rows(prev, Some(cols)).len().saturating_sub(1));
            }
            left += 1;
        }
        (line, row)
    }

    pub fn find(&self) -> Option<&crate::find::FindState> {
        self.find.as_ref()
    }
    pub fn top_line(&self) -> usize {
        self.top_line
    }
    pub fn column(&self) -> usize {
        self.column
    }
    /// The shown document's version: it advances when a refresh finds new content (0 before the first load).
    pub fn version(&self) -> u64 {
        self.doc.as_ref().map_or(0, |d| d.version())
    }
    /// Why the source cannot be read now, while its last content stays.
    pub fn unavailable(&self) -> Option<&str> {
        self.unavailable.as_deref()
    }

    /// Where the reader is in this tab: the selection (or the top line's start) with a context anchor.
    pub(crate) fn location(&self) -> Location {
        let Some(doc) = &self.doc else {
            // Not loaded: what it will be restored to, or the position it kept.
            return self.restore.clone().unwrap_or_else(|| Location {
                source: self.source.clone(),
                version: 0,
                range: 0..0,
                selected: false,
                top_line: self.top_line,
                anchor: Box::from(""),
                line: self.top_line,
                offset: 0,
            });
        };
        let start = doc.line_range(self.top_line.min(doc.line_count() - 1)).start;
        let range = self.selection.map_or(start..start, |s| s.range());
        let line = doc.line_of(range.start);
        Location {
            source: self.source.clone(),
            version: doc.version(),
            anchor: doc.line(line).chars().take(ANCHOR_CHARS).collect(),
            offset: range.start - doc.line_range(line).start,
            line,
            selected: self.selection.is_some(),
            range,
            top_line: self.top_line,
        }
    }

    /// Put the view where `at` was: exactly when the version is unchanged; else where its context line survives
    /// once, keeping the selection's place on that line; else near the old position, marked approximate.
    pub(crate) fn restore_to(&mut self, at: &Location) {
        self.place(at);
        // A navigation into a collapsed region unfolds it.
        self.reveal_view();
    }

    fn place(&mut self, at: &Location) {
        let Some(doc) = &self.doc else {
            self.restore = Some(at.clone());
            self.top_line = at.top_line;
            return;
        };
        let last = doc.line_count() - 1;
        let selection = |start: usize| at.selected.then(|| {
            let text = doc.text();
            let start = floor_char_boundary(text, start);
            Selection { anchor: start, head: floor_char_boundary(text, start + at.range.len()) }
        });
        if at.version == doc.version() {
            self.top_line = at.top_line.min(last);
            self.selection = selection(at.range.start);
            self.approximate = false;
            return;
        }
        // The anchor is a whole line (or its first ANCHOR_CHARS characters): it must match exactly and only once.
        let same = |l: usize| doc.line(l).chars().take(ANCHOR_CHARS).eq(at.anchor.chars());
        let mut matches = (0..doc.line_count()).filter(|&l| same(l));
        let unique = match (at.anchor.is_empty(), matches.next(), matches.next()) {
            (false, Some(l), None) => Some(l),
            _ => None,
        };
        match unique {
            Some(line) => {
                let start = doc.line_range(line).start + at.offset.min(doc.line_range(line).len());
                // Keep the anchor line where it was relative to the top of the view.
                let shift = line as isize - at.line as isize;
                self.top_line = (at.top_line as isize + shift).clamp(0, last as isize) as usize;
                self.selection = selection(start);
                self.approximate = false;
            }
            None => {
                self.top_line = at.top_line.min(last);
                self.selection = None;
                self.approximate = true;
            }
        }
    }
}

/// A place in a source that history can return to, without keeping the document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Location {
    pub source: PathBuf,
    /// The document version it was recorded against (0 if never loaded).
    pub version: u64,
    /// The selection, or the top line's start when there was none.
    pub range: Range<usize>,
    /// Whether `range` was a selection (else the view had none).
    pub selected: bool,
    pub top_line: usize,
    /// The line holding `range.start` (its first [`ANCHOR_CHARS`] characters), to find it again after the file
    /// changes, and `range.start`'s byte offset within it.
    pub anchor: Box<str>,
    /// The anchor's line number when recorded.
    pub line: usize,
    pub offset: usize,
}

fn floor_char_boundary(text: &str, mut i: usize) -> usize {
    i = i.min(text.len());
    while !text.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Reading history: committed navigations, Back and Forward.
#[derive(Default)]
pub struct History {
    entries: VecDeque<Location>,
    at: usize,
}

impl History {
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    pub fn can_back(&self) -> bool {
        self.at > 0
    }
    pub fn can_forward(&self) -> bool {
        self.at + 1 < self.entries.len()
    }

    /// A committed navigation from `from` (where the reader was, updated in place) to `to`. Forward entries go.
    pub fn visit(&mut self, from: Option<Location>, to: Location) {
        self.entries.truncate(if self.entries.is_empty() { 0 } else { self.at + 1 });
        if let Some(from) = from {
            match self.entries.back_mut() {
                // Still where history says: update that entry's position.
                Some(current) if current.source == from.source => *current = from,
                // Somewhere history did not record (a tab closed and another became active): a step of its own.
                _ => self.entries.push_back(from),
            }
        }
        if self.entries.back() != Some(&to) {
            self.entries.push_back(to);
        }
        while self.entries.len() > HISTORY {
            self.entries.pop_front();
        }
        self.at = self.entries.len().saturating_sub(1);
    }

    /// Step back from `here` (recorded as the current entry's position): where to go.
    pub fn back(&mut self, here: Location) -> Option<Location> {
        self.step(here, -1)
    }

    pub fn forward(&mut self, here: Location) -> Option<Location> {
        self.step(here, 1)
    }

    /// Back with no tab open: the current entry stays as recorded.
    pub fn back_from_nowhere(&mut self) -> Option<Location> {
        let here = self.entries.get(self.at)?.clone();
        // With every tab closed, Back first returns to where the reader last was.
        if self.at + 1 == self.entries.len() {
            return Some(here);
        }
        self.step(here, -1)
    }

    pub fn forward_from_nowhere(&mut self) -> Option<Location> {
        let here = self.entries.get(self.at)?.clone();
        self.step(here, 1)
    }

    fn step(&mut self, here: Location, by: isize) -> Option<Location> {
        let current = self.entries.get_mut(self.at)?;
        if current.source != here.source {
            // The reader moved off the current entry without a navigation (its tab closed): Back returns to it
            // rather than skipping it, Forward moves on without overwriting it.
            if by < 0 {
                return Some(current.clone());
            }
        } else {
            *current = here;
        }
        let to = self.at.checked_add_signed(by).filter(|&i| i < self.entries.len())?;
        self.at = to;
        Some(self.entries[to].clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(source: &str, line: usize) -> Location {
        Location { source: PathBuf::from(source), version: 1, range: 0..0, selected: false, top_line: line, anchor: Box::from(""), line: 0, offset: 0 }
    }

    #[test]
    fn back_forward_and_a_new_navigation_clearing_forward() {
        let mut h = History::default();
        h.visit(Some(at("a", 0)), at("b", 0));
        h.visit(Some(at("b", 5)), at("c", 0));
        assert_eq!(h.back(at("c", 9)).unwrap(), at("b", 5), "with the position b was left at");
        assert_eq!(h.back(at("b", 6)).unwrap(), at("a", 0));
        assert!(!h.can_back() && h.back(at("a", 1)).is_none());
        assert_eq!(h.forward(at("a", 2)).unwrap(), at("b", 6));
        assert_eq!(h.forward(at("b", 6)).unwrap(), at("c", 9));
        assert!(!h.can_forward());
        h.back(at("c", 9));
        h.visit(Some(at("b", 7)), at("d", 0));
        assert!(!h.can_forward(), "a new navigation after Back clears Forward");
        assert_eq!(h.back(at("d", 0)).unwrap(), at("b", 7));
    }

    #[test]
    fn history_is_bounded() {
        let mut h = History::default();
        for i in 0..HISTORY + 50 {
            h.visit(Some(at(&format!("{i}"), 0)), at(&format!("{}", i + 1), 0));
        }
        assert_eq!(h.len(), HISTORY);
        let (mut here, mut steps) = (at(&format!("{}", HISTORY + 50), 0), 0);
        while h.can_back() {
            here = h.back(here).unwrap();
            steps += 1;
        }
        assert_eq!(steps, HISTORY - 1);
        assert_eq!(here, at("51", 0), "the oldest kept entry");
    }

    #[test]
    fn restoring_finds_the_anchor_after_the_file_changes() {
        let doc = |text: &str, version| Arc::new(Document::decode(PathBuf::from("a.rs"), version, text.as_bytes()));
        let mut tab = Tab::new(TabId(1), PathBuf::from("a.rs"), false);
        tab.doc = Some(doc("one\ntwo\nfn target() {}\nend\n", 1));
        tab.selection = Some(Selection { anchor: 8, head: 10 });
        tab.top_line = 2;
        let here = tab.location();
        assert_eq!((here.version, here.range.clone(), &*here.anchor, here.offset), (1, 8..10, "fn target() {}", 0));
        // Same version: exact.
        tab.selection = None;
        tab.restore_to(&here);
        assert_eq!((tab.top_line, tab.selection.map(|s| s.range()), tab.approximate), (2, Some(8..10), false));
        // Lines inserted above: the unique context line is found and the selection keeps its place on it.
        tab.doc = Some(doc("new\nnew\none\ntwo\nfn target() {}\nend\n", 2));
        tab.restore_to(&here);
        assert_eq!((tab.top_line, tab.selection.map(|s| s.range()), tab.approximate), (4, Some(16..18), false));
        // The context line now appears twice: ambiguous, so near the old position and marked approximate.
        tab.doc = Some(doc("fn target() {}\none\ntwo\nfn target() {}\n", 3));
        tab.restore_to(&here);
        assert_eq!((tab.top_line, tab.selection, tab.approximate), (2, None, true));
        // Gone: clamped, approximate.
        tab.doc = Some(doc("x\n", 4));
        tab.restore_to(&here);
        assert_eq!((tab.top_line, tab.selection, tab.approximate), (1, None, true));
        // No selection recorded: none restored.
        let mut plain = Tab::new(TabId(2), PathBuf::from("a.rs"), false);
        plain.doc = Some(doc("a\nb\nc\n", 5));
        plain.top_line = 2;
        let at = plain.location();
        plain.restore_to(&at);
        assert_eq!(plain.selection, None);
    }
}

#[cfg(test)]
mod closed_tabs {
    use super::*;

    fn at(source: &str) -> Location {
        Location { source: PathBuf::from(source), version: 1, range: 0..0, selected: false, top_line: 0, anchor: Box::from(""), line: 0, offset: 0 }
    }

    /// a → b, b's tab closes (a shows), then c: b stays in history between them.
    #[test]
    fn a_closed_tabs_entry_survives_later_navigation() {
        let mut h = History::default();
        h.visit(Some(at("a")), at("b"));
        h.visit(Some(at("a")), at("c")); // the reader was on a (b closed) when it opened c
        assert_eq!(h.back(at("c")).unwrap(), at("a"));
        assert_eq!(h.back(at("a")).unwrap(), at("b"));
    }

    /// a → b → c, c closes (b shows): Back reopens c first, Forward does not overwrite it.
    #[test]
    fn back_after_closing_the_active_tab_reopens_it() {
        let mut h = History::default();
        h.visit(Some(at("a")), at("b"));
        h.visit(Some(at("b")), at("c"));
        assert_eq!(h.back(at("b")).unwrap(), at("c"), "returns to the closed source");
        assert_eq!(h.back(at("c")).unwrap(), at("b"));
        assert_eq!(h.forward(at("b")).unwrap(), at("c"));
    }
}
