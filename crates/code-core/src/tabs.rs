//! Tabs and reading history. A tab is a view of one source with its own presentation state; at most one tab is
//! the reusable preview. History records reading locations independently of tabs, so Back can reopen a source
//! whose tab has closed. Neither keeps a document snapshot alive beyond the tab showing it.

use std::{collections::VecDeque, ops::Range, path::PathBuf, sync::Arc};

use lenz_content::highlight::Parsed;

use crate::{cache::Stamp, text::TextSnapshot, session::Selection};

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

/// A tab's shown lines and what they were made for: its folds, collapsed generation and line count.
type VisibleCache = (Option<Arc<lenz_content::highlight::Folds>>, u64, usize, Arc<crate::folds::Visible>);

/// The identity prefix of untitled documents; never read, watched or persisted as a path.
pub(crate) const UNTITLED: &str = "untitled://";

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
    /// Whether this view has admitted content, rather than released cache-only content.
    pub(crate) loaded: bool,
    /// Session-owned source identity, retained when reloadable content is released.
    pub(crate) buffer: Option<crate::text::BufferId>,
    pub(crate) comparison_side: Option<crate::session::ComparisonSide>,
    /// Its language as last loaded, kept while its content is released (`None`: never loaded).
    pub(crate) language: Option<Option<lenz_content::highlight::Language>>,
    pub(crate) stamp: Option<Stamp>,
    /// The file's identity when a check last found it gone: a rename the watcher reports after that check is still
    /// followed by identity.
    pub(crate) vanished: Option<Stamp>,
    pub(crate) highlights: Option<Arc<Parsed>>,
    /// Lexical colours on unchanged bytes while the current draft is being parsed.
    pub(crate) held_highlights: Option<HeldHighlights>,
    /// Semantic colours only, retained until this draft has a current token payload.
    pub(crate) held_tokens: Option<HeldTokens>,
    pub(crate) csv: Option<crate::csv::View>,
    pub(crate) parsed_source: Option<crate::session::SemanticSource>,
    /// Fold regions of its document's version (they arrive with the highlights).
    pub(crate) folds: Option<Arc<lenz_content::highlight::Folds>>,
    /// Declarations of its document's version, from the same parse, while sticky scroll or breadcrumbs is on.
    pub(crate) declarations: Option<Arc<lenz_content::highlight::Declarations>>,
    /// Inlay hints of a document version (shown only while it is the document's).
    pub(crate) hints: Option<(u64, Arc<crate::decor::Hints>)>,
    pub(crate) decor_source: Option<crate::session::SemanticSource>,
    /// Semantic tokens of a document version.
    pub(crate) tokens: Option<(u64, Arc<crate::decor::Tokens>)>,
    /// Collapsed regions by first line and kind; kept across reloads where a region of that kind still starts there.
    pub(crate) collapsed: crate::folds::Collapsed,
    /// The shown lines last made, with the folds, collapsed generation and line count they were made for (the folds
    /// held, so a new set cannot reuse their address).
    visible: std::sync::Mutex<Option<VisibleCache>>,
    /// The primary selection: the one motions, Cut and Copy without a set of cursors act on. Write it through
    /// [`Tab::place_selection`] or [`Tab::set_selections`], which keep the others consistent.
    selection: Option<Selection>,
    /// Further selections made with Add Cursor Above/Below or Select Next Occurrence: disjoint, in source order, and
    /// never persisted (a restored view has one selection).
    extra: Vec<Selection>,
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
    /// Bounded logical layout derived from the registry snapshot.
    pub(crate) layout: std::sync::Mutex<crate::layout::Cache>,
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
    pub(crate) preview_source: Option<crate::session::SemanticSource>,
    /// While the preview of an edited version is being made: the last preview shown, which stays on screen, and the
    /// edits made since its version (each edit set in order), which carry its positions to the current text.
    pub(crate) held: Option<HeldPreview>,
    /// Annotate with Git Blame, while on: kept across releases, made again for new content.
    pub(crate) blame: Option<crate::blame::Blame>,
    /// Shown from a recovered draft: it stays named as recovered until it is saved, discarded or reloaded.
    pub(crate) recovered: bool,
}

/// A preview kept on screen while the edited version's preview is made.
#[derive(Clone, Debug)]
pub(crate) struct HeldPreview {
    pub(crate) preview: Arc<crate::markdown::Preview>,
    pub(crate) edits: Vec<Arc<[crate::text::TextEdit]>>,
}

impl HeldPreview {
    /// Edit sets a held preview is carried through before it is let go (the preview then waits for its version).
    pub(crate) const MAX_EDITS: usize = 1024;

    /// A source byte of the held preview's version, carried to the current text.
    pub(crate) fn carry(&self, byte: usize) -> usize {
        self.edits.iter().fold(byte, |byte, edits| crate::text::edited_byte(byte, edits))
    }
}

/// One colour payload and its surviving byte intervals in the current draft. Inserted bytes have no prior style.
/// The map has at most one interval per original byte; lookup never walks the edit history.
pub(crate) struct HeldColours<T> {
    data: Arc<T>,
    source: Arc<TextSnapshot>,
    ranges: Vec<(Range<usize>, usize)>,
}

pub(crate) type HeldHighlights = HeldColours<Parsed>;
pub(crate) type HeldTokens = HeldColours<crate::decor::Tokens>;

impl<T> Clone for HeldColours<T> {
    fn clone(&self) -> Self { Self { data: self.data.clone(), source: self.source.clone(), ranges: self.ranges.clone() } }
}

impl<T> HeldColours<T> {
    fn edited(mut self, edits: &[crate::text::TextEdit]) -> Option<Self> {
        let mut ranges: Vec<(Range<usize>, usize)> = Vec::new();
        let (mut at, mut delta, mut removed_through) = (0, 0isize, 0);
        for (range, original) in &self.ranges {
            let mut start = range.start.max(removed_through);
            let mut keep = |start: usize, end: usize, delta: isize| -> Option<()> {
                if start < end {
                    let mapped = start.checked_add_signed(delta)?..end.checked_add_signed(delta)?;
                    let original = original + start - range.start;
                    if let Some((last, from)) = ranges.last_mut()
                        && last.end == mapped.start && *from + last.len() == original {
                        last.end = mapped.end;
                    } else { ranges.push((mapped, original)); }
                }
                Some(())
            };
            while let Some(edit) = edits.get(at).filter(|edit| edit.range.start < range.end) {
                keep(start, edit.range.start.min(range.end), delta)?;
                removed_through = edit.range.end;
                start = start.max(removed_through);
                delta += edit.text.len() as isize - edit.range.len() as isize;
                at += 1;
            }
            keep(start, range.end, delta)?;
        }
        self.ranges = ranges;
        (!self.ranges.is_empty()).then_some(self)
    }

    fn source_byte(&self, byte: usize) -> Option<(usize, usize, Range<usize>)> {
        let (range, original) = self.ranges.get(self.ranges.partition_point(|(range, _)| range.end <= byte))?;
        if byte < range.start { return None; }
        let byte = original + byte - range.start;
        let line = self.source.line_of(byte);
        Some((line, byte - self.source.reader_line_range(line).start, *original..original + range.len()))
    }
}

impl HeldHighlights {
    pub(crate) fn capture_at(&self, byte: usize) -> Option<lenz_content::highlight::Capture> {
        let (line, byte, _) = self.source_byte(byte)?;
        self.data.capture_at(line, byte)
    }
}

impl HeldTokens {
    pub(crate) fn capture_at(&self, byte: usize) -> Option<lenz_content::highlight::Capture> {
        let (line, byte, surviving) = self.source_byte(byte)?;
        let tokens = self.data.line(line);
        let token = tokens.get(tokens.partition_point(|token| token.end as usize <= byte))
            .filter(|token| token.start as usize <= byte)?;
        let start = self.source.reader_line_range(line).start;
        // A changed token cannot lend its old colour to the surviving fragments.
        (surviving.start <= start + token.start as usize && start + token.end as usize <= surviving.end)
            .then_some(token.capture).flatten()
    }
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
            loaded: false,
            buffer: None,
            comparison_side: None,
            language: None,
            stamp: None,
            vanished: None,
            highlights: None,
            held_highlights: None,
            held_tokens: None,
            csv: None,
            parsed_source: None,
            folds: None,
            declarations: None,
            hints: None,
            decor_source: None,
            tokens: None,
            collapsed: Default::default(),
            visible: Default::default(),
            selection: None,
            extra: Vec::new(),
            expansion: None,
            find: None,
            find_latest: Default::default(),
            top_row: (0, 0),
            layout: Default::default(),
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
            preview_source: None,
            held: None,
            blame: None,
            recovered: false,
        }
    }

    pub fn id(&self) -> TabId {
        self.id
    }
    pub fn pane(&self) -> u8 {
        self.pane
    }
    pub(crate) fn visible_for(&self, source: &TextSnapshot) -> Arc<crate::folds::Visible> {
        let lines = source.line_count();
        let folds = self.folds.as_ref().filter(|_| self.parsed_source.as_ref().is_some_and(|s| s.matches(source)));
        let generation = self.collapsed.generation();
        let mut cache = self.visible.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let same = |f: &Option<Arc<lenz_content::highlight::Folds>>| match (f.as_ref(), folds) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        match cache.as_ref() {
            Some((f, g, l, v)) if same(f) && *g == generation && *l == lines => v.clone(),
            _ => {
                let v = Arc::new(crate::folds::Visible::new(folds.map(Arc::as_ref), self.collapsed.map(), lines));
                *cache = Some((folds.cloned(), generation, lines, v.clone()));
                v
            }
        }
    }
    /// The region starting on `line`, and whether it is collapsed.
    pub(crate) fn fold_at(&self, line: usize) -> Option<(lenz_content::highlight::Fold, bool)> {
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
    pub(crate) fn leave_hidden(&mut self, doc: &TextSnapshot) {
        let v = self.visible_for(doc);
        let moved = |s: Selection| if s.anchor == s.head && v.hidden(doc.line_of(s.head)) {
            let at = doc.reader_line_range(v.shown(doc.line_of(s.head))).end;
            Selection { anchor: at, head: at }
        } else { s };
        let primary = self.selection.map(moved);
        let extra: Vec<_> = std::mem::take(&mut self.extra).into_iter().map(moved).collect();
        if let Some(primary) = primary { self.set_selections(primary, extra); }
        self.top_line = v.shown(self.top_line);
    }
    /// Keep the selection's head at or below the top line (the adapter keeps it above the bottom).
    pub(crate) fn show_head(&mut self, doc: &TextSnapshot) {
        let Some(line) = self.selection.map(|s| doc.line_of(s.head)) else { return };
        self.top_line = self.top_line.min(self.visible_for(doc).shown(line));
    }
    /// Unfold what hides the top line and the caret (after a navigation or new folds).
    pub(crate) fn reveal_view(&mut self, doc: &TextSnapshot) {
        let head = self.selection.map(|s| doc.line_of(s.head));
        self.reveal(self.top_line);
        if let Some(l) = head {
            self.reveal(l);
        }
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
            loaded: self.loaded,
            buffer: self.buffer,
            comparison_side: None,
            language: self.language,
            stamp: self.stamp,
            vanished: self.vanished,
            highlights: self.highlights.clone(),
            held_highlights: self.held_highlights.clone(),
            held_tokens: self.held_tokens.clone(),
            csv: self.csv.as_ref().map(|csv| { let mut csv = csv.clone(); if csv.pending { csv.source = None; } csv }),
            parsed_source: self.parsed_source.clone(),
            folds: self.folds.clone(),
            declarations: self.declarations.clone(),
            hints: self.hints.clone(),
            decor_source: self.decor_source.clone(),
            tokens: self.tokens.clone(),
            collapsed: self.collapsed.clone(),
            visible: Default::default(),
            selection: self.selection,
            extra: self.extra.clone(),
            expansion: None,
            find: None,
            find_latest: Default::default(),
            top_row: (0, 0),
            layout: Default::default(),
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
            preview_source: self.preview_source.clone(),
            held: self.held.clone(),
            // Its own annotations, blamed for itself once shown.
            blame: self.blame.as_ref().map(|_| crate::blame::Blame::new()),
            recovered: self.recovered,
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
    pub fn comparison_side(&self) -> Option<crate::session::ComparisonSide> { self.comparison_side }
    pub fn is_saved_version(&self) -> bool { self.comparison_side == Some(crate::session::ComparisonSide::Saved) }
    /// A comparison side that shows a version of the file and refuses edits (last saved, disk or baseline).
    pub fn read_only_side(&self) -> Option<crate::session::ComparisonSide> { self.comparison_side.filter(|side| side.read_only().is_some()) }
    /// A New File document with no path yet: its session buffer is the only copy until Save As succeeds.
    pub fn untitled(&self) -> bool { self.retrieved.is_none() && self.source.to_str().is_some_and(|s| s.starts_with(UNTITLED)) }
    /// A view of an actual disk file (not a read-only comparison side, retrieved source or untitled document).
    pub(crate) fn is_file(&self) -> bool { self.read_only_side().is_none() && self.retrieved.is_none() && !self.untitled() }
    pub fn title(&self) -> String {
        if let Some(side) = self.comparison_side { return side.label().into(); }
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
        self.layout.get_mut().unwrap_or_else(std::sync::PoisonError::into_inner).clear();
        self.highlights = None;
        self.held_highlights = None;
        self.held_tokens = None;
        if let Some(csv) = &mut self.csv {
            if csv.input.is_some() {
                csv.pending = false;
                csv.error = Some("CSV source changed; pending field is preserved. Copy it or press Escape, then reopen the cell".into());
            } else { csv.data = None; csv.source = None; csv.pending = true; }
        }
        self.parsed_source = None;
        self.folds = None;
        self.declarations = None;
        self.hints = None;
        self.decor_source = None;
        self.tokens = None;
        self.expansion = None;
        self.preview = None;
        self.preview_source = None;
        self.held = None;
    }
    /// Keep semantic colours on unchanged bytes; modifiers and all semantic APIs stay current-only.
    pub(crate) fn hold_tokens(&mut self, previous: &Arc<TextSnapshot>, edits: &[crate::text::TextEdit]) -> Option<HeldTokens> {
        let held = if self.decor_source.as_ref().is_some_and(|source| source.matches(previous)) {
            self.tokens.as_ref().filter(|(version, tokens)| *version == previous.key().version && !tokens.is_empty()).map(|(_, tokens)| HeldTokens {
                data: tokens.clone(), source: previous.clone(), ranges: vec![(0..previous.len_bytes(), 0)],
            }).or_else(|| self.held_tokens.take())
        } else { self.held_tokens.take() };
        held?.edited(edits)
    }
    /// Keep only lexical colours attached to unchanged bytes; folds and semantic decorations stay current-only.
    pub(crate) fn hold_highlights(&mut self, previous: &Arc<TextSnapshot>, edits: &[crate::text::TextEdit]) -> Option<HeldHighlights> {
        let held = if self.parsed_source.as_ref().is_some_and(|source| source.matches(previous)) {
            self.highlights.as_ref().map(|parsed| HeldHighlights {
                data: parsed.clone(), source: previous.clone(), ranges: vec![(0..previous.len_bytes(), 0)],
            }).or_else(|| self.held_highlights.take())
        } else { self.held_highlights.take() };
        held?.edited(edits)
    }
    /// Before an edit replaces the shown version: the preview to keep showing meanwhile, with `edits` added to the
    /// way from its version to the new text. Only a shown preview of `previous` (or one already held) is kept, and
    /// only for so many edits; past that the preview waits for its new version.
    pub(crate) fn hold_preview(&mut self, previous: &crate::text::TextSnapshot, edits: &[crate::text::TextEdit]) -> Option<HeldPreview> {
        if self.markdown_source { return None; }
        let edits: Arc<[crate::text::TextEdit]> = edits.into();
        let current = self.preview_source.as_ref().is_some_and(|s| s.matches(previous));
        match (self.preview.take(), self.held.take()) {
            (Some(Ok(preview)), _) if current => Some(HeldPreview { preview, edits: vec![edits] }),
            (_, Some(mut held)) if held.edits.len() < HeldPreview::MAX_EDITS => { held.edits.push(edits); Some(held) }
            _ => None,
        }
    }
    /// The annotation column's width in cells (0 while Annotate with Git Blame is off).
    pub fn annotation_cols(&self) -> usize {
        if self.blame.is_some() { crate::blame::ANNOTATION_COLS } else { 0 }
    }
    /// Current attribution state while annotations are enabled.
    pub fn blame_status(&self) -> Option<crate::blame::Status<'_>> { self.blame.as_ref().map(crate::blame::Blame::status) }
    /// Line `line`'s blame annotation, once git has reached it.
    pub fn annotation(&self, line: usize) -> Option<crate::blame::Annotation<'_>> {
        self.annotation_with_width(line, crate::blame::ANNOTATION_COLS)
    }
    pub fn annotation_with_width(&self, line: usize, width: usize) -> Option<crate::blame::Annotation<'_>> {
        crate::blame::annotation(self.blame.as_ref()?, line, width)
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
    /// Whether this view shows work restored from a recovered draft.
    pub fn recovered(&self) -> bool {
        self.recovered
    }
    /// Whether the content is in memory now (an inactive tab's may have been released).
    pub fn loaded(&self) -> bool {
        self.loaded
    }
    /// The primary selection: what a single caret or selection command sees.
    pub fn selection(&self) -> Option<Selection> {
        self.selection
    }
    /// Every selection of this view: the primary first, then the others in source order.
    pub fn selections(&self) -> impl Iterator<Item = Selection> + '_ {
        self.selection.into_iter().chain(self.extra.iter().copied())
    }
    /// The selections after the primary (empty unless Add Cursor Above/Below or Select Next Occurrence was used).
    pub fn extra_selections(&self) -> &[Selection] {
        &self.extra
    }
    /// Make `selection` the only one: a click, motion, navigation or jump leaves one place.
    pub(crate) fn place_selection(&mut self, selection: Option<Selection>) {
        self.selection = selection;
        self.extra.clear();
    }
    /// Replace the primary and the others with one consistent set (see [`crate::selection::normalize`]).
    pub(crate) fn set_selections(&mut self, primary: Selection, others: impl IntoIterator<Item = Selection>) {
        let (primary, extra) = crate::selection::normalize(primary, others);
        self.selection = Some(primary);
        self.extra = extra;
    }
    /// Drop the secondary selections, keeping the primary.
    pub(crate) fn collapse_selections(&mut self) {
        self.extra.clear();
    }
    /// Carry every selection through a transaction: caret positions stay where the edit put them, and selections
    /// that an edit made identical or overlapping are merged.
    pub(crate) fn edit_selections(&mut self, transform: impl Fn(usize) -> usize, fallback: Option<Selection>) {
        let carry = |s: Selection| Selection { anchor: transform(s.anchor), head: transform(s.head) };
        let primary = self.selection.or(fallback).map(carry);
        let extra: Vec<_> = std::mem::take(&mut self.extra).into_iter().map(carry).collect();
        match primary {
            Some(primary) => self.set_selections(primary, extra),
            None => self.extra.clear(),
        }
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
    pub fn find(&self) -> Option<&crate::find::FindState> {
        self.find.as_ref()
    }
    pub fn top_line(&self) -> usize {
        self.top_line
    }
    pub fn column(&self) -> usize {
        self.column
    }
    /// Why the source cannot be read now, while its last content stays.
    pub fn unavailable(&self) -> Option<&str> {
        self.unavailable.as_deref()
    }

    /// Where the reader is in this tab: the selection (or the top line's start) with a context anchor.
    pub(crate) fn location(&self, source: Option<&TextSnapshot>) -> Location {
        let Some(doc) = source else {
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
        let start = doc.reader_line_range(self.top_line.min(doc.line_count() - 1)).start;
        let range = self.selection.map_or(start..start, |s| s.range());
        let line = doc.line_of(range.start);
        Location {
            source: self.source.clone(),
            version: doc.key().version,
            anchor: anchor(doc, line),
            offset: range.start - doc.reader_line_range(line).start,
            line,
            selected: self.selection.is_some(),
            range,
            top_line: self.top_line,
        }
    }

    /// Put the view where `at` was: exactly when the version is unchanged; else where its context line survives
    /// once, keeping the selection's place on that line; else near the old position, marked approximate.
    pub(crate) fn restore_to(&mut self, at: &Location, doc: &TextSnapshot) {
        self.place(at, doc);
        // A navigation into a collapsed region unfolds it.
        self.reveal_view(doc);
    }

    fn place(&mut self, at: &Location, doc: &TextSnapshot) {
        let last = doc.line_count() - 1;
        let selection = |start: usize| at.selected.then(|| {
            let start = doc.floor_char_boundary(start);
            Selection { anchor: start, head: doc.floor_char_boundary(start.saturating_add(at.range.len())) }
        });
        if at.version == doc.key().version {
            self.top_line = at.top_line.min(last);
            self.place_selection(selection(at.range.start));
            self.approximate = false;
            return;
        }
        // The anchor is a whole line (or its first ANCHOR_CHARS characters): it must match exactly and only once.
        let same = |l: usize| anchor_matches(doc, l, &at.anchor);
        let mut matches = (0..doc.line_count()).filter(|&l| same(l));
        let unique = match (at.anchor.is_empty(), matches.next(), matches.next()) {
            (false, Some(l), None) => Some(l),
            _ => None,
        };
        match unique {
            Some(line) => {
                let start = doc.reader_line_range(line).start + at.offset.min(doc.reader_line_range(line).len());
                // Keep the anchor line where it was relative to the top of the view.
                let shift = line as isize - at.line as isize;
                self.top_line = (at.top_line as isize + shift).clamp(0, last as isize) as usize;
                self.place_selection(selection(start));
                self.approximate = false;
            }
            None => {
                self.top_line = at.top_line.min(last);
                self.place_selection(None);
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

impl Location {
    pub(crate) fn edited(&mut self, previous: &TextSnapshot, next: &TextSnapshot, edits: &[crate::text::TextEdit]) {
        if self.version != previous.key().version { return; }
        let transform = |byte| crate::text::edited_byte(byte, edits).min(next.len_bytes());
        let top = previous.reader_line_range(self.top_line.min(previous.line_count() - 1)).start;
        self.range = transform(self.range.start)..transform(self.range.end);
        self.top_line = next.line_of(transform(top));
        self.line = next.line_of(self.range.start);
        self.offset = self.range.start - next.reader_line_range(self.line).start;
        self.anchor = anchor(next, self.line);
        self.version = next.key().version;
    }
}

/// The text a place keeps to find its line again: the line's first [`ANCHOR_CHARS`] characters. An environment
/// file's lines hold values (often secrets) and places are saved outside the session, so its places keep none and
/// stay on their line number instead.
pub(crate) fn anchor(doc: &TextSnapshot, line: usize) -> Box<str> {
    if private_lines(doc) { return Box::default(); }
    let range = doc.reader_line_range(line);
    // At most four UTF8 bytes per context character; never assemble a long source line.
    let end = doc.floor_char_boundary(range.start.saturating_add(ANCHOR_CHARS * 4).min(range.end));
    doc.range(range.start..end).unwrap_or_default().chars().take(ANCHOR_CHARS).collect()
}

pub(crate) fn anchor_matches(doc: &TextSnapshot, line: usize, anchor: &str) -> bool {
    // A place in an environment file has no text to compare: its line number is all it keeps.
    if private_lines(doc) { return anchor.is_empty(); }
    let range = doc.reader_line_range(line);
    let end = doc.floor_char_boundary(range.start.saturating_add(ANCHOR_CHARS * 4).min(range.end));
    doc.range(range.start..end).unwrap_or_default().chars().take(ANCHOR_CHARS).eq(anchor.chars())
}

/// Whether `doc`'s line text must not be kept by places: an environment file's.
pub(crate) fn private_lines(doc: &TextSnapshot) -> bool {
    doc.language() == Some(lenz_content::highlight::Language::Env)
}

/// Reading history: committed navigations, Back and Forward.
#[derive(Default)]
pub struct History {
    entries: VecDeque<Location>,
    at: usize,
}

impl History {
    pub(crate) fn edited(&mut self, paths: &std::collections::HashSet<PathBuf>, previous: &TextSnapshot, next: &TextSnapshot, edits: &[crate::text::TextEdit]) {
        for at in self.entries.iter_mut().filter(|at| paths.contains(&at.source)) { at.edited(previous, next, edits); }
    }

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

    /// An untitled document's steps now belong to its confirmed file.
    pub(crate) fn rename(&mut self, from: &std::path::Path, to: &std::path::Path) {
        for at in self.entries.iter_mut().filter(|at| at.source == from) { at.source = to.to_path_buf(); }
    }

    /// `from` (a file or a folder) was moved to `to`: Back and Forward reach everything inside it at its new path.
    pub(crate) fn rename_tree(&mut self, from: &std::path::Path, to: &std::path::Path) {
        for at in self.entries.iter_mut() {
            if let Ok(rest) = at.source.strip_prefix(from) { at.source = if rest.as_os_str().is_empty() { to.to_path_buf() } else { to.join(rest) }; }
        }
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
    fn pending_highlights_match_surviving_byte_identity_through_fragmented_replacements() {
        let text = "fn first() {}\nlet value = \"text\";\n// last\n";
        let doc = crate::document::Document::decode("colours.rs".into(), 1, text.as_bytes());
        let parsed = Arc::new(lenz_content::highlight::parse_while(lenz_content::highlight::Language::Rust, text.into(), || true).unwrap());
        let source = Arc::new(TextSnapshot::from_document(crate::text::BufferId(1), &doc));
        let mut held = Some(HeldHighlights { data: parsed, source, ranges: vec![(0..text.len(), 0)] });
        let mut oracle: Vec<_> = (0..text.len()).map(Some).collect();
        for round in 0..500 {
            let start = round * 17 % (oracle.len() + 1);
            let end = (start + round % 4).min(oracle.len());
            let mut edits = vec![crate::text::TextEdit { range: start..end, text: "x y\n".into() }];
            if end + 3 < oracle.len() { edits.push(crate::text::TextEdit { range: end + 1..end + 3, text: "z".into() }); }
            for edit in edits.iter().rev() { oracle.splice(edit.range.clone(), std::iter::repeat_n(None, edit.text.len())); }
            held = held.and_then(|held| held.edited(&edits));
            for (byte, expected) in oracle.iter().enumerate() {
                let mapped = held.as_ref().and_then(|held| held.ranges.iter().find(|(range, _)| range.contains(&byte)))
                    .map(|(range, original)| original + byte - range.start);
                assert_eq!(mapped, *expected, "byte identity after edit set {round}, byte {byte}");
            }
        }
    }

    #[test]
    fn pending_highlights_deletion_crossing_retained_intervals_keeps_only_survivors() {
        let text = "fn first() {}\n// tail\n";
        let doc = crate::document::Document::decode("colours.rs".into(), 1, text.as_bytes());
        let parsed = Arc::new(lenz_content::highlight::parse_while(lenz_content::highlight::Language::Rust, text.into(), || true).unwrap());
        let source = Arc::new(TextSnapshot::from_document(crate::text::BufferId(1), &doc));
        let held = HeldHighlights { data: parsed, source, ranges: vec![(0..text.len(), 0)] };
        let held = held.edited(&[crate::text::TextEdit { range: 3..3, text: "xx".into() }]).unwrap();
        let held = held.edited(&[crate::text::TextEdit { range: 1..7, text: "".into() }]).expect("the untouched prefix and suffix still have colours");
        assert_eq!(held.capture_at(0), Some(lenz_content::highlight::Capture::Keyword));
        assert_eq!(held.capture_at(1), Some(lenz_content::highlight::Capture::Function));
        assert_eq!(held.capture_at(10), Some(lenz_content::highlight::Capture::Comment));
    }

    #[test]
    #[ignore = "release pending-style lookup probe: run alone"]
    fn pending_highlight_lookup_costs() {
        let line = "fn sample() { let name = \"λ\"; }\n";
        let text = line.repeat(10_000);
        let doc = crate::document::Document::decode("colours.rs".into(), 1, text.as_bytes());
        let parsed = Arc::new(lenz_content::highlight::parse_while(lenz_content::highlight::Language::Rust, text.clone().into(), || true).unwrap());
        let source = Arc::new(TextSnapshot::from_document(crate::text::BufferId(1), &doc));
        let mut held = HeldHighlights { data: parsed.clone(), source, ranges: vec![(0..text.len(), 0)] };
        let started = std::time::Instant::now();
        for n in 0..512 {
            let at = n * line.len();
            held = held.edited(&[crate::text::TextEdit { range: at..at + 1, text: "x".into() }]).unwrap();
        }
        let edit_ms = started.elapsed().as_secs_f64() * 1000.0;
        let mut results = std::collections::BTreeMap::<&str, Vec<f64>>::new();
        for sample in 0..8 {
            for pending in [false, true] {
                let started = std::time::Instant::now();
                let mut captures = 0;
                for _ in 0..100 { for line_no in 5_000..5_060 { for byte in 0..line.len() - 1 {
                    let capture = if pending { held.capture_at(line_no * line.len() + byte) } else { parsed.capture_at(line_no, byte) };
                    captures += usize::from(std::hint::black_box(capture).is_some());
                } } }
                assert!(captures > 0);
                if sample > 0 { results.entry(if pending { "mapped" } else { "current" }).or_default().push(started.elapsed().as_secs_f64() * 10.0); }
            }
        }
        eprintln!("HIGHLIGHT_PERF {}", serde_json::json!({"viewport_ms":results,"edit_512_total_ms":edit_ms,"intervals":held.ranges.len(),"map_bytes":held.ranges.capacity()*size_of::<(Range<usize>,usize)>(),"retained_syntax_bytes":parsed.bytes(),"retained_snapshot_storage_bytes":held.source.storage_bytes(),"scope":"60 source rows, per-byte lexical lookup, 512 disjoint edits without parse acceptance; one warmup and seven samples; excludes adapters and display"}));
    }

    #[test]
    #[ignore = "release semantic colour lookup probe: run alone"]
    fn pending_semantic_colour_lookup_costs() {
        let line = "    System.out.println(\"Hello Lenz!\");\n";
        let text = line.repeat(10_000);
        let doc = crate::document::Document::decode("Main.java".into(), 1, text.as_bytes());
        let parsed = Arc::new(lenz_content::highlight::parse_while(lenz_content::highlight::Language::Java, text.clone().into(), || true).unwrap());
        let legend = crate::decor::Legend::new(&serde_json::json!({"tokenTypes":["property"]}));
        let data: Vec<usize> = (0..10_000).flat_map(|line| [usize::from(line > 0), 11, 3, 0, 0]).collect();
        let tokens = Arc::new(crate::decor::decode_tokens(&serde_json::to_vec(&serde_json::json!({"result":{"data":data}})).unwrap(), &doc, crate::semantics::lsp::Encoding::Utf16, &legend));
        assert_eq!(tokens.len(), 10_000);
        let source = Arc::new(TextSnapshot::from_document(crate::text::BufferId(1), &doc));
        let mut syntax = HeldHighlights { data: parsed.clone(), source: source.clone(), ranges: vec![(0..text.len(), 0)] };
        let mut semantic = HeldTokens { data: tokens.clone(), source, ranges: vec![(0..text.len(), 0)] };
        let started = std::time::Instant::now();
        for n in 0..512 {
            let at = n * line.len();
            let edits = [crate::text::TextEdit { range: at..at + 1, text: "x".into() }];
            syntax = syntax.edited(&edits).unwrap();
            semantic = semantic.edited(&edits).unwrap();
        }
        let edit_ms = started.elapsed().as_secs_f64() * 1000.0;
        let mut results = std::collections::BTreeMap::<&str, Vec<f64>>::new();
        for sample in 0..8 { for pending in [false, true] {
            let started = std::time::Instant::now();
            let mut properties = 0;
            for _ in 0..100 { for row in 5_000..5_060 { for byte in 0..line.len() - 1 {
                let capture = if pending {
                    semantic.capture_at(row * line.len() + byte).or_else(|| syntax.capture_at(row * line.len() + byte))
                } else {
                    let line_tokens = tokens.line(row);
                    let token = line_tokens.get(line_tokens.partition_point(|token| token.end as usize <= byte)).filter(|token| token.start as usize <= byte);
                    token.and_then(|token| token.capture).or_else(|| parsed.capture_at(row, byte))
                };
                properties += usize::from(std::hint::black_box(capture) == Some(lenz_content::highlight::Capture::Property));
            } } }
            assert_eq!(properties, 100 * 60 * 3);
            if sample > 0 { results.entry(if pending { "mapped" } else { "current" }).or_default().push(started.elapsed().as_secs_f64() * 10.0); }
        } }
        eprintln!("SEMANTIC_COLOUR_PERF {}", serde_json::json!({"viewport_ms":results,"edit_512_total_ms":edit_ms,"intervals_per_map":semantic.ranges.len(),"both_map_bytes":(syntax.ranges.capacity()+semantic.ranges.capacity())*size_of::<(Range<usize>,usize)>(),"retained_token_bytes":tokens.bytes(),"retained_syntax_bytes":parsed.bytes(),"retained_snapshot_storage_bytes":semantic.source.storage_bytes(),"scope":"60 source rows, per-byte combined lexical/semantic colour lookup, 512 disjoint edits; one warmup and seven samples; excludes adapters and display"}));
    }

    #[test]
    fn pending_highlights_coalesce_restored_ranges_and_release_fully_replaced_text() {
        let text = "// retained lexical colours\n".repeat(100);
        let doc = crate::document::Document::decode("colours.rs".into(), 1, text.as_bytes());
        let parsed = Arc::new(lenz_content::highlight::parse_while(lenz_content::highlight::Language::Rust, text.clone().into(), || true).unwrap());
        let arena = Arc::downgrade(&parsed);
        let source = Arc::new(TextSnapshot::from_document(crate::text::BufferId(1), &doc));
        let mut held = HeldHighlights { data: parsed, source, ranges: vec![(0..text.len(), 0)] };
        for at in 0..text.len() {
            held = held.edited(&[crate::text::TextEdit { range: at..at, text: "x".into() }]).unwrap();
            assert_eq!(held.capture_at(at), None, "inserted bytes have no original colour");
            held = held.edited(&[crate::text::TextEdit { range: at..at + 1, text: "".into() }]).unwrap();
            assert_eq!(held.capture_at(0), Some(lenz_content::highlight::Capture::Comment));
        }
        assert_eq!(held.ranges.len(), 1, "restored contiguous bytes do not accumulate an edit history");
        assert!(held.edited(&[crate::text::TextEdit { range: 0..text.len(), text: "new".into() }]).is_none());
        assert!(arena.upgrade().is_none(), "no surviving byte can use the old arena");
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
        let doc = |text: &str, version| TextSnapshot::from_document(crate::text::BufferId(1), &crate::document::Document::decode(PathBuf::from("a.rs"), version, text.as_bytes()));
        let mut tab = Tab::new(TabId(1), PathBuf::from("a.rs"), false);
        let current = doc("one\ntwo\nfn target() {}\nend\n", 1);
        tab.selection = Some(Selection { anchor: 8, head: 10 });
        tab.top_line = 2;
        let here = tab.location(Some(&current));
        assert_eq!((here.version, here.range.clone(), &*here.anchor, here.offset), (1, 8..10, "fn target() {}", 0));
        // Same version: exact.
        tab.selection = None;
        tab.restore_to(&here, &current);
        assert_eq!((tab.top_line, tab.selection.map(|s| s.range()), tab.approximate), (2, Some(8..10), false));
        // Lines inserted above: the unique context line is found and the selection keeps its place on it.
        let current = doc("new\nnew\none\ntwo\nfn target() {}\nend\n", 2);
        tab.restore_to(&here, &current);
        assert_eq!((tab.top_line, tab.selection.map(|s| s.range()), tab.approximate), (4, Some(16..18), false));
        // The context line now appears twice: ambiguous, so near the old position and marked approximate.
        let current = doc("fn target() {}\none\ntwo\nfn target() {}\n", 3);
        tab.restore_to(&here, &current);
        assert_eq!((tab.top_line, tab.selection, tab.approximate), (2, None, true));
        // Gone: clamped, approximate.
        let current = doc("x\n", 4);
        tab.restore_to(&here, &current);
        assert_eq!((tab.top_line, tab.selection, tab.approximate), (1, None, true));
        // No selection recorded: none restored.
        let mut plain = Tab::new(TabId(2), PathBuf::from("a.rs"), false);
        let current = doc("a\nb\nc\n", 5);
        plain.top_line = 2;
        let at = plain.location(Some(&current));
        plain.restore_to(&at, &current);
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
    fn a_moved_folder_moves_every_history_entry_inside_it() {
        let at = |p: &str| Location { source: PathBuf::from(p), version: 1, range: 0..0, selected: false, top_line: 0, anchor: "".into(), line: 0, offset: 0 };
        let mut h = History::default();
        h.visit(Some(at("/p/dir/a")), at("/p/dir/sub/b"));
        h.visit(None, at("/p/dir2/c"));
        h.rename_tree(std::path::Path::new("/p/dir"), std::path::Path::new("/p/dest/dir"));
        let sources: Vec<_> = h.entries.iter().map(|at| at.source.clone()).collect();
        assert_eq!(sources, [PathBuf::from("/p/dest/dir/a"), PathBuf::from("/p/dest/dir/sub/b"), PathBuf::from("/p/dir2/c")]);
    }

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
