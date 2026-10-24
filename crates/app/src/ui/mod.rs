//! Thin gpui adapter: paints what the view model says and forwards input to it.
mod context;
mod checkout;
mod horizontal;
mod panes;
mod pane_search;
mod editing;
mod menus;
mod message_panel;
mod palette_view;
mod toolbar;
use panes::PaneSizes;
mod markdown;
mod pr_details;
mod folder_summary;
mod present;
#[cfg(all(test,feature="gui-test"))]
mod parity_tests;

use std::{
    collections::{HashMap, HashSet},
    ops::Range,
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};

use gpui::{
    App, Bounds, ClipboardItem, ContentMask, Context, FocusHandle, Font, Hsla, KeyDownEvent, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, NavigationDirection, PathBuilder, Pixels, Render, ShapedLine,
    ScrollDelta, ScrollWheelEvent, SharedString, TextAlign, TextRun, Window, canvas, div, font,
    point, prelude::*, px, quad, rgb, rgba, size,
};

use crate::commands::{self, Command};
#[cfg(test)]
use crate::highlight::Language;

use lenz_engine::{HighlightLevel, LineEnding, Options, WhitespaceMode};

use crate::{
    blame,
    changeset::{ChangeSet, Content as Source, Spec, Status},
    pr,
    git,
    navigator::{self, NavRow},
    open,
    find::{self, Ended},
    write::WriteError,
    selection::{Pos, Selection},
    highlight::{self, Capture, Parsed},
    theme::{self},
    view::{Connector, Content, Kind, Row, ScrollMode, Side, View},
};

pub mod conflict_list;
mod image_view;
mod about;
pub(crate) mod themes;
pub mod merge_view;
pub mod version_picker;
mod picker;
pub mod pr_view;
pub mod repo_chooser;

use image_view::{ImageDoc, ImageView};
use picker::{Pane, Picker};

pub use crate::document::Placeholder;
#[cfg(test)]
use crate::{moves,write::{self,Snapshot}};
#[cfg(test)]
use lenz_engine::Budget;
use crate::document::{TextDocument, LONG_LINE};
use lenz_gpui::{clip, fill};

/// Text is shared with the terminal adapter; image renderer resources stay in GPUI.
pub struct Doc {
    text: TextDocument,
    image: Option<Arc<ImageDoc>>,
}

impl std::ops::Deref for Doc {
    type Target = TextDocument;
    fn deref(&self) -> &TextDocument { &self.text }
}
impl std::ops::DerefMut for Doc {
    fn deref_mut(&mut self) -> &mut TextDocument { &mut self.text }
}

/// How long a diff may take before Lenz shows the text alone.
const DIFF_PATIENCE: std::time::Duration = std::time::Duration::from_secs(2);

fn megabytes(bytes: u64) -> String { format!("{:.1} MB", bytes as f64 / (1 << 20) as f64) }
fn kilobytes(bytes: u64) -> String {
    if bytes < 1024 { format!("{bytes} bytes") } else { format!("{:.1} KB", bytes as f64 / 1024.0) }
}

impl Doc {
    pub fn new(left: &str, right: &str, lp: &Path, rp: &Path) -> Self {
        Self { text: TextDocument::new(left, right, lp, rp), image: None }
    }

    #[cfg(test)]
    fn edited(&self, side: Side, text: &str, snapshot: Snapshot, lp: &Path, rp: &Path) -> Self {
        Self { text: self.text.edited(side, text, snapshot, lp, rp), image: None }
    }

    pub fn load(set: &ChangeSet, index: usize, lp: &Path, rp: &Path, force: bool) -> Result<Self, String> {
        let prepared=crate::document::PreparedDocument::load(set,index,lp,rp,force)?;
        Ok(Self{text:prepared.text,image:prepared.image.and_then(Result::ok).map(|core|Arc::new(ImageDoc::from_core(core)))})
    }
}

/// Today's metrics at a 13 px font; everything that holds text scales with the configured size.
fn scaled(font_size: f32, at_13: f32) -> f32 { at_13 * font_size / 13.0 }
fn scale_of(at_13: f32) -> f32 { scaled(crate::settings::get().gui.font_size, at_13) }
fn line_h() -> f32 { scale_of(20.0) }
fn font_size() -> f32 { crate::settings::get().gui.font_size }
const GUTTER_W: f32 = 72.0;
fn num_w() -> f32 { scale_of(56.0) }

fn hdr_h() -> f32 { scale_of(24.0) }
/// The bottom bar with the panel switches.
fn footer_h() -> f32 { hdr_h() }
fn toolbar_h() -> f32 { scale_of(30.0) }
/// Window y where the panes' rows start.
/// The commit log's height in rows, when the Change Set Picker shows.
const LOG_ROWS: f32 = 14.0;
const SIDEBAR_W: f32 = 300.0;
const MENU_W: f32 = 320.0;
/// Blame Annotation column width.
const BLAME_W: f32 = 230.0;
const POPUP_W: f32 = 760.0;
/// Change Set search results panel height, in rows.
const PANEL_ROWS: f32 = 10.0;
const TOAST_FOR: std::time::Duration = std::time::Duration::from_millis(1600);

fn status_color(palette: &theme::Palette, status: &Status) -> u32 {
    let c = &palette.status;
    match status {
        Status::Added | Status::Untracked => c[0],
        Status::Deleted => c[1],
        Status::Modified | Status::ModeChanged { .. } => c[2],
        Status::Renamed { .. } => c[3],
        Status::Conflicted => c[4],
    }
}

fn syntax_color(palette: &theme::Palette, capture: Capture) -> u32 { palette.syntax_color(capture) }


fn kind_index(kind: Kind) -> usize {
    match kind {
        Kind::Inserted => 0,
        Kind::Deleted => 1,
        Kind::Modified => 2,
        Kind::Conflict => 3,
    }
}

fn tint(palette: &theme::Palette, kind: Kind) -> u32 {
    palette.block[kind_index(kind)]
}

/// Connector edges and empty-Side markers: brighter than the block tint.
fn edge(palette: &theme::Palette, kind: Kind) -> u32 {
    palette.edge[kind_index(kind)]
}

/// Ignored Changes: the colour mixed halfway towards the background.
fn faded(palette: &theme::Palette, color: u32) -> u32 {
    let mix = |shift: u32| ((((color >> shift) & 0xFF) + ((palette.bg >> shift) & 0xFF)) / 2) << shift;
    mix(16) | mix(8) | mix(0)
}

fn shade(palette: &theme::Palette, kind: Kind, ignored: bool, pick: fn(&theme::Palette, Kind) -> u32) -> u32 {
    if ignored { faded(palette, pick(palette, kind)) } else { pick(palette, kind) }
}

/// Non-default modes and file-level notices, shown in the corner.
fn status_text(options: &Options, mode: ScrollMode, line_endings: Option<(LineEnding, LineEnding)>) -> String {
    let mut parts = Vec::new();
    match options.whitespace {
        WhitespaceMode::Default => {}
        WhitespaceMode::Trim => parts.push("whitespace: trim".to_string()),
        WhitespaceMode::Ignore => parts.push("whitespace: ignore".to_string()),
        WhitespaceMode::IgnoreTrimChunks => parts.push("whitespace: ignore + trim chunks".to_string()),
    }
    if options.ignore_case {
        parts.push("ignoring case".to_string());
    }
    match options.highlight {
        HighlightLevel::Word => {}
        HighlightLevel::Char => parts.push("highlight: char".to_string()),
        HighlightLevel::Line => parts.push("highlight: line".to_string()),
    }
    match mode {
        ScrollMode::Shared => {}
        ScrollMode::Aligned => parts.push("scroll: aligned".to_string()),
        ScrollMode::Padded => parts.push("scroll: padded".to_string()),
        ScrollMode::Independent => parts.push("scroll: independent".to_string()),
    }
    if let Some((l, r)) = line_endings {
        parts.push(format!("line endings differ: {l:?} vs {r:?}"));
    }
    parts.join(" · ")
}

/// The GUI's monospace font from the settings file and the installed fonts, and a notice when the configured
/// family is missing.
pub fn pick_mono(cx: &App) -> (Font, Option<String>) {
    lenz_gpui::pick_mono(cx, crate::settings::get().gui.font_family.as_deref())
}

/// The shared pane geometry for `font` at the configured size.
fn metrics(font: &Font) -> lenz_gpui::Metrics<'_> {
    lenz_gpui::Metrics { font, font_size: font_size(), line_h: line_h(), num_w: num_w() }
}

/// A loaded Comparison. Revisiting it updates recency; cold entries can be loaded again.
struct Loaded {
    doc: Arc<Doc>,
    columns: [Option<usize>; 2],
    /// Compact highlights per Side, filled in by a background parse after first paint.
    parsed: [Option<Arc<Parsed>>; 2],
    /// Declarations per Side, from the same parse, while sticky scroll is on.
    declarations: [Option<Arc<crate::highlight::Declarations>>; 2],
    parsing: bool,
    parse_id: Arc<()>,
    used: u64,
}

impl Loaded {
    fn new(doc: Arc<Doc>, used: u64) -> Self {
        Self { doc, columns: [None; 2], parsed: [None, None], declarations: [None, None], parsing: false, parse_id: Arc::new(()), used }
    }

    fn release_syntax(&mut self) {
        self.parsed = [None, None];
        self.declarations = [None, None];
        self.parsing = false;
        self.parse_id = Arc::new(());
    }

    /// Source strings, compact syntax and a conservative allowance for per-line diff/display data.
    fn bytes(&self) -> usize {
        let d = &self.doc;
        2 * (d.text_left.len() + d.text_right.len()) + 96 * (d.raw_left.len() + d.raw_right.len())
            + self.parsed.iter().flatten().map(|p| p.bytes()).sum::<usize>()
            + self.declarations.iter().flatten().flat_map(|d| &d.items).map(|d| d.bytes()).sum::<usize>()
            // both Sides decoded and as textures, and the Difference texture once shown: five 4-byte buffers
            + d.image.as_ref().map_or(0, |i| i.size.0 as usize * i.size.1 as usize * 20)
    }
}

/// Bound reloadable data, keeping the current file and its undo history regardless of size.
fn trim_loaded(loaded: &mut HashMap<usize, Loaded>, cache: &mut DiffCache, current: usize, limit: usize) {
    let mut bytes: usize = loaded.values().map(Loaded::bytes).sum();
    while bytes > limit || loaded.len() > 8 {
        let Some(index) = loaded.iter().filter(|(i, _)| **i != current).min_by_key(|(_, l)| l.used).map(|(&i, _)| i) else { break };
        bytes -= loaded.remove(&index).unwrap().bytes();
        cache.0.remove(&index);
    }
}

const LOADED_CACHE_BYTES: usize = 128 << 20;

/// How a Comparison is labelled in the header and title.
struct Labels {
    left: String,
    right: String,
    status: &'static str,
}

pub struct Viewer {
    panes: PaneSizes,
    palette: Arc<theme::Palette>,
    theme_chooser: Option<themes::ThemeChooser>,
    /// The highlighted theme, shown live while the chooser is open; Esc drops it, Enter selects it.
    theme_preview: Option<Arc<theme::Palette>>,
    comparison: crate::comparison_session::ComparisonSession,
    markdown_poll_task: Option<gpui::Task<()>>,
    /// The Folders revision `nav` was built from.
    nav_revision: u64,
    /// The Folder Summary's first row, and whether the wheel moved it away from the keyboard row.
    summary_scroll: f32,
    summary_free: bool,
    /// Polls the summary's counting while it runs.
    folders_task: Option<gpui::Task<()>>,
    markdown: markdown::Adapter,
    /// Local images of the Markdown Preview, kept across layouts of one document.
    markdown_images: markdown::ImageCache,
    #[cfg(all(test, feature="gui-test"))]
    inspected_position: Option<(usize, Side, Option<usize>, Option<usize>)>,
    set: Arc<ChangeSet>,
    /// `--name` (difftool's `$MERGED`) overrides the path shown and used for language detection.
    name: Option<String>,
    /// Bumped whenever the shown Comparison or its options change; stale background results are dropped.
    /// Dropping this skips queued work for an obsolete generation.
    loaded: HashMap<usize, Loaded>,
    font: Font,
    focus: FocusHandle,
    started: Instant,
    timings: bool,
    first_paint_reported: std::rc::Rc<std::cell::Cell<bool>>,
    nav: Vec<NavRow>,
    /// Files visibility and Focus, shared with the core.
    layout: crate::layout::Layout,
    /// Adapter keyboard focus; opening a file does not transfer it to the diff.
    nav_focused: bool,
    panel_focused: bool,
    diff_focus: Side,
    /// File Navigator scroll position, in rows.
    nav_scroll: f32,
    /// Rows the Focus commit message panel is scrolled by.
    message_scroll: f32,
    /// The commit message panel has the keyboard; its selection (line, character) and the press a drag extends.
    message_focused: bool,
    message_sel: Option<((usize, usize), (usize, usize))>,
    message_press: Option<(usize, usize)>,
    file_search: crate::pane_search::PaneSearch,
    file_search_scroll: Option<f32>,
    log_search_scroll: Option<f32>,
    /// File Navigator row under the pointer (for the rename tooltip).
    nav_hover: Option<usize>,
    switcher: Option<Switcher>,
    /// Collapsed Regions on/off for every Comparison (on by default).
    /// A brief message (mode changes, "press again for next file") and when it appeared.
    toast: Option<(String, Instant)>,
    /// The window title last set, so render only touches it on change.
    shown_title: String,
    /// Caret and selection; its Side is the focused one.
    sel: Option<Selection>,
    dragging: bool,
    menu: Option<Menu>,
    /// The last log frame painted, held briefly while a restarted log has no rows.
    held_log: Option<picker::LogFrame>,
    /// A change set whose first file is being read before it replaces the shown one, and its token.
    incoming: Option<(Arc<()>, Arc<ChangeSet>, Spec)>,
    /// The command palette over the shared catalog.
    command_palette: Option<crate::palette::Palette>,
    /// The Diff options popover under the toolbar's ⚙.
    diff_options: Option<toolbar::DiffOptions>,
    /// The `ctrl-g` go-to-line prompt.
    goto: Option<String>,
    /// New Branch… or Checkout Tag or Revision…, drawn in the Go to line box.
    git_prompt: Option<checkout::GitPrompt>,
    search: Option<SetSearch>,
    /// 'Unable to calculate diff' when the diff gave up.
    /// The generation whose diff is shown.
    /// Refinement budget of the shown Comparison and options.
    /// Each Comparison's Folded Change Blocks, by their text, so folds outlive re-diffs.
    /// Drag selection: pointer x, rows and pixels to auto-scroll per tick (0 inside the pane).
    drag: (f32, f32, f32),
    drag_ticking: bool,
    /// Horizontal scroll of both Sides' text, in pixels.
    hscroll: [f32; 2],
    /// Width of one character of the monospace font.
    char_w: f32,
    /// Width of a pane's text area, as last painted.
    text_w: [f32; 2],
    spec: Spec,
    blame_on: bool,
    /// Sticky scroll: pin the enclosing declarations at the top of each pane (for this run; the settings file sets
    /// the start).
    sticky_on: bool,
    blames: HashMap<BlameKey, Blame>,
    popup: Option<CommitPopup>,
    /// Blame Annotation under the pointer: Side, line, pointer position.
    help: bool,
    help_scroll: usize,
    about:Option<about::About>,
    report_request: crate::diagnostics::ReportRequest,
    /// The Change Set Picker, for a Change Set with a repository behind it (None for plain files and difftool).
    repo: Option<crate::repository_session::RepositorySession>,
    /// The restored position, applied once the repository settles; then the place being recorded.
    pending_place: Option<crate::place::Restore>,
    place_recorder: Option<crate::place::Recorder>,
    /// A loop polls `repo` while it has background work.
    repo_polling: bool,
    /// Shown instead of the panes when the picked Change Set has no files.
    nothing: Option<String>,
    /// Zoom, pan and mode of an image Comparison.
    iview: ImageView,
    /// Render textures belonging to the current image, released on switch or replacement.
    image_uploads: lenz_gpui::image::Uploads,
    /// The pull request under review.
    pr_dialog:Option<pr_details::Dialog>,
    /// The Merge View, shown instead of the 2-way panes.
    merge: Option<merge_view::MergeState>,
    /// The Version Picker, shown over the 2-way view.
    picker_v: Option<version_picker::PickerState>,
    /// Moves made this session, per Comparison, for undo and redo.
    edits: crate::edit_session::EditSession,
    file_actions: crate::file_actions::FileActions,
    applied_scroll: f32,
    /// Files Lenz wrote this session (as their Sides name them), marked in the File Navigator.
    edited: HashSet<PathBuf>,
    /// Why the last move or undo wasn't written; shown like the banner until the next one or another file.
    write_error: Option<String>,
    /// The Conflict List, while an operation is in progress in the picker's repository.
    conflicts: Option<conflict_list::ConflictList>,
}

/// The dialog that owns input now, topmost first. Input goes to it before toolbars, menus and documents.
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub(super) enum Modal { Question, Discard, Checkout, GitPrompt, About, Theme, Help, Palette, DiffOptions, Menu, Popup, GoTo, Switcher, Find, SearchEditing, MergeFind, ConflictConfirm }

impl Viewer {
    pub(super) fn modal(&self)->Option<Modal>{
        Some(if self.merge.as_ref().is_some_and(|s|s.ask.is_some())||self.picker_v.as_ref().is_some_and(|s|s.ask.is_some()){Modal::Question}
        else if self.discard_modal(){Modal::Discard}
        else if self.checkout_modal(){Modal::Checkout}
        else if self.about.is_some(){Modal::About}
        else if self.theme_chooser.is_some(){Modal::Theme}
        else if self.help{Modal::Help}
        else if self.command_palette.is_some(){Modal::Palette}
        else if self.diff_options.is_some(){Modal::DiffOptions}
        else if self.menu.is_some(){Modal::Menu}
        else if self.popup.is_some(){Modal::Popup}
        else if self.git_prompt.is_some(){Modal::GitPrompt}
        else if self.goto.is_some(){Modal::GoTo}
        else if self.switcher.is_some(){Modal::Switcher}
        else if self.comparison.find.is_some(){Modal::Find}
        else if self.search.as_ref().is_some_and(|s|s.editing){Modal::SearchEditing}
        else if self.merge.as_ref().is_some_and(|s|s.find_editing()){Modal::MergeFind}
        else if self.conflicts.as_ref().is_some_and(|s|s.confirming()){Modal::ConflictConfirm}
        else {return None})
    }
}

impl std::ops::Deref for Viewer {
    type Target = crate::comparison_session::ComparisonSession;
    fn deref(&self) -> &Self::Target { &self.comparison }
}
impl std::ops::DerefMut for Viewer {
    fn deref_mut(&mut self) -> &mut Self::Target { &mut self.comparison }
}

/// `ctrl-f`: find in one Side of the shown Comparison.
use crate::find::Find as FindBar;
use lenz_content::find::FindOption;

/// The find option toggles as they read in every find input.
const TOGGLES: &str = "Aa  W  .*";
/// Each toggle's option, start and length (in chars) within [`TOGGLES`].
fn toggle_spans() -> [(FindOption, usize, usize); 3] {
    [(FindOption::Case, 0, 2), (FindOption::Word, 4, 1), (FindOption::Regex, 7, 2)]
}

/// `ctrl-shift-f`: shared Change Set search; adapters own pixel layout.
use crate::search_session::{ChangeSetSearch as SetSearch, PanelRow};

/// Right-click menu.
struct Menu {
    x: f32,
    y: f32,
    items: Vec<(String, MenuAction)>,
    hover: Option<usize>,
}

impl Viewer {
    /// Open a menu; one with nothing in it is not shown at all.
    fn show_menu(&mut self, menu: Menu) {
        self.menu = (!menu.items.is_empty()).then_some(menu);
    }
}

enum MenuAction {
    /// A catalog action, as the ☰ menu lists them.
    Run(crate::catalog::Action),
    PrLine(Side),
    File(crate::file_actions::FileAction,usize,Arc<ChangeSet>),
    RepositoryRefresh,
    Command(Command),
    InteractionCommand(commands::Interaction),
    TextCopy(commands::Interaction,Side,Option<Selection>,u64),
    ToggleBlame,
    Fold(usize),
    /// Copy a Change Block onto this Side.
    Move(usize, Side),
    /// A Merge View action on one merge block.
    Merge(merge_view::Control),
    /// Comparison, Side, 1-based line, and whether to use the system default app.
    Open(usize, Side, Option<usize>, bool),
    /// The picker's: walk these branches, compare a branch with the current one, compare two log rows,
    /// pick a branch to compare a commit with, compare two revisions, open a URL, clear the filter at a commit,
    /// open a worktree in a new window, show a file's history.
    Walk(crate::history::Branches),
    CompareWithBranch { row: u32, id: String },
    /// A sidebar row's entry, as the core lists them.
    Source(crate::repository_session::SourceEntry),
    Browse(String),
    /// A repository command on a log row, for the commit `id` the menu was opened on.
    RepositoryRow { row: u32, id: String, command: crate::repository_session::RepositoryCommand },
    /// Start `lenz` with these arguments in the repository (a commit or file in a new window).
    Spawn(Vec<String>),
    /// The Conflict List's: stage a Conflicted File, or put a marked one's conflict back.
    MarkResolved(String),
    Reopen(String),
    /// Put this text on the clipboard.
    Clipboard(String),
}

/// Toolbar buttons.
#[derive(Clone, Copy, Debug)]
enum Tool {
    /// The Diff options popover.
    DiffOptions,
    /// The ☰ menu: settings, theme, keys, about.
    AppMenu,
    /// Back and Forward over the places the view has been.
    Back,
    Forward,
    /// A Merge View toolbar action.
    Merge(crate::merge_session::Act),
    /// The Version Picker's Save, Done, Cancel, Mark resolved.
    Pick(version_picker::PickAct),
    /// The keyboard shortcuts overlay.
    Help,
    PrevChange,
    NextChange,
    PrevFile,
    NextFile,
    #[cfg(all(test,feature="gui-test"))]
    About,
    #[cfg(all(test,feature="gui-test"))]
    Theme,
    /// Show or hide a panel, or enter or leave Focus Mode.
    Layout(crate::layout::LayoutCommand),
    /// A catalog action: a pull request's tab.
    Run(crate::catalog::Action),
}

/// Every binding, for the `?` overlay (the spec's keymap; macOS uses cmd for ctrl).
const KEYMAP: &[(&str, &str)] = &[
    ("n / p,  F7 / shift-F7,  alt-↓ / alt-↑", "Next / previous Change Block (press again at the edge for the next file)"),
    ("ctrl-] / ctrl-[  (] / [ in the diff)", "Next / previous file"),
    ("alt-← / alt-→,  mouse back / forward,  ◀ ▶", "Back / Forward: the previous commit, branch, filter or file history, with its file and line"),
    ("ctrl-p", "Go to changed file (name:line opens at a line)"),
    ("ctrl-g  (macOS cmd-l)", "Go to line[:column]"),
    ("ctrl-f;  F3 / Enter,  shift-F3", "Find in focused Files, Commits or text Side; next / previous match"),
    ("ctrl-shift-p,  F10,  : in the diff", "Command palette: type to filter, arrows and Enter run, Esc closes"),
    ("ctrl-shift-f  (alt-o: changed lines only)", "Find across the Change Set"),
    ("alt-c / alt-w / alt-r", "Find options: match case / whole word / regular expression"),
    ("arrows / shift-arrows", "Move the caret / extend the selection"),
    ("ctrl-a / ctrl-c", "Select all / copy"),
    ("Home / End;  ctrl-Home / ctrl-End", "Line start / end; file start / end"),
    ("PgUp / PgDn,  space", "Page up / down"),
    ("z", "Collapsed Regions on / off"),
    ("x / shift-x", "Fold or unfold the current Change Block / unfold all"),
    ("alt-shift-→ / alt-shift-←,  ≫ / ≪ in the gutter", "Move the change onto the right / left Side in a draft"),
    ("ctrl-z / ctrl-shift-z", "Undo / redo a draft change in this file"),
    ("ctrl-s; toolbar / context menu", "Save changes; discard unsaved changes or revert a working-tree hunk"),
    ("o / shift-o", "Open the file in your editor at the line / with the default app"),
    ("w / i / h / s", "Whitespace Mode / Ignore Case / Highlight Level / Scroll Mode"),
    ("alt-b", "Blame Annotation"),
    ("palette: View: Sticky Scroll;  click a pinned line", "Pin the enclosing declarations at the top of each pane / jump to one"),
    ("alt-1 / alt-2 / alt-3", "Files / Branches / History: show or hide (leaves Focus)"),
    ("f,  Enter (in the log without a search) / Esc", "Focus: the diff alone (Branches, History and Files hidden) / restore the panels"),
    ("type in Files or Commits; ↑ ↓ / Enter / Esc", "Find candidates / open candidate / cancel search"),
    ("↑ ↓, shift, ctrl-↑ ↓ (in the log)", "Pick a commit, a range, or a second commit to compare"),
    ("Tab / shift-Tab", "Move between sidebar, log and diff"),
    ("shift-wheel", "Scroll sideways"),
    ("Esc", "Close overlay"),
    ("q,  ctrl-w,  ctrl-q", "Quit"),
    ("ctrl-/  (in the log)", "Filter commits: text, author:, path:, after:, before: — Enter searches messages"),
    ("ctrl-b / ctrl-g / ctrl-m / ctrl-t  (in the log)", "Branches menu / write the commit-graph / merge: vs parent 2 / branch compare: tip ⟷ tip"),
    ("m, 0, 1, + -, , .  (images)", "Image mode / fit / 1:1 / zoom / Swipe split or Onion-skin opacity"),
    ("n / p  (images)", "Next / previous Changed Region"),
    ("v / d / r / T  (pull requests)", "Viewed and next file / description / refresh / outdated threads"),
    ("alt-shift-→ / ← / ↓ / backspace  (merge)", "Take Ours / Theirs / both / keep the Base for the current change"),
    ("shift-n / shift-p  (merge)", "Next / previous unresolved Conflict"),
    ("b / w / s  (merge)", "Base in the middle / Whitespace Mode / Shared or Padded scrolling"),
    ("ctrl-z / ctrl-shift-z;  ctrl-s;  ctrl-enter  (merge)", "Undo / redo; save the Result; Done"),
    ("?", "This help"),
];

// Help uses the same wrapped rows for painting and bounded keyboard/wheel movement.
fn help_layout(viewport:gpui::Size<Pixels>)->(Rect,usize){
    let vw=f32::from(viewport.width).max(0.0);let vh=f32::from(viewport.height).max(0.0);
    let w=(vw-16.0).clamp(0.0,900.0);let h=(vh-16.0).clamp(0.0,760.0);
    (((vw-w)/2.0,(vh-h)/2.0,w,h),((h-88.0).max(0.0)/line_h())as usize)
}
fn help_close_bounds(viewport:gpui::Size<Pixels>)->Bounds<Pixels>{
    let ((x,y,w,h),_)=help_layout(viewport);
    Bounds{origin:point(px(x+8.0),px((y+h-30.0).max(y))),size:size(px((w-16.0).max(0.0)),px(24.0_f32.min(h)))}
}
fn help_rows(viewport:gpui::Size<Pixels>,char_w:f32)->Vec<(String,bool)>{
    let ((_,_,w,_),_)=help_layout(viewport);
    let cols=((w-24.0).max(0.0)/char_w.max(1.0)).floor().max(1.0)as usize;
    KEYMAP.iter().flat_map(|(keys,action)|{
        pr_details::wrap(keys,cols).into_iter().map(|row|(row,true))
            .chain(pr_details::wrap(action,cols).into_iter().map(|row|(row,false)))
            .chain(std::iter::once((String::new(),false)))
    }).collect()
}

/// A toolbar button's tooltip.
struct Tip(SharedString, Arc<theme::Palette>);

impl Render for Tip {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_2()
            .py_1()
            .bg(rgb(self.1.status_bg))
            .border_1()
            .border_color(rgb(self.1.border))
            .text_color(rgb(self.1.fg))
            .text_size(px(12.0))
            .child(self.0.clone())
    }
}

/// The shared core owns streamed annotations and their loading state.
type Blame = crate::annotation_session::Annotations;

type BlameKey = (String, blame::Version);

/// The commit popup opened from a Blame Annotation.
struct CommitPopup {
    sha: String,
    original_line: usize,
    original_path: Arc<str>,
    inspection: crate::annotation_session::CommitInspection,
    scroll: usize,
}

impl CommitPopup {
    /// Text rows, then the button labels with their keys.
    fn rows(&self, palette: &theme::Palette) -> Vec<(String, u32)> {
        let mut rows = vec![(format!("commit {}", self.sha), palette.accent)];
        match (self.inspection.details(), self.inspection.error()) {
            (None, None) => rows.push(("loading…".into(), palette.dim)),
            (None, Some(e)) => rows.push((e.to_string(), palette.error)),
            (Some(d), _) => {
                rows.push((format!("Author: {} <{}>", d.author, d.email), palette.fg));
                rows.push((format!("Date:   {}", blame::date(d.author_time)), palette.fg));
                rows.push((String::new(), palette.fg));
                let message: Vec<&str> = d.message.lines().collect();
                rows.extend(message.iter().take(8).map(|l| (format!("    {l}"), palette.fg)));
                if message.len() > 8 {
                    rows.push(("    …".into(), palette.dim));
                }
                rows.push((String::new(), palette.fg));
                rows.extend(d.files.iter().take(10).map(|(k, f)| (format!("{k} {f}"), palette.dim)));
                if d.files.len() > 10 {
                    rows.push((format!("… {} more files", d.files.len() - 10), palette.dim));
                }
            }
        }
        rows
    }
}

const POPUP_BUTTONS: [(&str, &str); 3] =
    [("s", "Show this commit's changes"), ("c", "Copy hash"), ("a", "Annotate previous revision")];

/// x, y, width, height.
type Rect = (f32, f32, f32, f32);

/// Popup box and each button's (x, width) on its last row, for a window `w` wide.
fn popup_layout(rows:usize,viewport:[f32;2],char_w:f32)->(Rect,Vec<Rect>,usize) {
    let [vw,vh]=viewport;let bw=POPUP_W.min((vw-16.).max(0.));
    let widths:Vec<f32>=POPUP_BUTTONS.iter().map(|(key,label)|(label.len()+key.len()+3) as f32*char_w+16.).collect();
    let stack=widths.iter().sum::<f32>()+20.*widths.len() as f32>bw;
    let button_rows=if stack{POPUP_BUTTONS.len()}else{1};
    let visible=(((vh-16.-24.-button_rows as f32*line_h()).max(0.)/line_h()).floor() as usize).min(rows);
    let height=visible as f32*line_h()+24.+button_rows as f32*line_h();
    let bx=(vw-bw)/2.;let by=(toolbar_h()+hdr_h()+40.).min((vh-height-8.).max(0.));
    let mut x=bx+8.;let row_y=by+16.+visible as f32*line_h();
    let buttons=widths.into_iter().enumerate().map(|(i,width)| {
        if stack {(bx+8.,row_y+i as f32*line_h(),(bw-16.).max(0.),line_h())}
        else {let at=(x,row_y,width,line_h());x+=width+10.;at}
    }).collect();
    ((bx,by,bw,height),buttons,visible)
}

impl Menu {
    fn start(&self,rows:usize)->usize{self.hover.unwrap_or(0).saturating_sub(rows.saturating_sub(1))}
    fn width(&self, char_w: f32) -> f32 {
        if !self.items.is_empty() {
            let chars = self.items.iter().map(|(label, _)| label.chars().count()).max().unwrap_or(0);
            MENU_W.max(chars as f32 * char_w + 24.0)
        } else { MENU_W }
    }

    fn layout(&self,viewport:[f32;2],char_w:f32)->(f32,f32,f32,usize) {
        let width=self.width(char_w).min((viewport[0]-8.).max(0.));
        let rows=(((viewport[1]-10.).max(0.)/line_h()).floor() as usize).min(16).min(self.items.len());
        let height=rows as f32*line_h()+8.;
        (self.x.clamp(4.,(viewport[0]-width-4.).max(4.)),self.y.clamp(1.,(viewport[1]-height-1.).max(1.)),width,rows)
    }
    fn item_at(&self, x: f32, y: f32, char_w: f32,viewport:[f32;2]) -> Option<usize> {
        let (mx,my,width,rows)=self.layout(viewport,char_w);
        let i = ((y - my - 4.0) / line_h()).floor();
        (x >= mx && x < mx+width && i >= 0.0 && (i as usize) < self.items.len().saturating_sub(self.start(rows)).min(rows)).then_some(i as usize+self.start(rows))
    }
}

/// The `ctrl-p` fuzzy file switcher.
struct Switcher {
    query: String,
    selected: usize,
    hits: Vec<usize>,
}

use crate::comparison_session::{DiffCache, Place, Transition};

impl Viewer {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        set: Arc<ChangeSet>,
        spec: Spec,
        blame: bool,
        name: Option<String>,
        doc: Arc<Doc>,
        view: View,
        gave_up: bool,
        options: Options,
        font: Font,
        started: Instant,
        timings: bool,
        cx: &mut Context<Self>,
    ) -> Self {
        themes::install(cx);
        cx.observe_global::<themes::GuiThemes>(|v,cx|{let global=cx.global::<themes::GuiThemes>();let palette=global.0.snapshot().palette;let notice=global.1.clone();let changed=!Arc::ptr_eq(&v.palette,&palette);v.palette=palette;if let Some(n)=notice{v.notice(n,cx);}
            if changed||v.theme_chooser.is_some(){cx.notify();}}).detach();
        let mut comparison = crate::comparison_session::ComparisonSession::new(view, options);
        comparison.view.set_unified(crate::settings::get().view.unified);
        comparison.banner = gave_up.then(|| "Unable to calculate diff — file too big / too many changes".into());
        let mut viewer = Viewer {
            palette: cx.global::<themes::GuiThemes>().0.snapshot().palette,
            theme_chooser: None,
            theme_preview: None,
            comparison,
            markdown_poll_task: None,
            nav_revision: 0,
            summary_scroll: 0.0,
            summary_free: false,
            folders_task: None,
            markdown: markdown::Adapter::default(),
            markdown_images: markdown::ImageCache::default(),
            #[cfg(all(test, feature="gui-test"))]
            inspected_position: None,
            set,
            name: name.filter(|n| !n.is_empty()),
            loaded: HashMap::new(),
            font,
            focus: cx.focus_handle(),
            started,
            timings,
            first_paint_reported: Default::default(),
            nav: Vec::new(),
            layout: crate::layout::Layout::default(),
            nav_focused: false,
            panel_focused: false,
            diff_focus: Side::Left,
            nav_scroll: 0.0,
            message_scroll: 0.0,
            message_focused: false,
            message_sel: None,
            message_press: None,
            file_search: Default::default(),
            file_search_scroll: None,
            log_search_scroll: None,
            nav_hover: None,
            switcher: None,
            toast: None,
            shown_title: String::new(),
            sel: None,
            dragging: false,
            menu: None,
            held_log: None,
            incoming: None,
            command_palette: None,
            diff_options: None,
            goto: None,
            git_prompt: None,
            search: None,
            drag: (0.0, 0.0, 0.0),
            drag_ticking: false,
            hscroll: [0.0; 2],
            char_w: 0.0,
            text_w: [600.0; 2],
            panes: PaneSizes::default(),
            spec,
            blame_on: blame,
            sticky_on: crate::settings::get().view.sticky_scroll,
            blames: HashMap::new(),
            popup: None,
            help: false,help_scroll:0,about:None,report_request:Default::default(),
            repo: None,
            pending_place: None,
            place_recorder: None,
            repo_polling: false,
            nothing: None,
            iview: ImageView::fresh(),
            image_uploads: Default::default(),
            pr_dialog:None,
            merge: None,
            picker_v: None,
            edits: crate::edit_session::EditSession::default(),
            file_actions: Default::default(),
            applied_scroll: 0.0,
            edited: HashSet::new(),
            write_error: None,
            conflicts: None,
        };
        viewer.sync_nav();
        // archives start collapsed: the first visible file, or the summary over file 0
        let start = viewer.comparison.folders.start(&viewer.set);
        viewer.sync_nav();
        viewer.layout.set_files(viewer.set.comparisons.len() > 1);
        if !viewer.set.comparisons.is_empty(){viewer.loaded.insert(0, Loaded::new(doc, 0));}else{viewer.nothing=Some("No comparison open".into());}
        viewer.ensure_blame(cx);
        if start != 0 { viewer.open(start, Place::FirstChange, cx); }
        viewer.poll_folders(cx);
        if let Some(notice)=cx.global::<themes::GuiThemes>().1.clone(){viewer.notice(notice,cx);}
        viewer
    }

    pub fn focus_handle(&self) -> FocusHandle {
        self.focus.clone()
    }

    /// Small files can finish colouring during window setup without competing with first paint.
    pub fn prepare_small_syntax(&mut self, cx: &mut Context<Self>) {
        if self.merge.is_none() && self.doc().is_some_and(|d| d.text_left.len() + d.text_right.len() <= 64 << 10) {
            self.parse_in_background(self.comparison.current, cx);
        }
    }

    fn doc(&self) -> Option<&Arc<Doc>> {
        self.loaded.get(&self.comparison.current).map(|l| &l.doc)
    }

    fn labels(&self, index: usize) -> Labels {
        let Some(c) = self.set.comparisons.get(index) else {
            return Labels { left: String::new(), right: String::new(), status: "" };
        };
        let named = |p: &str| self.name.clone().unwrap_or_else(|| p.to_string());
        let status = navigator::badge(&c.status);
        let left = if matches!(c.status, Status::Added | Status::Untracked) { "(absent)".into() } else { named(&c.old_path) };
        let right = if c.status == Status::Deleted { "(deleted)".into() } else { named(&c.path) };
        Labels { left, right, status }
    }

    /// Give the window a Change Set Picker over the repository at `root`, shown (or in Focus Mode).
    /// A brief message (from outside the window's own actions).
    pub fn notice(&mut self, message: String, cx: &mut Context<Self>) {
        self.toast(message, cx);
    }

    /// The repository behind the Change Set: its shared session, opened on the comparison already shown.
    /// Reopens at the launch's saved place (from [`crate::place::for_launch`]) and records the place as it changes.
    pub fn restore_place(&mut self, launch: Option<crate::place::Launch>, cx: &mut Context<Self>) {
        let Some(launch) = launch else { return };
        if let (Some(commit), Some(repo)) = (launch.restore.as_ref().and_then(|r| r.commit.clone()), self.repo.as_mut()) {
            repo.want_commit(commit);
        }
        self.pending_place = launch.restore;
        self.place_recorder = Some(crate::place::Recorder::new(launch.file));
        cx.notify();
    }

    /// Opens the restored file at its line once the repository has settled, then records where the user is.
    fn restore_and_record_place(&mut self, cx: &mut Context<Self>) {
        let settled = self.repo.as_ref().is_none_or(|r| r.settled());
        // a commit the log never reached (amended or rebased away) leaves the normal start, not its file in another commit
        if settled && self.doc().is_some() && let Some(pending) = self.pending_place.take()
            && pending.commit.as_deref().is_none_or(|c| self.repo.as_ref().is_some_and(|r| r.restored(c)))
            && let Some(index) = pending.file.as_deref().and_then(|f| self.set.comparisons.iter().position(|c| c.path == f))
        {
            self.open(index, crate::comparison_session::Place::Line(Side::Right, pending.line.unwrap_or(1)), cx);
        }
        if self.pending_place.is_none() && self.repo.is_some() {
            let file = self.set.comparisons.get(self.comparison.current).map(|c| c.path.clone());
            let line = self.doc().is_some().then(|| self.comparison.view.anchor_line_on(Side::Right) as u32 + 1);
            if let Some(r) = self.repo.as_mut() { r.set_position(file, line); }
        }
        if self.pending_place.is_none() && self.place_recorder.is_some() && let Some(root) = self.set.git_root().map(Path::to_path_buf) {
            let source = self.repo.as_ref().and_then(|r| r.place_source()).unwrap_or(crate::place::Source::Local);
            let file = self.set.comparisons.get(self.comparison.current).map(|c| c.path.clone());
            let line = self.doc().is_some().then(|| self.comparison.view.anchor_line_on(Side::Right) as u32 + 1);
            let place = crate::place::Place { version: crate::place::VERSION, root, source, file, line };
            if let Some(recorder) = self.place_recorder.as_mut() { recorder.record(place); }
        }
    }

    pub fn with_session(&mut self, session: crate::repository_session::RepositorySession, cx: &mut Context<Self>) {
        self.repo = Some(session);
        // a pull request's fetch refs go when Lenz does; the session's own cleanup may not run at exit
        cx.on_app_quit(|v: &mut Viewer, _| {
            if let Some(r) = v.repo.as_ref().filter(|r| r.fetched_pr_refs()) {
                let _ = crate::pr::remove_refs(&r.picker.root);
            }
            async {}
        })
        .detach();
        self.poll_repository(cx);
        self.refresh_conflicts(cx);
    }

    /// Show another Change Set (picked in the picker): its first file, everything per-file reset.
    pub(super) fn show_change_set(&mut self, set: impl Into<Arc<ChangeSet>>, cx: &mut Context<Self>) {
        self.show_change_set_with(set.into(), None, cx);
    }

    /// Show `set`, its first file already read when `first` is given (see `stage_change_set`).
    pub(super) fn show_change_set_with(&mut self, set: Arc<ChangeSet>, first: Option<Arc<Doc>>, cx: &mut Context<Self>) {
        if !self.edits_may_close(cx) { return; }
        // a change set shown directly supersedes one still being read
        self.incoming = None;
        self.leave_conflicts();
        // Search result indices and file actions belong to exactly this Change Set.
        // Dropping search cancels its producer; queued batches are fenced by its token.
        self.search = None;
        self.switcher = None;
        self.file_search = Default::default();
        self.file_search_scroll = None;
        self.menu = None;
        self.command_palette = None;
        self.diff_options = None;
        self.applied_scroll = 0.0;
        let empty = set.comparisons.is_empty();
        self.nothing = empty.then(|| "No changes in this selection".into());
        self.set = set;
        // the session indexes Viewed Marks and review threads into the comparison on screen
        if let Some(r) = self.repo.as_mut() { r.showing(self.set.clone(), self.spec.clone()); }
        self.comparison.markdown_invalidate(); self.markdown=markdown::Adapter::default();
        self.markdown_poll_task = None;
        self.loaded.clear();
        self.comparison.cache = DiffCache::default();
        self.comparison.folds.clear();
        self.edits = crate::edit_session::EditSession::default();
        self.blames.clear();
        self.nav_scroll = 0.0;
        self.sync_nav();
        self.sel = None;
        self.comparison.find = None;
        self.popup = None;
        if empty {
            let rows=self.comparison.view.viewport_rows();let mode=self.comparison.view.mode();
            self.comparison.current=0;self.comparison.begin();
            self.comparison.view=View::new(0,0,&[],rows);self.comparison.view.set_mode(mode);
            cx.notify();return;
        }
        if let Some(doc) = first { self.loaded.insert(0, Loaded::new(doc, self.comparison.generation)); }
        self.comparison.current = usize::MAX; // so open() treats file 0 as new
        let start = self.comparison.folders.start(&self.set);
        // with every file in a collapsed archive, the archive's summary covers file 0
        if self.summary_shown() { self.show_file(start, Place::FirstChange, cx); } else { self.open(start, Place::FirstChange, cx); }
        self.sync_nav();
        self.poll_folders(cx);
        cx.notify();
    }

    pub fn title(&self) -> String {
        if let Some(t) = self.merge_title().or_else(|| self.picker_v_title()) {
            return t;
        }
        if let Some(s) = self.repo.as_ref().and_then(|r| r.pr.as_ref()) {
            return format!("#{} {} — Lenz", s.pr.number, s.pr.title);
        }
        if let Some(r) = self.repo.as_ref() {
            let repo = r.picker.root.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned());
            if let Some(t) = r.selection_title() { return format!("{} — {repo} — Lenz", t.text); }
            if r.picker.shown { return format!("{repo} — {} — Lenz", r.picker.title_branch); }
        }
        let l = self.labels(self.comparison.current);
        let mut title = if l.left == l.right { l.right } else { format!("{} ⟷ {}", l.left, l.right) };
        if self.set.comparisons.len() > 1 {
            title += &format!(" — file {}/{}", self.comparison.current + 1, self.set.comparisons.len());
        }
        title + " — Lenz"
    }

    pub fn refresh_title(&mut self, cx: &mut Context<Self>) {
        let Some(root) = self.repo.as_ref().map(|r| &r.picker).map(|p| p.root.clone()) else { return };
        cx.spawn(async move |this, cx| {
            let reading = root.clone();
            let branch = cx.background_executor().spawn(async move {
                git::run(&reading, &["rev-parse", "--abbrev-ref", "HEAD"]).ok()
                    .map(|b| String::from_utf8_lossy(&b).trim().to_string())
            }).await;
            this.update(cx, |v: &mut Viewer, cx| {
                if let (Some(p), Some(branch)) = (v.repo.as_mut().map(|r| &mut r.picker).filter(|p| p.root == root), branch)
                    && p.title_branch != branch
                {
                    p.title_branch = branch;
                    cx.notify();
                }
            }).ok();
        }).detach();
    }

    /// The blame cache key of a Side of the shown Comparison, while annotations are on and it is git-backed.
    fn blame_key(&self, side: Side) -> Option<BlameKey> {
        self.blame_key_at(self.comparison.current, side)
    }

    fn blame_key_at(&self, index: usize, side: Side) -> Option<BlameKey> {
        if !self.blame_on {
            return None;
        }
        let c = self.set.comparisons.get(index)?;
        let version = blame::version(&self.spec, c, side)?;
        Some((if side == Side::Left { c.old_path.clone() } else { c.path.clone() }, version))
    }

    /// Width of a Side's Blame Annotation column (0 when it has none).
    fn ann_w(&self, side: Side) -> f32 {
        if self.blame_key(side).is_some() {
            let width=self.comparison_widths((self.panes.viewport[0]-self.diff_x()).max(0.0))[side as usize];
            BLAME_W.min((width-num_w()-16.0-40.0).max(0.0))
        } else { 0.0 }
    }

    fn toggle_blame(&mut self, cx: &mut Context<Self>) {
        self.blame_on = !self.blame_on;
        let git = self.set.comparisons.get(self.comparison.current).is_some_and(|comparison|[Side::Left, Side::Right].iter().any(|&s|blame::version(&self.spec,comparison,s).is_some()));
        if self.blame_on && !git {
            self.toast("No git history for this file".into(), cx);
        }
        self.ensure_blame(cx);
    }

    /// Start `git blame` for the shown Sides that lack it, streaming lines in as git reports them.
    fn ensure_blame(&mut self, cx: &mut Context<Self>) {
        self.trim_blames();
        let keys = [self.blame_key(Side::Left), self.blame_key(Side::Right)];
        for (side, key) in [Side::Left, Side::Right].into_iter().zip(keys) {
            let Some(key) = key.filter(|k| !self.blames.contains_key(k)) else { continue };
            let mut annotations = Blame::default();
            annotations.begin(self.set.clone(), self.comparison.current, side, Some(key.1.clone()), None);
            let request = annotations.request_id();
            self.blames.insert(key.clone(), annotations);
            cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(std::time::Duration::from_millis(100)).await;
                    let busy = this.update(cx, |viewer: &mut Viewer, cx| {
                        if request.strong_count() == 0 { return false; }
                        let Some(b) = viewer.blames.get_mut(&key) else { return false };
                        let changed = b.poll();
                        let busy = b.busy();
                        let error = (!busy).then(|| b.error().map(str::to_string)).flatten();
                        if let Some(error) = error { viewer.toast(format!("git blame: {error}"), cx); }
                        if changed { cx.notify(); }
                        busy
                    }).unwrap_or(false);
                    if !busy { break; }
                }
            }).detach();
        }
    }

    /// Keep completed annotations for the same hot text files as the document cache.
    /// Hidden unfinished requests are dropped, so queued work loses its owner.
    fn trim_blames(&mut self) {
        if !self.blame_on {
            self.blames.clear();
            return;
        }
        let active = [self.blame_key(Side::Left), self.blame_key(Side::Right)];
        let retained: HashSet<_> = self.loaded.iter()
            .filter(|(_, loaded)| loaded.doc.placeholder.is_none() && loaded.doc.image.is_none())
            .flat_map(|(&index, _)| [self.blame_key_at(index, Side::Left), self.blame_key_at(index, Side::Right)].into_iter().flatten())
            .collect();
        self.blames.retain(|key, annotations| active.iter().flatten().any(|current| current == key)
            || (!annotations.busy() && annotations.error().is_none() && retained.contains(key)));
    }

    /// Open the commit popup for a line's Blame Annotation and fetch the commit's details.
    fn open_commit(&mut self, side: Side, line: u32, cx: &mut Context<Self>) {
        let Some(b) = self.blame_key(side).and_then(|k| self.blames.get(&k)) else { return };
        let Some(blame::BlameLine { commit, original_line, original_path }) = b.line(line as usize).cloned() else { return };
        if !commit.is_committed() { self.toast("Not committed yet".into(), cx); return; }
        let sha = commit.sha.clone();
        let Some(root) = self.set.git_root().map(Path::to_path_buf) else { return };
        let mut inspection = crate::annotation_session::CommitInspection::default();
        inspection.open(root, sha.clone());
        self.popup = Some(CommitPopup { sha: sha.clone(), original_line, original_path, inspection,scroll:0 });
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(std::time::Duration::from_millis(20)).await;
                let busy = this.update(cx, |viewer: &mut Viewer, cx| {
                    let Some(p) = viewer.popup.as_mut().filter(|p| p.sha == sha) else { return false };
                    let changed = p.inspection.poll(); let busy = p.inspection.busy();
                    if changed { cx.notify(); }
                    busy
                }).unwrap_or(false);
                if !busy { break; }
            }
        }).detach();
    }

    fn popup_rows(&self,viewport:[f32;2])->Vec<(String,u32)> {
        let ((_,_,width,_),_,_)=popup_layout(0,viewport,self.char_w);
        let cols=((width-24.).max(0.)/self.char_w.max(1.)).floor().max(1.) as usize;
        self.popup.iter().flat_map(|p|p.rows(&self.palette)).flat_map(|(line,color)|pr_details::wrap(&line,cols).into_iter().map(move|row|(row,color))).collect()
    }

    /// A commit popup button: `s` show the commit, `c` copy its hash, `a` annotate the revision before it.
    fn popup_action(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(p) = self.popup.as_ref() else { return };
        let (sha, line, path) = (p.sha.clone(), p.original_line, p.original_path.to_string());
        let root = self.set.git_root().map(Path::to_path_buf).unwrap_or_default();
        let parent = blame::parent_or_empty_tree(&root, &sha).unwrap_or_else(|_| format!("{sha}^1"));
        let args: Vec<String> = match key {
            "c" => {
                cx.write_to_clipboard(ClipboardItem::new_string(sha.clone()));
                self.toast(format!("Copied {}", &sha[..sha.len().min(10)]), cx);
                return;
            }
            "s" => vec![parent, sha],
            "a" => ["--blame", "--line", &line.to_string(), &parent, &sha, "--", &path].map(String::from).to_vec(),
            _ => return,
        };
        self.popup = None;
        // a new window, like IntelliJ's
        let spawned = std::env::current_exe().and_then(|exe| std::process::Command::new(exe).args(&args).current_dir(root).spawn());
        if let Err(e) = spawned {
            self.toast(format!("Cannot start lenz: {e}"), cx);
        }
    }

    /// The Side whose Blame Annotation column holds `x`.
    fn ann_at(&self, window: &Window, x: f32) -> Option<Side> {
        let sw = self.diff_x();
        let widths = self.comparison_widths(f32::from(window.viewport_size().width) - sw);
        let pane_w = widths[0];
        let rx = sw + pane_w + GUTTER_W;
        if x >= sw && x < sw + self.ann_w(Side::Left) {
            Some(Side::Left)
        } else if x >= rx && x < rx + self.ann_w(Side::Right) {
            Some(Side::Right)
        } else {
            None
        }
    }

    /// A toolbar button; dropdowns open a menu under the button at `x`.
    fn tool(&mut self, tool: Tool, x: f32, window: &mut Window, cx: &mut Context<Self>) {
        if let Tool::Merge(act)=tool {self.merge_act(act,cx);return;}
        if self.modal().is_some_and(|m|matches!(m,Modal::Discard|Modal::Checkout|Modal::About|Modal::Theme)){return;}
        if self.help||self.popup.is_some()||self.goto.is_some()||self.git_prompt.is_some()||self.checkout_modal()||self.switcher.is_some(){return;}
        if (self.merge.as_ref().is_some_and(|s|s.ask.is_some())||self.picker_v.as_ref().is_some_and(|s|s.ask.is_some())) && !matches!(tool,Tool::Pick(version_picker::PickAct::Save|version_picker::PickAct::Done|version_picker::PickAct::Cancel)){return;}
        if self.conflicts.as_ref().is_some_and(|s|s.confirming()){return;}
        match tool {
            Tool::DiffOptions=>self.toggle_diff_options(x,cx),
            Tool::AppMenu=>self.open_app_menu(x,cx),
            Tool::Help=>self.run_action(crate::catalog::Action::Help(crate::catalog::HelpAction::Shortcuts),window,cx),
            Tool::Run(action)=>self.run_action(action,window,cx),
            Tool::Back=>self.navigate(false,cx),
            Tool::Forward=>self.navigate(true,cx),
            Tool::Merge(act)=>self.merge_act(act,cx),
            Tool::Pick(act) => {
                self.picker_v_act(act, cx);
            }
            Tool::PrevChange => self.step_change(false, cx),
            Tool::NextChange => self.step_change(true, cx),
            Tool::PrevFile => self.step_file(false, cx),
            Tool::NextFile => self.step_file(true, cx),
            Tool::Layout(command)=>self.layout_command(command,cx),
            #[cfg(all(test,feature="gui-test"))]
            Tool::About=>self.interaction(commands::Interaction::About,cx),
            #[cfg(all(test,feature="gui-test"))]
            Tool::Theme=>self.open_themes(cx),
        }
        cx.notify();
    }

    /// Show or hide a panel, or enter or leave Focus, through the shared layout.
    pub(super) fn layout_command(&mut self,command:crate::layout::LayoutCommand,cx:&mut Context<Self>){
        if let Some(why)=self.layout.disabled(command,self.repo.as_ref()){self.toast(why.into(),cx);return;}
        self.layout.execute(command,self.repo.as_mut());
        // details are a Focus tab: the Overview shows the diff
        if !self.layout.focused(self.repo.as_ref()) && self.pr_details_shown() {
            if let Some(pr)=self.repo.as_mut().and_then(|r|r.pr.as_mut()) {pr.execute(crate::pr_session::PrCommand::Close);}
            self.pr_dialog=None;
        }
        if !self.layout.files(){self.nav_focused=false;}
        self.reveal_in_nav();
        self.poll_repository(cx);
        cx.notify();
    }

    fn reload(&mut self,cx:&mut Context<Self>){
        if self.repo.is_some(){self.refresh_repository(cx);return;}
        self.set.invalidate_archive_cache();
        self.comparison.folders.invalidate_counts();
        let index=self.comparison.current;let anchor=self.comparison.view.anchor_line();
        for side in [Side::Left,Side::Right]{if let Some(key)=self.blame_key(side){self.blames.remove(&key);}}
        self.loaded.remove(&index);self.comparison.cache.0.remove(&index);self.iview=ImageView::fresh();self.sel=None;self.comparison.find=None;
        self.load(index,Place::Anchor(anchor),false,cx);self.poll_folders(cx);self.ensure_blame(cx);self.refresh_repository(cx);cx.notify();
    }
    fn interaction_disabled(&self,command:commands::Interaction)->Option<&'static str>{
        if matches!(command,commands::Interaction::Copy|commands::Interaction::CopyMarkdownSource) && (self.comparison.markdown_mode()==crate::markdown::PresentationMode::Rendered || command==commands::Interaction::CopyMarkdownSource) && let Some(doc)=self.doc() {
            return self.comparison.copy_disabled(command,commands::CopyContext{set:&self.set,index:self.comparison.current,doc,view:&self.comparison.view,selection:self.sel,side:self.diff_focus,path_label:self.name.as_deref()});
        }
        let side=if self.markdown_active(){self.diff_focus}else{self.sel.map_or(Side::Right,|s|s.side)};
        let mapped=self.comparison.markdown_inspection_position(side);
        if matches!(command,commands::Interaction::Editor|commands::Interaction::DefaultApp|commands::Interaction::InspectBlame) && let Err(reason)=mapped { return Some(reason); }
        let line=mapped.ok().flatten().map_or_else(||self.sel.map_or(self.comparison.view.anchor_line_on(side)as usize,|s|s.head.line as usize),|p|p.line as usize);if matches!(command,commands::Interaction::CopyLeftChange|commands::Interaction::CopyRightChange)&&let Some(doc)=self.doc()&&let Some(reason)=self.comparison.copy_disabled(command,commands::CopyContext{set:&self.set,index:self.comparison.current,doc,view:&self.comparison.view,selection:self.sel,side,path_label:self.name.as_deref()}){return Some(reason);}command.disabled(&commands::InteractionContext{find_open:self.find_open(),document:self.doc().is_some(),text:self.image_doc().is_none()&&self.doc().is_some_and(|d|d.placeholder_kind().is_none()),selection:self.sel.is_some_and(|s|!s.is_empty()),repository:self.set.git_root().is_some(),annotation:self.blame_key(side).and_then(|k|self.blames.get(&k)).and_then(|b|b.line(line)).is_some()})}
    fn report_last_error(&mut self, cx: &mut Context<Self>) {
        match self.report_request.start() {
            Ok(false) => return,
            Err(error) => { self.toast(format!("Cannot prepare report: {error}"), cx); return; }
            Ok(true) => {}
        }
        self.toast("Preparing issue draft; review the local report before sharing".into(), cx);
        cx.spawn(async move |this, cx| {
            loop {
                let done = this.update(cx, |viewer, cx| {
                    let Some(result) = viewer.report_request.poll() else { return false };
                    match result {
                        Ok(report) => { cx.open_url(&report.issue_url); viewer.toast(format!("Review report: {}", report.path.display()), cx); }
                        Err(error) => viewer.toast(error.to_string(), cx),
                    }
                    true
                }).unwrap_or(true);
                if done { break; }
                cx.background_executor().timer(std::time::Duration::from_millis(20)).await;
            }
        }).detach();
    }

    fn close_about(&mut self,window:&mut Window){
        if let Some(about)=self.about.take(){let _=window.drop_image(about.image);}
    }
    pub(crate) fn interaction(&mut self,command:commands::Interaction,cx:&mut Context<Self>){
        use commands::Interaction::*;
        if command==Find {self.find_focused(cx);return;}
        if let Some(reason)=self.interaction_disabled(command){self.toast(reason.into(),cx);return;}
        match command{StickyScroll=>self.toggle_sticky(cx),FindCase|FindWord|FindRegex=>{if let Some(option)=command.find_option(){self.toggle_find_option(option);}},ResetPaneSizes=>self.reset_pane_sizes(),ScrollLeft|ScrollRight if self.markdown_active()=>self.markdown_horizontal(self.diff_focus,if command==ScrollLeft{-4.0*self.char_w}else{4.0*self.char_w}),ScrollLeft|ScrollRight=>self.scroll_horizontal(self.sel.map_or(self.diff_focus,|s|s.side),if command==ScrollLeft{-4.0*self.char_w}else{4.0*self.char_w}),ChangeTheme=>self.open_themes(cx),ReportLastError=>self.report_last_error(cx),About=>{self.help=false;self.menu=None;self.command_palette=None;self.diff_options=None;self.about=Some(about::About::open());},Find=>self.open_find(),FindSet=>{let(query,changed)=self.search.as_ref().map_or((String::new(),false),|s|(s.query.clone(),s.changed_only));self.search=Some(SetSearch::new(query,changed,self.comparison.find_options));},GoToLine=>self.goto=Some(String::new()),Copy|CopyMarkdownSource|CopyPath|CopyReference|CopyLeftChange|CopyRightChange=>{let side=if self.markdown_active(){self.diff_focus}else{self.sel.map_or(Side::Right,|s|s.side)};if let Some(doc)=self.doc(){match self.comparison.copy_text(command,commands::CopyContext{set:&self.set,index:self.comparison.current,doc,view:&self.comparison.view,selection:self.sel,side,path_label:self.name.as_deref()}){Ok(text)=>cx.write_to_clipboard(ClipboardItem::new_string(text)),Err(reason)=>self.toast(reason.into(),cx)}}},SelectAll if self.markdown_active()=>self.markdown_select_all(),SelectAll=>{let side=self.sel.map_or(Side::Right,|s|s.side);self.sel=Some(Selection::all(side,self.raw(side)));self.selection_to_primary(cx);},Editor|DefaultApp=>self.open_focused(command==DefaultApp,cx),Blame=>self.toggle_blame(cx),InspectBlame=>{let side=if self.markdown_active(){self.diff_focus}else{self.sel.map_or(Side::Right,|s|s.side)};match self.comparison.markdown_inspection_position(side){Ok(mapped)=>{let line=mapped.map_or_else(||self.sel.map_or(self.comparison.view.anchor_line_on(side)as u32,|s|s.head.line),|p|p.line);self.open_commit(side,line,cx);},Err(reason)=>self.toast(reason.into(),cx)}},Reload=>self.reload(cx)}
        cx.notify();
    }

    fn comparison_command(&mut self,command:Command,cx:&mut Context<Self>){
        let large=self.large();let loading=self.doc().is_none();
        match self.comparison.command(command,&self.set.comparisons,loading,large){
            crate::comparison_session::Transition::Presentation(mode)=>{
                let anchor=(mode==crate::markdown::PresentationMode::Source).then(||self.comparison.markdown_anchor(self.diff_focus)).flatten();
                self.markdown_command(crate::markdown::MarkdownCommand::SetMode(mode));
                if let Some(anchor)=anchor {self.markdown_command(crate::markdown::MarkdownCommand::ShowSource(anchor));}
                if mode==crate::markdown::PresentationMode::Source {self.markdown=markdown::Adapter::default();}
                self.poll_markdown(cx);
            },
            crate::comparison_session::Transition::Repaint=>{if matches!(command,Command::ToggleFold|Command::UnfoldAll)&&let Some(doc)=self.doc().cloned(){self.comparison.remember_folds(&doc);}},
            crate::comparison_session::Transition::Recompare=>self.recompare(cx),
            crate::comparison_session::Transition::Load{index,place,force}=>{if force{self.load(index,place,true,cx);}else{self.open(index,place,cx);}},
            crate::comparison_session::Transition::Notice(message)=>self.toast(message,cx)
        }
        cx.notify();
    }

    fn refresh_markdown(&mut self, cx: &mut Context<Self>) {
        let doc = self.doc().cloned();
        self.comparison.markdown_refresh(doc.as_deref().map(|doc| &doc.text));
        self.poll_markdown(cx);
    }
    /// Only a pending shared request needs a timer; idle Source views schedule nothing.
    fn poll_markdown(&mut self, cx: &mut Context<Self>) {
        self.markdown_poll_task = None;
        if !self.comparison.markdown_loading() { return; }
        self.markdown_poll_task = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(std::time::Duration::from_millis(20)).await;
                let pending = this.update(cx, |viewer: &mut Viewer, cx| {
                    if viewer.comparison.markdown_poll() { cx.notify(); }
                    viewer.comparison.markdown_loading()
                }).unwrap_or(false);
                if !pending { break; }
            }
        }));
    }

    /// Fold or unfold Change Block `block` (or every block with `None` meaning unfold all), remembering folds by text.
    fn fold(&mut self, block: Option<usize>, cx: &mut Context<Self>) {
        if let Some(doc) = self.doc().cloned() { self.comparison.fold(&doc, block); }
        cx.notify();
    }

    /// The Change Block a key acts on: the caret's, when it sits in one on screen, else the current change.
    fn key_block(&self) -> Option<usize> {
        let caret = self.sel.filter(|s| self.frame_has(s.side, s.head.line));
        caret.and_then(|s| self.comparison.view.block_index(s.side, s.head.line)).or_else(|| self.comparison.view.current_change())
    }

    /// Copy the other Side's Change Block into a draft; Save writes it.
    fn move_change(&mut self, block: usize, onto: Side, cx: &mut Context<Self>) {
        self.edit_current(crate::edit_session::EditCommand::Apply { block, onto }, cx);
    }

    fn undo(&mut self, redo: bool, cx: &mut Context<Self>) {
        self.edit_current(if redo { crate::edit_session::EditCommand::Redo } else { crate::edit_session::EditCommand::Undo }, cx);
    }

    fn edit_current(&mut self, command: crate::edit_session::EditCommand, cx: &mut Context<Self>) {
        use crate::edit_session::{EditCommand, EditError};
        self.write_error = None;
        let Some(doc) = self.doc().cloned() else { return };
        let index = self.comparison.current;
        let (lp,rp) = self.paths(index);
        match self.comparison.edit(&mut self.edits, &doc, command, [&lp,&rp]) {
            Ok(written) => {
                let written_paths=if command==EditCommand::Save {self.edits.edited.clone()}else{HashSet::new()};
                let side_changed = if written.document.text_left != doc.text_left { Side::Left } else { Side::Right };
                let doc = Arc::new(Doc { text: written.document, image: None });
                let mut loaded = Loaded::new(doc.clone(), self.comparison.generation);
                let side = match command {
                    EditCommand::Apply { onto, .. } => onto,
                    _ => side_changed,
                };
                loaded.parsed[1 - side as usize] = self.loaded.get(&index).and_then(|l| l.parsed[1 - side as usize].clone());
                loaded.declarations[1 - side as usize] = self.loaded.get(&index).and_then(|l| l.declarations[1 - side as usize].clone());
                self.loaded.insert(index, loaded);
                for (i,c) in self.set.comparisons.iter().enumerate().filter(|&(i,_)| i != index && command==EditCommand::Save) {
                    if !self.edits.dirty(i) && [&c.left,&c.right].into_iter().any(|s| matches!(s, Source::File(p) if written_paths.contains(p))) {
                        self.loaded.remove(&i); self.comparison.cache.0.remove(&i);
                    }
                }
                if command==EditCommand::Save {
                    self.edited.extend(written_paths);
                    for side in [Side::Left,Side::Right] {if let Some(key)=self.blame_key(side){self.blames.remove(&key);}}
                    self.ensure_blame(cx);
                } else if let Some(key)=self.blame_key(side) {self.blames.remove(&key);self.ensure_blame(cx);}
                self.comparison.cache.0.remove(&index); self.sel = None;self.applied_scroll=(self.edits.applied(index).len() as f32-self.panel_rows()).max(0.0);
                self.comparison.markdown_invalidate(); self.markdown=markdown::Adapter::default(); self.markdown_poll_task = None;
                self.comparison.begin(); self.show_text(&doc, Place::Anchor(written.anchor)); self.diff(doc, Place::Anchor(written.anchor), cx);
                let message = match command {
                    EditCommand::Apply { .. } => "Change applied to draft — Save writes it; applied-history rows Undo individual moves",
                    EditCommand::Undo => "Change undone in draft", EditCommand::Redo => "Change redone in draft", EditCommand::UndoApplied(_) => "Applied change undone in draft",
                    EditCommand::Save => "Changes saved", EditCommand::Discard => "Unsaved changes discarded",
                };
                self.toast(message.into(), cx);
            }
            Err(EditError::Write(WriteError::ChangedOnDisk)) => {
                self.write_error = Some("The file changed on disk — nothing written; your unsaved draft is retained. Discard unsaved changes to reload.".into());
            }
            Err(e) => self.write_error = Some(e.to_string()),
        }
        cx.notify();
    }

    /// Open a Side of Comparison `index` in the editor at `line`, or (`default_app`) with the system default app.
    /// Revision and index Sides open as read-only copies in the cache.
    fn open_file(&mut self, index: usize, side: Side, line: Option<usize>, column: Option<usize>, default_app: bool, cx: &mut Context<Self>) {
        #[cfg(all(test, feature="gui-test"))]
        { let _=(default_app,cx); self.inspected_position=Some((index,side,line,column)); }
        #[cfg(not(all(test, feature="gui-test")))]
        {
            let binary=self.loaded.get(&index).is_some_and(|l|l.doc.placeholder.is_some()||l.doc.notes.iter().any(|n|n.contains("textconv")));
            let plan=match open::comparison_plan(&self.set,index,side,binary,line,column,default_app){Ok(plan)=>plan,Err(e)=>return self.toast(e,cx)};
            if let Err(e) = open::spawn(&plan.launch) {
                return self.toast(format!("Cannot start {}: {e}", plan.launch.program), cx);
            }
            if let Some(t) = plan.toast { self.toast(t, cx); }
        }
    }

    /// `o` / `shift-o`: the focused Side (right if none) at the caret, else at the anchor line.
    fn open_focused(&mut self, default_app: bool, cx: &mut Context<Self>) {
        if self.markdown_active() {
            match self.comparison.markdown_inspection_position(self.diff_focus) {
                Ok(Some(p)) => { let side=self.diff_focus; self.open_file(self.comparison.current,side,Some(p.line as usize+1),Some(self.raw(side).get(p.line as usize).map_or(1,|line|line[..p.col.min(line.len())].chars().count()+1)),default_app,cx); },
                Err(reason) => self.toast(reason.into(),cx),
                Ok(None) => {},
            }
            return;
        }

        let side = self.sel.map_or(Side::Right, |s| s.side);
        let (line, column) = match self.sel.filter(|s| s.side == side) {
            Some(s) => {
                let text = self.raw(side).get(s.head.line as usize).map_or("", String::as_str);
                (s.head.line as usize + 1, text[..s.head.col.min(text.len())].chars().count() + 1)
            }
            None => (self.anchor_line(side) as usize + 1, 1),
        };
        self.open_file(self.comparison.current, side, Some(line), Some(column), default_app, cx);
    }

    /// The raw line of `side` at the anchor.
    fn anchor_line(&self, side: Side) -> u32 {
        let f = self.comparison.view.frame();
        let rows = if side == Side::Left { f.left } else { f.right };
        let a = self.comparison.view.viewport_rows() / 3.0;
        rows.iter().filter(|r| r.y <= a).filter_map(|r| match r.content {
            Content::Line(l) => Some(l),
            _ => None,
        }).next_back().unwrap_or(0)
    }

    /// The shown Comparison waits behind the Large-file gate.
    fn large(&self) -> bool {
        self.doc().is_some_and(|d| matches!(d.placeholder, Some(Placeholder::Large { .. })))
    }

    fn paths(&self, index: usize) -> (PathBuf, PathBuf) {
        let c = &self.set.comparisons[index];
        let named = |p: &str| PathBuf::from(self.name.clone().unwrap_or_else(|| p.to_string()));
        (named(&c.old_path), named(&c.path))
    }

    /// Show Comparison `index` in place of any Folder Summary, its folders expanded so Files has its row.
    fn open(&mut self, index: usize, place: Place, cx: &mut Context<Self>) {
        if index >= self.set.comparisons.len() {
            return;
        }
        if self.summary_shown() { self.comparison.folders.open(&self.set, index); } else { self.comparison.folders.reveal(&self.set.comparisons, index); }
        self.sync_nav();
        self.show_file(index, place, cx);
    }

    /// Show Comparison `index`: its text as soon as it's read, its Change Blocks once diffed.
    fn show_file(&mut self, index: usize, place: Place, cx: &mut Context<Self>) {
        if index >= self.set.comparisons.len() {
            return;
        }
        if index == self.comparison.current {
            if let Place::Line(side, line) = place {
                self.comparison.view.go_to_line(side, line);
            }
            self.conflict_open(cx); // again, after Cancel
            return;
        }
        // keep the finished diff of the file being left
        if let Some(l) = self.loaded.get_mut(&self.comparison.current)
            && l.parsing && ((l.doc.lang_left.is_some() && !l.doc.text_left.is_empty() && l.parsed[0].is_none())
                || (l.doc.lang_right.is_some() && !l.doc.text_right.is_empty() && l.parsed[1].is_none()))
        {
            l.release_syntax();
        }
        if self.comparison.diffed == self.comparison.generation && self.comparison.banner.is_none() && self.loaded.contains_key(&self.comparison.current) {
            let rows = self.comparison.view.viewport_rows();
            let view = std::mem::replace(&mut self.comparison.view, View::new(1, 1, &[], rows));
            self.comparison.cache.put(self.comparison.current, self.comparison.options, view, std::mem::take(&mut self.comparison.budget));
        }
        self.comparison.current = index;self.applied_scroll=0.0;
        self.iview = ImageView::fresh();
        self.sel = None;
        self.comparison.find = None;
        self.reveal_in_nav();
        self.popup = None;
        self.load(index, place, false, cx);
        self.ensure_blame(cx);
        self.conflict_open(cx);
    }

    /// Read Comparison `index` (once; again with `force` past the Large-file gate), show it, diff it.
    fn load(&mut self, index: usize, place: Place, force: bool, cx: &mut Context<Self>) {
        if index>=self.set.comparisons.len(){return;}
        self.hscroll = [0.0; 2];
        self.comparison.markdown_invalidate(); self.markdown=markdown::Adapter::default();
        self.markdown_poll_task = None;
        self.comparison.begin();
        let generation = self.comparison.generation;
        if let Some(loaded) = self.loaded.get_mut(&index) {
            loaded.used = generation;
        }
        trim_loaded(&mut self.loaded, &mut self.comparison.cache, self.comparison.current, LOADED_CACHE_BYTES);
        self.trim_blames();
        self.comparison.banner = None;
        self.write_error = None;
        // A retained draft can be displayed without a loader. In particular,
        // removal of its disk file or cancellation of a worker must not hide it.
        let recovering=self.edits.dirty(index) && (force || !self.loaded.contains_key(&index));
        if recovering {
            let (lp,rp)=self.paths(index);
            if let Some(text)=self.edits.recover(index,[&lp,&rp]) {
                let doc=Arc::new(Doc{text,image:None});
                self.loaded.insert(index,Loaded::new(doc.clone(),generation));
                self.comparison.cache.0.remove(&index);
                self.show_text(&doc,place);
                cx.notify();
            }
        }
        if let Some(doc) = self.loaded.get(&index).filter(|_| !force && !recovering).map(|l| l.doc.clone()) {
            if let Some((mut view, budget)) = self.comparison.cache.take(index, self.comparison.options) {
                // back where the user left it, unless asked for a particular place
                view.set_mode(self.comparison.view.mode());
                view.set_viewport_rows(self.comparison.view.viewport_rows());
                match place {
                    Place::Line(side, line) => view.go_to_line(side, line),
                    Place::FirstChange => view.jump_to_first_change(),
                    Place::LastChange => view.jump_to_last_change(),
                    Place::Top | Place::Anchor(_) => {}
                }
                let empty = view.change_count() == 0;
                (self.comparison.view, self.comparison.budget, self.comparison.diffed) = (view, budget, generation);
                self.refresh_markdown(cx);
                let skip = match place {
                    Place::FirstChange => Some(index + 1),
                    Place::LastChange => index.checked_sub(1),
                    _ => None,
                };
                if let Some(next) = skip.filter(|n| empty && *n < self.set.comparisons.len()) {
                    self.open(next, place, cx);
                }
                return;
            }
            self.show_text(&doc, place);
            self.diff(doc, place, cx);
            return;
        }
        let set = self.set.clone();
        let source = Arc::downgrade(&set);
        let (lp, rp) = self.paths(index);
        let current = Arc::downgrade(&self.comparison.work_id);
        cx.spawn(async move |this, cx| {
            let doc = cx
                .background_executor()
                .spawn(async move {
                    crate::work::document(current, move || Doc::load(&set, index, &lp, &rp, force)
                        .unwrap_or_else(|e| Doc { text: TextDocument::placeholder(&lp, &rp, Placeholder::Unreadable(e)), image: None }))
                })
                .await;
            let Some(doc) = doc else {
                this.update(cx,|viewer:&mut Viewer,cx| {
                    if source.ptr_eq(&Arc::downgrade(&viewer.set)) && viewer.comparison.generation==generation
                        && let Some(doc)=viewer.loaded.get(&index).map(|l|l.doc.clone()) {
                        viewer.show_text(&doc,place);viewer.diff(doc,place,cx);cx.notify();
                    }
                }).ok();
                return;
            };
            this.update(cx, |viewer: &mut Viewer, cx| {
                if !source.ptr_eq(&Arc::downgrade(&viewer.set)) || viewer.comparison.generation != generation {
                    return;
                }
                if let Some(Placeholder::Unreadable(error)) = doc.text.placeholder_kind() { crate::diagnostics::comparison_failure(error); }
                let (lp,rp)=viewer.paths(index);
                let draft=if doc.placeholder.is_some() {viewer.edits.recover(index,[&lp,&rp])}else{None}
                    .or_else(||viewer.edits.overlay(index,&doc.text,[&lp,&rp]));
                let doc=draft.map_or(doc,|text|Doc{text,image:None});
                let doc = Arc::new(doc);
                viewer.loaded.insert(index, Loaded::new(doc.clone(), generation));
                trim_loaded(&mut viewer.loaded, &mut viewer.comparison.cache, viewer.comparison.current, LOADED_CACHE_BYTES);
                viewer.trim_blames();
                viewer.show_text(&doc, place);
                viewer.diff(doc, place, cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// The text alone, uncoloured, while the diff runs (or when it gave up).
    fn show_text(&mut self, doc: &Doc, place: Place) { self.comparison.show_text(doc, place); }

    /// Re-diff the current Comparison under the current options, keeping the anchor line and Scroll Mode.
    fn recompare(&mut self, cx: &mut Context<Self>) {
        let Some(doc) = self.doc().cloned() else { return };
        self.comparison.begin();
        self.comparison.banner = None;
        self.diff(doc, Place::Anchor(self.comparison.view.anchor_line()), cx);
    }

    /// Diff in the background: the lines, then the shown blocks' Inline Changes. Past `DIFF_PATIENCE` it gives
    /// up for this file and options, showing the text alone under a banner; a late result is dropped.
    fn diff(&mut self, doc: Arc<Doc>, place: Place, cx: &mut Context<Self>) {
        if doc.placeholder.is_some() || doc.image.is_some() {
            self.comparison.diffed = self.comparison.generation;
            return;
        }
        let (generation, index, options, rows) = (self.comparison.generation, self.comparison.current, self.comparison.options, self.comparison.view.viewport_rows());
        let (mode, collapse) = (self.comparison.view.mode(), self.comparison.collapse);
        let folds = self.comparison.folds.get(&index).cloned().unwrap_or_default();
        let position=matches!(place,Place::Anchor(_)).then(||self.comparison.view.recompare_position());
        let work = doc.clone();
        let current = Arc::downgrade(&self.comparison.work_id);
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { crate::work::document(current, move || {
                    crate::comparison_session::prepare_with_position(&work, options, rows, mode, collapse, &folds, place,position)
                }) })
                .await;
            let Some((view, budget)) = result else { return };
            this.update(cx, |viewer: &mut Viewer, cx| {
                if viewer.comparison.generation != generation {
                    return; // the user moved on, or it took too long
                }
                if let Some(next) = viewer.comparison.complete(generation, view, budget, place, &viewer.set.comparisons) {
                    viewer.open(next, place, cx);
                } else { viewer.refresh_markdown(cx); }
                cx.notify();
            })
            .ok();
        })
        .detach();
        // The deadline must not keep a full evicted document alive for two seconds.
        let doc = Arc::downgrade(&doc);
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(DIFF_PATIENCE).await;
            this.update(cx, |viewer: &mut Viewer, cx| {
                if viewer.comparison.generation == generation && viewer.comparison.diffed != generation {
                    let Some(doc) = doc.upgrade() else { return };
                    viewer.comparison.expire(generation, &doc);
                    viewer.refresh_markdown(cx);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// Refine Change Blocks scrolled into view, one batch at a time, once the diff is in.
    fn refine_more(&mut self, cx: &mut Context<Self>) {
        if self.comparison.refining || self.comparison.diffed != self.comparison.generation || self.comparison.banner.is_some() {
            return;
        }
        let todo = self.comparison.refine_targets();
        let Some(doc) = self.doc().cloned().filter(|_| !todo.is_empty()) else { return };
        self.comparison.refining = true;
        let (generation, options, mut budget) = (self.comparison.generation, self.comparison.options, self.comparison.budget.clone());
        let current = Arc::downgrade(&self.comparison.work_id);
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { crate::work::document(current, move || {
                    let results = doc.refine(todo, &options, &mut budget);
                    (results, budget)
                }) })
                .await;
            this.update(cx, |viewer: &mut Viewer, cx| {
                viewer.comparison.refining = false;
                if viewer.comparison.generation == generation && let Some((results, budget)) = result {
                    viewer.comparison.view.apply_refinements(results);
                    viewer.comparison.budget = budget;
                    viewer.refresh_markdown(cx);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn parse_in_background(&mut self, index: usize, cx: &mut Context<Self>) {
        let sticky = self.sticky_on;
        let Some(loaded) = self.loaded.get_mut(&index).filter(|l| !l.parsing) else { return };
        loaded.parsing = true;
        let doc = loaded.doc.clone();
        let source = Arc::downgrade(&loaded.parse_id);
        for (side, lang) in [(0, doc.lang_left), (1, doc.lang_right)] {
            let Some(lang) = lang else { continue };
            let declarations = sticky && crate::highlight::has_declarations(lang);
            if loaded.parsed[side].is_some() && (!declarations || loaded.declarations[side].is_some()) { continue; }
            let text = if side == 0 { doc.text_left.clone() } else { doc.text_right.clone() };
            if text.is_empty() { continue; }
            let source = source.clone();
            cx.spawn(async move |this, cx| {
                let current = source.clone();
                let want = highlight::Want { folds: false, declarations };
                let syntax = cx.background_executor().spawn(async move {
                    highlight::parse_with(lang, text, want, move || current.strong_count() > 0)
                }).await;
                this.update(cx, |viewer: &mut Viewer, cx| {
                    if let Some(loaded) = viewer.loaded.get_mut(&index).filter(|l| source.ptr_eq(&Arc::downgrade(&l.parse_id))) {
                        let (parsed, declarations) = syntax.map_or((None, None), |s| (Some(Arc::new(s.parsed)), s.declarations.map(Arc::new)));
                        loaded.parsed[side] = parsed;
                        loaded.declarations[side] = declarations;
                        trim_loaded(&mut viewer.loaded, &mut viewer.comparison.cache, viewer.comparison.current, LOADED_CACHE_BYTES);
                        viewer.trim_blames();
                        cx.notify();
                    }
                })
                .ok();
            })
            .detach();
        }
    }

    /// Sticky scroll on or off; turning it on parses the shown file again when its declarations are missing.
    fn toggle_sticky(&mut self, cx: &mut Context<Self>) {
        self.sticky_on = !self.sticky_on;
        let index = self.comparison.current;
        // A side parsed without declarations is parsed again (spans come back with them), as the terminal does.
        if self.sticky_on && let Some(l) = self.loaded.get_mut(&index) {
            let langs = [l.doc.lang_left, l.doc.lang_right];
            let missing = (0..2).any(|s| langs[s].is_some_and(crate::highlight::has_declarations) && l.declarations[s].is_none());
            if missing {
                l.release_syntax();
                self.parse_in_background(index, cx);
            }
        }
    }

    /// The sticky line under a point in a text pane, as (its Side, its 0-based line).
    fn sticky_at(&self, window: &Window, x: f32, y: f32) -> Option<(Side, u32)> {
        let (pane, _) = self.pane_at(window, x)?;
        let row = ((y - self.top()) / line_h()).floor();
        if row < 0.0 { return None; }
        let lines = self.sticky_lines();
        let pane = self.pane_side(pane) as usize;
        lines.get(pane)?.iter().find(|s| s.row as f32 == row).map(|s| (s.side, s.line))
    }

    /// The header lines each pane pins (index 0 left, 1 right; Unified: index 1), while sticky scroll is on.
    fn sticky_lines(&self) -> [Vec<crate::sticky::StickyLine>; 2] {
        let Some(l) = self.loaded.get(&self.comparison.current).filter(|l| self.sticky_on && self.merge.is_none() && l.doc.placeholder.is_none()) else {
            return Default::default();
        };
        let rows = self.comparison.view.viewport_rows().max(0.0) as usize;
        let declarations = [l.declarations[0].as_deref(), l.declarations[1].as_deref()];
        crate::sticky::lines(&self.comparison.view.frame(), self.comparison.view.unified(), declarations, crate::sticky::cap(rows))
    }

    /// Next/previous Change Block; at the edge of the file a second press crosses into the
    /// next/previous file that has changes.
    fn step_change(&mut self, forward: bool, cx: &mut Context<Self>) {
        let command = if forward { Command::NextChange } else { Command::PreviousChange };
        match self.comparison.command(command, &self.set.comparisons, false, self.large()) {
            Transition::Repaint => {},
            Transition::Load { index, place, force: _ } => { self.toast.take(); self.open(index, place, cx); },
            Transition::Notice(message) => self.toast(message, cx),
            Transition::Presentation(_) => unreachable!("change navigation never changes presentation"),
            Transition::Recompare => unreachable!("change navigation never changes options"),
        }
    }

    /// Paint a brief message now and repaint when it expires.
    fn toast(&mut self, message: String, cx: &mut Context<Self>) {
        // A long message (a checkout's warning) stays long enough to read.
        let shown_for = TOAST_FOR.max(std::time::Duration::from_millis(50) * message.chars().count() as u32).min(std::time::Duration::from_secs(10));
        self.toast = Some((message, Instant::now()));
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(shown_for).await;
            this.update(cx, |viewer: &mut Viewer, cx| {
                if viewer.toast.as_ref().is_some_and(|(_, at)| at.elapsed() >= shown_for) {
                    viewer.toast = None;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// The File Navigator's width (0 when hidden).
    fn sidebar_w(&self) -> f32 {
        // a PR's Conversation and Commits read without the file list; Files changed has it
        if self.layout.files() && !self.pr_details_shown() { crate::pane_layout::clamp_size((self.panes.viewport[0]-self.picker_w()).max(0.0),self.panes.navigator,100.0,120.0) } else { 0.0 }
    }

    /// Window x where the File Navigator starts (right of the picker's sidebar).
    fn nav_x(&self) -> f32 {
        self.picker_w()
    }

    /// Window x where the diff panes start.
    fn diff_x(&self) -> f32 {
        self.nav_x() + self.sidebar_w()
    }

    fn in_nav(&self, x: f32) -> bool {
        x >= self.nav_x() && x < self.diff_x()
    }

    /// Window y where the panes' rows start: below the toolbar, the log (when shown) and the header.
    fn top(&self) -> f32 {
        toolbar_h() + self.log_h() + hdr_h()
    }

    /// Scroll the File Navigator so the current file's row is visible.
    fn reveal_in_nav(&mut self) {
        // the shown summary's heading, else the current file
        let row = if self.comparison.folders.summary().is_some() {
            self.comparison.folders.summary_row(&self.set, &self.nav)
        } else {
            self.nav.iter().position(|r| matches!(r, NavRow::File { index, .. } if *index == self.comparison.current))
        };
        let Some(row) = row else {
            return;
        };
        let visible = self.comparison.view.viewport_rows() - 1.0;
        let row = row as f32;
        if row < self.nav_scroll || row > self.nav_scroll + visible {
            self.nav_scroll = (row - visible / 3.0).max(0.0);
        }
    }

    /// ] / [: the next or previous file a collapsed folder doesn't hide.
    fn step_file(&mut self, forward: bool, cx: &mut Context<Self>) {
        match self.comparison.folders.step(&self.set.comparisons, self.comparison.current, forward) {
            Some(index) => self.open(index, Place::Top, cx),
            None => self.toast(format!("No {} visible file", if forward { "later" } else { "earlier" }), cx),
        }
    }

    /// Movement belongs to the File Navigator while it owns keyboard focus.
    fn nav_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        if !self.layout.files() || !self.nav_focused { return false; }
        // the keys walk files and folder headings, as a tree; the cursor is the shown summary's heading, else the file
        let summary = self.comparison.folders.summary();
        let cursor = if summary.is_some() {
            self.comparison.folders.summary_row(&self.set, &self.nav)
        } else {
            self.nav.iter().position(|r| matches!(r, NavRow::File { index, .. } if *index == self.comparison.current))
        };
        let walkable: Vec<usize> = (0..self.nav.len()).filter(|&r| !matches!(self.nav[r], NavRow::Group(..))).collect();
        let at = cursor.and_then(|row| walkable.iter().position(|&r| r == row));
        let step = |delta: isize| walkable.get(at.map_or(0, |a| a.saturating_add_signed(delta).min(walkable.len().saturating_sub(1)))).copied();
        let heading = cursor.and_then(|row| match &self.nav[row] { NavRow::Dir { path, collapsed, .. } => Some((path.clone(), *collapsed)), _ => None });
        let page = (self.comparison.view.viewport_rows() - 1.0).max(1.0) as isize;
        let plain = !ev.keystroke.modifiers.modified();
        let row = match ev.keystroke.key.as_str() {
            // ↓ from the last file gives the commit message panel the keyboard, headings above it or not
            "down" if summary.is_none() && self.nav.iter().rev().find_map(|r| if let NavRow::File { index, .. } = r { Some(*index) } else { None }) == Some(self.comparison.current) && self.enter_message_from_files(cx) => return true,
            "up" => step(-1),
            "down" => step(1),
            "pageup" => step(-page),
            "pagedown" | "space" => step(page),
            "home" => walkable.first().copied(),
            "end" => walkable.last().copied(),
            // on a heading Left collapses and Right expands it; Left on a file goes to its folder's heading
            "left" if plain => {
                match heading {
                    Some((path, false)) => self.toggle_folder(&path, cx),
                    Some(_) => {}
                    None => { self.comparison.folders.show_summary_of(&self.set, self.comparison.current); self.summary_scroll = 0.0; self.after_folders(cx); }
                }
                return true;
            }
            "right" if plain => { if let Some((path, true)) = heading { self.toggle_folder(&path, cx); } return true; }
            "left" | "right" if !(ev.keystroke.modifiers.alt && ev.keystroke.modifiers.shift) => return true,
            "enter" if heading.is_some() => { if let Some((path, _)) = heading { self.toggle_folder(&path, cx); } return true; }
            "enter" if self.repo.is_some() && !self.layout.focused(self.repo.as_ref()) => {
                self.nav_focused = false;
                self.layout_command(crate::layout::LayoutCommand::ToggleFocus, cx);
                return true;
            }
            "enter" | "escape" => { self.nav_focused = false; cx.notify(); return true; }
            _ => return false,
        };
        let Some(row) = row else { return true };
        match self.nav.get(row).cloned() {
            Some(NavRow::File { index, .. }) => {
                if self.pr_details_shown() { self.pr_command(crate::pr_session::PrCommand::Close, cx); }
                self.open(index, Place::Top, cx);
                self.reveal_in_nav();
            }
            Some(NavRow::Dir { path, .. }) => {
                let group = self.nav[..row].iter().filter(|r| matches!(r, NavRow::Group(..))).count().checked_sub(1);
                self.show_summary(&path, group, cx);
            }
            _ => {}
        }
        cx.notify();
        true
    }

    fn panel_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        if !self.panel_focused || self.panel_h() == 0.0 { return false; }
        let visible=self.panel_rows();
        let key = ev.keystroke.key.as_str();
        if key == "escape" {
            self.panel_focused = false;
        } else if matches!(key, "left" | "right") {
            return !(ev.keystroke.modifiers.alt && ev.keystroke.modifiers.shift);
        } else {
            let amount = match key {
                "up" => -1.0, "down" => 1.0,
                "pageup" => -(visible - 1.0).max(1.0), "pagedown" | "space" => (visible - 1.0).max(1.0),
                "home" => f32::NEG_INFINITY, "end" => f32::INFINITY,
                _ => return false,
            };
            if let Some(search) = &mut self.search {
                search.scroll_by(amount, visible);
            } else {
                let max = (self.edits.applied(self.comparison.current).len() as f32 - visible).max(0.0);
                self.applied_scroll = (self.applied_scroll + amount).clamp(0.0, max);
            }
        }
        cx.notify();
        true
    }

    fn cycle_pane(&mut self, backwards: bool, cx: &mut Context<Self>) {
        use navigator::InputPane as P;
        let current = if self.nav_focused { P::Files }
            else if self.panel_focused && self.panel_h() > 0.0 { P::Results }
            else if self.repo.as_ref().map(|r| &r.picker).is_some_and(|p| p.shown && p.focus == Pane::Sidebar) { P::Sources }
            else if self.repo.as_ref().map(|r| &r.picker).is_some_and(|p| p.shown && p.focus == Pane::Log) { P::Log }
            else if let Some(merge) = &self.merge { [P::Left, P::Middle, P::Right][merge.find_pane()] }
            else if self.sel.map_or(self.diff_focus, |s| s.side) == Side::Left { P::Left } else { P::Right };
        let panes = [(self.picker_w() > 0.0).then_some(P::Sources),
            (self.log_h() > 0.0).then_some(P::Log), self.layout.files().then_some(P::Files),
            Some(P::Left), self.merge.is_some().then_some(P::Middle), Some(P::Right),
            (self.panel_h() > 0.0).then_some(P::Results)];
        let Some(target) = navigator::cycle_pane(current, &panes, backwards) else { return; };
        self.nav_focused = target == P::Files;
        self.panel_focused = target == P::Results;
        if let Some(r) = self.repo.as_mut() {
            r.execute(crate::repository_session::RepositoryCommand::Focus(match target { P::Sources => Pane::Sidebar, P::Log => Pane::Log,
                P::Files | P::Left | P::Middle | P::Right | P::Results => Pane::Diff }));
        }
        match target {
            P::Left | P::Middle | P::Right => {
                if let Some(merge) = &mut self.merge {
                    merge.focus_pane(match target { P::Left => 0, P::Middle => 1, P::Right => 2, _ => unreachable!() });
                } else {
                    self.diff_focus = if target == P::Left { Side::Left } else { Side::Right };
                    if self.sel.is_some_and(|s| s.side != self.diff_focus) { self.sel = None; }
                }
            }
            P::Sources | P::Log | P::Files | P::Results => {}
        }
        cx.notify();
    }

    fn update_switcher(&mut self) {
        let Some(sw) = self.switcher.as_mut() else { return };
        let (query, _) = navigator::query_line(&sw.query);
        let paths: Vec<&str> = self.set.comparisons.iter().map(|c| c.path.as_str()).collect();
        sw.hits = navigator::rank(query, &paths);
        sw.selected = sw.selected.min(sw.hits.len().saturating_sub(1));
    }

    /// Keys while the switcher is open; returns whether it handled the key.
    fn switcher_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        let Some(sw) = self.switcher.as_mut() else { return false };
        let ks = &ev.keystroke;
        match ks.key.as_str() {
            "escape" => self.switcher = None,
            "enter" => {
                let target = sw.hits.get(sw.selected).copied();
                let (_, line) = navigator::query_line(&sw.query);
                self.switcher = None;
                if let Some(index) = target {
                    let place = line.map_or(Place::Top, |l| Place::Line(Side::Right, l));
                    self.open(index, place, cx);
                }
            }
            "down" => sw.selected = (sw.selected + 1).min(sw.hits.len().saturating_sub(1)),
            "up" => sw.selected = sw.selected.saturating_sub(1),
            "backspace" => {
                sw.query.pop();
                self.update_switcher();
            }
            _ if !(ks.modifiers.control || ks.modifiers.platform) => {
                let typed = ks.key_char.clone().or_else(|| (ks.key.chars().count() == 1).then(|| ks.key.clone()));
                if let Some(t) = typed {
                    sw.query.push_str(&t);
                    sw.selected = 0;
                    self.update_switcher();
                }
            }
            _ => {}
        }
        cx.notify();
        true
    }

    /// Raw lines of a Side of the shown Comparison.
    fn raw(&self, side: Side) -> &[String] {
        match self.doc() {
            Some(d) if side == Side::Left => &d.raw_left,
            Some(d) => &d.raw_right,
            None => &[],
        }
    }

    /// The Side whose pane holds `x`, and `x` relative to where that pane's text starts.
    fn pane_at(&self, window: &Window, x: f32) -> Option<(Side, f32)> {
        let sw = self.diff_x();
        let widths = self.comparison_widths(f32::from(window.viewport_size().width) - sw);
        let pane_w = widths[0];
        let rx = sw + pane_w + GUTTER_W;
        if x >= sw && x < sw + pane_w {
            Some((Side::Left, x - sw - self.ann_w(Side::Left) - num_w() - 8.0 + self.hscroll[Side::Left as usize]))
        } else if x >= rx && x < rx + widths[1] {
            Some((Side::Right, x - rx - self.ann_w(Side::Right) - num_w() - 8.0 + self.hscroll[Side::Right as usize]))
        } else {
            None
        }
    }

    /// The Side whose row is at window `y`: in Unified one pane holds both (deleted rows are the left Side's).
    fn side_at(&self, side: Side, y: f32) -> Side {
        if !self.comparison.view.unified() { return side; }
        let other = if side == Side::Left { Side::Right } else { Side::Left };
        if self.content_at(side, y).is_none() && self.content_at(other, y).is_some() { other } else { side }
    }

    /// The Side whose pane geometry (scroll, gutter) applies: Unified paints both in the right pane.
    pub(super) fn pane_side(&self, side: Side) -> Side { if self.comparison.view.unified() { Side::Right } else { side } }

    /// The Ctrl+F bar's text and the Side it searches.
    /// The text font's character width, measured on the first frame (7.8 px until then).
    fn text_char_w(&self) -> f32 {
        if self.char_w > 0.0 { self.char_w } else { 7.8 }
    }
    fn find_control(&self) -> Option<lenz_gpui::find::FindBox> {
        let f = self.comparison.find.as_ref()?;
        let side = self.pane_side(f.side);
        let widths = self.comparison_widths(self.panes.viewport[0] - self.diff_x());
        let x = if side == Side::Left { self.diff_x().min(self.panes.viewport[0]) } else { self.panes.viewport[0] - widths[1] };
        let pane = Bounds { origin: point(px(x), px(self.top())), size: size(px(widths[side as usize]), px((self.content_bottom() - self.top()).max(0.0))) };
        let o = self.comparison.find_options;
        Some(lenz_gpui::find::FindBox {
            bounds: Bounds::default(), toggles: [(FindOption::Case, Bounds::default()); 3],
            on: [o.case, o.word, o.regex], query: f.query.clone(), counter: f.counter(),
            error: f.error().is_some(), focused: self.modal() == Some(Modal::Find),
        }.layout(pane, line_h(), self.text_char_w()))
    }
    #[cfg(all(test, feature = "gui-test"))]
    pub(super) fn find_toggles(&self) -> Vec<(FindOption, (f32, f32, f32, f32))> {
        self.find_control().map_or_else(Vec::new, |b| b.toggles.into_iter().filter(|(_, b)| b.size.width > px(0.0)).map(|(o, b)| (o, (f32::from(b.origin.x), f32::from(b.origin.y), f32::from(b.size.width), f32::from(b.size.height)))).collect())
    }
    pub(super) fn find_bar_box(&self) -> Option<(f32, f32, f32, f32)> {
        let b = self.find_control()?.bounds;
        Some((f32::from(b.origin.x), f32::from(b.origin.y), f32::from(b.size.width), f32::from(b.size.height)))
    }
    /// The Find in Change Set prompt while the query is typed.
    pub(super) fn search_prompt(&self) -> Option<String> {
        self.search.as_ref().filter(|s| s.editing).map(|s| {
            let only = if s.changed_only { "on" } else { "off" };
            let error = s.query_error().map_or(String::new(), |e| format!("   {e}"));
            format!("Find in Change Set: {}▏   {TOGGLES}   alt-o changed lines only: {only}{error}", s.query)
        })
    }

    /// The Go to line box's text.
    fn goto_prompt(&self) -> Option<String> {
        if let Some(text) = self.git_prompt_text() {
            return Some(text);
        }
        let q = self.goto.as_ref()?;
        let side = if self.sel.is_some_and(|s| s.side == Side::Left) { "left" } else { "right" };
        Some(format!("Go to line[:column] on the {side} Side: {q}▏"))
    }

    /// Whether a point lies on the open Go to file or Go to line box (as painted).
    /// The Change Set search prompt's option toggles as painted: which option, and its box (x, y, width, height).
    pub(super) fn search_toggles(&self) -> Vec<(FindOption, (f32, f32, f32, f32))> {
        let (Some(prompt), None) = (self.search_prompt(), self.goto_prompt()) else { return Vec::new() };
        let Some(s) = self.search.as_ref() else { return Vec::new() };
        let [vw, _] = self.panes.viewport;
        let (x0, w) = (self.diff_x(), vw - self.diff_x());
        let bw = prompt.chars().count() as f32 * self.text_char_w() + 24.0;
        let (bx, by) = (x0 + (w - bw) / 2.0, toolbar_h() + self.log_h() + 60.0);
        let lead = "Find in Change Set: ".chars().count() + s.query.chars().count() + "▏   ".chars().count();
        toggle_spans().into_iter().map(|(o, at, len)| (o, (bx + 12.0 + (lead + at) as f32 * self.text_char_w() - 2.0, by + 2.0, len as f32 * self.text_char_w() + 4.0, line_h() + 8.0))).collect()
    }

    fn prompt_box_contains(&self, x: f32, y: f32) -> bool {
        let [vw, _] = self.panes.viewport;
        let top = toolbar_h() + self.log_h();
        let (bx, by, bw, bh) = if let Some(s) = &self.switcher {
            let (x0, w) = (self.picker_w(), vw - self.picker_w());
            let bw = 720.0f32.min(w - 40.0);
            (x0 + (w - bw) / 2.0, top + 60.0, bw, (s.hits.len() as f32 + 1.0) * line_h() + 16.0)
        } else if let Some(prompt) = self.goto_prompt().or_else(|| self.search_prompt()) {
            let (x0, w) = (self.diff_x(), vw - self.diff_x());
            let bw = prompt.chars().count() as f32 * self.text_char_w() + 24.0;
            (x0 + (w - bw) / 2.0, top + 60.0, bw, line_h() + 12.0)
        } else {
            return false;
        };
        x >= bx && x < bx + bw && y >= by && y < by + bh
    }

    /// What the row at window `y` on `side` shows.
    /// A point inside `line` of `side` as painted, for adapter tests.
    #[cfg(all(test,feature="gui-test"))]
    pub(super) fn point_on_line(&self,window:&Window,side:Side,line:u32)->(f32,f32){
        let frame=self.comparison.view.frame();
        let rows=if side==Side::Left{frame.left}else{frame.right};
        let row=rows.iter().find(|r|matches!(r.content,Content::Line(l) if l==line)).expect("line painted");
        let (start,_)=self.pane_span(window,side);
        (start+12.,self.top()+(row.y+0.5)*line_h())
    }

    fn content_at(&self, side: Side, y: f32) -> Option<Content> {
        let frame = self.comparison.view.frame();
        let rows = if side == Side::Left { frame.left } else { frame.right };
        let at = (y - self.top()) / line_h();
        rows.iter().find(|r| (r.y..r.y + 1.0).contains(&at)).map(|r| r.content)
    }

    /// The caret position under the pointer: `line` of `side`, `x` from the text start.
    fn pos_at(&self, window: &Window, side: Side, line: u32, x: f32) -> Pos {
        let raw = self.raw(side).get(line as usize).map_or("", String::as_str);
        Pos::new(line, lenz_gpui::column_at(window, metrics(&self.font), raw, x))
    }

    fn menu_item(&mut self,menu:&Menu,i:usize,_window:&mut Window,cx:&mut Context<Self>){
                match &menu.items[i] {
                    (_,MenuAction::File(action,index,set))=>self.file_action(*action,*index,set,cx),
                    (_, MenuAction::TextCopy(command,side,selection,generation)) => {
                        if *generation!=self.comparison.generation{self.toast("Comparison changed — reopen the menu".into(),cx);}
                        else if let Some(doc)=self.doc(){match commands::copy_text(*command,commands::CopyContext{set:&self.set,index:self.comparison.current,doc,view:&self.comparison.view,selection:*selection,side:*side,path_label:self.name.as_deref()}){Ok(text)=>cx.write_to_clipboard(ClipboardItem::new_string(text)),Err(reason)=>self.toast(reason.into(),cx)}}
                    }
                    (_, MenuAction::Run(action))=>self.run_action(*action,_window,cx),
                    (_, MenuAction::Source(entry))=>self.source_entry(entry.clone(),_window,cx),
                    (_, MenuAction::PrLine(side))=>{match self.comparison.markdown_inspection_position(*side){Ok(Some(p))=>{if let Some(threads)=self.threads_at(*side,p.line){self.pr_command(crate::pr_session::PrCommand::ShowThreads(threads),cx);}},Err(reason)=>self.toast(reason.into(),cx),Ok(None)=>{}}},
                    (_, MenuAction::RepositoryRefresh)=>self.refresh_repository(cx),
                    (_, MenuAction::ToggleBlame) => self.toggle_blame(cx),
                    (_, MenuAction::Command(command)) => self.comparison_command(*command, cx),
                    (_, MenuAction::InteractionCommand(command))=>self.interaction(*command,cx),
                    (_, MenuAction::Fold(i)) => self.fold(Some(*i), cx),
                    (_, MenuAction::Move(i, onto)) => self.move_change(*i, *onto, cx),
                    (_, MenuAction::Merge(c)) => self.merge_control(*c, cx),
                    (_, MenuAction::Open(index, side, line, default_app)) => self.open_file(*index, *side, *line, None, *default_app, cx),
                    (_, MenuAction::MarkResolved(path)) => self.mark_resolved(path.clone(), cx),
                    (_, MenuAction::Reopen(path)) => self.reopen_conflict(path.clone(), cx),
                    (_, MenuAction::Clipboard(text)) => { cx.write_to_clipboard(ClipboardItem::new_string(text.clone())); self.toast("Copied".into(), cx); }
                    (_, action) => {
                        self.picker_menu_action(action, cx);
                        // a submenu opens where the menu was
                        if let Some(m) = self.menu.as_mut() {
                            (m.x, m.y) = (menu.x, menu.y);
                        }
                    }
                }
    }

    /// The Ctrl+F bar stays while it is clicked; any other click does what it would and then closes the bar.
    fn on_mouse_down(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let (x, y) = (f32::from(ev.position.x), f32::from(ev.position.y));
        let outside_find = self.find_bar_box().is_some_and(|(bx, by, bw, bh)| !(x >= bx && x < bx + bw && y >= by && y < by + bh));
        // A find option toggle in the find bar or the Change Set search prompt flips that option; either stays.
        if ev.button == MouseButton::Left && matches!(self.modal(), Some(Modal::Find | Modal::SearchEditing)) && let Some(option) = self.find_control().and_then(|b| b.option_at(ev.position)).or_else(|| self.search_toggles().into_iter().find(|(_, (tx, ty, tw, th))| x >= *tx && x < tx + tw && y >= *ty && y < ty + th).map(|(o, _)| o)) {
            self.toggle_find_option(option);
            cx.notify();
            return;
        }
        if self.modal() == Some(Modal::Find) && self.find_control().is_some_and(|b| b.bounds.contains(&ev.position)) {
            return;
        }
        // a press on the panel takes the keyboard back
        (self.message_focused, self.message_press) = (false, None);
        if ev.button == MouseButton::Left { self.message_sel = None; }
        self.mouse_down(ev, window, cx);
        if outside_find && self.comparison.find.is_some() {
            self.comparison.find = None;
            cx.notify();
        }
    }

    fn mouse_down(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.resize_blocked(){self.panes.drag=None;}
        if self.discard_modal(){return;}
        if self.theme_click(ev,window,cx) { return; }
        // dialogs close from their Close bar or from a click outside them
        if self.about.is_some(){if about::About::close_bounds(window.viewport_size()).contains(&ev.position)||!about::About::contains(window.viewport_size(),ev.position){self.close_about(window);cx.notify();}return;}
        if self.help {let ((hx,hy,hw,hh),_)=help_layout(window.viewport_size());let (px_,py_)=(f32::from(ev.position.x),f32::from(ev.position.y));
            if help_close_bounds(window.viewport_size()).contains(&ev.position)||!(px_>=hx&&px_<hx+hw&&py_>=hy&&py_<hy+hh){self.help=false;cx.notify();}return;}
        if self.palette_click(f32::from(ev.position.x),f32::from(ev.position.y),window,cx)||self.diff_options_click(f32::from(ev.position.x),f32::from(ev.position.y),window,cx){return;}
        if self.modal().is_none()&&self.header_switch_click(f32::from(ev.position.x),f32::from(ev.position.y),window,cx){return;}
        if self.modal().is_none()&&self.panel_switch_click(f32::from(ev.position.x),f32::from(ev.position.y),window,cx){return;}
        if self.merge.as_ref().is_some_and(|s| s.ask.is_some()) || self.picker_v.as_ref().is_some_and(|s| s.ask.is_some()){return;}
        let (x, y) = (f32::from(ev.position.x), f32::from(ev.position.y));
        if self.begin_resize(x,y) { cx.notify(); return; }
        let band_top = self.content_bottom() - self.panel_h() - self.picker_v_height();
        if self.picker_v.is_some() && self.menu.is_none() && x >= self.diff_x() && y >= band_top && self.picker_v_click(y - band_top, cx) {
            return;
        }
        if self.merge.is_some() && self.menu.is_none() && x >= self.diff_x() && y >= self.top() - hdr_h() {
            self.nav_focused = false;
            self.panel_focused = false;
            self.merge_click(x - self.diff_x(), y - self.top(), cx);
            return;
        }
        if let Some(menu) = self.menu.take() {
            if let Some(i) = menu.item_at(x, y, self.char_w,self.panes.viewport) {
                self.menu_item(&menu,i,window,cx);
            }
            cx.notify();
            return;
        }
        if self.popup.is_some() {
            let viewport=[f32::from(window.viewport_size().width),f32::from(window.viewport_size().height)];
            let rows=self.popup_rows(viewport).len();
            let ((bx,by,bw,bh),buttons,_)=popup_layout(rows,viewport,self.char_w);
            let hit=buttons.iter().position(|(bx,by,w,h)|x>=*bx&&x<bx+w&&y>=*by&&y<by+h);
            match hit {
                Some(i) => self.popup_action(POPUP_BUTTONS[i].0, cx),
                None if !(x >= bx && x < bx + bw && y >= by && y < by + bh) => self.popup = None,
                None => {}
            }
            cx.notify();
            return;
        }
        // Go to file / line, Find in Change Set while typed: a click outside the box closes it
        if self.switcher.is_some() || self.goto.is_some() || self.git_prompt.is_some() || self.search.as_ref().is_some_and(|s| s.editing) {
            if !self.prompt_box_contains(x, y) {
                self.switcher = None;self.goto = None;self.git_prompt = None;
                if self.search.as_ref().is_some_and(|s| s.editing) && let Some(s) = self.search.take() {s.cancel.store(true, std::sync::atomic::Ordering::Relaxed);}
                cx.notify();
            }
            return;
        }
        if self.message_click(x, y, cx) {
            self.nav_focused = false;
            self.panel_focused = false;
            return;
        }
        // a details tab takes clicks over the diff area once every overlay has had its turn
        if self.switcher.is_none() && self.goto.is_none() && self.pr_dialog_click(ev, cx) {return;}
        if self.switcher.is_none() && self.goto.is_none() && self.summary_click(f32::from(ev.position.x), f32::from(ev.position.y), cx) {return;}
        if self.switcher.is_none() && self.goto.is_none() && self.picker_click(x, y, ev, cx) {
            self.nav_focused = false;
            self.panel_focused = false;
            return;
        }
        if self.in_nav(x) && (self.top() - hdr_h()..self.top()).contains(&y) {
            self.nav_focused = true;
            self.panel_focused = false;
            cx.notify();
            if self.nav_header_click(cx) { return; }
        }
        if y < self.top() {
            return;
        }
        if !self.markdown_active() && let Some(side) = self.ann_at(window, x) {
            if let Some(Content::Line(line)) = self.content_at(side, y) {
                self.open_commit(side, line, cx);
                cx.notify();
            }
            return;
        }
        let panel_top = self.content_bottom() - self.panel_h();
        if y >= panel_top {
            self.nav_focused = false;
            self.panel_focused = self.panel_h() > 0.0;
            self.panel_click(y, panel_top, cx);
            cx.notify();
            return;
        }
        if self.in_nav(x) {
            self.nav_focused = true;
            self.panel_focused = false;
            self.dragging = false;
            cx.notify();
            let row = ((y - self.nav_top()) / line_h() + self.nav_scroll) as usize;
            if let Some(NavRow::File { index, .. }) = self.nav.get(row) {
                let index = *index;
                // the Viewed Mark box at the start of the row
                if self.repo.as_ref().is_some_and(|r| r.pr.is_some()) && x < self.nav_x() + 16.0 + 8.0 + 2.0 * self.char_w {
                    self.toggle_viewed(index, false, cx);
                    return;
                }
                if self.pr_details_shown() { self.pr_command(crate::pr_session::PrCommand::Close, cx); }
                self.open(index, Place::Top, cx);
                if ev.click_count >= 2 && self.repo.is_some() && !self.layout.focused(self.repo.as_ref()) {
                    self.layout_command(crate::layout::LayoutCommand::ToggleFocus, cx);
                }
                cx.notify();
            } else if let Some(NavRow::Dir { path, .. }) = self.nav.get(row) {
                // the arrow collapses or expands; the name shows the folder's summary
                let path = path.clone();
                if x < self.nav_x() + 8.0 + (2.0 * self.char_w).max(16.0) {
                    self.toggle_folder(&path, cx);
                } else {
                    let group = self.nav[..row].iter().filter(|r| matches!(r, NavRow::Group(..))).count().checked_sub(1);
                    self.show_summary(&path, group, cx);
                }
            }
            return;
        }
        self.nav_focused = false;
        self.panel_focused = false;
        if self.markdown_click(ev,window,cx){return;}
        if self.image_doc().is_some() && self.image_drag(x, y, true, window, cx) {
            return;
        }
        if let Some((side, line)) = self.sticky_at(window, x, y) {
            self.comparison.view.go_to_line(side, line + 1);
            cx.notify();
            return;
        }
        let Some((side, tx)) = self.pane_at(window, x).map(|(side, tx)| (self.side_at(side, y), tx)) else {
            // the gutter: ≫ / ≪ beside a Connector move its block, the chevron (or the band) folds or unfolds it
            let at = (y - self.top()) / line_h();
            let gx = self.diff_x() + self.comparison_widths(f32::from(window.viewport_size().width) - self.diff_x())[0];
            let writable = self.doc().map_or([false, false], |d| d.writable.each_ref().map(|w| w.is_ok()));
            let arrow = self.comparison.view.frame().connectors.iter().find_map(|c| {
                let (to_right, to_left) = arrow_rows(c);
                if writable[1] && x < gx + ARROW_W && (at - to_right).abs() < 0.5 {
                    Some((c.block, Side::Right))
                } else if writable[0] && x >= gx + GUTTER_W - ARROW_W && (at - to_left).abs() < 0.5 {
                    Some((c.block, Side::Left))
                } else {
                    None
                }
            });
            if let Some((block, onto)) = arrow {
                self.move_change(block, onto, cx);
                cx.notify();
                return;
            }
            let hit = self.comparison.view.frame().connectors.into_iter().find(|c| {
                let (top, bottom) = (c.left.0.min(c.right.0), c.left.1.max(c.right.1).max(c.left.0.min(c.right.0) + 1.0));
                c.foldable && (top..bottom).contains(&at)
            });
            if let Some(c) = hit {
                self.fold(Some(c.block), cx);
            }
            return;
        };
        self.diff_focus = side;
        if self.sel.is_some_and(|s| s.side != side) { self.sel = None; }
        if let Some(Content::Folded { block, .. }) = self.content_at(side, y) {
            self.fold(Some(block as usize), cx);
            return;
        }
        if self.large() {
            self.load(self.comparison.current, Place::Top, true, cx);
            return;
        }
        if let (true, Some(Content::Line(line))) = (tx < 0.0, self.content_at(side, y))
            && let Some(threads) = self.threads_at(side, line)
        {
            if let Some(s) = self.repo.as_mut().and_then(|r| r.pr.as_mut()) {
                s.execute(crate::pr_session::PrCommand::ShowThreads(threads));
            }
            cx.notify();
            return;
        }
        match self.content_at(side, y) {
            // a click on a Collapsed Region expands it
            Some(Content::Fold { id, .. }) => self.comparison.view.expand(id),
            Some(Content::Line(line)) => {
                let pos = self.pos_at(window, side, line, tx);
                let lines = self.raw(side);
                // Caret, word, group, then again.
                let clicks = (ev.click_count.max(1) - 1) % 3 + 1;
                self.sel = Some(match (self.sel, clicks) {
                    (Some(mut sel), _) if ev.modifiers.shift && sel.side == side => {
                        sel.head = pos;
                        sel
                    }
                    (_, 2 | 3) => Selection::word(side, lines, pos),
                    _ => Selection::caret(side, pos),
                });
                // A third click: the word's syntax group, found off the UI thread; kept only while the word is.
                if clicks == 3 && let (Some(word), Some(doc)) = (self.sel, self.doc()) {
                    let job = crate::selection::group_job(word, self.raw(side), doc.text_arc(side), doc.language(side));
                    cx.spawn(async move |this, cx| {
                        let group = cx.background_executor().spawn(async move { job() }).await;
                        this.update(cx, |v: &mut Viewer, cx| {
                            if v.sel == Some(word) {
                                v.sel = Some(group);
                                cx.notify();
                            }
                        }).ok();
                    }).detach();
                }
                self.dragging = true;
            }
            _ => {}
        }
        self.comparison.press_again.reset();
        cx.notify();
    }

    /// Window x range of a Side's text area.
    fn pane_span(&self, window: &Window, side: Side) -> (f32, f32) {
        let side = self.pane_side(side);
        let sw = self.diff_x();
        let widths = self.comparison_widths(f32::from(window.viewport_size().width) - sw);
        let pane_w = widths[0];
        let x0 = if side == Side::Left { sw } else { sw + pane_w + GUTTER_W };
        (x0 + self.ann_w(side) + num_w(), x0 + widths[side as usize])
    }

    /// While a drag selection sits past a pane edge: scroll every 30 ms and pull the selection along.
    fn drag_tick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(std::time::Duration::from_millis(30)).await;
                let go = this
                    .update_in(cx, |v: &mut Viewer, window, cx| {
                        let (x, rows, px) = v.drag;
                        let Some(sel) = v.sel.filter(|_| v.dragging && (rows != 0.0 || px != 0.0)) else {
                            v.drag_ticking = false;
                            return false;
                        };
                        v.comparison.view.scroll_by(rows, sel.side);
                        v.scroll_horizontal(v.pane_side(sel.side), px);
                        // the selection's head follows to the first or last line on screen
                        let frame = v.comparison.view.frame();
                        let shown = if sel.side == Side::Left { frame.left } else { frame.right };
                        let max = v.comparison.view.viewport_rows() - 1.0;
                        let lines = shown.iter().filter(|r| r.y >= 0.0 && r.y <= max).filter_map(|r| match r.content {
                            Content::Line(l) => Some(l),
                            _ => None,
                        });
                        let edge = if rows < 0.0 { lines.min() } else if rows > 0.0 { lines.max() } else { Some(sel.head.line) };
                        if let Some(line) = edge {
                            let tx = v.pane_at(window, x).filter(|(s, _)| *s == v.pane_side(sel.side)).map_or(if px < 0.0 { 0.0 } else { f32::MAX }, |p| p.1);
                            v.sel = Some(Selection { head: v.pos_at(window, sel.side, line, tx), ..sel });
                        }
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);
                if !go {
                    break;
                }
            }
        })
        .detach();
    }

    fn on_mouse_up(&mut self, _ev: &MouseUpEvent, _window: &mut Window, cx: &mut Context<Self>) {
        self.message_release(cx);
        self.pr_dialog_release();
        self.panes.drag=None;
        self.markdown.dragging=false;
        if self.theme_chooser.is_some(){return;}
        self.iview.drag = None;
        if std::mem::take(&mut self.dragging) {
            self.selection_to_primary(cx);
        }
    }

    /// Linux primary selection follows the selection.
    fn selection_to_primary(&self, cx: &mut Context<Self>) {
        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        if let Some(sel) = self.sel.filter(|s| !s.is_empty()) {
            cx.write_to_primary(ClipboardItem::new_string(sel.text(self.raw(sel.side))));
        }
        let _ = cx;
    }

    fn on_right_down(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        // an open menu is replaced by the new one
        if self.modal().is_some_and(|m|m!=Modal::Menu){return;}
        // the diff under a details tab is hidden: no menu for it
        if self.pr_dialog_menu(ev, cx){return;}
        let (x, y) = (f32::from(ev.position.x), f32::from(ev.position.y));
        if self.switcher.is_none() && self.message_menu(x, y, cx) {
            return;
        }
        if self.merge.is_some() && x >= self.diff_x() && y >= self.top() {
            if let Some(items) = self.merge_menu(x - self.diff_x(), y - self.top()) {
                self.show_menu(Menu { x, y, items, hover: None });
                cx.notify();
            }
            return;
        }
        if self.picker_menu(x, y, cx) {
            return;
        }
        if self.in_nav(x) && y >= self.nav_top() && self.switcher.is_none() {
            let row = ((y - self.nav_top()) / line_h() + self.nav_scroll) as usize;
            if let Some(NavRow::File { index, .. }) = self.nav.get(row) {
                let index = *index;
                let deleted = self.set.comparisons[index].status == Status::Deleted;
                let side = if deleted { Side::Left } else { Side::Right };
                let mut items = self.conflict_menu(index);
                items.extend(self.file_menu(index));
                items.extend([
                    ("Open with Default App".to_string(), MenuAction::Open(index, side, None, true)),
                ]);
                if self.repo.is_some() && let Spec::Git { cached, revs, .. } = &self.spec {
                        let mut args: Vec<String> = cached.then(|| "--staged".to_string()).into_iter().chain(revs.iter().cloned()).collect();
                        args.extend(["--".to_string(), self.set.comparisons[index].path.clone()]);
                        items.push(("Open in New Window".to_string(), MenuAction::Spawn(args)));
                }
                self.show_menu(Menu { x, y, items, hover: None });
                cx.notify();
            }
            return;
        }
        let Some((side, _)) = self.pane_at(window, x).filter(|_| y >= self.top() && self.switcher.is_none()).map(|(side, tx)| (self.side_at(side, y), tx)) else { return };
        if self.set.comparisons.get(self.comparison.current).is_none(){return;}
        self.diff_focus=side;
        if self.markdown_active(){
            self.markdown_context_pointer(side,x,y,window);
            let mut items=vec![(crate::catalog::pointer::SHOW_IN_SOURCE.into(),MenuAction::Command(Command::SetPresentation(crate::markdown::PresentationMode::Source)))];
            for command in [commands::Interaction::Copy,commands::Interaction::Editor] {let label=self.interaction_disabled(command).map_or_else(||command.label().into(),|why|format!("{} ({why})",command.label()));items.push((label,MenuAction::InteractionCommand(command)));}
            if self.blame_key(side).is_some(){let command=commands::Interaction::InspectBlame;let label=self.interaction_disabled(command).map_or_else(||command.label().into(),|why|format!("{} ({why})",command.label()));items.push((label,MenuAction::InteractionCommand(command)));}
            if self.repo.as_ref().is_some_and(|r| r.pr.is_some()){let label=self.comparison.markdown_inspection_position(side).err().map_or_else(||"Show review threads at source line".into(),|why|format!("Show review threads at source line ({why})"));items.push((label,MenuAction::PrLine(side)));}
            self.show_menu(Menu{x,y,items,hover:None});cx.notify();return;
        }
        if self.sel.is_none_or(|s|s.is_empty())&&let Some(Content::Line(line))=self.content_at(side,y){
            let tx=self.pane_at(window,x).map_or(0.,|(_,tx)|tx);
            self.sel=Some(Selection::caret(side,self.pos_at(window,side,line,tx)));
        }
        // only what is under the pointer; everything else is in the palette
        let mut items = Vec::new();
        if let Some(doc)=self.doc().filter(|_|self.image_doc().is_none()&&self.sel.is_some_and(|s|!s.is_empty())){
            for command in [commands::Interaction::Copy,commands::Interaction::CopyReference]{
                let context=commands::CopyContext{set:&self.set,index:self.comparison.current,doc,view:&self.comparison.view,selection:self.sel,side,path_label:self.name.as_deref()};
                if commands::copy_disabled(command,context).is_none(){items.push((command.label().into(),MenuAction::TextCopy(command,side,self.sel,self.comparison.generation)));}
            }
        }
        let block = match self.content_at(side, y) {
            Some(Content::Line(line)) => self.comparison.view.block_index(side, line),
            Some(Content::Folded { block, .. }) => Some(block as usize),
            _ => None,
        };
        if let Some(i) = block {
            if self.comparison.view.can_fold(i) {
                let label = crate::catalog::pointer::fold(self.comparison.view.folded(i));
                items.push((label.into(), MenuAction::Fold(i)));
            }
            let writable = self.doc().map_or([false, false], |d| d.writable.each_ref().map(|w| w.is_ok()));
            if writable[1] {
                items.push((crate::catalog::pointer::move_change(Side::Right).into(), MenuAction::Move(i, Side::Right)));
            }
            if writable[0] {
                items.push((crate::catalog::pointer::move_change(Side::Left).into(), MenuAction::Move(i, Side::Left)));
            }
        }
        let at = match self.content_at(side, y) {
            Some(Content::Line(l)) => Some(l as usize + 1),
            _ => None,
        };
        items.push((commands::Interaction::Editor.label().into(), MenuAction::Open(self.comparison.current, side, at, false)));
        if self.blame_on {
            if self.interaction_disabled(commands::Interaction::InspectBlame).is_none() {
                items.push((commands::Interaction::InspectBlame.label().into(), MenuAction::InteractionCommand(commands::Interaction::InspectBlame)));
            }
            items.push((crate::catalog::pointer::HIDE_ANNOTATIONS.into(), MenuAction::ToggleBlame));
        } else if self.set.comparisons.get(self.comparison.current).is_some_and(|c|blame::version(&self.spec,c,side).is_some()) {
            items.push((crate::catalog::pointer::ANNOTATE_WITH_BLAME.into(), MenuAction::ToggleBlame));
        }
        let w = f32::from(window.viewport_size().width);
        self.show_menu(Menu { x: x.min(w - MENU_W - 4.0), y, items, hover: None });
        cx.notify();
    }

    fn on_mouse_move(&mut self, ev: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.resize_blocked(){self.panes.drag=None;}
        if self.modal().is_some_and(|m|matches!(m,Modal::Question|Modal::Discard|Modal::Checkout|Modal::About|Modal::Theme|Modal::Help)){return;}
        let (x, y) = (f32::from(ev.position.x), f32::from(ev.position.y));
        if let Some(menu) = self.menu.as_mut() {
            let hover = menu.item_at(x, y, self.char_w,self.panes.viewport);
            if hover != menu.hover {
                menu.hover = hover;
                cx.notify();
            }
            return;
        }
        if self.markdown_active()&&(self.help||self.goto.is_some()||self.git_prompt.is_some()||self.switcher.is_some()||self.popup.is_some()){return;}
        if self.resize_move(x,y,ev.pressed_button==Some(MouseButton::Left),cx){return;}
        if ev.pressed_button==Some(MouseButton::Left) && (self.message_drag(x,y,cx) || self.pr_dialog_drag(x,y,cx)) {return;}
        if self.markdown_drag(ev,window,cx){return;}
        if self.image_doc().is_some() && self.image_drag(x, y, ev.pressed_button == Some(MouseButton::Left), window, cx) {
            return;
        }
        if self.dragging && ev.pressed_button == Some(MouseButton::Left) {
            let Some(sel) = self.sel else { return };
            // the drag stays on the Side it started on
            let pane = self.pane_at(window, x).filter(|(side, _)| *side == self.pane_side(sel.side));
            let tx = pane.map_or(if x < self.diff_x() + 1.0 { 0.0 } else { f32::MAX }, |(_, tx)| tx);
            if let Some(Content::Line(line)) = self.content_at(sel.side, y) {
                let head = self.pos_at(window, sel.side, line, tx);
                if head != sel.head {
                    self.sel = Some(Selection { head, ..sel });
                    cx.notify();
                }
            }
            // past an edge of the pane, the view keeps scrolling while the button is held
            let bottom = self.content_bottom() - self.panel_h() - self.picker_v_height();
            let (start, end) = self.pane_span(window, sel.side);
            let top = self.top();
            let rows = (if y < top { y - top } else if y > bottom { y - bottom } else { 0.0 }) / line_h();
            let px = if x < start { x - start } else if x > end { x - end } else { 0.0 };
            self.drag = (x, (rows * 0.5).clamp(-6.0, 6.0), (px * 0.5).clamp(-60.0, 60.0));
            if (self.drag.1 != 0.0 || self.drag.2 != 0.0) && !self.drag_ticking {
                self.drag_ticking = true;
                self.drag_tick(window, cx);
            }
            return;
        }
        let hover = (self.in_nav(x) && y >= self.nav_top() && y < self.content_bottom()).then(|| ((y - self.nav_top()) / line_h() + self.nav_scroll) as usize);
        if hover != self.nav_hover {
            self.nav_hover = hover;
            cx.notify();
        }
    }

    fn copy(&mut self, cx: &mut Context<Self>) {
        if self.message_copy(cx) { return; }
        if self.comparison.markdown_mode()==crate::markdown::PresentationMode::Rendered { self.interaction(commands::Interaction::Copy,cx);return; }

        let Some(sel) = self.sel.filter(|s| !s.is_empty()) else { return };
        let text = sel.text(self.raw(sel.side));
        let lines = text.lines().count().max(1);
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.toast(format!("Copied {lines} line{}", if lines == 1 { "" } else { "s" }), cx);
    }

    /// Caret keys on the focused Side; false when `key` isn't one.
    fn caret_key(&mut self, key: &str, shift: bool, cmd: bool) -> bool {
        let Some(mut sel) = self.sel else { return false };
        use crate::selection::CaretCommand;
        let command=match key{"left"=>CaretCommand::Left,"right"=>CaretCommand::Right,"up"=>CaretCommand::Up,"down"=>CaretCommand::Down,"pageup"=>CaretCommand::PageUp,"pagedown"=>CaretCommand::PageDown,"home"=>CaretCommand::Home(cmd),"end"=>CaretCommand::End(cmd),_=>return false};
        let Some(doc)=self.doc().cloned()else{return false;};sel=self.comparison.move_source_caret(&doc,sel,command,shift);self.sel=Some(sel);
        self.reveal_column(sel.side, sel.head);
        true
    }

    /// Scroll sideways just enough to show `at`.
    fn reveal_column(&mut self, side: Side, at: Pos) {
        let line = self.raw(side).get(at.line as usize).map_or("", String::as_str);
        let end=at.col.min(line.len());let cols = horizontal::line_columns(&line[..end]);
        let x = cols as f32 * self.char_w;
        let margin = 4.0 * self.char_w;
        let width=(self.text_w[side as usize]-self.ann_w(side)).max(0.0);
        let offset = &mut self.hscroll[side as usize];
        if x < *offset + margin {
            *offset = (x - margin).max(0.0);
        } else if x > *offset + width - margin {
            *offset = x - width + margin;
        }
        self.clamp_horizontal(side);
    }

    /// Typed text for prompts: the character a key produces, if any.
    fn typed(ev: &KeyDownEvent) -> Option<String> {
        let ks = &ev.keystroke;
        if ks.modifiers.control || ks.modifiers.platform || ks.modifiers.alt {
            return None;
        }
        match ks.key.as_str() {
            "space" => Some(" ".into()),
            k => ks.key_char.clone().or_else(|| (k.chars().count() == 1).then(|| k.to_string())),
        }
    }

    /// Open the find bar on the focused Side (right if none), seeded with a one-line selection.
    fn open_find(&mut self) {
        let side = self.diff_focus;
        let seed = if self.markdown_active(){self.comparison.markdown_selection().and_then(|selection|self.comparison.markdown_projection()?.copy_visible(selection).ok()).filter(|s|!s.is_empty()&&!s.contains('\n'))}else{self.sel.filter(|s| s.side == side && !s.is_empty() && s.anchor.line == s.head.line).map(|s| s.text(self.raw(side)))};
        let query = seed.or_else(|| self.comparison.find.as_ref().map(|f| f.query.clone())).unwrap_or_default();
        self.comparison.find = Some(FindBar { query, side, hits: Vec::new(), at: None, error: None, capped: false });
        self.refind();
    }

    /// A find, merge find or Change Set search input is open: the find option toggles apply to it.
    pub(super) fn find_open(&self) -> bool {
        self.comparison.find.is_some() || self.search.as_ref().is_some_and(|s| s.editing) || self.merge.as_ref().is_some_and(|m| m.find_editing())
    }

    /// Flip one find option for the open input (Change Set search, merge find or the find bar); the window keeps it.
    pub(super) fn toggle_find_option(&mut self, option: FindOption) {
        if let Some(s) = self.search.as_mut().filter(|s| s.editing) {
            s.toggle_option(option);
            self.comparison.find_options = s.options();
        } else if let Some(m) = self.merge.as_mut().filter(|m| m.find_editing()) {
            self.comparison.find_options = m.toggle_find_option(option);
        } else if let Some(doc) = self.doc().cloned() && self.comparison.find.is_some() {
            let side = self.comparison.find.as_ref().map_or(Side::Right, |f| f.side);
            if let Some((line, range)) = self.comparison.toggle_find_option(&doc, option) { self.select_hit(side, line, range); }
        } else {
            self.comparison.find_options = self.comparison.find_options.toggled(option);
        }
    }

    /// Recompute the find bar's hits and show the first at or after the caret.
    fn refind(&mut self) {
        let Some(f) = self.comparison.find.as_ref() else { return };
        let (side, query) = (f.side, f.query.clone());
        let from = match self.sel.filter(|s| s.side == side) {
            Some(sel) => sel.ordered().0,
            None => Pos::new(self.comparison.view.anchor_line() as u32, 0),
        };
        if let Some(doc)=self.doc().cloned() && let Some((line,range))=self.comparison.search(&doc,query,side,from){self.select_hit(side,line,range);}
    }

    /// Show a match: expanded, at the anchor, selected.
    fn select_hit(&mut self, side: Side, line: u32, r: Range<usize>) {
        if self.markdown_active(){self.diff_focus=side;return;}
        let sel = Selection { side, anchor: Pos::new(line, r.start), head: Pos::new(line, r.end) };
        let shown = self.frame_has(side, line);
        if !shown {
            self.comparison.view.go_to_line(side, line + 1);
        }
        self.sel = Some(sel);
    }

    /// Whether `line` of `side` is on screen (not hidden in a Collapsed Region).
    fn frame_has(&self, side: Side, line: u32) -> bool {
        self.comparison.view.line_visible(side,line)
    }

    /// F3 / shift-F3: next/previous match of the find bar, else of the Change Set search.
    fn find_step(&mut self, forward: bool, cx: &mut Context<Self>) {
        if self.comparison.find.as_ref().is_some_and(|f|!f.hits.is_empty()) {
            if let Some((side,line,range,wrapped))=self.comparison.search_step(forward){if wrapped{self.toast(if forward{"Wrapped to the first match"}else{"Wrapped to the last match"}.into(),cx);}self.select_hit(side,line,range);}
        } else if let Some(s) = self.search.as_ref().filter(|s| !s.hits.is_empty()) {
            let (i, boundary) = s.next_index(forward).expect("nonempty search results");
            if boundary { self.toast(SetSearch::boundary_message(forward).into(), cx); }
            self.show_set_hit(i, cx);
        }
    }

    fn show_set_hit(&mut self, i: usize, cx: &mut Context<Self>) {
        let visible=self.panel_rows();
        let Some(s) = self.search.as_mut() else { return };
        let Some(hit) = s.select(i, visible) else { return };
        if hit.index == self.comparison.current {
            self.select_hit(hit.side, hit.line, hit.range);
        } else {
            self.open(hit.index, Place::Line(hit.side, hit.line + 1), cx);
            self.sel = Some(Selection { side: hit.side, anchor: Pos::new(hit.line, hit.range.start), head: Pos::new(hit.line, hit.range.end) });
        }
    }

    /// Keys while the find bar or the Change Set search prompt takes typing.
    fn find_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        let key = ev.keystroke.key.as_str();
        let shift = ev.keystroke.modifiers.shift;
        if let Some(s) = self.search.as_mut().filter(|s| s.editing) {
            match key {
                "escape" => self.search = None,
                "enter" => {
                    s.editing = false;
                    self.start_search(cx);
                }
                "backspace" => {
                    let mut query=s.query.clone();query.pop();s.execute(crate::search_session::SearchCommand::Query(query),&self.set,PANEL_ROWS);
                }
                "o" if ev.keystroke.modifiers.alt => {s.execute(crate::search_session::SearchCommand::ToggleChangedOnly,&self.set,PANEL_ROWS);},
                "c" | "w" | "r" if ev.keystroke.modifiers.alt => {
                    let option = match key { "c" => FindOption::Case, "w" => FindOption::Word, _ => FindOption::Regex };
                    s.toggle_option(option);
                    self.comparison.find_options = s.options();
                },
                _ => match Self::typed(ev) {
                    Some(t) => {let mut query=s.query.clone();query.push_str(&t);s.execute(crate::search_session::SearchCommand::Query(query),&self.set,PANEL_ROWS);},
                    None => return true,
                },
            }
            cx.notify();
            return true;
        }
        if self.nav_focused || self.repo.as_ref().map(|r| &r.picker).is_some_and(|p|p.shown && p.focus!=Pane::Diff) {return false;}
        if self.comparison.find.is_none() { return false; }
        if ev.keystroke.modifiers.alt && let Some(option) = match key { "c" => Some(FindOption::Case), "w" => Some(FindOption::Word), "r" => Some(FindOption::Regex), _ => None } {
            self.toggle_find_option(option);
            cx.notify();
            return true;
        }
        let Some(f) = self.comparison.find.as_mut() else { return false };
        match key {
            "escape" => self.comparison.find = None,
            "enter" => self.find_step(!shift, cx),
            "f3" => return false,
            // Keep plain caret/viewport navigation available while Find owns typing.
            "left"|"right"|"up"|"down"|"pageup"|"pagedown"|"home"|"end"
                if !ev.keystroke.modifiers.control && !ev.keystroke.modifiers.platform && !ev.keystroke.modifiers.alt => return false,
            "backspace" => {
                f.query.pop();
                self.refind();
            }
            _ => match Self::typed(ev) {
                Some(t) => {
                    f.query.push_str(&t);
                    self.refind();
                }
                None => return true,
            },
        }
        cx.notify();
        true
    }

    /// Run the Change Set search in the background, pulling streamed results every 100 ms.
    fn start_search(&mut self, cx: &mut Context<Self>) {
        let Some(search) = self.search.as_mut() else { return };
        search.start(self.set.clone());
        let cancel = search.cancel.clone();
        if !search.busy() { cx.notify(); return; }
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(std::time::Duration::from_millis(100)).await;
                let live = this.update(cx, |viewer: &mut Viewer, cx| {
                    let Some(search) = viewer.search.as_mut().filter(|search| Arc::ptr_eq(&search.cancel, &cancel)) else { return false };
                    if search.poll() { cx.notify(); }
                    search.busy()
                }).unwrap_or(false);
                if !live { break; }
            }
        }).detach();
    }

    /// Find matches to highlight on a Side of the shown Comparison, sorted by line.
    fn found(&self, side: Side, rows: &[Row]) -> Vec<(u32, Range<usize>)> {
        let mut v = Vec::new();
        if let Some(f) = self.comparison.find.as_ref().filter(|f| f.side == side) {
            v.extend(visible_find_hits(&f.hits, rows, self.raw(side)));
        }
        if let Some(s) = &self.search {
            v.extend(s.hits_for_side(self.comparison.current,side).iter().map(|h| (h.line, h.range.clone())));
        }
        v.sort_by_key(|h| h.0);
        v
    }

    fn panel_h(&self) -> f32 {
        if self.search.as_ref().is_some_and(|s| !s.editing) || !self.edits.applied(self.comparison.current).is_empty() { crate::pane_layout::clamp_size((self.content_bottom()-self.top()).max(0.0),self.panes.panel,2.0*line_h(),2.0*line_h()) } else { 0.0 }
    }

    /// Click in the results panel: a hit opens its file at the match.
    fn panel_click(&mut self, y: f32, top: f32, cx: &mut Context<Self>) {
        if !self.search.as_ref().is_some_and(|s| !s.editing) {
            let row = ((y - top) / line_h()).floor() - 1.0;
            if row >= 0.0 && let Some(record) = self.edits.applied(self.comparison.current).get(row as usize + self.applied_scroll.floor() as usize) {
                let (id, reason) = (record.id, self.comparison.applied_undo_disabled(record));
                if let Some(reason) = reason { self.toast(reason.into(), cx); }
                else { self.edit_current(crate::edit_session::EditCommand::UndoApplied(id), cx); }
            }
            return;
        }
        let Some(s) = self.search.as_ref() else { return };
        let row = ((y - top) / line_h()).floor() - 1.0 + s.scroll.floor();
        if row < 0.0 {
            return;
        }
        if let Some(PanelRow::Hit(i)) = s.rows().get(row as usize) {
            let i = *i;
            self.show_set_hit(i, cx);
        }
    }

    /// Keys while the go-to-line prompt is open; returns whether it handled the key.
    fn goto_key(&mut self, ev: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        let Some(query) = self.goto.as_mut() else { return false };
        match ev.keystroke.key.as_str() {
            "escape" => self.goto = None,
            "backspace" => {
                query.pop();
            }
            "enter" => {
                let query = self.goto.take().unwrap_or_default();
                match lenz_content::goto::parse_line_column(&query).map(|at| (at.line, at.column)) {
                    Ok((line, col)) => {
                        let side = if self.markdown_active(){self.diff_focus}else{self.sel.map_or(Side::Right, |s| s.side)};
                        self.comparison.go_to_line(side, line);
                        if self.markdown_active(){cx.notify();return true;}
                        let line = (line - 1).min(self.raw(side).len().saturating_sub(1) as u32);
                        let text = self.raw(side).get(line as usize).map_or("", String::as_str);
                        let chars = col.unwrap_or(1).saturating_sub(1) as usize;
                        let byte = text.char_indices().nth(chars).map_or(text.len(), |(i, _)| i);
                        self.sel = Some(Selection::caret(side, Pos::new(line, byte)));
                    }
                    Err(e) => self.toast(format!("Go to line: {e}"), cx),
                }
            }
            k if k.len() == 1 && (k.as_bytes()[0].is_ascii_digit() || k == ":" || k == ",") => query.push_str(k),
            _ => {}
        }
        cx.notify();
        true
    }

    fn on_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.panes.drag=None;
        if self.discard_modal(){match ev.keystroke.key.as_str(){"escape"=>self.file_actions.cancel(),"enter" if !self.file_actions.busy()=>self.confirm_discard(cx),_=>{}}cx.notify();return;}
        if self.checkout_key(ev,cx)||self.git_prompt_key(ev,cx){return;}
        if self.theme_key(ev,cx) { return; }
        if let Some(about)=&mut self.about {
            let max=about::About::max_scroll(window.viewport_size(),self.char_w);
            about.scroll=about.scroll.min(max);
            match ev.keystroke.key.as_str(){"escape"|"enter"=>self.close_about(window),"up"=>about.scroll=about.scroll.saturating_sub(1),"down"=>about.scroll=about.scroll.saturating_add(1).min(max),"home"=>about.scroll=0,"end"=>about.scroll=max,_=>{}}
            cx.notify();return;
        }
        if self.help {
            let (_, visible)=help_layout(window.viewport_size());
            let max=help_rows(window.viewport_size(),self.char_w).len().saturating_sub(visible);
            let page=visible.saturating_sub(1).max(1);
            self.help_scroll=self.help_scroll.min(max);
            match ev.keystroke.key.as_str() {
                "escape"|"enter"=>self.help=false,
                "up"=>self.help_scroll=self.help_scroll.saturating_sub(1),
                "down"=>self.help_scroll=self.help_scroll.saturating_add(1).min(max),
                "pageup"=>self.help_scroll=self.help_scroll.saturating_sub(page),
                "pagedown"|"space"=>self.help_scroll=self.help_scroll.saturating_add(page).min(max),
                "home"=>self.help_scroll=0,
                "end"=>self.help_scroll=max,
                _=>{}
            }
            cx.notify();
            return;
        }
        if self.merge.as_ref().is_some_and(|s| s.ask.is_some()) {
            self.merge_key(ev, cx);
            return;
        }
        if self.picker_v.as_ref().is_some_and(|s| s.ask.is_some()) {
            self.picker_v_key(ev, cx);
            return;
        }
        if self.palette_key(ev,window,cx)||self.diff_options_key(ev,window,cx){return;}
        if self.menu.is_some(){
            let key=ev.keystroke.key.as_str();
            if matches!(key,"up"|"down"|"pageup"|"pagedown"|"home"|"end"){
                if let Some(menu)=self.menu.as_mut(){let at=menu.hover.unwrap_or(0);let last=menu.items.len().saturating_sub(1);menu.hover=Some(match key{"up"=>at.saturating_sub(1),"down"=>(at+1).min(last),"pageup"=>at.saturating_sub(16),"pagedown"=>(at+16).min(last),"home"=>0,"end"=>last,_=>unreachable!()});}
            }else if key=="enter"{if let Some(menu)=self.menu.take()&&let Some(i)=menu.hover.or((!menu.items.is_empty()).then_some(0)){self.menu_item(&menu,i,window,cx);}}
            else{self.menu=None;}
            cx.notify();return;
        }
        if self.switcher_key(ev, cx) || self.goto_key(ev, cx) || self.filter_key(ev, cx) {
            return;
        }
        // Back and Forward wherever the keyboard is, before any pane's own arrow keys
        let m = &ev.keystroke.modifiers;
        if m.alt && !m.shift && !m.control && !m.platform && matches!(ev.keystroke.key.as_str(), "left" | "right") && self.modal().is_none() {
            return self.navigate(ev.keystroke.key == "right", cx);
        }
        let pane_modal = self.merge.as_ref().is_some_and(|s| s.ask.is_some() || s.find_editing())
            || self.picker_v.as_ref().is_some_and(|s| s.ask.is_some())
            || self.conflicts.as_ref().is_some_and(|s| s.confirming());
        let palette_key=ev.keystroke.key=="f10" && !ev.keystroke.modifiers.shift
            || ev.keystroke.key=="p" && ev.keystroke.modifiers.shift && (ev.keystroke.modifiers.control||ev.keystroke.modifiers.platform);
        if palette_key && !pane_modal && !self.help && self.popup.is_none()
            && self.comparison.find.is_none() && !self.search.as_ref().is_some_and(|s|s.editing) {
            self.open_palette(cx);return;
        }
        if !self.help && self.popup.is_none() && !pane_modal
            && !self.search.as_ref().is_some_and(|s|s.editing)
            && self.pane_search_key(ev, cx) { return; }
        if self.find_key(ev, cx) { return; }
        if !self.help&&self.popup.is_none()&&!pane_modal&&ev.keystroke.key=="f10"&&ev.keystroke.modifiers.shift{
            if self.nav_focused{let index=self.file_search.current().unwrap_or(self.comparison.current);self.show_menu(Menu{x:self.picker_w()+8.0,y:self.top(),items:self.file_menu(index),hover:Some(0)});cx.notify();}
            else if self.picker_shown()&&self.repo.as_ref().map(|r| &r.picker).is_some_and(|p|p.focus!=crate::repository_session::Pane::Diff){let p=self.repo.as_ref().map(|r| &r.picker).unwrap();let (x,row,scroll)=if p.focus==crate::repository_session::Pane::Log{(self.picker_w()+8.0,p.pane_search.current().map_or(p.anchor as f32,|r|r as f32),p.scroll)}else{(8.0,p.sources.selected as f32,p.sources.scroll)};self.picker_menu(x,toolbar_h()+(row-scroll+1.0)*line_h()+2.0,cx);}
            else{let (start,end)=self.pane_span(window,self.diff_focus);let ev=MouseDownEvent{position:point(px((start+num_w()+16.0).min(end-1.0)),px(self.top()+1.0)),button:MouseButton::Right,..Default::default()};self.on_right_down(&ev,window,cx);}
            return;
        }
        if !self.help && self.popup.is_none() && !pane_modal && !self.nav_focused && !self.panel_focused && self.pr_dialog_key(ev, cx) { return; }
        if !self.help && self.popup.is_none() && !pane_modal && !self.nav_focused && !self.panel_focused && !self.message_focused && self.summary_key(ev, cx) { return; }
        if !self.help && self.popup.is_none() && !pane_modal && ev.keystroke.key == "tab" {
            if !self.search.as_ref().is_some_and(|s| s.editing) && self.comparison.find.is_none() {
                self.cycle_pane(ev.keystroke.modifiers.shift, cx);
            }
            return;
        }
        if self.sidebar_key(ev, cx) { return; }
        if !self.help && self.popup.is_none() && !pane_modal && (self.message_key(ev, cx) || self.nav_key(ev, cx) || self.panel_key(ev, cx)) { return; }
        // what Files or the results panel left would act on the hidden diff
        if !self.help && self.popup.is_none() && !pane_modal && self.pr_dialog_key(ev, cx) { return; }
        if !pane_modal && (ev.keystroke.key=="?" || ev.keystroke.key=="/"&&ev.keystroke.modifiers.shift) {
            self.help=true;self.help_scroll=0;cx.notify();return;
        }
        if !self.help && (self.conflict_key(ev, cx) || self.merge_key(ev, cx) || self.picker_v_key(ev, cx)) {
            return;
        }

        if self.popup.is_some() {
            match ev.keystroke.key.as_str() {
                "escape"=>self.popup=None,
                k @ ("up"|"down"|"pageup"|"pagedown"|"home"|"end")=>{
                    let viewport=[f32::from(window.viewport_size().width),f32::from(window.viewport_size().height)];
                    let rows=self.popup_rows(viewport).len();let (_,_,visible)=popup_layout(rows,viewport,self.char_w);
                    let max=rows.saturating_sub(visible);let page=visible.saturating_sub(1).max(1);
                    if let Some(p)=&mut self.popup {p.scroll=match k {"up"=>p.scroll.saturating_sub(1),"down"=>(p.scroll+1).min(max),"pageup"=>p.scroll.saturating_sub(page),"pagedown"=>(p.scroll+page).min(max),"home"=>0,_=>max};}
                }
                k=>self.popup_action(k,cx),
            }
            cx.notify();
            return;
        }
        let page = (self.comparison.view.viewport_rows() - 2.0).max(1.0);
        let alt = ev.keystroke.modifiers.alt;
        let key = ev.keystroke.key.as_str();
        if !(matches!(key, "n" | "p" | "f7") || alt && matches!(key, "up" | "down")) {
            self.comparison.press_again.reset();
        }
        let cmd = ev.keystroke.modifiers.control || ev.keystroke.modifiers.platform;
        let shift = ev.keystroke.modifiers.shift;
        // the Change Set Picker: its panes, Focus Mode, and the log's own keys
        if self.repo.is_some() {
            let shown = self.picker_shown();
            match key {
                // the palette's Focus key, from the Overview however its panels are set
                "f" if !cmd && !alt && !self.layout.focused(self.repo.as_ref()) => return self.layout_command(crate::layout::LayoutCommand::ToggleFocus, cx),
                "enter" if shown && self.repo.as_ref().map(|r| &r.picker).is_some_and(|p| p.focus == Pane::Log) => return self.layout_command(crate::layout::LayoutCommand::ToggleFocus, cx),
                "escape" if self.layout.focused(self.repo.as_ref()) && self.search.is_none() && self.sel.is_none_or(|s| s.is_empty()) => return self.layout_command(crate::layout::LayoutCommand::LeaveFocus, cx),
                "escape" if !shown && self.search.is_none() && self.sel.is_none_or(|s| s.is_empty()) => return self.set_picker_shown(true, cx),
                "escape" if shown && self.log_escape(cx) => return,
                "2" | "3" if alt => {
                    (self.nav_focused, self.panel_focused) = (false, false);
                    return self.layout_command(if key == "2" { crate::layout::LayoutCommand::ToggleBranches } else { crate::layout::LayoutCommand::ToggleHistory }, cx);
                }
                k if !(alt && shift && matches!(k, "left" | "right")) && self.log_key(k, shift, cmd, cx) => {
                    cx.notify();
                    return;
                }
                _ => {}
            }
        }
        if self.nothing.is_some() && !matches!(key, "q" | "w" | "?" | "/" | ":" | "d" | "r" | "t") {
            return;
        }
        if !cmd && !alt && self.pr_key(key, shift, cx) {
            return;
        }
        if !cmd && !alt && self.image_doc().is_some() && self.image_key(&ev.keystroke.key_char.clone().unwrap_or_else(|| key.to_string()), window, cx) {
            return;
        }
        // Find option toggles (they say why when no find is open), never the plain letters.
        if alt && !cmd && !shift && let Some(toggle) = match key { "c" => Some(commands::Interaction::FindCase), "w" => Some(commands::Interaction::FindWord), "r" => Some(commands::Interaction::FindRegex), _ => None } {
            self.interaction(toggle, cx);
            return;
        }
        if self.markdown_key(key,cmd,shift,alt,window,cx){return;}
        let lead = self.sel.map_or(self.diff_focus, |s| s.side);
        if cmd && alt && !shift && matches!(key,"left"|"right") && self.image_doc().is_none(){
            self.interaction(if key=="left"{commands::Interaction::ScrollLeft}else{commands::Interaction::ScrollRight},cx);return;
        }
        match ev.keystroke.key.as_str() {
            "c" if cmd => self.copy(cx),
            "a" if cmd => {
                let side = self.sel.map_or(Side::Right, |s| s.side);
                self.sel = Some(Selection::all(side, self.raw(side)));
                self.selection_to_primary(cx);
            }
            "f" if cmd && shift => {
                let (query, changed_only) =
                    self.search.as_ref().map_or((String::new(), false), |s| (s.query.clone(), s.changed_only));
                self.search = Some(SetSearch::new(query, changed_only, self.comparison.find_options));
            }
            "f" if cmd => self.open_find(),
            "b" if alt => self.toggle_blame(cx),
            "enter" if self.large() => self.load(self.comparison.current, Place::Top, true, cx),
            "f3" => self.find_step(!shift, cx),
            "escape" if self.search.is_some() => {
                if let Some(s) = self.search.take() {
                    s.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                }
            }
            "g" if ev.keystroke.modifiers.control => self.goto = Some(String::new()),
            "l" if ev.keystroke.modifiers.platform => self.goto = Some(String::new()),
            "escape" if self.layout.focused(self.repo.as_ref()) && self.search.is_none() && self.sel.is_none_or(|s| s.is_empty()) => self.layout_command(crate::layout::LayoutCommand::LeaveFocus, cx),
            "escape" => self.sel = self.sel.map(|s| Selection::caret(s.side, s.head)),
            "q" | "w" if cmd => self.quit_if_ready(cx),
            "q" => self.quit_if_ready(cx),
            "s" if cmd => self.edit_menu(crate::edit_session::EditMenuCommand::Save,cx),
            "w" => self.comparison_command(Command::CycleWhitespace, cx),
            "i" => self.comparison_command(Command::ToggleCase, cx),
            "h" => self.comparison_command(Command::CycleHighlight, cx),
            "s" => self.comparison_command(Command::CycleScroll, cx),
            ":" if self.merge.is_none() && self.picker_v.is_none() => self.open_palette(cx),
            "p" if cmd => {
                self.switcher = Some(Switcher { query: String::new(), selected: 0, hits: Vec::new() });
                self.update_switcher();
            }
            "1" if alt => self.layout_command(crate::layout::LayoutCommand::ToggleFiles, cx),
            "right" | "left" if alt && shift => match self.key_block() {
                Some(i) => self.move_change(i, if key == "right" { Side::Right } else { Side::Left }, cx),
                None => self.toast("No change here to move — n / p to one first".into(), cx),
            },
            "z" if cmd => self.undo(shift, cx),
            // file stepping visits every file, changed or not
            "]" => self.step_file(true, cx),
            "[" => self.step_file(false, cx),
            "right" if alt => self.navigate(true, cx),
            "left" if alt => self.navigate(false, cx),
            "n" | "down" if alt || ev.keystroke.key == "n" => self.step_change(true, cx),
            "p" | "up" if alt || ev.keystroke.key == "p" => self.step_change(false, cx),
            "f7" => self.step_change(!ev.keystroke.modifiers.shift, cx),
            "o" if !cmd => self.open_focused(shift, cx),
            "x" if shift => self.fold(None, cx),
            "x" => match self.comparison.view.current_change() {
                Some(i) => self.fold(Some(i), cx),
                None => self.toast("No change here to fold — n / p to one first".into(), cx),
            },
            "z" => {
                self.comparison.collapse = !self.comparison.collapse;
                self.comparison.view.set_collapsed(self.comparison.collapse);
                self.toast(if self.comparison.collapse { "Collapse unchanged: on" } else { "Collapse unchanged: off" }.into(), cx);
            }
            k if !alt && self.caret_key(k, shift, cmd) => {
                if shift {
                    self.selection_to_primary(cx);
                }
            }
            "down" => self.comparison.view.scroll_by(1.0, lead),
            "up" => self.comparison.view.scroll_by(-1.0, lead),
            "pagedown" | "space" => self.comparison.view.scroll_by(page, lead),
            "pageup" => self.comparison.view.scroll_by(-page, lead),
            "home" => self.comparison.view.scroll_by(f32::NEG_INFINITY, lead),
            "end" => self.comparison.view.scroll_by(f32::INFINITY, lead),
            _ => return,
        }
        cx.notify();
    }

    fn on_wheel(&mut self, ev: &ScrollWheelEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.discard_modal() || self.checkout_modal() || self.merge.as_ref().is_some_and(|s|s.ask.is_some()) || self.picker_v.as_ref().is_some_and(|s|s.ask.is_some()){return;}
        if self.theme_chooser.is_some()||self.diff_options.is_some(){return;}
        // the palette takes the wheel as row moves; nothing underneath scrolls
        if let Some(p)=self.command_palette.as_mut(){
            let dy=f32::from(ev.delta.pixel_delta(px(line_h())).y);
            if dy!=0.0{p.command(crate::palette::PaletteCommand::Move(if dy>0.0{-1}else{1}));cx.notify();}
            return;
        }
        if let Some(about)=&mut self.about{let max=about::About::max_scroll(window.viewport_size(),self.char_w);about.scroll=about.scroll.min(max);let delta=ev.delta.pixel_delta(px(line_h()));if f32::from(delta.y)>0.0{about.scroll=about.scroll.saturating_sub(3);}else{about.scroll=about.scroll.saturating_add(3).min(max);}cx.notify();return;}
        if self.help {
            let (_,visible)=help_layout(window.viewport_size());
            let max=help_rows(window.viewport_size(),self.char_w).len().saturating_sub(visible);
            self.help_scroll=self.help_scroll.min(max);
            let dy=f32::from(ev.delta.pixel_delta(px(line_h())).y);
            if dy>0.0{self.help_scroll=self.help_scroll.saturating_sub(3);}
            else if dy<0.0{self.help_scroll=self.help_scroll.saturating_add(3).min(max);}
            cx.notify();return;
        }
        if self.popup.is_some() {
            let viewport=[f32::from(window.viewport_size().width),f32::from(window.viewport_size().height)];
            let count=self.popup_rows(viewport).len();let (_,_,visible)=popup_layout(count,viewport,self.char_w);
            let max=count.saturating_sub(visible);let delta=ev.delta.pixel_delta(px(line_h()));
            if let Some(p)=&mut self.popup {p.scroll=if f32::from(delta.y)>0. {p.scroll.saturating_sub(3)}else{(p.scroll+3).min(max)};}
            cx.notify();return;
        }
        if self.switcher.is_some() || self.goto.is_some() || self.git_prompt.is_some() || self.help
            || self.search.as_ref().is_some_and(|s| s.editing)
            || self.repo.as_ref().map(|r| &r.picker).is_some_and(|p| p.filter_editing) { return; }
        let delta = lenz_gpui::scroll::pixels(ev.delta, line_h(), self.char_w);
        let rows = delta.y / line_h();
        if let Some(menu)=self.menu.as_mut(){let at=menu.hover.unwrap_or(0) as f32;menu.hover=Some((at+rows).clamp(0.0,menu.items.len().saturating_sub(1)as f32)as usize);cx.notify();return;}
        if self.message_wheel(f32::from(ev.position.x), f32::from(ev.position.y), rows, cx) { return; }
        if self.menu.is_none() && self.command_palette.is_none() && self.diff_options.is_none() && self.pr_dialog_wheel(ev,cx){return;}
        if self.menu.is_none() && self.command_palette.is_none() && self.diff_options.is_none() && self.summary_wheel(f32::from(ev.position.x),rows,cx){return;}
        let (x, sw) = (f32::from(ev.position.x), self.diff_x());
        let y = f32::from(ev.position.y);
        if y < toolbar_h() { return; }
        if self.picker_v_wheel(x,y,rows,cx){return;}
        if self.picker_wheel(x, f32::from(ev.position.y), rows, cx) {
            return;
        }
        let visible=self.panel_rows();
        let panel_top = self.content_bottom() - self.panel_h();
        if f32::from(ev.position.y) >= panel_top {
            if let Some(s) = self.search.as_mut() {
                let max = (s.rows().len() as f32 - visible).max(0.0);
                s.scroll = (s.scroll + rows).clamp(0.0, max);
                cx.notify();
            } else {
                let max=(self.edits.applied(self.comparison.current).len() as f32-self.panel_rows()).max(0.0);
                self.applied_scroll=(self.applied_scroll+rows).clamp(0.0,max);cx.notify();
            }
            return;
        }
        if self.in_nav(x) {
            let max = (self.nav.len() as f32 - self.comparison.view.viewport_rows() + 1.0).max(0.0);
            self.nav_scroll = (self.nav_scroll + rows).clamp(0.0, max);
            cx.notify();
            return;
        }
        if x < sw || y < self.top() { return; }
        // Horizontal wheel input belongs to the pointed text pane, including merges.
        let delta = lenz_gpui::scroll::shifted(delta, ev.modifiers.shift);
        let (mut dx, rows) = (delta.x, delta.y / line_h());
        if let Some(s) = self.merge.as_mut() {
            let pane=s.pane_at(x-sw);
            if dx!=0.0{s.scroll_horizontal(pane,dx,self.char_w);}
            if rows!=0.0{s.scroll_by(rows);}
            cx.notify();return;
        }
        if self.markdown_active(){let side=self.pane_at(window,x).map_or(self.diff_focus,|p|p.0);self.markdown_scroll(side,rows*line_h(),dx,window);cx.notify();return;}
        if self.image_doc().is_some(){dx=0.0;}
        if dx != 0.0 {
            let widths=self.comparison_widths(f32::from(window.viewport_size().width)-sw);
            let side=if x<sw+widths[0]{Side::Left}else{Side::Right};
            self.text_w=widths.map(|w|(w-num_w()-16.0).max(0.0));
            self.scroll_horizontal(side,dx);
            cx.notify();
        }
        if self.image_doc().is_some() {
            self.zoom_image_at(1.15f32.powf(-rows / 3.0), x, f32::from(ev.position.y), window);
            cx.notify();
            return;
        }
        // the Side under the pointer leads (matters in Independent mode)
        let lead = self.pane_at(window,x).map_or(self.diff_focus,|p|p.0);
        if rows != 0.0 {
            self.comparison.view.scroll_by(rows, lead);
            cx.notify();
        }
    }
}

/// A File Navigator row ready to paint: text parts with colours, and whether it's the current file.
struct SideRow {
    parts: Vec<(String, u32)>,
    indent: f32,
    current: bool,
    found: Vec<Range<usize>>,
}

impl Viewer {
    /// Lenz wrote one of Comparison `index`'s files this session.
    fn edited_in_lenz(&self, index: usize) -> bool {
        let c = &self.set.comparisons[index];
        [&c.left, &c.right].into_iter().any(|s| matches!(s, Source::File(p) if self.edited.contains(p)))
    }

    fn sidebar_rows(&self) -> Vec<SideRow> {
        // a shown Folder Summary marks its heading, not the file behind it
        let summary=self.comparison.folders.summary_row(&self.set, &self.nav);
        let candidate=self.file_search.current().or(if summary.is_some(){None}else{Some(self.comparison.current)}).unwrap_or(usize::MAX);
        let mut rows:Vec<SideRow>=self.nav
            .iter().enumerate()
            .map(|(row_index,row)| match row {
                NavRow::Group(name, n) => SideRow { parts: vec![(format!("{} · {n}", name.to_uppercase()), self.palette.accent)], indent: 0.0, current: false, found:Vec::new() },
                // the heading whose summary shows reads as the current row, in the text colour
                NavRow::Dir { path, collapsed: false, .. } => { let current = summary==Some(row_index); SideRow { parts: vec![(format!("▾ {path}"), if current { self.palette.fg } else { self.palette.dim })], indent: 0.0, current, found:Vec::new() } }
                NavRow::Dir { path, files, .. } => { let current = summary==Some(row_index); SideRow { parts: vec![(format!("▸ {path}"), if current { self.palette.fg } else { self.palette.dim }), (format!(" · {files} file{}", if *files == 1 { "" } else { "s" }), self.palette.num_fg)], indent: 0.0, current, found:Vec::new() } }
                NavRow::File { index, name, from } => {
                    if let Some(parts) = self.conflict_row(*index) {
                        return SideRow { parts, indent: 16.0, current: *index == candidate, found:Vec::new() };
                    }
                    let c = &self.set.comparisons[*index];
                    let color = status_color(&self.palette, &c.status);
                    let name=if self.file_search.query().is_empty(){name.as_str()}else{c.path.as_str()};
                    let mut parts = vec![(navigator::badge(&c.status).to_string(), color), (format!(" {name}"), color)];
                    if let Some(st) = self.view_status(*index) {
                        let (mark, mc) = match st {
                            pr::ViewStatus::Viewed => ("☑ ", self.palette.status[0]),
                            pr::ViewStatus::ChangedSinceViewed => ("◐ ", self.palette.accent),
                            pr::ViewStatus::Unviewed => ("☐ ", self.palette.dim),
                        };
                        parts.insert(0, (mark.to_string(), mc));
                    }
                    if let Some(from) = from {
                        parts.push((format!("  ← {from}"), self.palette.dim));
                    }
                    if self.edits.dirty(*index) {
                        parts.push(("  ● unsaved".into(),self.palette.accent));
                    } else if self.edited_in_lenz(*index) {
                        parts.push(("  ●".into(), self.palette.accent));
                    }
                    SideRow { parts, indent: 16.0, current: *index == candidate, found:Vec::new() }
                }
            })
            .collect();
        if !self.file_search.query().is_empty() {
            for row in rows.iter_mut().skip(self.nav_scroll.floor() as usize).take(self.comparison.view.viewport_rows().ceil() as usize+2) {
                let text:String=row.parts.iter().map(|p|p.0.as_str()).collect();
                row.found=self.file_search.ranges(&text);
            }
        }
        rows
    }
}

#[allow(clippy::too_many_arguments)] // Palette plus adapter layout inputs.
fn paint_sidebar(palette: &theme::Palette,
    window: &mut Window,
    cx: &mut App,
    font: &Font,
    bounds: Bounds<Pixels>,
    sidebar_w: f32,
    (rows, header): &(Vec<SideRow>, String),
    scroll: f32,
    tooltip: Option<&(usize, String)>,
) {
    let (x0, top) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
    let h = f32::from(bounds.size.height);
    fill(window, x0, top, sidebar_w, h, palette.sidebar_bg);
    fill(window, x0, top, sidebar_w, hdr_h(), palette.header_bg);
    window.with_content_mask(Some(clip(bounds, x0, sidebar_w - 1.0)), |window| text(window, cx, font, x0 + 8.0, top + 2.0, header, palette.dim));
    let list_top = top + hdr_h();
    let list = Bounds { origin: point(px(x0), px(list_top)), size: size(px(sidebar_w), px((h - hdr_h()).max(0.0))) };
    window.with_content_mask(Some(ContentMask { bounds: list }), |window| {
        let first = scroll.floor() as usize;
        for (i, row) in rows.iter().enumerate().skip(first).take(((h - hdr_h()) / line_h()) as usize + 2) {
            let y = list_top + (i as f32 - scroll) * line_h();
            if row.current {
                fill(window, x0, y, sidebar_w, line_h(), palette.selected_bg);
            }
            let s: String = row.parts.iter().map(|p| p.0.as_str()).collect();
            let mut at = 0;
            let spans: Vec<(Range<usize>, u32)> = row
                .parts
                .iter()
                .map(|(t, c)| {
                    at += t.len();
                    (at - t.len()..at, *c)
                })
                .collect();
            let found:Vec<_>=row.found.iter().cloned().map(|r|(r,palette.found_bg)).collect();
            colored_text(palette, window, cx, font, x0 + 8.0 + row.indent, y, &s, &spans, &found);
        }
    });
    fill(window, x0 + sidebar_w - 1.0, top, 1.0, h, palette.border);
    if let Some((row, tip)) = tooltip {
        let y = list_top + (*row as f32 + 1.0 - scroll) * line_h();
        let tw = tip.chars().count() as f32 * 7.8 + 16.0;
        fill(window, x0 + 24.0, y, tw, line_h(), palette.status_bg);
        text(window, cx, font, x0 + 32.0, y, tip, palette.fg);
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_switcher(palette: &theme::Palette,
    window: &mut Window,
    cx: &mut App,
    font: &Font,
    x0: f32,
    top: f32,
    w: f32,
    query: &str,
    selected: usize,
    hits: &[(String, String, u32)],
) {
    let bw = 720.0f32.min(w - 40.0);
    let (bx, by) = (x0 + (w - bw) / 2.0, top + 60.0);
    let bh = (hits.len() as f32 + 1.0) * line_h() + 16.0;
    fill(window, bx - 1.0, by - 1.0, bw + 2.0, bh + 2.0, palette.border);
    fill(window, bx, by, bw, bh, palette.status_bg);
    let prompt = format!("Go to changed file: {query}▏");
    let spans = [(0.."Go to changed file: ".len(), palette.dim)];
    colored_text(palette, window, cx, font, bx + 10.0, by + 6.0, &prompt, &spans, &[]);
    for (i, (name, dir, color)) in hits.iter().enumerate() {
        let y = by + 8.0 + (i as f32 + 1.0) * line_h();
        if i == selected {
            fill(window, bx, y, bw, line_h(), palette.selected_bg);
        }
        let line = format!("{name}  {dir}");
        let spans = [(0..name.len(), *color), (name.len()..line.len(), palette.dim)];
        colored_text(palette, window, cx, font, bx + 10.0, y, &line, &spans, &[]);
    }
}

/// One painted row with its text resolved.
struct Painted {
    y: f32,
    /// A numbered source line: syntax colours (`fg`) and Inline Changes and find hits (`bg`) as display ranges.
    line: Option<lenz_gpui::TextRow>,
    /// Folded Change Block or Collapsed Region text.
    label: String,
    tint: Option<(Kind, bool)>,
    /// Blame Annotation: its text (at the start of a same-commit run) and age shade.
    ann: Option<(Option<String>, u32)>,
    /// A PR review thread sits on this line.
    thread: bool,
}

/// Disjoint visible runs: folded lines must not be queried or allocated by the highlighter.
fn visible_runs(rows: &[Row]) -> Vec<Range<usize>> {
    let mut runs: Vec<Range<usize>> = Vec::new();
    for line in rows.iter().filter_map(|r| match r.content {
        Content::Line(i) => Some(i as usize),
        Content::Filler | Content::Fold { .. } | Content::Folded { .. } => None,
    }) {
        match runs.last_mut() {
            Some(r) if r.end == line => r.end += 1,
            _ => runs.push(line..line + 1),
        }
    }
    runs
}

fn visible_find_hits(hits: &[(u32, Range<usize>)], rows: &[Row], raw: &[String]) -> Vec<(u32, Range<usize>)> {
    let mut visible = Vec::new();
    for row in rows {
        let Content::Line(line) = row.content else { continue };
        let cut = lenz_content::display::displayed_bytes(&raw[line as usize]);
        let start = hits.partition_point(|h| h.0 < line);
        let end = hits.partition_point(|h| h.0 < line || (h.0 == line && h.1.start < cut));
        visible.extend_from_slice(&hits[start..end]);
    }
    visible
}

/// `found`: find matches on this Side as (line, byte range), sorted by line.
fn resolve(palette: &theme::Palette, rows: &[Row], raw: &[String], parsed: Option<&Parsed>, sel: Option<&Selection>, found: &[(u32, Range<usize>)]) -> Vec<Painted> {
    let spans: Vec<_> = parsed.map(|p| visible_runs(rows).into_iter().map(|r| {
        let spans = p.spans(r.clone());
        (r, spans)
    }).collect()).unwrap_or_default();
    rows.iter()
        .map(|r| match r.content {
            Content::Line(i) => {
                let full = raw[i as usize].as_str();
                // very long lines are cut short on screen (shaping them every frame is the cost); copy has them whole
                let (text, map) = lenz_content::display::display_line(full);
                let cut = map.len() - 1; // the drawn prefix
                let at = |b: usize| map[b.min(map.len() - 1)];
                let syntax: Vec<(Range<usize>, u32)> = spans
                    .iter()
                    .find(|(r, _)| r.contains(&(i as usize)))
                    .and_then(|(r, s)| s.get(i as usize - r.start))
                    .map(|line| line.iter().filter(|(range, _)| range.start < cut).map(|(range, c)| (at(range.start)..at(range.end), syntax_color(palette, *c))).collect())
                    .unwrap_or_default();
                let inline: Vec<(Range<usize>, u32)> = match r.tint {
                    Some((kind, ignored)) => {
                        r.inline.iter().filter(|range| range.start < cut).map(|range| (at(range.start)..at(range.end), crate::theme::change_color(palette, kind, ignored, &palette.inline))).collect()
                    }
                    None => Vec::new(),
                };
                // Changed words keep their syntax colors where those read on the word's fill, else the plain
                // foreground (strings measured 1.1:1 on the dark theme's modified-word color).
                let on_words: Vec<_> = inline.iter().flat_map(|(word, fill)| {
                    let colored = syntax.iter().filter(|(r, _)| r.start < word.end && word.start < r.end)
                        .map(|(r, c)| (r.start.max(word.start)..r.end.min(word.end), palette.readable_on(*c, *fill)));
                    colored.chain([(word.clone(), palette.fg)]).collect::<Vec<_>>()
                }).collect();
                let syntax: Vec<_> = on_words.into_iter().chain(syntax).collect();
                let selected = sel.and_then(|s| s.on_line(i, raw[i as usize].len())).map(|r| at(r.start)..at(r.end));
                let caret = sel.filter(|s| s.head.line == i).map(|s| at(s.head.col));
                let first = found.partition_point(|h| h.0 < i);
                let found = found[first..].iter().take_while(|h| h.0 == i).filter(|h| h.1.start < cut).map(|h| (at(h.1.start)..at(h.1.end), palette.found_bg));
                let inline = found.chain(inline).collect();
                let line = lenz_gpui::TextRow { y: r.y, number: i + 1, text, fg: syntax, bg: inline, selected, caret, extra: Vec::new() };
                Painted { y: r.y, line: Some(line), label: String::new(), tint: r.tint, ann: None, thread: false }
            }
            Content::Filler => {
                Painted { y: r.y, line: None, label: String::new(), tint: r.tint, ann: None, thread: false }
            }
            Content::Folded { left, right, .. } => {
                let lines = |n: u32| format!("{n} line{}", if n == 1 { "" } else { "s" });
                let text = match r.tint.map(|t| t.0) {
                    Some(Kind::Inserted) => format!("▸ inserted: {}", lines(right)),
                    Some(Kind::Deleted) => format!("▸ deleted: {}", lines(left)),
                    _ => format!("▸ modified: {} → {}", lines(left), lines(right)),
                };
                Painted { y: r.y, line: None, label: text, tint: r.tint, ann: None, thread: false }
            }
            Content::Fold { hidden, .. } => Painted {
                y: r.y,
                line: None,
                label: format!("⋯  {hidden} unchanged lines"),
                tint: None,
                ann: None,
                thread: false,
            },
        })
        .collect()
}

impl Viewer {
    fn banner(&self) -> Option<String> {
        self.repo.as_ref().and_then(|r| r.pr_loading()).map(|message| format!("◌ {message}"))
            .or_else(|| self.comparison.markdown_notice().map(str::to_owned)).or_else(|| self.comparison.banner.clone()).or_else(|| self.write_error.clone())
    }
}

impl Render for Viewer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.restore_and_record_place(cx);
        self.sync_horizontal();
        self.markdown_release_images(window);
        // a PR details tab, however it opened, closes a Folder Summary: they share the diff area
        if self.pr_details_shown() && self.summary_shown() { self.comparison.folders.close_summary(); self.comparison.folders.reveal(&self.set.comparisons, self.comparison.current); }
        self.sync_nav();
        // the content ends above the bottom bar
        self.panes.viewport=[f32::from(window.viewport_size().width).max(0.0),(f32::from(window.viewport_size().height)-footer_h()).max(0.0)];
        self.palette = self.theme_preview.clone().filter(|_| self.theme_chooser.is_some()).unwrap_or_else(|| cx.global::<themes::GuiThemes>().0.snapshot().palette);
        let palette = self.palette.clone();
        let message_lines = self.message_lines().len();
        self.clamp_message_scroll(message_lines);
        if self.message_h() == 0.0 { self.message_focused=false; self.message_press=None; self.message_sel=None; }
        let panel_h = self.panel_h();
        let visible=self.panel_rows();
        let log_rows=self.log_rows();
        if let Some(picker)=self.repo.as_mut().map(|r| &mut r.picker) {picker.scroll=picker.scroll.min((picker.rows() as f32-log_rows).max(0.0));}
        if let Some(search)=&mut self.search {search.scroll_by(0.0,visible);}
        self.applied_scroll=self.applied_scroll.min((self.edits.applied(self.comparison.current).len() as f32-visible).max(0.0));
        let rows = ((self.content_bottom() - self.top() - panel_h - self.picker_v_height()) / line_h()).max(0.0);
        self.comparison.view.set_viewport_rows(rows);
        self.nav_scroll=self.nav_scroll.clamp(0.0,(self.nav.len() as f32-rows+1.0).max(0.0));
        self.poll_pane_search(window, cx);
        // A paused log resumes once scrolling or selection nears its end (the poll loop stopped at the pause).
        let log_rows = self.log_rows();
        if self.repo.as_mut().map(|r| &mut r.picker).is_some_and(|p| p.paused && p.resume_stream(log_rows)) {
            self.poll_repository(cx);
        }
        // Get the text and diff on screen before syntax jobs compete for CPU and memory bandwidth.
        let painted = self.first_paint_reported.get();
        let defer_syntax = self.merge.as_ref().map_or_else(
            || self.doc().is_some_and(|d| (d.lang_left.is_some() || d.lang_right.is_some()) && d.text_left.len() + d.text_right.len() > 64 << 10),
            |s| s.lang.is_some(),
        );
        if !painted && defer_syntax {
            window.request_animation_frame();
        } else if self.merge.is_none() {
            self.parse_in_background(self.comparison.current, cx);
            self.refine_more(cx);
        } else if let Some(loaded) = self.loaded.get_mut(&self.comparison.current).filter(|l| l.parsing) {
            loaded.release_syntax();
        }
        let sidebar_frame = self.sidebar_frame(self.panes.viewport[1] - toolbar_h());
        let image_frame = self.image_frame(window);

        if painted {
            self.merge_reparse(cx);
        }
        let picker_v_frame = self.picker_v_frame();
        let version_h=self.picker_v_height();
        let merge_frame = self.merge_frame(rows, self.merge_widths(f32::from(window.viewport_size().width) - self.diff_x()));
        if self.char_w == 0.0 {
            let run = TextRun { len: 10, font: self.font.clone(), color: Hsla::default(), background_color: None, underline: None, strikethrough: None };
            let ten = window.text_system().shape_line("0000000000".into(), px(font_size()), &[run], None);
            self.char_w = f32::from(ten.width) / 10.0;
        }
        let widths = self.comparison_widths(f32::from(window.viewport_size().width) - self.diff_x());
        self.text_w = widths.map(|w| (w - num_w() - 16.0).max(0.0));
        for side in [Side::Left,Side::Right]{self.clamp_horizontal(side);}
        let markdown_frame=self.markdown_frame(window,cx);
        let hscroll = self.hscroll;
        let frame = self.comparison.view.frame();
        let unified = self.comparison.view.unified();
        let command_palette=self.palette_paint();
        let log_frame = self.shown_log_frame();
        let diff_options=self.diff_options_paint();
        let header_switch=self.header_switch();
        let loaded = self.loaded.get(&self.comparison.current);
        let (left, right, line_endings) = match loaded.filter(|l| markdown_frame.is_none() && self.merge.is_none() && l.doc.placeholder.is_none()) {
            Some(l) => (
                resolve(&palette, &frame.left, &l.doc.raw_left, l.parsed[0].as_deref(), self.sel.as_ref().filter(|s| s.side == Side::Left), &self.found(Side::Left, &frame.left)),
                resolve(&palette, &frame.right, &l.doc.raw_right, l.parsed[1].as_deref(), self.sel.as_ref().filter(|s| s.side == Side::Right), &self.found(Side::Right, &frame.right)),
                l.doc.line_endings,
            ),
            None => (Vec::new(), Vec::new(), None),
        };
        let (mut left, mut right) = (left, right);
        // Sticky lines: each pane's rows, painted over its top rows (syntax colours only).
        let sticky: [Vec<Painted>; 2] = match loaded.filter(|_| markdown_frame.is_none()) {
            Some(l) => self.sticky_lines().map(|lines| {
                let Some(side) = lines.first().map(|s| s.side) else { return Vec::new() };
                let rows: Vec<Row> = lines.iter().map(|s| Row { y: s.row as f32, content: Content::Line(s.line), tint: None, inline: Vec::new() }).collect();
                let (raw, parsed) = if side == Side::Left { (&l.doc.raw_left, l.parsed[0].as_deref()) } else { (&l.doc.raw_right, l.parsed[1].as_deref()) };
                resolve(&palette, &rows, raw, parsed, None, &[])
            }),
            None => Default::default(),
        };
        let threads = self.thread_lines();
        for (rows, lines) in [(&mut left, &threads[0]), (&mut right, &threads[1])] {
            for row in rows.iter_mut() {
                row.thread = row.line.as_ref().is_some_and(|l| lines.contains(&(l.number - 1)));
            }
        }
        let ann_w = [self.ann_w(Side::Left), self.ann_w(Side::Right)];
        for (side, rows) in [(Side::Left, &mut left), (Side::Right, &mut right)] {
            let Some(b) = self.blame_key(side).and_then(|k| self.blames.get(&k)) else { continue };
            let (oldest, newest) = b.age_extent().unwrap_or((i64::MAX, i64::MIN));
            let fit = ((ann_w[side as usize] - 12.0).max(0.0) / self.char_w.max(1.0)) as usize;
            let mut prev: Option<&str> = None;
            for row in rows.iter_mut() {
                let Some(n) = row.line.as_ref().map(|l| l.number) else {
                    prev = None;
                    continue;
                };
                row.ann = Some(match b.line(n as usize - 1) {
                    Some(blame::BlameLine { commit: c, .. }) if c.is_committed() => {
                        let label = (prev != Some(c.sha.as_str())).then(|| {
                            format!("{} {}", blame::date(c.author_time), c.author).chars().take(fit).collect()
                        });
                        let age = blame::age_shade(c.author_time, newest, oldest);
                        prev = Some(&c.sha);
                        let mix = |a: u32, b: u32, s: f32| (a as f32 + (b as f32 - a as f32) * s) as u32;
                        let shade = |shift: u32| mix((palette.gutter_bg >> shift) & 0xFF, (palette.blame_new >> shift) & 0xFF, age) << shift;
                        (label, shade(16) | shade(8) | shade(0))
                    }
                    Some(_) => {
                        let label = (prev != Some("")).then(|| "Not committed".to_string());
                        prev = Some("");
                        (label, palette.gutter_bg)
                    }
                    None => {
                        prev = None;
                        (None, palette.gutter_bg)
                    }
                });
                // a run's label repeats on the first fully visible row
                if row.y < 0.0 {
                    prev = None;
                }
            }
        }
        let popup=self.popup.as_ref().map(|p| {
            let rows=self.popup_rows(self.panes.viewport);
            let (_,_,visible)=popup_layout(rows.len(),self.panes.viewport,self.char_w);
            (rows,p.scroll.min(self.popup_rows(self.panes.viewport).len().saturating_sub(visible)))
        });
        let popup_viewport=self.panes.viewport;
        let char_w = self.char_w;
        let connectors = if loaded.is_some() && self.nothing.is_none() { frame.connectors } else { Vec::new() };
        let (left, right) = if self.nothing.is_some() { (Vec::new(), Vec::new()) } else { (left, right) };
        let nothing = self.nothing.clone();
        let (picker_w, log_h) = (self.picker_w(), self.log_h());
        let comparison = self.set.comparisons.get(self.comparison.current);
        let identical = loaded.is_some_and(|l| l.doc.text_left == l.doc.text_right);
        let notice = comparison.and_then(|comparison|navigator::notice(&comparison.status, identical));
        // a Merge View has notices of its own; the 2-way Comparison underneath isn't shown
        let merging = self.merge_notices();
        let mut status = status_text(&self.comparison.options, self.comparison.view.mode(), line_endings.filter(|_| merging.is_none()));
        if self.comparison.markdown_loading()||self.markdown_building(){status=format!("Preparing Markdown… · {status}");}
        let notes = loaded.map_or(Vec::new(), |l| l.doc.notes.clone());
        // each note goes in front: the merge's in reverse, to read in order
        let notes: Vec<String> = merging.map_or_else(|| notice.into_iter().chain(notes).collect(), |m| m.into_iter().rev().collect());
        for n in notes {
            status = if status.is_empty() { n } else { format!("{n} · {status}") };
        }
        let encodings = loaded.map_or([None, None], |l| l.doc.encodings.clone());
        let banner = self.banner();
        let arrows = loaded.map_or([false, false], |l| l.doc.writable.each_ref().map(|w| w.is_ok()));
        let placeholder = loaded.and_then(|l| l.doc.placeholder.as_ref()).map(|p| match p {
            Placeholder::Binary { sizes, differ } => format!(
                "Binary file — {} → {} — {}",
                kilobytes(sizes[0]),
                kilobytes(sizes[1]),
                if *differ { "bytes differ" } else { "identical bytes" }
            ),
            Placeholder::Large { bytes } => format!("Large file ({}) — press Enter or click here to Compare anyway", megabytes(*bytes)),
            Placeholder::Unreadable(e) => format!("Cannot read this file: {e}"),
        });
        let sidebar = self.layout.files().then(|| (self.sidebar_rows(), self.file_search_header()));
        let message = self.message_paint();
        let tooltip = self.nav_hover.and_then(|row| {
            let NavRow::File { index, from, .. } = self.nav.get(row)? else { return None };
            let renamed = match (from, &self.set.comparisons[*index].status) {
                (Some(from), Status::Renamed { similarity }) => Some(format!("renamed from {from} ({similarity}% similar)")),
                _ => None,
            };
            let edited = self.edited_in_lenz(*index).then(|| "● edited in Lenz".to_string());
            let tip: Vec<String> = renamed.into_iter().chain(edited).collect();
            (!tip.is_empty()).then(|| (row, tip.join(" · ")))
        });
        let switcher = self.switcher.as_ref().map(|sw| {
            let hits: Vec<(String, String, u32)> = sw
                .hits
                .iter()
                .take(14)
                .filter_map(|&i| {
                    let c = self.set.comparisons.get(i)?;
                    let (dir, name) = c.path.rsplit_once('/').unwrap_or(("", c.path.as_str()));
                    Some((format!("{} {name}", navigator::badge(&c.status)), dir.to_string(), status_color(&palette, &c.status)))
                })
                .collect();
            (sw.query.clone(), sw.selected, hits)
        });
        let (sw_w, nav_scroll) = (self.sidebar_w(), self.nav_scroll);
        let labels = self.labels(self.comparison.current);
        let mut position = if self.set.comparisons.len() > 1 {
            format!("   file {}/{}", self.comparison.current + 1, self.set.comparisons.len())
        } else {
            String::new()
        };
        let changes = self.comparison.view.change_count();
        if changes > 0 {
            let at = self.comparison.view.current_change().map_or("–".into(), |j| (j + 1).to_string());
            position += &format!("   change {at}/{changes}");
        }
        let menu=self.menu.as_ref().map(|m| {
            let (x,y,width,rows)=m.layout(self.panes.viewport,char_w);let start=m.start(rows);
            (x,y,width,m.items.iter().skip(start).take(rows).map(|i|i.0.clone()).collect::<Vec<_>>(),m.hover.map(|i|i-start),start, m.items.len())
        });
        let goto = self.goto_prompt();
        let find_control = self.find_control();
        let search_prompt = self.search_prompt();
        let search_x = self.search_prompt().map_or(0.0, |p| { let [vw, _] = self.panes.viewport; self.diff_x() + (vw - self.diff_x() - (p.chars().count() as f32 * self.text_char_w() + 24.0)) / 2.0 });
        let search_marks: Vec<(f32, f32, bool)> = self.search_toggles().into_iter().map(|(o, (x, _, w, _))| (x - search_x, w, self.search.as_ref().is_some_and(|s| s.options().get(o)))).collect();
        let panel = self.search.as_ref().filter(|s| !s.editing).map(|s| {
            let files = s.rows().iter().filter(|r| matches!(r, PanelRow::File(..))).count();
            let state = match s.ended {
                None => "  searching…".to_string(),
                Some(Ended::Capped) => format!("  stopped at {} matches", find::MAX_HITS),
                Some(Ended::Done) => String::new(),
            };
            let only = if s.changed_only { " (changed lines only)" } else { "" };
            let head = format!("Find in Change Set: \"{}\"{only} — {} matches in {files} files{state}   F3/shift-F3 step · Esc closes", s.query, s.hits.len());
            let rows: Vec<(String, u32, bool)> = s
                .rows()
                .iter()
                .skip(s.scroll.floor() as usize)
                .take(self.panel_rows().ceil() as usize)
                .map(|r| match *r {
                    PanelRow::File(index, n) => {
                        let c = &self.set.comparisons[index];
                        (format!("▾ {}  ({n})", c.path), status_color(&palette, &c.status), false)
                    }
                    PanelRow::Hit(i) => {
                        let h = &s.hits[i];
                        let side = if h.side == Side::Left { "L" } else { "R" };
                        (format!("    {side} {:>5}: {}", h.line + 1, h.preview.trim_start()), palette.fg, s.at == Some(i))
                    }
                })
                .collect();
            (head, rows)
         }).or_else(|| {
            let records = self.edits.applied(self.comparison.current);
            (!records.is_empty()).then(|| (
                "Applied changes · click [Undo] to restore one · ctrl-z / ctrl-shift-z undo / redo".into(),
                records.iter().skip(self.applied_scroll.floor() as usize).take(self.panel_rows().ceil() as usize).map(|r| {
                    let action = self.comparison.applied_undo_disabled(r).map_or("[Undo]".into(), |why| format!("({why})"));
                    (format!("{}  {action}", r.label()), palette.accent, false)
                }).collect::<Vec<_>>()
            ))
        });
        let toast = self.toast.as_ref().filter(|(_, at)| at.elapsed() < TOAST_FOR).map(|(m, _)| m.clone());
        let title = self.title();
        if title != self.shown_title {
            window.set_window_title(&title);
            self.shown_title = title;
        }
        let loading = loaded.is_none()&&nothing.is_none();
        let font = self.font.clone();
        let reported = self.first_paint_reported.clone();
        let (started, timings) = (self.started, self.timings);

        let resize_handles=self.resize_handles();
        let help_overlay=self.help.then(||{
            let viewport=window.viewport_size();let rows=help_rows(viewport,char_w);
            let (_,visible)=help_layout(viewport);
            self.help_scroll=self.help_scroll.min(rows.len().saturating_sub(visible));
            let scroll=self.help_scroll;let palette=palette.clone();let font=font.clone();
            div().id("help-dialog").debug_selector(||"help-dialog".into()).absolute().size_full()
                .on_mouse_down(MouseButton::Left,cx.listener(|this,ev,window,cx|{this.on_mouse_down(ev,window,cx);cx.stop_propagation();}))
                .on_mouse_down(MouseButton::Right,|_,_,cx|cx.stop_propagation())
                .child(canvas(|_,_,_|{},move|bounds,_,window,cx|{
                    let ((x,y,w,h),visible)=help_layout(bounds.size);
                    let clip=Bounds{origin:point(px(x),px(y)),size:size(px(w),px(h))};
                    fill(window,x,y,w,h,palette.border);
                    fill(window,x+1.0,y+1.0,(w-2.0).max(0.0),(h-2.0).max(0.0),palette.status_bg);
                    window.with_content_mask(Some(ContentMask{bounds:clip}),|window|{
                        text(window,cx,&font,x+12.0,y+8.0,"Key bindings",palette.accent);
                        let content=Bounds{origin:point(px(x+12.0),px(y+36.0)),size:size(px((w-24.0).max(0.0)),px(visible as f32*line_h()))};
                        window.with_content_mask(Some(ContentMask{bounds:content}),|window|{
                            for (i,(line,keys)) in rows.iter().skip(scroll).take(visible).enumerate(){text(window,cx,&font,x+12.0,y+36.0+i as f32*line_h(),line,if *keys{palette.accent}else{palette.fg});}
                        });
                        let close=help_close_bounds(bounds.size);let cy=f32::from(close.origin.y);
                        fill(window,x+8.0,cy,f32::from(close.size.width),f32::from(close.size.height),palette.selected_bg);
                        text(window,cx,&font,x+12.0,cy,"Close · Esc / Enter",palette.accent);
                        text(window,cx,&font,x+12.0,cy-line_h(),"↑/↓ PgUp/PgDn Home/End · wheel",palette.dim);
                    });
                }).size_full())
        });
        let toolbar = self.toolbar(cx);
        let footer = self.footer();
        if let Some(about)=&mut self.about{about.scroll=about.scroll.min(about::About::max_scroll(window.viewport_size(),self.char_w));}
        let about_palette=palette.clone();
        let discard_overlay=self.discard_overlay(cx);
        let checkout_overlay=self.checkout_overlay(cx);
        let theme_overlay=self.theme_overlay(window,cx);
        let pr_details=self.pr_details_frame(window,cx);
        let summary_frame=self.summary_frame();
        // the summary covers the comparison behind it, with its loading and placeholder lines
        let (placeholder,loading)=if summary_frame.is_some(){(None,false)}else{(placeholder,loading)};
        let about_overlay=self.about.clone().map(|about|{let font=self.font.clone();div().absolute().size_full()
            .on_mouse_down(MouseButton::Left,cx.listener(|this,ev,window,cx|{this.on_mouse_down(ev,window,cx);cx.stop_propagation();}))
            .on_mouse_down(MouseButton::Right,|_,_,cx|cx.stop_propagation())
            .child(canvas(|_,_,_|{},move |bounds,_,window,cx|about.paint(&about_palette, bounds,&font,char_w,window,cx)).size_full())});
        div().relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(palette.bg))
            .track_focus(&self.focus)
            // capture phase: before any handler can stop propagation
            .capture_key_down(|_, window, _| present::soon(window))
            .capture_any_mouse_down(|_, window, _| present::soon(window))
            .on_mouse_down(
                MouseButton::Navigate(NavigationDirection::Back),
                cx.listener(|this, _, window, cx| this.tool(Tool::Back, 0.0, window, cx)),
            )
            .on_mouse_down(
                MouseButton::Navigate(NavigationDirection::Forward),
                cx.listener(|this, _, window, cx| this.tool(Tool::Forward, 0.0, window, cx)),
            )
            .child(toolbar)
            .on_key_down(cx.listener(Self::on_key))
            .on_scroll_wheel(cx.listener(Self::on_wheel))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::on_right_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, cx| {
                        // the Change Set Picker takes the left column and the top band; the rest is as before
                        let (ox, oy, ow, oh) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y), f32::from(bounds.size.width), f32::from(bounds.size.height));
                        if let Some(f) = &log_frame {
                            picker::paint_log(&palette, window, cx, &font, ox + picker_w, oy, ow - picker_w, f);
                        }
                        if let Some(f) = &sidebar_frame {
                            picker::paint_picker_sidebar(&palette, window, cx, &font, ox, oy, oh, picker_w, f);
                        }
                        let bounds = Bounds { origin: point(px(ox + picker_w), px(oy + log_h)), size: size(px((ow - picker_w).max(0.0)), px((oh - log_h - message.1).max(0.0))) };
                        if message.1 > 0.0 {
                            let (mx, my, mw) = (ox + picker_w, oy + oh - message.1, (ow - picker_w).max(0.0));
                            let panel = Bounds { origin: point(px(mx), px(my)), size: size(px(mw), px(message.1)) };
                            fill(window, mx, my, mw, message.1, palette.sidebar_bg);
                            fill(window, mx, my, mw, 1.0, palette.border);
                            window.with_content_mask(Some(ContentMask { bounds: panel }), |window| {
                                for (i, row) in message.0.iter().enumerate() {
                                    let y = my + 1.0 + i as f32 * line_h();
                                    if row.current { fill(window, mx, y, mw, line_h(), palette.selected_bg); }
                                    if let Some((at, w)) = row.span { fill(window, mx + 8.0 + at, y, w, line_h(), palette.sel_bg); }
                                    text(window, cx, &font, mx + 8.0, y, &row.text, row.color);
                                }
                            });
                        }
                        let full_w = f32::from(bounds.size.width);
                        let h = f32::from(bounds.size.height);
                        let (left_edge, top) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
                        if let Some(rows) = &sidebar {
                            paint_sidebar(&palette, window, cx, &font, bounds, sw_w, rows, nav_scroll, tooltip.as_ref());
                        }
                        // the diff area sits right of the File Navigator
                        let x0 = left_edge + sw_w;
                        let w = (full_w - sw_w).max(0.0);
                        let bounds = Bounds { origin: point(px(x0), bounds.origin.y), size: size(px(w.max(0.0)), bounds.size.height) };
                        let pane_w = widths[0];
                        // header: status, both Sides' paths, position in the Change Set
                        fill(window, x0, top, w, hdr_h(), palette.header_bg);
                        let tag = |e: &Option<String>| e.as_ref().map_or(String::new(), |e| format!("  [{e}]"));
                        let header_left = format!("{} {}{}{}", labels.status, labels.left, tag(&encodings[0]), position);
                        // a PR tab covers the diff: its file labels go too, the tabs stay
                        // the labels keep clear of the header switch: the left one ends before it, the right one starts
                        // after the tabs that reach into the right pane and ends before the next cell
                        let left_w = header_switch.first().map_or(pane_w, |c| pane_w.min(c.0 - x0 - 8.0)).max(0.0);
                        if pr_details.is_none() { window.with_content_mask(Some(clip(bounds, x0, left_w)), |window| {
                            text(window, cx, &font, x0 + 8.0, top + 2.0, &header_left, palette.fg)
                        }); }
                        let rx_h = x0 + pane_w + GUTTER_W;
                        let rx = header_switch.iter().filter(|c| c.0 < rx_h + 8.0).map(|c| c.0 + c.1).fold(rx_h, f32::max);
                        let next = header_switch.iter().map(|c| c.0).filter(|cx_| *cx_ > rx).fold(rx_h + widths[1], f32::min);
                        let right_w = (next - rx - 8.0).clamp(0.0, widths[1]);
                        if pr_details.is_none() { window.with_content_mask(Some(clip(bounds, rx, right_w)), |window| {
                            text(window, cx, &font, rx + 8.0, top + 2.0, &format!("{}{}", labels.right, tag(&encodings[1])), palette.fg)
                        }); }
                        for (sx, sw, label, on, _) in &header_switch {
                            fill(window, *sx, top + 2.0, *sw, hdr_h() - 4.0, if *on { palette.selected_bg } else { palette.header_bg });
                            text(window, cx, &font, sx + char_w, top + 2.0, label, if *on { palette.fg } else { palette.dim });
                        }
                        let y0 = top + hdr_h();
                        let bounds = Bounds { origin: point(px(x0), px(y0)), size: size(bounds.size.width, px((h - hdr_h() - panel_h).max(0.0))) };
                        let h = (h - hdr_h() - panel_h).max(0.0);
                        if let Some(n) = &nothing {
                            text(window, cx, &font, x0 + 24.0, y0 + 24.0, n, palette.dim);
                        }
                        if loading {
                            text(window, cx, &font, x0 + 16.0, y0 + 8.0, "Loading…", palette.num_fg);
                        }
                        let gx = x0 + pane_w;
                        let rx = gx + GUTTER_W;
                        if let Some(f) = &merge_frame {
                            merge_view::paint(&palette, window, cx, &font, x0, top, w, h + hdr_h(), f);
                        } else if let Some(f) = &image_frame {
                            image_view::paint_images(&palette, window, cx, &font, (x0, y0, w, (h-version_h).max(0.0)), f);
                        } else if let Some(frame)=&markdown_frame {
                            frame.paint(&palette,bounds,widths,&font,window,cx);
                            // Inline has one pane: its margin, not a gutter between panes
                            fill(window,gx,y0,GUTTER_W,h,if frame.inline(){palette.bg}else{palette.gutter_bg});
                        } else if unified {
                            // one column: deleted rows (left) and the rest (right) interleave by row
                            paint_side(&palette, window, cx, &font, bounds, rx, widths[1], hscroll[1], ann_w[1], &left);
                            paint_side(&palette, window, cx, &font, bounds, rx, widths[1], hscroll[1], ann_w[1], &right);
                            paint_sticky(&palette, window, cx, &font, bounds, rx, widths[1], hscroll[1], ann_w[1], &sticky[1]);
                            fill(window, gx, y0, GUTTER_W, h, palette.bg);
                        } else {
                            paint_side(&palette, window, cx, &font, bounds, x0, pane_w, hscroll[0], ann_w[0], &left);
                            paint_side(&palette, window, cx, &font, bounds, rx, widths[1], hscroll[1], ann_w[1], &right);
                            paint_sticky(&palette, window, cx, &font, bounds, x0, pane_w, hscroll[0], ann_w[0], &sticky[0]);
                            paint_sticky(&palette, window, cx, &font, bounds, rx, widths[1], hscroll[1], ann_w[1], &sticky[1]);
                            fill(window, gx, y0, GUTTER_W, h, palette.gutter_bg);
                            paint_connectors(&palette, window, bounds, x0, widths, &connectors, arrows);
                        }
                        if let Some(f) = &picker_v_frame {
                            version_picker::paint(&palette, window, cx, &font, x0, y0 + h - f.height(), w, f);
                        }
                        if !status.is_empty() {
                            let sw = status.chars().count() as f32 * 7.8 + 16.0;
                            let (sx, sy) = (x0 + w - sw - 8.0, y0 + h - line_h() - 8.0 - picker_v_frame.as_ref().map_or(0.0, |f| f.height()));
                            fill(window, sx, sy, sw, line_h(), palette.status_bg);
                            text(window, cx, &font, sx + 8.0, sy, &status, palette.fg);
                        }
                        // a details tab covers the diff, under toasts, menus, the palette and popups
                        if let Some((page, scroll, sel, rect, char_w, reading)) = &pr_details {
                            pr_details::Dialog { page: page.clone(), scroll: *scroll, sel: *sel, reading: reading.clone(), ..Default::default() }.paint(&palette, *rect, &font, *char_w, window, cx);
                        }
                        if let Some(summary) = &summary_frame { summary.paint(&palette, &font, char_w, window, cx); }
                        if let Some(message) = &toast {
                            let tw = message.chars().count() as f32 * 7.8 + 24.0;
                            let (tx, ty) = (x0 + (w - tw) / 2.0, y0 + h - line_h() * 3.0);
                            fill(window, tx - 1.0, ty - 1.0, tw + 2.0, line_h() + 10.0, palette.border);
                            fill(window, tx, ty, tw, line_h() + 8.0, palette.status_bg);
                            text(window, cx, &font, tx + 12.0, ty + 4.0, message, palette.fg);
                        }
                        if let Some(message) = &placeholder {
                            text(window, cx, &font, x0 + 24.0, y0 + 40.0, message, palette.fg);
                        }
                        if let Some(message) = &banner {
                            fill(window, x0, y0, w, line_h() + 8.0, palette.banner_bg);
                            text(window, cx, &font, x0 + 12.0, y0 + 4.0, message, palette.fg);
                        }
                        if let Some(control) = &find_control {
                            control.paint(window, cx, metrics(&font), &palette, point(px(ox), px(oy - toolbar_h())), char_w);
                        }
                        if let Some((head, rows)) = &panel {
                            let py = y0 + h;
                            let full = Bounds { origin: point(px(left_edge), px(py)), size: size(px(full_w), px(panel_h)) };
                            fill(window, left_edge, py, full_w, panel_h, palette.sidebar_bg);
                            fill(window, left_edge, py, full_w, line_h(), palette.header_bg);
                            window.with_content_mask(Some(ContentMask { bounds: full }), |window| {
                                text(window, cx, &font, left_edge + 8.0, py, head, palette.dim);
                                for (k, (line, color, selected)) in rows.iter().enumerate() {
                                    let ry = py + (k as f32 + 1.0) * line_h();
                                    if *selected {
                                        fill(window, left_edge, ry, full_w, line_h(), palette.selected_bg);
                                    }
                                    text(window, cx, &font, left_edge + 8.0, ry, line, *color);
                                }
                            });
                        }
                        if let Some(prompt) = &search_prompt {
                            let bw = prompt.chars().count() as f32 * char_w + 24.0;
                            let (bx, by) = (x0 + (w - bw) / 2.0, top + 60.0);
                            fill(window, bx - 1.0, by - 1.0, bw + 2.0, line_h() + 14.0, palette.border);
                            fill(window, bx, by, bw, line_h() + 12.0, palette.status_bg);
                            for (dx, mw, on) in &search_marks {
                                fill(window, bx + dx, by + 2.0, *mw, line_h() + 8.0, palette.border);
                                fill(window, bx + dx + 1.0, by + 3.0, mw - 2.0, line_h() + 6.0, if *on { palette.selected_bg } else { palette.status_bg });
                            }
                            text(window, cx, &font, bx + 12.0, by + 6.0, prompt, palette.fg);
                        }
                        if let Some(prompt) = &goto {
                            let bw = prompt.chars().count() as f32 * char_w + 24.0;
                            let (bx, by) = (x0 + (w - bw) / 2.0, top + 60.0);
                            fill(window, bx - 1.0, by - 1.0, bw + 2.0, line_h() + 14.0, palette.border);
                            fill(window, bx, by, bw, line_h() + 12.0, palette.status_bg);
                            text(window, cx, &font, bx + 12.0, by + 6.0, prompt, palette.fg);
                        }
                        if let Some((mx, my, mw, items, hover, start, total)) = &menu {
                            let mh = items.len() as f32 * line_h() + 8.0;
                            paint_raised(window, *mx, *my, *mw, mh, &palette);
                            for (i, label) in items.iter().enumerate() {
                                let iy = my + 4.0 + i as f32 * line_h();
                                if *hover == Some(i) {
                                    fill(window, *mx, iy, *mw, line_h(), palette.selected_bg);
                                }
                                let bounds=Bounds{origin:point(px(*mx),px(iy)),size:size(px(*mw),px(line_h()))};
                                window.with_content_mask(Some(ContentMask{bounds}),|window|text(window,cx,&font,mx+12.,iy,label,palette.fg));
                            }
                            if *start>0 {text(window,cx,&font,mx+2.,my+4.,"↑",palette.dim);}
                            if start+items.len()<*total {text(window,cx,&font,mx+2.,my+4.+items.len().saturating_sub(1) as f32*line_h(),"↓",palette.dim);}
                        }
                        if let Some(d) = &diff_options {
                            paint_raised(window, d.x, d.y, d.width, d.height, &palette);
                            for (r, (label, choices)) in d.rows.iter().enumerate() {
                                let ry = d.y + 4.0 + r as f32 * line_h();
                                text(window, cx, &font, d.x + 12.0, ry, label, palette.dim);
                                for (choice, cx_, current, focused) in choices {
                                    let cw = (choice.chars().count() + 2) as f32 * char_w;
                                    if *current { fill(window, *cx_, ry, cw, line_h(), palette.selected_bg); }
                                    if *focused { fill(window, *cx_, ry + line_h() - 2.0, cw, 2.0, palette.accent); }
                                    text(window, cx, &font, cx_ + char_w, ry, choice, palette.fg);
                                }
                            }
                        }
                        if let Some(cp) = &command_palette {
                            let mh = (cp.lines.len().max(1) + 1) as f32 * line_h() + 8.0;
                            paint_raised(window, cp.x, cp.y, cp.width, mh, &palette);
                            let bounds=Bounds{origin:point(px(cp.x),px(cp.y)),size:size(px(cp.width),px(mh))};
                            window.with_content_mask(Some(ContentMask{bounds}),|window| {
                                text(window, cx, &font, cp.x + 12.0, cp.y + 4.0, &cp.query, palette.accent);
                                if cp.empty {text(window, cx, &font, cp.x + 12.0, cp.y + 4.0 + line_h(), "  No matching commands", palette.dim);}
                                for (i, (label, right, disabled, selected)) in cp.lines.iter().enumerate() {
                                    let iy = cp.y + 4.0 + (i + 1) as f32 * line_h();
                                    if *selected { fill(window, cp.x, iy, cp.width, line_h(), palette.selected_bg); }
                                    let rw = right.chars().count() as f32 * char_w;
                                    text(window, cx, &font, cp.x + cp.width - rw - 12.0, iy, right, palette.dim);
                                    let lb=Bounds{origin:point(px(cp.x),px(iy)),size:size(px((cp.width-rw-24.0).max(0.0)),px(line_h()))};
                                    window.with_content_mask(Some(ContentMask{bounds:lb}),|window|text(window, cx, &font, cp.x + 12.0, iy, label, if *disabled {palette.dim} else {palette.fg}));
                                }
                            });
                        }
                        if let Some((rows,scroll))=&popup {
                            let ((bx,by,bw,bh),buttons,visible)=popup_layout(rows.len(),popup_viewport,char_w);
                            let bounds=Bounds{origin:point(px(bx),px(by)),size:size(px(bw),px(bh))};
                            fill(window,bx-1.,by-1.,bw+2.,bh+2.,palette.border);
                            fill(window,bx,by,bw,bh,palette.status_bg);
                            window.with_content_mask(Some(ContentMask{bounds}),|window| {
                                for (i,(line,color)) in rows.iter().skip(*scroll).take(visible).enumerate(){text(window,cx,&font,bx+12.,by+8.+i as f32*line_h(),line,*color);}
                                for ((key,label),(x,y,w,h)) in POPUP_BUTTONS.iter().zip(buttons){
                                    fill(window,x,y,w,h,palette.selected_bg);text(window,cx,&font,x+8.,y,&format!("[{key}] {label}"),palette.fg);
                                }
                            });
                        }
                        if let Some((query, selected, hits)) = &switcher {
                            paint_switcher(&palette, window, cx, &font, left_edge, top, full_w, query, *selected, hits);
                        }
                        if !reported.replace(true) && timings {
                            eprintln!("lenz: first paint {:.0} ms", started.elapsed().as_secs_f64() * 1000.0);
                        }
                    },
                )
                .w_full().flex_1(),
            ).child(footer).children(resize_handles).children(help_overlay).children(theme_overlay).children(about_overlay).children(discard_overlay).children(checkout_overlay)
    }
}

/// A popup, menu or palette box: soft shadow, border, and a background lifted off what is under it.
fn paint_raised(window: &mut Window, x: f32, y: f32, w: f32, h: f32, palette: &theme::Palette) {
    let (bg, border) = palette.raised();
    for (spread, alpha) in [(6.0, 0.10), (3.0, 0.16)] {
        fill_hsla(window, x - spread + 2.0, y - spread + 4.0, w + 2.0 * spread, h + 2.0 * spread, gpui::hsla(0.0, 0.0, 0.0, alpha));
    }
    fill(window, x - 1.0, y - 1.0, w + 2.0, h + 2.0, border);
    fill(window, x, y, w, h, bg);
}

fn fill_hsla(window: &mut Window, x: f32, y: f32, w: f32, h: f32, color: Hsla) {
    window.paint_quad(quad(
        Bounds { origin: point(px(x), px(y)), size: size(px(w.max(0.0)), px(h.max(0.0))) },
        px(0.),
        color,
        px(0.),
        gpui::transparent_black(),
        Default::default(),
    ));
}

/// A pane's sticky lines over its top rows: the header background, the lines, and a border under them.
#[allow(clippy::too_many_arguments)]
fn paint_sticky(palette: &theme::Palette, window: &mut Window, cx: &mut App, font: &Font, bounds: Bounds<Pixels>, x: f32, w: f32, hscroll: f32, ann_w: f32, rows: &[Painted]) {
    let Some(n) = rows.iter().map(|r| r.y + 1.0).reduce(f32::max) else { return };
    let y0 = f32::from(bounds.origin.y);
    let area = Bounds { origin: bounds.origin, size: size(bounds.size.width, px(n * line_h())) };
    fill(window, x, y0, w, n * line_h(), palette.header_bg);
    paint_side(palette, window, cx, font, area, x, w, hscroll, ann_w, rows);
    fill(window, x, y0 + n * line_h() - 1.0, w, 1.0, palette.border);
}

#[allow(clippy::too_many_arguments)]
fn paint_side(palette: &theme::Palette,
    window: &mut Window,
    cx: &mut App,
    font: &Font,
    bounds: Bounds<Pixels>,
    x: f32,
    w: f32,
    hscroll: f32,
    ann_w: f32,
    rows: &[Painted],
) {
    let y0 = f32::from(bounds.origin.y);
    window.with_content_mask(Some(clip(bounds, x, w)), |window| {
        if ann_w > 0.0 {
            fill(window, x, y0, ann_w, f32::from(bounds.size.height), palette.gutter_bg);
        }
        for row in rows {
            let y = y0 + row.y * line_h();
            if let Some((label, shade)) = &row.ann {
                fill(window, x, y, ann_w - 2.0, line_h(), *shade);
                if let Some(label) = label {
                    window.with_content_mask(Some(clip(bounds, x, ann_w - 6.0)), |window| {
                        text(window, cx, font, x + 6.0, y, label, if *shade == palette.gutter_bg { palette.dim } else { palette.fg })
                    });
                }
            }
        }
        let (x, w) = (x + ann_w, w - ann_w);
        for row in rows {
            let y = y0 + row.y * line_h();
            match (row.tint, row.line.as_ref().map(|l| l.number)) {
                (Some((kind, ignored)), Some(_)) => fill(window, x, y, w, line_h(), shade(palette, kind, ignored, tint)),
                // Filler Rows: a lighter wash of the block's tint
                (Some((kind, ignored)), None) => {
                    fill_hsla(window, x, y, w, line_h(), rgba((shade(palette, kind, ignored, tint) << 8) | 0x60).into())
                }
                _ => {}
            }
            if let Some(line) = &row.line {
                lenz_gpui::paint_row(window, cx, metrics(font), palette, bounds, x, w, hscroll, line);
                if row.thread {
                    // a PR review thread: click the number to read it
                    window.paint_quad(quad(
                        Bounds { origin: point(px(x + 2.0), px(y + 5.0)), size: size(px(10.0), px(10.0)) },
                        px(5.0),
                        Hsla::from(rgb(palette.accent)),
                        px(0.),
                        gpui::transparent_black(),
                        Default::default(),
                    ));
                }
            } else if let (Some((kind, ignored)), false) = (row.tint, row.label.is_empty()) {
                // Folded Change Block: one row in the block's tint
                fill(window, x, y, w, line_h(), shade(palette, kind, ignored, tint));
                text(window, cx, font, x + num_w() + 8.0, y, &row.label, palette.fg);
            } else if !row.label.is_empty() {
                // Collapsed Region: a quiet band with its line count
                fill(window, x, y + 2.0, w, line_h() - 4.0, palette.fold_bg);
                text(window, cx, font, x + num_w() + 8.0, y, &row.label, palette.dim);
            }
        }
    });
}

/// Width of the gutter strip at each edge where a move arrow sits.
const ARROW_W: f32 = 22.0;

/// The rows the move arrows sit on: ≫ (by the gutter's left edge) level with the left block's first line,
/// ≪ (by the right edge) with the right block's; on the boundary where that Side is empty.
fn arrow_rows(c: &Connector) -> (f32, f32) {
    let first = |(top, bottom): (f32, f32)| if bottom > top { top + 0.5 } else { top };
    (first(c.left), first(c.right))
}

/// Filled bezier bands across the gutter, plus marker lines where a Side is empty. `arrows`: whether the
/// left / right Side can take a move (≪ / ≫ are drawn).
fn paint_connectors(palette: &theme::Palette, window: &mut Window, bounds: Bounds<Pixels>, x0: f32, widths: [f32; 2], connectors: &[Connector], arrows: [bool; 2]) {
    let y0 = f32::from(bounds.origin.y);
    let pane_w=widths[0];
    let (gx, gx1) = (x0 + pane_w, x0 + pane_w + GUTTER_W);
    let (mid0, mid1) = (gx + GUTTER_W * 0.45, gx + GUTTER_W * 0.55);
    let y = |rows: f32| y0 + rows * line_h();
    window.with_content_mask(Some(clip(bounds, gx, GUTTER_W)), |window| {
        for c in connectors {
            let (lt, rt) = (y(c.left.0), y(c.right.0));
            // keep empty Sides visible as a 1px edge
            let lb = y(c.left.1).max(lt + 1.0);
            let rb = y(c.right.1).max(rt + 1.0);
            let mut band = PathBuilder::fill();
            band.move_to(point(px(gx), px(lt)));
            band.cubic_bezier_to(point(px(gx1), px(rt)), point(px(mid0), px(lt)), point(px(mid1), px(rt)));
            band.line_to(point(px(gx1), px(rb)));
            band.cubic_bezier_to(point(px(gx), px(lb)), point(px(mid1), px(rb)), point(px(mid0), px(lb)));
            band.close();
            if let Ok(path) = band.build() {
                window.paint_path(path, Hsla::from(rgb(shade(palette, c.kind, c.ignored, tint))));
            }
            // fold chevron: ▾ unfolded, ▸ folded
            if c.foldable {
                let (cx_, cy) = (gx + GUTTER_W / 2.0, lt.min(rt) + line_h() / 2.0);
                let mut chevron = PathBuilder::fill();
                if c.folded {
                    chevron.move_to(point(px(cx_ - 3.0), px(cy - 5.0)));
                    chevron.line_to(point(px(cx_ + 4.0), px(cy)));
                    chevron.line_to(point(px(cx_ - 3.0), px(cy + 5.0)));
                } else {
                    chevron.move_to(point(px(cx_ - 5.0), px(cy - 3.0)));
                    chevron.line_to(point(px(cx_ + 5.0), px(cy - 3.0)));
                    chevron.line_to(point(px(cx_), px(cy + 4.0)));
                }
                chevron.close();
                if let Ok(path) = chevron.build() {
                    window.paint_path(path, Hsla::from(rgb(shade(palette, c.kind, c.ignored, edge))));
                }
            }
            // ≫ copies the left version over the right, ≪ the right over the left
            let (to_right, to_left) = arrow_rows(c);
            for (on, row, ax, dir) in [(arrows[1], to_right, gx + ARROW_W / 2.0, 1.0), (arrows[0], to_left, gx1 - ARROW_W / 2.0, -1.0)] {
                if !on {
                    continue;
                }
                for base in [ax - dir * 5.0, ax] {
                    let mut arrow = PathBuilder::stroke(px(1.5));
                    arrow.move_to(point(px(base), px(y(row) - 4.0)));
                    arrow.line_to(point(px(base + dir * 4.0), px(y(row))));
                    arrow.line_to(point(px(base), px(y(row) + 4.0)));
                    if let Ok(path) = arrow.build() {
                        window.paint_path(path, Hsla::from(rgb(shade(palette, c.kind, c.ignored, edge))));
                    }
                }
            }
            for (a, b) in [(lt, rt), (lb, rb)] {
                let mut line = PathBuilder::stroke(px(1.));
                line.move_to(point(px(gx), px(a)));
                line.cubic_bezier_to(point(px(gx1), px(b)), point(px(mid0), px(a)), point(px(mid1), px(b)));
                if let Ok(path) = line.build() {
                    window.paint_path(path, Hsla::from(rgb(shade(palette, c.kind, c.ignored, edge))));
                }
            }
        }
    });
    // empty Side markers across the pane, where content was added or removed
    for c in connectors {
        let color = shade(palette, c.kind, c.ignored, edge);
        if c.left.0 == c.left.1 {
            fill(window, x0, y(c.left.0) - 1.0, pane_w, 2.0, color);
        }
        if c.right.0 == c.right.1 {
            fill(window, gx1, y(c.right.0) - 1.0, widths[1], 2.0, color);
        }
    }
}

/// Text with syntax colours (foreground) and Inline Change backgrounds.
#[allow(clippy::too_many_arguments)]
fn colored_text(palette: &theme::Palette,
    window: &mut Window,
    cx: &mut App,
    font: &Font,
    x: f32,
    y: f32,
    s: &str,
    fg: &[(Range<usize>, u32)],
    bg: &[(Range<usize>, u32)],
) -> Option<ShapedLine> {
    lenz_gpui::colored_text(window, cx, metrics(font), palette.fg, x, y, s, fg, bg)
}

fn text(window: &mut Window, cx: &mut App, font: &Font, x: f32, y: f32, s: &str, color: u32) {
    lenz_gpui::text(window, cx, metrics(font), x, y, s, color)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commit_popup_buttons_and_scroll_content_fit_narrow_windows() {
        for viewport in [[1500.,850.],[900.,650.],[640.,480.],[400.,240.]] {
            let ((x,y,width,height),buttons,visible)=popup_layout(100,viewport,7.8);
            assert!(x>=0.&&y>=0.&&x+width<=viewport[0]&&y+height<=viewport[1]);
            assert!(visible<100);assert_eq!(buttons.len(),POPUP_BUTTONS.len());
            for &(bx,by,bw,bh) in &buttons {
                assert!(bx>=x&&by>=y&&bx+bw<=x+width&&by+bh<=y+height);
            }
            for pair in buttons.windows(2) {
                assert!(pair[0].0+pair[0].2<=pair[1].0||pair[0].1+pair[0].3<=pair[1].1);
            }
        }
    }

    #[test]
    fn command_menu_hits_include_long_disabled_reasons() {
        let label = "Fold/unfold current change (no multiline change selected)";
        let menu = Menu { x: 20.0, y: 40.0, items: vec![(label.into(), MenuAction::Command(Command::ToggleFold))], hover: None };
        let label_end = menu.x + 12.0 + label.len() as f32 * 7.8;
        assert_eq!(menu.item_at(label_end, menu.y + 10.0, 7.8,[1800.,900.]), Some(0));
        assert_eq!(menu.item_at(label_end + 20.0, menu.y + 10.0, 7.8,[1800.,900.]), None);
    }

    #[test]
    fn a_crlf_move_can_be_undone_after_the_comparison_is_reloaded() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("crlf.txt");
        let original = b"\xEF\xBB\xBFold\r\nsame\r\n";
        std::fs::write(&path, original).unwrap();
        let (snapshot, before) = Snapshot::read(&path, None).unwrap();
        let doc = Doc::new("new\nsame\n", &before.text, &path, &path);
        let moved = moves::moved_text(&before.text, &doc.text_left, 0..1, 0..1);
        let snapshot = write::write_text(&snapshot, &moved).unwrap();
        let edited = doc.edited(Side::Right, &moved, snapshot, &path, &path);
        let (snapshot, reloaded) = Snapshot::read(&path, None).unwrap();
        assert_eq!(&*edited.text_right, reloaded.text, "the cached text must match the writer's line endings");
        let mut edits = moves::Edits::default();
        edits.record(Side::Right, &before.text, &edited.text_right);
        let (_, undone) = edits.next(false, [&doc.text_left, &reloaded.text]).unwrap();
        write::write_text(&snapshot, &undone).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), original);
    }

    #[test]
    fn reloadable_cache_evicts_old_files_and_diffs_but_pins_the_current_file() {
        let doc = Arc::new(Doc::new("old", "new", Path::new("a.txt"), Path::new("a.txt")));
        let weight = Loaded::new(doc.clone(), 0).bytes();
        let mut loaded: HashMap<_, _> = (0..3).map(|i| (i, Loaded::new(doc.clone(), i as u64))).collect();
        let mut cache = DiffCache::default();
        let obsolete = Arc::downgrade(&loaded[&0].parse_id);
        for i in 0..3 { cache.put(i, Options::default(), doc.view(&[], 40.0), Budget::default()); }
        trim_loaded(&mut loaded, &mut cache, 2, 2 * weight);
        assert!(!loaded.contains_key(&0));
        assert_eq!(obsolete.strong_count(), 0, "eviction cancels queued syntax work");
        assert!(!cache.0.contains_key(&0));
        assert!(loaded.contains_key(&1));
        trim_loaded(&mut loaded, &mut cache, 2, 0);
        assert_eq!(loaded.keys().copied().collect::<Vec<_>>(), [2]);
        assert!(cache.0.contains_key(&2));
    }

    #[test]
    fn folded_syntax_queries_only_visible_runs_and_keeps_multiline_context() {
        let raw = ["/* start", "hidden", "still hidden", "end */", "let n = 42;"]
            .map(str::to_string);
        let parsed = highlight::parse_while(Language::Rust, raw.join("\n").into(), || true).unwrap();
        let row = |line| Row { y: 0.0, content: Content::Line(line), tint: None, inline: Vec::new() };
        let rows = [row(0), row(3), row(4)];
        assert_eq!(visible_runs(&rows), [0..1, 3..5]);
        let painted = resolve(&theme::DARCULA, &rows, &raw, Some(&parsed), None, &[]);
        assert_eq!(painted[1].line.as_ref().unwrap().fg, vec![(0..6, syntax_color(&theme::DARCULA, Capture::Comment))]);
        assert!(painted[2].line.as_ref().unwrap().fg.contains(&(0..3, syntax_color(&theme::DARCULA, Capture::Keyword))));
        // The gap's size cannot make the amount of highlighted text grow.
        assert_eq!(visible_runs(&[row(0), row(99_999)]), [0..1, 99_999..100_000]);
    }

    #[test]
    fn find_highlights_only_copy_matches_on_visible_lines() {
        let hits: Vec<_> = (0..100_000).flat_map(|line| [(line, 0..1), (line, 3..4)]).collect();
        let raw = vec!["abcd".to_string(); 100_000];
        let row = |line| Row { y: 0.0, content: Content::Line(line), tint: None, inline: Vec::new() };
        // A large folded region lies between two visible lines.
        let visible = visible_find_hits(&hits, &[row(1), row(99_999)], &raw);
        assert_eq!(visible.len(), 4, "matches hidden in folds must not be copied on every paint");
        assert_eq!(visible, [(1, 0..1), (1, 3..4), (99_999, 0..1), (99_999, 3..4)]);
        assert!(visible_find_hits(&hits, &[], &raw).is_empty());
        assert!(visible_find_hits(&[], &[row(1)], &raw).is_empty());
    }

    #[test]
    fn changed_words_keep_readable_syntax_colors_over_their_highlight() {
        let raw = vec!["let a = \"changed\";".to_string()];
        let lang = crate::highlight::detect(std::path::Path::new("a.rs"), "").unwrap();
        let parsed = crate::highlight::parse_while(lang, raw[0].as_str().into(), || true).unwrap();
        let word = 9..16;
        let rows = [Row { y: 0.0, content: Content::Line(0), tint: Some((Kind::Modified, false)), inline: vec![word.clone()] }];
        for theme in theme::builtin_entries() {
            let palette=&theme.palette;
            let painted = resolve(palette, &rows, &raw, Some(&parsed), None, &[]);
            let color_at = |at: usize| painted[0].line.as_ref().unwrap().fg.iter().find(|(r, _)| r.contains(&at)).map(|(_, c)| *c);
            let fill = palette.inline[2];
            assert_eq!(color_at(word.start), Some(palette.readable_on(palette.syntax.string, fill)), "the changed word keeps its color where it reads");
            assert_eq!(color_at(0), Some(palette.syntax.keyword), "unchanged code keeps its syntax colors");
            assert!(painted[0].line.as_ref().unwrap().bg.iter().any(|(r, color)| *r == word&&*color==palette.inline[2]), "the word keeps its highlight");
        }
    }

    #[test]
    fn long_line_find_highlights_stop_at_the_displayed_prefix() {
        let raw = vec!["e ".repeat(LONG_LINE * 10)];
        let rows = [Row { y: 0.0, content: Content::Line(0), tint: None, inline: Vec::new() }];
        let found = find::in_lines(&raw, &lenz_content::find::Matcher::new(&lenz_content::find::Query { text: "e".into(), ..Default::default() }).unwrap());
        assert_eq!(visible_find_hits(&found, &rows, &raw).len(), LONG_LINE / 2);
        let painted = resolve(&theme::DARCULA, &rows, &raw, None, None, &found);
        assert_eq!(painted[0].line.as_ref().unwrap().bg.len(), LONG_LINE / 2);
        assert!(painted[0].line.as_ref().unwrap().bg.iter().all(|(r, _)| !r.is_empty() && r.end <= LONG_LINE));
    }

    #[test]
    fn a_revisited_comparison_reuses_its_diff_unless_the_options_changed() {
        let mut cache = DiffCache::default();
        let options = Options::default();
        cache.put(3, options, View::new(10, 10, &[], 20.0), Budget::default());
        assert!(cache.take(3, options).is_some());
        assert!(cache.take(3, options).is_none(), "taken once");
        cache.put(3, options, View::new(10, 10, &[], 20.0), Budget::default());
        let other = Options { ignore_case: true, ..options };
        assert!(cache.take(3, other).is_none(), "diffed under other options");
        assert!(cache.take(3, options).is_none(), "the stale entry is gone");
    }

    #[test]
    fn a_move_is_written_and_re_diffed_keeping_folds_and_the_place() {
        let left: String = (0..60).map(|i| format!("line {i}\n")).collect();
        let right = left.replace("line 10\nline 11\n", "ten\neleven\n").replace("line 40\n", "forty\n");
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.txt");
        std::fs::write(&file, &right).unwrap();
        let (snapshot, _) = Snapshot::read(&file, None).unwrap();
        let mut doc = Doc::new(&left, &right, &file, &file);
        doc.writable[1] = Ok(snapshot);
        let options = Options::default();
        let mut view = doc.view(&doc.line_diff(&options), 20.0);
        view.set_collapsed(true);
        view.toggle_fold(0);
        let folds: Vec<_> = view.folded_blocks().iter().map(|b| doc.block_text(b)).collect();
        view.place_left_at(12.0);
        let anchor = view.anchor_line();
        // ≫ on the second block, then the re-diff as Viewer::write_side and Viewer::diff do it
        let (l, r) = view.block_ranges()[1].clone();
        let text = moves::moved_text(&doc.text_right, &doc.text_left, r, l);
        let Ok(snapshot) = &doc.writable[1] else { unreachable!() };
        let doc = doc.edited(Side::Right, &text, write::write_text(snapshot, &text).unwrap(), &file, &file);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), left.replace("line 10\nline 11\n", "ten\neleven\n"));
        let mut view = doc.view(&doc.line_diff(&options), 20.0);
        view.set_collapsed(true);
        doc.refold(&mut view, &folds);
        view.place_left_at(anchor);
        assert_eq!(view.block_ranges(), vec![(10..12, 10..12)]);
        assert!(view.folded(0), "the fold on the untouched block survives");
        assert_eq!(view.anchor_line(), anchor);
        assert!(doc.writable[1].as_ref().is_ok_and(|s| !s.changed()), "the next move writes through the new snapshot");
    }

    #[test]
    fn chrome_scales_with_the_font_size() {
        assert_eq!((scaled(13.0, 20.0), scaled(13.0, 56.0)), (20.0, 56.0));
        assert_eq!(scaled(26.0, 20.0), 40.0);
        assert_eq!(scaled(8.0, 20.0), 20.0 * 8.0 / 13.0);
    }

}
